use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, Collection, CollectionItem,
    CollectionMetadata, EnvironmentResolutionError, FormField, Header, HttpRequest, ItemMetadata,
    QueryParameter, RequestBody, RequestUpdate, resolve_environment,
};
use probe_opencollection::{
    CreateError, SaveError, StructureError, StructureOperation, create_bundled_workspace,
    create_bundled_workspace_from_collection, load_workspace, load_workspace_from_str,
};

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection")
        .join(path)
}

fn temporary_path(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "probe-persistence-{}-{unique}-{suffix}",
        std::process::id()
    ))
}

fn copy_directory(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

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

#[test]
fn bundled_structure_edits_save_reload_and_preserve_unknown_fields() {
    let path = temporary_path("phase16-bundled.yml");
    fs::copy(fixture("phase16-bundled.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    let created = loaded
        .apply_structure(StructureOperation::CreateRequest {
            parent: Some("items/1".to_owned()),
            index: Some(0),
            name: "Created".to_owned(),
            method: Some("PUT".to_owned()),
            url: Some("https://example.com/created".to_owned()),
        })
        .unwrap();
    assert_eq!(created.selector.as_deref(), Some("items/1/items/0"));
    loaded
        .apply_structure(StructureOperation::RenameRequest {
            selector: "items/1/items/0".to_owned(),
            name: "Renamed".to_owned(),
        })
        .unwrap();
    let moved = loaded
        .apply_structure(StructureOperation::MoveRequest {
            selector: "items/1/items/0".to_owned(),
            parent: None,
            index: Some(0),
        })
        .unwrap();
    assert_eq!(moved.selector.as_deref(), Some("items/0"));
    loaded
        .apply_structure(StructureOperation::CreateFolder {
            parent: None,
            index: None,
            name: "Empty".to_owned(),
        })
        .unwrap();
    loaded
        .apply_structure(StructureOperation::DeleteRequest {
            selector: "items/1".to_owned(),
        })
        .unwrap();

    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(reloaded.requests().len(), 2);
    let renamed = reloaded
        .workspace()
        .request(reloaded.request_key("items/0").unwrap())
        .unwrap();
    assert_eq!(renamed.metadata.name.as_deref(), Some("Renamed"));
    assert_eq!(renamed.method.as_deref(), Some("PUT"));
    assert_eq!(reloaded.folders().len(), 2);
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("vendor.example"));
    assert!(saved.contains("x-folder: retained"));
    fs::remove_file(path).unwrap();
}

#[test]
fn bundled_move_handles_reordering_and_destination_index_shifts() {
    let path = temporary_path("phase16-bundled-moves.yml");
    fs::copy(fixture("phase16-bundled.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    let moved = loaded
        .apply_structure(StructureOperation::MoveRequest {
            selector: "items/0".to_owned(),
            parent: Some("items/1".to_owned()),
            index: Some(1),
        })
        .unwrap();
    assert_eq!(moved.selector.as_deref(), Some("items/0/items/1"));
    assert_eq!(
        moved.selector_remaps.get("items/0").map(String::as_str),
        Some("items/0/items/1")
    );
    assert_eq!(
        moved.selector_remaps.get("items/1").map(String::as_str),
        Some("items/0")
    );
    assert_eq!(
        moved
            .selector_remaps
            .get("items/1/items/0")
            .map(String::as_str),
        Some("items/0/items/0")
    );
    let moved_key = loaded.request_key("items/0/items/1").unwrap();
    loaded.request_mut(moved_key).unwrap().url = Some("https://example.com/unsaved".to_owned());
    let reordered = loaded
        .apply_structure(StructureOperation::ReorderRequest {
            selector: "items/0/items/1".to_owned(),
            index: 0,
        })
        .unwrap();
    assert_eq!(reordered.selector.as_deref(), Some("items/0/items/0"));
    assert_eq!(
        reordered
            .selector_remaps
            .get("items/0/items/1")
            .map(String::as_str),
        Some("items/0/items/0")
    );
    assert_eq!(
        reordered
            .selector_remaps
            .get("items/0/items/0")
            .map(String::as_str),
        Some("items/0/items/1")
    );
    assert_eq!(
        loaded
            .workspace()
            .request(loaded.request_key("items/0/items/0").unwrap())
            .unwrap()
            .url
            .as_deref(),
        Some("https://example.com/unsaved")
    );

    let reloaded = load_workspace(&path).unwrap();
    let first = reloaded
        .workspace()
        .request(reloaded.request_key("items/0/items/0").unwrap())
        .unwrap();
    assert_eq!(first.metadata.name.as_deref(), Some("Alpha"));
    fs::remove_file(path).unwrap();
}

#[test]
fn bundled_duplicate_request_copies_request_after_original() {
    let path = temporary_path("phase16-bundled-duplicate.yml");
    fs::copy(fixture("phase16-bundled.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    let duplicated = loaded
        .apply_structure(StructureOperation::DuplicateRequest {
            selector: "items/0".to_owned(),
        })
        .unwrap();

    assert_eq!(duplicated.selector.as_deref(), Some("items/1"));
    assert_eq!(
        duplicated
            .selector_remaps
            .get("items/0")
            .map(String::as_str),
        Some("items/0")
    );
    assert_eq!(
        duplicated
            .selector_remaps
            .get("items/1")
            .map(String::as_str),
        Some("items/2")
    );
    assert_eq!(
        duplicated
            .selector_remaps
            .get("items/1/items/0")
            .map(String::as_str),
        Some("items/2/items/0")
    );

    let reloaded = load_workspace(&path).unwrap();
    let copy = reloaded
        .workspace()
        .request(reloaded.request_key("items/1").unwrap())
        .unwrap();
    assert_eq!(copy.metadata.name.as_deref(), Some("Alpha Copied"));
    assert_eq!(copy.method.as_deref(), Some("GET"));
    assert_eq!(copy.url.as_deref(), Some("https://example.com/alpha"));
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("x-request: retained"));
    fs::remove_file(path).unwrap();
}

#[test]
fn unbundled_structure_edits_persist_paths_order_and_unknown_fields() {
    let root = temporary_path("phase16-unbundled");
    copy_directory(&fixture("phase16-unbundled"), &root);
    let unsupported = fs::read(root.join("group/unsupported.yml")).unwrap();
    let mut loaded = load_workspace(&root).unwrap();

    let created = loaded
        .apply_structure(StructureOperation::CreateRequest {
            parent: Some("group".to_owned()),
            index: Some(0),
            name: "Created Request".to_owned(),
            method: Some("PATCH".to_owned()),
            url: Some("https://example.com/created".to_owned()),
        })
        .unwrap();
    assert_eq!(
        created.selector.as_deref(),
        Some("group/created-request.yml")
    );
    let renamed = loaded
        .apply_structure(StructureOperation::RenameRequest {
            selector: "group/created-request.yml".to_owned(),
            name: "Renamed Request".to_owned(),
        })
        .unwrap();
    assert_eq!(
        renamed.selector.as_deref(),
        Some("group/renamed-request.yml")
    );
    let folder = loaded
        .apply_structure(StructureOperation::CreateFolder {
            parent: None,
            index: Some(0),
            name: "Destination".to_owned(),
        })
        .unwrap();
    assert_eq!(folder.selector.as_deref(), Some("destination"));
    let moved = loaded
        .apply_structure(StructureOperation::MoveRequest {
            selector: "group/renamed-request.yml".to_owned(),
            parent: Some("destination".to_owned()),
            index: Some(0),
        })
        .unwrap();
    assert_eq!(
        moved.selector.as_deref(),
        Some("destination/renamed-request.yml")
    );
    loaded
        .apply_structure(StructureOperation::DeleteRequest {
            selector: "alpha.yml".to_owned(),
        })
        .unwrap();

    let reloaded = load_workspace(&root).unwrap();
    assert_eq!(
        reloaded.requests()[0].selector(),
        "destination/renamed-request.yml"
    );
    assert_eq!(reloaded.folders()[0].selector(), "destination");
    assert!(
        fs::read_to_string(root.join("opencollection.yml"))
            .unwrap()
            .contains("vendor.example")
    );
    assert!(
        fs::read_to_string(root.join("group/folder.yml"))
            .unwrap()
            .contains("x-folder: retained")
    );
    assert_eq!(
        fs::read(root.join("group/unsupported.yml")).unwrap(),
        unsupported
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unbundled_folder_edits_and_explicit_reordering_survive_reload() {
    let root = temporary_path("phase16-unbundled-folders");
    copy_directory(&fixture("phase16-unbundled"), &root);
    let mut loaded = load_workspace(&root).unwrap();

    loaded
        .apply_structure(StructureOperation::CreateFolder {
            parent: None,
            index: None,
            name: "Destination".to_owned(),
        })
        .unwrap();
    let renamed = loaded
        .apply_structure(StructureOperation::RenameFolder {
            selector: "group".to_owned(),
            name: "Renamed Group".to_owned(),
        })
        .unwrap();
    assert_eq!(renamed.selector.as_deref(), Some("renamed-group"));
    let moved = loaded
        .apply_structure(StructureOperation::MoveFolder {
            selector: "renamed-group".to_owned(),
            parent: Some("destination".to_owned()),
            index: Some(0),
        })
        .unwrap();
    assert_eq!(moved.selector.as_deref(), Some("destination/renamed-group"));
    assert!(
        loaded
            .request_key("destination/renamed-group/nested.yml")
            .is_some()
    );
    let reordered = loaded
        .apply_structure(StructureOperation::ReorderFolder {
            selector: "destination".to_owned(),
            index: 0,
        })
        .unwrap();
    assert_eq!(reordered.index, Some(0));
    loaded
        .apply_structure(StructureOperation::DeleteFolder {
            selector: "destination/renamed-group".to_owned(),
        })
        .unwrap();

    let reloaded = load_workspace(&root).unwrap();
    assert_eq!(reloaded.folders()[0].selector(), "destination");
    assert_eq!(reloaded.workspace().folder_count(), 1);
    assert_eq!(reloaded.workspace().request_count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unbundled_duplicate_request_copies_request_after_original() {
    let root = temporary_path("phase16-unbundled-duplicate");
    copy_directory(&fixture("phase16-unbundled"), &root);
    let mut loaded = load_workspace(&root).unwrap();

    let duplicated = loaded
        .apply_structure(StructureOperation::DuplicateRequest {
            selector: "alpha.yml".to_owned(),
        })
        .unwrap();

    assert_eq!(duplicated.selector.as_deref(), Some("alpha-copied.yml"));
    assert_eq!(duplicated.index, Some(1));
    assert_eq!(
        duplicated
            .selector_remaps
            .get("alpha.yml")
            .map(String::as_str),
        Some("alpha.yml")
    );

    let reloaded = load_workspace(&root).unwrap();
    let copy = reloaded
        .workspace()
        .request(reloaded.request_key("alpha-copied.yml").unwrap())
        .unwrap();
    assert_eq!(copy.metadata.name.as_deref(), Some("Alpha Copied"));
    assert_eq!(copy.method.as_deref(), Some("GET"));
    assert_eq!(copy.url.as_deref(), Some("https://example.com/alpha"));
    let saved = fs::read_to_string(root.join("alpha-copied.yml")).unwrap();
    assert!(saved.contains("x-request: retained"));
    assert!(saved.contains("seq: 2"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn structure_edits_reject_duplicates_invalid_destinations_and_conflicts() {
    let root = temporary_path("phase16-errors");
    copy_directory(&fixture("phase16-unbundled"), &root);
    let mut loaded = load_workspace(&root).unwrap();

    for reserved in ["opencollection.yml", "group/unsupported.yml"] {
        let rejected = loaded
            .apply_structure(StructureOperation::DeleteRequest {
                selector: reserved.to_owned(),
            })
            .unwrap_err();
        assert!(matches!(
            rejected,
            StructureError::ItemNotFound {
                kind: probe_opencollection::ItemKind::Request,
                ..
            }
        ));
        assert!(root.join(reserved).exists());
    }
    fs::create_dir(root.join("rogue")).unwrap();
    fs::write(
        root.join("rogue/folder.yml"),
        "info: { name: Rogue, type: folder }\n",
    )
    .unwrap();
    let unsupported_parent = loaded
        .apply_structure(StructureOperation::CreateRequest {
            parent: Some("rogue".to_owned()),
            index: None,
            name: "Unsafe".to_owned(),
            method: None,
            url: None,
        })
        .unwrap_err();
    assert!(matches!(
        unsupported_parent,
        StructureError::DestinationNotFound(_)
    ));
    assert!(!root.join("rogue/unsafe.yml").exists());
    fs::remove_dir_all(root.join("rogue")).unwrap();

    let duplicate = loaded
        .apply_structure(StructureOperation::CreateRequest {
            parent: None,
            index: None,
            name: "Alpha".to_owned(),
            method: None,
            url: None,
        })
        .unwrap_err();
    assert!(matches!(duplicate, StructureError::DuplicateDestination(_)));
    let descendant = loaded
        .apply_structure(StructureOperation::MoveFolder {
            selector: "group".to_owned(),
            parent: Some("group".to_owned()),
            index: None,
        })
        .unwrap_err();
    assert!(matches!(descendant, StructureError::InvalidDestination(_)));

    fs::write(
        root.join("external.yml"),
        "info: { name: External, type: http }\nhttp: { method: GET }\n",
    )
    .unwrap();
    let external_creation = loaded
        .apply_structure(StructureOperation::CreateFolder {
            parent: None,
            index: None,
            name: "Should Conflict".to_owned(),
        })
        .unwrap_err();
    assert!(matches!(
        external_creation,
        StructureError::ConcurrentModification(_)
    ));
    fs::remove_file(root.join("external.yml")).unwrap();

    fs::write(
        root.join("alpha.yml"),
        fs::read_to_string(root.join("alpha.yml")).unwrap() + "\nexternal: true\n",
    )
    .unwrap();
    let conflict = loaded
        .apply_structure(StructureOperation::DeleteRequest {
            selector: "alpha.yml".to_owned(),
        })
        .unwrap_err();
    assert!(matches!(
        conflict,
        StructureError::ConcurrentModification(_)
    ));
    assert!(root.join("alpha.yml").exists());
    fs::remove_dir_all(root).unwrap();
}

fn resolved_variable(
    loaded: &probe_opencollection::LoadedWorkspace,
    environment: &str,
    name: &str,
) -> Option<String> {
    resolve_environment(loaded.workspace().environments(), environment)
        .ok()
        .and_then(|resolved| resolved.variable(name).map(str::to_owned))
}

#[test]
fn bundled_environment_set_unset_save_reload_preserves_unknown_fields() {
    let path = temporary_path("bundled-env.yml");
    fs::write(
        &path,
        concat!(
            "opencollection: 1.0.0\n",
            "info:\n  name: Env persist\n",
            "bundled: true\n",
            "config:\n",
            "  environments:\n",
            "    - name: base\n",
            "      vendor.example: retained-base\n",
            "      variables:\n",
            "        - name: host\n",
            "          value: api.example.com\n",
            "          description: Canonical host\n",
            "        - name: region\n",
            "          value:\n",
            "            - title: AU\n",
            "              selected: true\n",
            "              value: au\n",
            "              note: default\n",
            "            - title: US\n",
            "              value: us\n",
            "    - name: development\n",
            "      extends: base\n",
            "      variables:\n",
            "        - name: host\n",
            "          value: dev.example.com\n",
            "        - name: token\n",
            "          value: development-token\n",
            "items:\n",
            "  - info:\n",
            "      name: Health\n",
            "      type: http\n",
            "    http:\n",
            "      method: GET\n",
            "      url: https://example.com/health\n",
        ),
    )
    .unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    loaded
        .update_environment_variable("development", "host", "local.example.com".to_owned())
        .unwrap();
    loaded
        .update_environment_variable("development", "baseUrl", "https://local.example".to_owned())
        .unwrap();
    loaded
        .update_environment_variable("base", "region", "nz".to_owned())
        .unwrap();
    loaded
        .unset_environment_variable("development", "host")
        .unwrap();

    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(
        resolved_variable(&reloaded, "development", "host").as_deref(),
        Some("api.example.com")
    );
    assert_eq!(
        resolved_variable(&reloaded, "development", "baseUrl").as_deref(),
        Some("https://local.example")
    );
    assert_eq!(
        resolved_variable(&reloaded, "base", "region").as_deref(),
        Some("nz")
    );
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("vendor.example: retained-base"));
    assert!(saved.contains("description: Canonical host"));
    assert!(saved.contains("note: default"));
    assert!(saved.contains("title: US"));
    fs::remove_file(path).unwrap();
}

#[test]
fn unbundled_environment_set_unset_save_reload_preserves_unknown_fields() {
    let root = temporary_path("unbundled-env");
    copy_directory(&fixture("unbundled"), &root);
    fs::write(
        root.join("environments/base.yml"),
        concat!(
            "name: base\n",
            "variables:\n",
            "  - name: host\n",
            "    value: api.example.com\n",
        ),
    )
    .unwrap();
    fs::write(
        root.join("environments/development.yml"),
        concat!(
            "name: development\n",
            "extends: base\n",
            "color: green\n",
            "vendor.example: retained-env\n",
            "variables:\n",
            "  - name: host\n",
            "    value: dev.example.com\n",
            "    description: Child host\n",
            "  - name: baseUrl\n",
            "    value: https://{{host}}\n",
        ),
    )
    .unwrap();

    let mut loaded = load_workspace(&root).unwrap();
    loaded
        .update_environment_variable("development", "host", "local.example.com".to_owned())
        .unwrap();
    loaded
        .unset_environment_variable("development", "host")
        .unwrap();
    loaded
        .update_environment_variable("development", "token", "dev-token".to_owned())
        .unwrap();

    let reloaded = load_workspace(&root).unwrap();
    assert_eq!(
        resolved_variable(&reloaded, "development", "host").as_deref(),
        Some("api.example.com")
    );
    assert_eq!(
        resolved_variable(&reloaded, "development", "token").as_deref(),
        Some("dev-token")
    );
    let saved = fs::read_to_string(root.join("environments/development.yml")).unwrap();
    assert!(saved.contains("vendor.example: retained-env"));
    assert!(saved.contains("color: green"));
    let base = fs::read_to_string(root.join("environments/base.yml")).unwrap();
    assert!(base.contains("value: api.example.com"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn environment_update_refuses_externally_modified_document() {
    let path = temporary_path("env-conflict.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let mut external = fs::read_to_string(&path).unwrap();
    external.push_str("external: true\n");
    fs::write(&path, &external).unwrap();

    let error = loaded
        .update_environment_variable("development", "token", "rotated".to_owned())
        .expect_err("external modification should be rejected");
    assert!(matches!(error, SaveError::ConcurrentModification(_)));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    assert_eq!(
        resolved_variable(&loaded, "development", "token").as_deref(),
        Some("rotated")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn environment_replace_preserves_unknown_and_secret_fields() {
    let path = temporary_path("env-replace.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let mut replacement = loaded.workspace().environments()[1].clone();
    replacement.extends = None;
    replacement.variables.retain(|variable| match variable {
        probe_core::EnvironmentVariable::Plain(variable) => {
            variable.name.as_deref() != Some("host")
        }
        probe_core::EnvironmentVariable::Secret(_) => true,
    });
    replacement
        .variables
        .push(probe_core::EnvironmentVariable::Plain(
            probe_core::Variable {
                name: Some("region".to_owned()),
                value: Some(probe_core::VariableValueSet::Single(
                    probe_core::VariableValue::String("ap-southeast-2".to_owned()),
                )),
                disabled: true,
            },
        ));

    let prepared = loaded
        .prepare_environment_replace("development", replacement)
        .unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_replace(saved);

    let source = fs::read_to_string(&path).unwrap();
    assert!(!source.contains("extends: base"));
    assert!(source.contains("name: region"));
    assert!(source.contains("disabled: true"));
    assert!(source.contains("secret: true"));
    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(
        loaded.workspace().environments()[1],
        reloaded.workspace().environments()[1]
    );
    assert_eq!(reloaded.workspace().environments()[1].extends, None);
    assert!(reloaded.workspace().environments()[1].variables.iter().all(
        |variable| match variable {
            probe_core::EnvironmentVariable::Plain(variable) => {
                variable.name.as_deref() != Some("host")
            }
            probe_core::EnvironmentVariable::Secret(_) => true,
        }
    ));

    let mut base = loaded.workspace().environments()[0].clone();
    base.variables
        .retain(|variable| matches!(variable, probe_core::EnvironmentVariable::Plain(_)));
    let prepared = loaded.prepare_environment_replace("base", base).unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_replace(saved);
    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(
        loaded.workspace().environments()[0],
        reloaded.workspace().environments()[0]
    );
    assert!(
        reloaded.workspace().environments()[0]
            .variables
            .iter()
            .any(|variable| matches!(
                variable,
                probe_core::EnvironmentVariable::Secret(secret)
                    if secret.name.as_deref() == Some("secretToken")
            ))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn environment_replace_rejects_variable_name_collisions_without_writing() {
    let path = temporary_path("env-replace-collisions.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let original = fs::read_to_string(&path).unwrap();
    let loaded = load_workspace(&path).unwrap();
    let base = loaded.workspace().environments()[0].clone();
    let mut replacement = base.clone();
    replacement.variables.retain(|variable| {
        !matches!(
            variable,
            probe_core::EnvironmentVariable::Secret(secret)
                if secret.name.as_deref() == Some("secretToken")
        )
    });
    replacement
        .variables
        .push(probe_core::EnvironmentVariable::Plain(
            probe_core::Variable {
                name: Some("secretToken".to_owned()),
                value: Some(probe_core::VariableValueSet::Single(
                    probe_core::VariableValue::String("plain".to_owned()),
                )),
                disabled: false,
            },
        ));

    let error = loaded
        .prepare_environment_replace("base", replacement)
        .unwrap_err();
    assert!(
        matches!(
            error,
            SaveError::Environment(EnvironmentResolutionError::DuplicateVariable {
                ref environment,
                ref variable,
            }) if environment == "base" && variable == "secretToken"
        ),
        "{error:?}"
    );
    assert_eq!(loaded.workspace().environments()[0], base);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    fs::remove_file(path).unwrap();
}

#[test]
fn bundled_environment_delete_updates_following_indices() {
    let path = temporary_path("env-delete.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let prepared = loaded.prepare_environment_delete("development").unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_delete(saved);
    assert!(
        loaded
            .workspace()
            .environments()
            .iter()
            .all(|environment| environment.name != "development")
    );
    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(reloaded.workspace().environments().len(), 1);
    fs::remove_file(path).unwrap();
}

#[test]
fn unbundled_environment_delete_removes_only_the_environment_document() {
    let root = temporary_path("unbundled-env-delete");
    copy_directory(&fixture("unbundled"), &root);
    fs::write(
        root.join("environments/development.yml"),
        "name: development\nvariables:\n  - name: token\n    value: dev\n",
    )
    .unwrap();
    let mut loaded = load_workspace(&root).unwrap();
    let prepared = loaded.prepare_environment_delete("development").unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_delete(saved);
    assert!(!root.join("environments/development.yml").exists());
    assert!(root.join("opencollection.yml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unbundled_environment_rename_moves_the_document() {
    let root = temporary_path("unbundled-env-rename");
    copy_directory(&fixture("unbundled"), &root);
    let mut loaded = load_workspace(&root).unwrap();
    let mut replacement = loaded
        .workspace()
        .environments()
        .iter()
        .find(|environment| environment.name == "development")
        .cloned()
        .unwrap();
    replacement.name = "staging".to_owned();
    let prepared = loaded
        .prepare_environment_replace("development", replacement)
        .unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_replace(saved);

    assert!(!root.join("environments/development.yml").exists());
    assert!(root.join("environments/staging.yml").exists());
    let saved = fs::read_to_string(root.join("environments/staging.yml")).unwrap();
    assert!(saved.contains("name: staging"));
    assert!(saved.contains("color: green"));
    let reloaded = load_workspace(&root).unwrap();
    assert_eq!(
        loaded
            .workspace()
            .environments()
            .iter()
            .map(|environment| environment.name.as_str())
            .collect::<Vec<_>>(),
        reloaded
            .workspace()
            .environments()
            .iter()
            .map(|environment| environment.name.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        reloaded
            .workspace()
            .environments()
            .iter()
            .any(|environment| environment.name == "staging")
    );
    loaded
        .create_environment("development".to_owned(), None)
        .unwrap();
    assert!(root.join("environments/development.yml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unbundled_environment_rename_refuses_an_existing_destination() {
    let root = temporary_path("unbundled-env-rename-conflict");
    copy_directory(&fixture("unbundled"), &root);
    let loaded = load_workspace(&root).unwrap();
    let original = fs::read_to_string(root.join("environments/development.yml")).unwrap();
    fs::write(
        root.join("environments/staging.yml"),
        "name: staging\nvariables:\n  - name: token\n    value: staging\n",
    )
    .unwrap();
    let mut replacement = loaded
        .workspace()
        .environments()
        .iter()
        .find(|environment| environment.name == "development")
        .cloned()
        .unwrap();
    replacement.name = "staging".to_owned();
    let error = loaded
        .prepare_environment_replace("development", replacement)
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(matches!(error, SaveError::ConcurrentModification(_)));
    assert_eq!(
        fs::read_to_string(root.join("environments/development.yml")).unwrap(),
        original
    );
    assert_eq!(
        loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == "development")
            .map(|environment| environment.name.as_str()),
        Some("development")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn environment_update_rejects_secrets_and_missing_environments() {
    let path = temporary_path("env-secrets.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    let secret = loaded
        .update_environment_variable("development", "secretToken", "nope".to_owned())
        .unwrap_err();
    assert!(matches!(
        secret,
        SaveError::Environment(EnvironmentResolutionError::SecretVariableUnavailable(_))
    ));
    let missing = loaded
        .update_environment_variable("production", "host", "nope".to_owned())
        .unwrap_err();
    assert!(matches!(
        missing,
        SaveError::Environment(EnvironmentResolutionError::EnvironmentNotFound(_))
    ));
    let unset_missing = loaded
        .unset_environment_variable("development", "baseUrl")
        .unwrap_err();
    assert!(matches!(
        unset_missing,
        SaveError::Environment(EnvironmentResolutionError::VariableNotFound { .. })
    ));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        fs::read_to_string(fixture("phase4-environments.yml")).unwrap()
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn environment_update_rejects_stdin_workspaces() {
    let source = fs::read_to_string(fixture("phase4-environments.yml")).unwrap();
    let mut loaded = load_workspace_from_str(&source).unwrap();
    let error = loaded
        .update_environment_variable("development", "token", "rotated".to_owned())
        .unwrap_err();
    assert!(matches!(error, SaveError::ReadOnlySource));
    assert_eq!(
        resolved_variable(&loaded, "development", "token").as_deref(),
        Some("rotated")
    );
}

#[test]
fn create_bundled_workspace_handles_successful_creation_variants() {
    let directory = temporary_path("created");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("pets.yml");
    let loaded = create_bundled_workspace(&path, None, false).expect("collection should create");

    assert_eq!(loaded.workspace().metadata().name.as_deref(), Some("pets"));
    assert_eq!(loaded.workspace().request_count(), 0);
    assert_eq!(loaded.workspace().folder_count(), 0);
    assert!(loaded.workspace().environments().is_empty());
    assert!(!loaded.uses_path_locators());

    let reloaded = load_workspace(&path).expect("created collection should reload");
    assert_eq!(
        reloaded.workspace().metadata().name.as_deref(),
        Some("pets")
    );
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains("opencollection: 1.0.0"));
    assert!(source.contains("bundled: true"));
    fs::remove_dir_all(directory).unwrap();

    let path = temporary_path("untitled");
    let created = path.with_extension("yml");
    let loaded = create_bundled_workspace(&path, Some(" Pet Store "), false)
        .expect("collection should create");

    assert_eq!(
        loaded.workspace().metadata().name.as_deref(),
        Some("Pet Store")
    );
    assert!(created.is_file());
    fs::remove_file(created).unwrap();

    let path = temporary_path("true.yml");
    let loaded =
        create_bundled_workspace(&path, Some("true"), false).expect("collection should create");

    assert_eq!(loaded.workspace().metadata().name.as_deref(), Some("true"));
    let reloaded = load_workspace(&path).expect("quoted name should reload");
    assert_eq!(
        reloaded.workspace().metadata().name.as_deref(),
        Some("true")
    );
    fs::remove_file(path).unwrap();

    let path = temporary_path("nested").join("api").join("collection.yml");
    let loaded = create_bundled_workspace(&path, None, false).expect("collection should create");

    assert_eq!(
        loaded.workspace().metadata().name.as_deref(),
        Some("collection")
    );
    fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
}

#[test]
fn create_bundled_workspace_handles_existing_paths() {
    let path = temporary_path("existing.yml");
    fs::write(&path, "keep me\n").unwrap();

    let error = create_bundled_workspace(&path, None, false).unwrap_err();
    assert!(matches!(error, CreateError::AlreadyExists(_)));
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me\n");

    let loaded =
        create_bundled_workspace(&path, Some("Replaced"), true).expect("replace should succeed");
    assert_eq!(
        loaded.workspace().metadata().name.as_deref(),
        Some("Replaced")
    );
    fs::remove_file(path).unwrap();

    let path = temporary_path("collection.yml");
    fs::create_dir(&path).unwrap();

    let error = create_bundled_workspace(&path, None, false).unwrap_err();
    assert!(matches!(error, CreateError::IsDirectory(_)));
    fs::remove_dir(path).unwrap();
}

#[test]
fn bundled_environment_create_save_reload_preserves_existing_environments() {
    let path = temporary_path("bundled-env-create.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    loaded
        .create_environment("staging".to_owned(), Some("base".to_owned()))
        .unwrap();
    loaded
        .update_environment_variable("staging", "host", "staging.example.com".to_owned())
        .unwrap();

    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(reloaded.workspace().environments().len(), 3);
    assert_eq!(
        resolved_variable(&reloaded, "staging", "host").as_deref(),
        Some("staging.example.com")
    );
    assert_eq!(
        resolved_variable(&reloaded, "staging", "tenant").as_deref(),
        Some("au")
    );
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("name: staging"));
    assert!(saved.contains("extends: base"));
    assert!(saved.contains("name: development"));
    fs::remove_file(path).unwrap();
}

#[test]
fn bundled_environment_create_appends_to_empty_config() {
    let path = temporary_path("bundled-env-empty.yml");
    create_bundled_workspace(&path, Some("Empty"), false).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    loaded
        .create_environment("production".to_owned(), None)
        .unwrap();

    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(reloaded.workspace().environments().len(), 1);
    assert_eq!(reloaded.workspace().environments()[0].name, "production");
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("config:"));
    assert!(saved.contains("name: production"));
    fs::remove_file(path).unwrap();
}

#[test]
fn unbundled_environment_create_save_reload_creates_environment_file() {
    let root = temporary_path("unbundled-env-create");
    copy_directory(&fixture("unbundled"), &root);
    fs::remove_file(root.join("environments/development.yml")).unwrap();
    let mut loaded = load_workspace(&root).unwrap();
    assert!(loaded.workspace().environments().is_empty());

    loaded
        .create_environment("staging".to_owned(), None)
        .unwrap();
    loaded
        .update_environment_variable(
            "staging",
            "baseUrl",
            "https://staging.example.com".to_owned(),
        )
        .unwrap();

    let reloaded = load_workspace(&root).unwrap();
    assert_eq!(reloaded.workspace().environments().len(), 1);
    assert_eq!(reloaded.workspace().environments()[0].name, "staging");
    assert_eq!(
        resolved_variable(&reloaded, "staging", "baseUrl").as_deref(),
        Some("https://staging.example.com")
    );
    let saved = fs::read_to_string(root.join("environments/staging.yml")).unwrap();
    assert!(saved.contains("name: staging"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn environment_create_rejects_duplicates_missing_parents_and_stdin() {
    let path = temporary_path("env-create-errors.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    let duplicate = loaded
        .create_environment("development".to_owned(), None)
        .unwrap_err();
    assert!(matches!(
        duplicate,
        SaveError::Environment(EnvironmentResolutionError::DuplicateEnvironment(_))
    ));
    let missing_parent = loaded
        .create_environment("staging".to_owned(), Some("missing".to_owned()))
        .unwrap_err();
    assert!(matches!(
        missing_parent,
        SaveError::Environment(EnvironmentResolutionError::ParentEnvironmentNotFound { .. })
    ));

    let source = fs::read_to_string(fixture("phase4-environments.yml")).unwrap();
    let mut stdin_loaded = load_workspace_from_str(&source).unwrap();
    let read_only = stdin_loaded
        .create_environment("staging".to_owned(), None)
        .unwrap_err();
    assert!(matches!(read_only, SaveError::ReadOnlySource));
    assert_eq!(stdin_loaded.workspace().environments().len(), 2);

    fs::remove_file(path).unwrap();
}

#[test]
fn environment_create_refuses_externally_modified_document() {
    let path = temporary_path("env-create-conflict.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let mut external = fs::read_to_string(&path).unwrap();
    external.push_str("external: true\n");
    fs::write(&path, &external).unwrap();

    let error = loaded
        .create_environment("staging".to_owned(), None)
        .unwrap_err();
    assert!(matches!(error, SaveError::ConcurrentModification(_)));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    assert_eq!(loaded.workspace().environments().len(), 2);
    fs::remove_file(path).unwrap();
}
