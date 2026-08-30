#[allow(dead_code, unused_imports)]
mod common;

use common::*;

#[test]
fn help_starts_successfully() {
    let output = probe()
        .arg("--help")
        .output()
        .expect("probe binary should start");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("collection validate"));
    assert!(output.stderr.is_empty());
}

#[test]
fn validates_unbundled_workspace_as_json() {
    let output = probe()
        .args(["collection", "validate"])
        .arg(fixture("unbundled"))
        .arg("--json")
        .output()
        .expect("validate command should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["valid"], true);
    assert_eq!(value["collection"]["name"], "Unbundled fixture");
    assert_eq!(value["counts"]["requests"], 2);
    assert_eq!(value["counts"]["folders"], 1);
    assert_eq!(value["counts"]["environments"], 1);
}

#[test]
fn creates_a_bundled_collection_as_json() {
    let path = temporary_path("pets.yml");
    let value = run_json(&[
        "collection",
        "create",
        path.to_str().expect("temp path should be utf-8"),
        "--name",
        "Pet Store",
    ]);

    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["created"], true);
    assert_eq!(value["collection"]["name"], "Pet Store");
    assert_eq!(value["counts"]["requests"], 0);
    assert_eq!(value["counts"]["folders"], 0);
    assert_eq!(value["counts"]["environments"], 0);
    let created = PathBuf::from(value["path"].as_str().expect("path should be a string"));
    assert!(created.is_file());
    let loaded = probe()
        .args(["collection", "validate"])
        .arg(&created)
        .arg("--json")
        .output()
        .expect("validate should run");
    assert!(loaded.status.success());
    fs::remove_file(created).unwrap();
}

#[test]
fn create_refuses_to_overwrite_an_existing_file() {
    let path = temporary_path("existing.yml");
    fs::write(&path, "keep me\n").unwrap();
    let output = probe()
        .args(["collection", "create"])
        .arg(&path)
        .arg("--json")
        .output()
        .expect("create command should run");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["error"]["category"], "invalid_arguments");
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me\n");
    fs::remove_file(path).unwrap();
}

#[test]
fn imports_a_yaak_export_as_bundled_opencollection_json() {
    let destination = temporary_path("yaak-import.yml");
    let output = probe()
        .args(["collection", "import", "yaak"])
        .arg(yaak_fixture("export-v4.json"))
        .arg(&destination)
        .arg("--json")
        .output()
        .expect("Yaak import should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["imported"], true);
    assert_eq!(value["partial"], false);
    assert_eq!(value["sourceFormat"], "yaak_export");
    assert_eq!(value["workspace"]["id"], "wk_1");
    assert_eq!(value["counts"]["requests"], 1);
    assert_eq!(value["counts"]["folders"], 1);
    assert_eq!(value["counts"]["environments"], 1);
    assert!(destination.is_file());

    let validate = probe()
        .args(["collection", "validate"])
        .arg(&destination)
        .arg("--json")
        .output()
        .unwrap();
    assert!(validate.status.success());
    fs::remove_file(destination).unwrap();
}

#[test]
fn imports_a_postman_v21_collection_as_bundled_opencollection_json() {
    let destination = temporary_path("postman-import.yml");
    let output = probe()
        .args(["collection", "import", "postman"])
        .arg(postman_fixture("collection-v2.1.json"))
        .arg(&destination)
        .arg("--json")
        .output()
        .expect("Postman import should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["imported"], true);
    assert_eq!(value["partial"], false);
    assert_eq!(value["sourceFormat"], "postman_collection_v2_1");
    assert_eq!(value["collection"]["id"], "pm_v21");
    assert_eq!(value["counts"]["requests"], 1);
    assert_eq!(value["counts"]["folders"], 1);
    assert_eq!(value["counts"]["environments"], 1);
    assert_eq!(
        value["collectionVariablesEnvironment"],
        "Postman Collection Variables"
    );
    assert!(destination.is_file());

    let validate = probe()
        .args(["collection", "validate"])
        .arg(&destination)
        .arg("--json")
        .output()
        .unwrap();
    assert!(validate.status.success());
    fs::remove_file(destination).unwrap();
}

#[test]
fn postman_import_is_strict_and_never_overwrites() {
    let destination = temporary_path("postman-lossy.yml");
    let source = postman_fixture("collection-lossy.json");
    let strict = probe()
        .args(["collection", "import", "postman"])
        .arg(&source)
        .arg(&destination)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(8));
    assert!(!destination.exists());

    let partial = probe()
        .args(["collection", "import", "postman"])
        .arg(&source)
        .arg(&destination)
        .args(["--allow-partial", "--json"])
        .output()
        .unwrap();
    assert!(partial.status.success());
    let value: Value = serde_json::from_slice(&partial.stdout).unwrap();
    assert_eq!(value["partial"], true);

    fs::write(&destination, "keep me\n").unwrap();
    let conflict = probe()
        .args(["collection", "import", "postman"])
        .arg(postman_fixture("collection-v2.json"))
        .arg(&destination)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    assert_eq!(conflict.status.code(), Some(2));
    let conflict_json: Value = serde_json::from_slice(&conflict.stdout).unwrap();
    assert_eq!(conflict_json["error"]["category"], "invalid_arguments");
    assert!(conflict.stderr.is_empty());
    assert_eq!(fs::read_to_string(&destination).unwrap(), "keep me\n");
    fs::remove_file(destination).unwrap();
}

#[test]
fn malformed_and_unsupported_postman_sources_are_invalid_imports() {
    for (fixture_name, suffix) in [
        ("malformed.json", "postman-malformed.yml"),
        ("collection-v3.json", "postman-v3.yml"),
    ] {
        let destination = temporary_path(suffix);
        let output = probe()
            .args(["collection", "import", "postman"])
            .arg(postman_fixture(fixture_name))
            .arg(&destination)
            .arg("--json")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        assert!(output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["category"], "invalid_import");
        assert!(!destination.exists());
    }
}

#[test]
fn postman_human_output_is_stable_and_stays_on_stdout() {
    let destination = temporary_path("postman-human.yml");
    let output = probe()
        .args(["collection", "import", "postman"])
        .arg(postman_fixture("collection-v2.json"))
        .arg(&destination)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let persisted = destination.canonicalize().unwrap();
    assert_eq!(
        stdout,
        format!(
            "Imported Postman collection\nName: Postman V2\nPath: {}\nRequests: 1\nFolders: 0\nEnvironments: 0\nCollection variables environment: none\nWarnings: 0\n",
            persisted.display()
        )
    );
    fs::remove_file(destination).unwrap();
}

#[test]
fn workspace_option_is_rejected_for_postman_import() {
    let destination = temporary_path("postman-workspace.yml");
    let output = probe()
        .args(["collection", "import", "postman"])
        .arg(postman_fixture("collection-v2.json"))
        .arg(&destination)
        .args(["--workspace", "wk_1", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_arguments");
    assert!(!destination.exists());
}

#[test]
fn yaak_import_is_strict_by_default_and_partial_mode_is_explicit() {
    let source = temporary_path("yaak-lossy.json");
    let destination = temporary_path("yaak-lossy.yml");
    fs::write(
        &source,
        r#"{"yaakSchema":4,"resources":{"workspaces":[{"model":"workspace","id":"wk_1","name":"Mixed"}],"grpcRequests":[{"model":"grpc_request","id":"gr_1","workspaceId":"wk_1"}]}}"#,
    )
    .unwrap();

    let strict = probe()
        .args(["collection", "import", "yaak"])
        .arg(&source)
        .arg(&destination)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(8));
    let strict_json: Value = serde_json::from_slice(&strict.stdout).unwrap();
    assert_eq!(strict_json["error"]["category"], "unsupported_import");
    assert_eq!(
        strict_json["error"]["details"]["diagnostics"][0]["code"],
        "unsupported_resource"
    );
    assert!(!destination.exists());

    let partial = probe()
        .args(["collection", "import", "yaak"])
        .arg(&source)
        .arg(&destination)
        .args(["--allow-partial", "--json"])
        .output()
        .unwrap();
    assert!(partial.status.success());
    let partial_json: Value = serde_json::from_slice(&partial.stdout).unwrap();
    assert_eq!(partial_json["partial"], true);
    assert_eq!(partial_json["warnings"][0]["severity"], "lossy");

    fs::remove_file(source).unwrap();
    fs::remove_file(destination).unwrap();
}
#[test]
fn yaak_import_lists_multiple_workspaces_and_never_overwrites() {
    let source = temporary_path("yaak-multi.json");
    let destination = temporary_path("yaak-existing.yml");
    fs::write(
        &source,
        r#"{"yaakSchema":4,"resources":{"workspaces":[{"model":"workspace","id":"wk_a","name":"A"},{"model":"workspace","id":"wk_b","name":"B"}]}}"#,
    )
    .unwrap();

    let ambiguous = probe()
        .args(["collection", "import", "yaak"])
        .arg(&source)
        .arg(&destination)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(ambiguous.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&ambiguous.stdout).unwrap();
    assert_eq!(value["error"]["category"], "workspace_selection_required");
    assert_eq!(value["error"]["details"]["workspaces"][1]["id"], "wk_b");

    fs::write(&destination, "keep me\n").unwrap();
    let conflict = probe()
        .args(["collection", "import", "yaak"])
        .arg(&source)
        .arg(&destination)
        .args(["--workspace", "wk_b", "--json"])
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    assert_eq!(fs::read_to_string(&destination).unwrap(), "keep me\n");

    fs::remove_file(source).unwrap();
    fs::remove_file(destination).unwrap();
}
