use super::environment_persistence::resolved_variable;
use super::*;

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
