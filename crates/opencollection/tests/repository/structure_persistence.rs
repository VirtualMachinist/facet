use super::*;

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
