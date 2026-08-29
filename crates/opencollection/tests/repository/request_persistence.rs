use super::*;

#[test]
fn creates_a_bundled_workspace_from_a_domain_collection_without_overwriting() {
    let path = temporary_path("domain-import.yml");
    let collection = Collection {
        metadata: CollectionMetadata {
            name: Some("Imported Pets".to_owned()),
            ..CollectionMetadata::default()
        },
        items: vec![CollectionItem::HttpRequest(HttpRequest {
            metadata: ItemMetadata {
                name: Some("List pets".to_owned()),
                ..ItemMetadata::default()
            },
            method: Some("GET".to_owned()),
            url: Some("https://example.com/pets".to_owned()),
            ..HttpRequest::default()
        })],
        ..Collection::default()
    };

    let loaded = create_bundled_workspace_from_collection(&path, &collection).unwrap();
    assert_eq!(loaded.workspace().request_count(), 1);
    assert_eq!(
        loaded.workspace().metadata().name.as_deref(),
        Some("Imported Pets")
    );
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("opencollection: 1.0.0"));
    assert!(saved.contains("name: List pets"));

    let error = create_bundled_workspace_from_collection(&path, &collection).unwrap_err();
    assert!(matches!(error, CreateError::AlreadyExists(_)));
    fs::remove_file(path).unwrap();
}

#[test]
fn loads_bundled_file_with_structural_selectors() {
    let loaded = load_workspace(fixture("phase1-bundled.yml"))
        .expect("bundled workspace fixture should load");
    let selectors: Vec<_> = loaded
        .requests()
        .iter()
        .map(|request| request.selector())
        .collect();

    assert_eq!(selectors, ["items/0/items/0", "items/1"]);
    let key = loaded
        .request_key("items/0/items/0")
        .expect("selector should resolve");
    assert_eq!(
        loaded
            .workspace()
            .request(key)
            .and_then(|request| request.metadata.name.as_deref()),
        Some("List pets")
    );
    assert_eq!(loaded.folders()[0].selector(), "items/0");
    assert_eq!(
        loaded.folder_selector(loaded.folders()[0].key()),
        Some("items/0")
    );
}

#[test]
fn loads_unbundled_directory_with_relative_path_selectors() {
    let loaded =
        load_workspace(fixture("unbundled")).expect("unbundled workspace fixture should load");
    let selectors: Vec<_> = loaded
        .requests()
        .iter()
        .map(|request| request.selector())
        .collect();

    assert_eq!(selectors, ["health.yml", "users/list-users.yml"]);
    assert_eq!(loaded.workspace().request_count(), 2);
    assert_eq!(loaded.workspace().folder_count(), 1);
    assert_eq!(loaded.folders()[0].selector(), "users");
    assert_eq!(loaded.folder_key("users"), Some(loaded.folders()[0].key()));
    assert_eq!(loaded.workspace().environments().len(), 1);
    assert_eq!(loaded.workspace().environments()[0].name, "development");
}

#[test]
fn unbundled_loader_preserves_valid_dot_probe_items() {
    let root = temporary_path("dot-probe-items");
    copy_directory(&fixture("phase16-unbundled"), &root);
    let folder = root.join(".probe-visible");
    fs::create_dir(&folder).unwrap();
    fs::write(
        folder.join("folder.yml"),
        "info: { name: Visible folder, type: folder }\n",
    )
    .unwrap();
    fs::write(
        root.join(".probe-visible.yml"),
        "info: { name: Visible request, type: http }\nhttp: { method: GET }\n",
    )
    .unwrap();

    let loaded = load_workspace(&root).unwrap();

    assert_eq!(loaded.workspace().folder_count(), 2);
    assert_eq!(loaded.workspace().request_count(), 3);
    assert!(loaded.folder_key(".probe-visible").is_some());
    assert!(loaded.request_key(".probe-visible.yml").is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundled_selector_uses_source_position_when_unsupported_items_are_skipped() {
    let loaded = load_workspace(fixture("phase3-bundled-selectors.yml"))
        .expect("bundled selector fixture should load");

    assert_eq!(loaded.requests().len(), 1);
    assert_eq!(loaded.requests()[0].selector(), "items/1");
}

#[test]
fn loads_bundled_workspace_from_yaml_source() {
    let source = std::fs::read_to_string(fixture("phase1-bundled.yml")).unwrap();
    let loaded = load_workspace_from_str(&source).expect("source workspace should load");

    let selectors: Vec<_> = loaded
        .requests()
        .iter()
        .map(|request| request.selector())
        .collect();
    assert_eq!(selectors, ["items/0/items/0", "items/1"]);
}

#[test]
fn bundled_update_save_reload_preserves_unknown_fields() {
    let path = temporary_path("bundled.yml");
    fs::copy(fixture("phase1-round-trip.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).expect("workspace should load");

    loaded
        .update_request(
            "items/0",
            &RequestUpdate {
                method: Some("PUT".to_owned()),
                url: Some("https://api.example.com/pets/42".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .expect("request should save");
    loaded
        .update_request(
            "items/0",
            &RequestUpdate {
                name: Some("Replace pet".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .expect("a second update should use the refreshed source snapshot");

    let reloaded = load_workspace(&path).expect("saved workspace should reload");
    let key = reloaded.request_key("items/0").unwrap();
    let request = reloaded.workspace().request(key).unwrap();
    assert_eq!(request.metadata.name.as_deref(), Some("Replace pet"));
    assert_eq!(request.method.as_deref(), Some("PUT"));
    assert_eq!(
        request.url.as_deref(),
        Some("https://api.example.com/pets/42")
    );

    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("vendor.example"));
    assert!(saved.contains("description: Creates a pet"));
    assert!(saved.contains("runtime:"));
    assert!(saved.contains("encodeUrl: true"));
    fs::remove_file(path).unwrap();
}

#[test]
fn desktop_editable_fields_survive_a_prepared_save_and_reload() {
    let path = temporary_path("desktop-fields.yml");
    fs::copy(fixture("phase1-round-trip.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).expect("workspace should load");
    let mut properties = std::collections::BTreeMap::new();
    properties.insert(
        "username".to_owned(),
        AuthenticationValue::String("probe".to_owned()),
    );
    let update = RequestUpdate {
        method: Some("PATCH".to_owned()),
        url: Some("https://api.example.com/pets/42".to_owned()),
        headers: Some(vec![Header {
            name: "X-Probe".to_owned(),
            value: "desktop".to_owned(),
            disabled: true,
        }]),
        query_parameters: Some(vec![QueryParameter {
            name: "preview".to_owned(),
            value: "true".to_owned(),
            disabled: false,
        }]),
        path_parameters: Some(vec![QueryParameter {
            name: "petId".to_owned(),
            value: "42".to_owned(),
            disabled: false,
        }]),
        body: Some(Some(RequestBody::Single(Body::FormUrlEncoded(vec![
            FormField {
                name: "name".to_owned(),
                value: "Milo".to_owned(),
                disabled: false,
            },
        ])))),
        authentication: Some(Some(Authentication {
            kind: AuthenticationKind::Basic,
            properties,
        })),
        ..RequestUpdate::default()
    };

    let prepared = loaded
        .prepare_request_save("items/0", update)
        .expect("save should prepare");
    let completed = prepared.execute().expect("save should execute");
    loaded.complete_request_save(completed);

    let reloaded = load_workspace(&path).expect("saved workspace should reload");
    let request = reloaded
        .workspace()
        .request(reloaded.request_key("items/0").unwrap())
        .unwrap();
    assert_eq!(request.method.as_deref(), Some("PATCH"));
    assert_eq!(request.headers[0].name, "X-Probe");
    assert!(request.headers[0].disabled);
    assert_eq!(request.query_parameters[0].name, "preview");
    assert_eq!(request.path_parameters[0].name, "petId");
    assert_eq!(request.path_parameters[0].value, "42");
    assert!(matches!(
        request.body,
        Some(RequestBody::Single(Body::FormUrlEncoded(_)))
    ));
    assert_eq!(
        request.authentication.as_ref().map(|auth| &auth.kind),
        Some(&AuthenticationKind::Basic)
    );

    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("vendor.example"));
    assert!(saved.contains("description: Payload media type"));
    fs::remove_file(path).unwrap();
}

#[test]
fn every_supported_body_and_authentication_shape_survives_desktop_style_saves() {
    let path = temporary_path("desktop-body-shapes.yml");
    fs::copy(fixture("phase1-bodies-auth-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let snapshots: Vec<_> = loaded
        .requests()
        .iter()
        .map(|located| {
            (
                located.selector().to_owned(),
                loaded.workspace().request(located.key()).unwrap().clone(),
            )
        })
        .collect();

    for (selector, request) in &snapshots {
        let update = RequestUpdate {
            method: request.method.clone(),
            url: request.url.clone(),
            headers: Some(request.headers.clone()),
            query_parameters: Some(request.query_parameters.clone()),
            path_parameters: Some(request.path_parameters.clone()),
            body: Some(request.body.clone()),
            authentication: Some(request.authentication.clone()),
            ..RequestUpdate::default()
        };
        let saved = loaded
            .prepare_request_save(selector, update)
            .unwrap()
            .execute()
            .unwrap();
        loaded.complete_request_save(saved);
    }

    let reloaded = load_workspace(&path).unwrap();
    for (selector, expected) in snapshots {
        let actual = reloaded
            .workspace()
            .request(reloaded.request_key(&selector).unwrap())
            .unwrap();
        assert_eq!(actual, &expected, "request {selector} changed after save");
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn updates_a_nested_bundled_request_by_structural_locator() {
    let path = temporary_path("nested-bundled.yml");
    fs::copy(fixture("phase1-bundled.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).expect("workspace should load");

    loaded
        .update_request(
            "items/0/items/0",
            &RequestUpdate {
                url: Some("https://api.example.com/v2/pets".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .expect("nested request should save");

    let reloaded = load_workspace(&path).expect("saved workspace should reload");
    let key = reloaded.request_key("items/0/items/0").unwrap();
    assert_eq!(
        reloaded
            .workspace()
            .request(key)
            .and_then(|request| request.url.as_deref()),
        Some("https://api.example.com/v2/pets")
    );
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("name: ownerId"));
    assert!(saved.contains("type: path"));
    fs::remove_file(path).unwrap();
}

#[test]
fn unbundled_update_preserves_request_extensions() {
    let root = temporary_path("unbundled");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("opencollection.yml"),
        "opencollection: 1.0.0\ninfo:\n  name: Persistence\nbundled: false\n",
    )
    .unwrap();
    fs::write(
        root.join("health.yml"),
        concat!(
            "info:\n  name: Health\n  type: http\n",
            "http:\n  method: GET\n  url: https://example.com/health\n",
            "extensions:\n  vendor.example:\n    color: blue\n",
        ),
    )
    .unwrap();
    let mut loaded = load_workspace(&root).expect("workspace should load");

    loaded
        .update_request(
            "health.yml",
            &RequestUpdate {
                url: Some("https://example.com/ready".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .expect("request should save");

    let reloaded = load_workspace(&root).expect("saved workspace should reload");
    let key = reloaded.request_key("health.yml").unwrap();
    assert_eq!(
        reloaded
            .workspace()
            .request(key)
            .and_then(|request| request.url.as_deref()),
        Some("https://example.com/ready")
    );
    let saved = fs::read_to_string(root.join("health.yml")).unwrap();
    assert!(saved.contains("vendor.example"));
    assert!(saved.contains("color: blue"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn refuses_to_overwrite_an_externally_modified_document() {
    let path = temporary_path("conflict.yml");
    fs::copy(fixture("phase1-round-trip.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).expect("workspace should load");
    let mut external = fs::read_to_string(&path).unwrap();
    external.push_str("external: true\n");
    fs::write(&path, &external).unwrap();

    let error = loaded
        .update_request(
            "items/0",
            &RequestUpdate {
                url: Some("https://should-not-be-written.example".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .expect_err("external modification should be rejected");

    assert!(matches!(error, SaveError::ConcurrentModification(_)));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    let key = loaded.request_key("items/0").unwrap();
    assert_eq!(
        loaded
            .workspace()
            .request(key)
            .and_then(|request| request.url.as_deref()),
        Some("https://should-not-be-written.example")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn independently_loaded_writers_detect_the_first_save() {
    let path = temporary_path("two-writers.yml");
    fs::copy(fixture("phase1-round-trip.yml"), &path).unwrap();
    let mut first = load_workspace(&path).unwrap();
    let mut second = load_workspace(&path).unwrap();

    first
        .update_request(
            "items/0",
            &RequestUpdate {
                name: Some("First writer".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .unwrap();
    let error = second
        .update_request(
            "items/0",
            &RequestUpdate {
                name: Some("Second writer".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .unwrap_err();

    assert!(matches!(error, SaveError::ConcurrentModification(_)));
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("name: First writer")
    );
    fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn bundled_update_preserves_a_symlink_and_updates_its_target() {
    use std::os::unix::fs::symlink;

    let target = temporary_path("symlink-target.yml");
    let link = temporary_path("symlink.yml");
    fs::copy(fixture("phase1-round-trip.yml"), &target).unwrap();
    symlink(&target, &link).unwrap();
    let mut loaded = load_workspace(&link).unwrap();

    loaded
        .update_request(
            "items/0",
            &RequestUpdate {
                name: Some("Updated through symlink".to_owned()),
                ..RequestUpdate::default()
            },
        )
        .unwrap();

    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("name: Updated through symlink")
    );
    fs::remove_file(link).unwrap();
    fs::remove_file(target).unwrap();
}

#[test]
fn rejects_a_bundled_flag_that_disagrees_with_the_source_kind() {
    let error = load_workspace_from_str(
        "opencollection: 1.0.0\ninfo: { name: Wrong mode }\nbundled: false\n",
    )
    .unwrap_err();

    assert!(matches!(
        error,
        probe_opencollection::LoadError::InvalidMode { .. }
    ));
}
