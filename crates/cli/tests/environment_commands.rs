#[allow(dead_code, unused_imports)]
mod common;

use common::*;

#[test]
fn sets_and_unsets_environment_variables_as_json() {
    let workspace = temporary_path("phase-env-workspace.yml");
    fs::copy(fixture("phase4-environments.yml"), &workspace).unwrap();

    let output = probe()
        .args(["environment", "set"])
        .arg(&workspace)
        .args([
            "--environment",
            "development",
            "--name",
            "token",
            "--value",
            "rotated",
            "--json",
        ])
        .output()
        .expect("environment set should run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["environment"], "development");
    assert_eq!(value["name"], "token");
    assert_eq!(value["operation"], "set");
    assert_eq!(value["value"], "rotated");

    let override_host = probe()
        .args(["environment", "set"])
        .arg(&workspace)
        .args([
            "--environment",
            "development",
            "--name",
            "host",
            "--value",
            "local.example.com",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(override_host.status.success());

    let unset = probe()
        .args(["environment", "unset"])
        .arg(&workspace)
        .args(["--environment", "development", "--name", "host", "--json"])
        .output()
        .unwrap();
    assert!(unset.status.success());
    let value: Value = serde_json::from_slice(&unset.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["operation"], "unset");
    assert_eq!(value["environment"], "development");
    assert_eq!(value["name"], "host");
    assert!(value.get("value").is_none());

    let resolved = probe()
        .args(["request", "get"])
        .arg(&workspace)
        .args(["items/0", "--environment", "development", "--json"])
        .output()
        .unwrap();
    assert!(resolved.status.success());
    let value: Value = serde_json::from_slice(&resolved.stdout).unwrap();
    assert_eq!(value["headers"][0]["value"], "Bearer rotated");
    assert_eq!(value["url"], "https://api.example.com/au/users");
    fs::remove_file(workspace).unwrap();
}

#[test]
fn environment_commands_have_stable_errors() {
    let missing = probe()
        .args(["environment", "set"])
        .arg(fixture("phase4-environments.yml"))
        .args([
            "--environment",
            "production",
            "--name",
            "token",
            "--value",
            "nope",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(value["error"]["category"], "environment_not_found");
    assert_eq!(value["error"]["exitCode"], 5);

    let secret = probe()
        .args(["environment", "set"])
        .arg(fixture("phase4-environments.yml"))
        .args([
            "--environment",
            "development",
            "--name",
            "secretToken",
            "--value",
            "nope",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(secret.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&secret.stdout).unwrap();
    assert_eq!(value["error"]["category"], "secret_variable_unavailable");

    let unset_missing = probe()
        .args(["environment", "unset"])
        .arg(fixture("phase4-environments.yml"))
        .args([
            "--environment",
            "development",
            "--name",
            "baseUrl",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(unset_missing.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&unset_missing.stdout).unwrap();
    assert_eq!(value["error"]["category"], "variable_not_found");

    let invalid = probe()
        .args(["environment", "set"])
        .arg(fixture("phase4-environments.yml"))
        .args(["--json"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_arguments");
}

#[test]
fn lists_environments_as_json() {
    let output = probe()
        .args(["environment", "list"])
        .arg(fixture("phase4-environments.yml"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["environments"].as_array().unwrap().len(), 2);
    assert_eq!(value["environments"][0]["name"], "base");
    assert_eq!(value["environments"][1]["name"], "development");
    assert_eq!(value["environments"][1]["extends"], "base");
}

#[test]
fn creates_environment_as_json() {
    let workspace = temporary_path("phase-env-create.yml");
    fs::copy(fixture("phase4-environments.yml"), &workspace).unwrap();

    let output = probe()
        .args(["environment", "create"])
        .arg(&workspace)
        .args(["--name", "staging", "--extends", "base", "--json"])
        .output()
        .expect("environment create should run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["environment"], "staging");
    assert_eq!(value["extends"], "base");
    assert_eq!(value["operation"], "create");

    let list = probe()
        .args(["environment", "list"])
        .arg(&workspace)
        .arg("--json")
        .output()
        .unwrap();
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(listed["environments"].as_array().unwrap().len(), 3);
    assert_eq!(listed["environments"][2]["name"], "staging");
    assert_eq!(listed["environments"][2]["extends"], "base");

    fs::remove_file(workspace).unwrap();
}

#[test]
fn environment_create_has_stable_errors() {
    let duplicate = probe()
        .args(["environment", "create"])
        .arg(fixture("phase4-environments.yml"))
        .args(["--name", "development", "--json"])
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&duplicate.stdout).unwrap();
    assert_eq!(value["error"]["category"], "duplicate_environment");

    let missing_parent = probe()
        .args(["environment", "create"])
        .arg(fixture("phase4-environments.yml"))
        .args(["--name", "staging", "--extends", "missing", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing_parent.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&missing_parent.stdout).unwrap();
    assert_eq!(value["error"]["category"], "parent_environment_not_found");

    let invalid = probe()
        .args(["environment", "create"])
        .arg(fixture("phase4-environments.yml"))
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_arguments");
}

#[test]
fn deletes_environment_as_json() {
    let workspace = temporary_path("phase-env-delete.yml");
    fs::copy(fixture("phase4-environments.yml"), &workspace).unwrap();

    let output = probe()
        .args(["environment", "delete"])
        .arg(&workspace)
        .args(["--environment", "development", "--json"])
        .output()
        .expect("environment delete should run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["environment"], "development");
    assert_eq!(value["operation"], "delete");

    let list = probe()
        .args(["environment", "list"])
        .arg(&workspace)
        .arg("--json")
        .output()
        .unwrap();
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(listed["environments"].as_array().unwrap().len(), 1);
    assert_eq!(listed["environments"][0]["name"], "base");

    fs::remove_file(workspace).unwrap();
}

#[test]
fn renames_environment_as_json() {
    let workspace = temporary_path("phase-env-rename.yml");
    fs::copy(fixture("phase4-environments.yml"), &workspace).unwrap();

    let output = probe()
        .args(["environment", "rename"])
        .arg(&workspace)
        .args([
            "--environment",
            "development",
            "--name",
            "staging",
            "--json",
        ])
        .output()
        .expect("environment rename should run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["environment"], "staging");
    assert_eq!(value["previousEnvironment"], "development");
    assert_eq!(value["operation"], "rename");

    let list = probe()
        .args(["environment", "list"])
        .arg(&workspace)
        .arg("--json")
        .output()
        .unwrap();
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(listed["environments"].as_array().unwrap().len(), 2);
    assert_eq!(listed["environments"][0]["name"], "base");
    assert_eq!(listed["environments"][1]["name"], "staging");
    assert_eq!(listed["environments"][1]["extends"], "base");

    fs::remove_file(workspace).unwrap();
}

#[test]
fn deletes_unbundled_environment_leaf() {
    let workspace = temporary_path("phase-env-unbundled-delete");
    copy_directory(&fixture("unbundled"), &workspace);

    let output = probe()
        .args(["environment", "delete"])
        .arg(&workspace)
        .args(["--environment", "development", "--json"])
        .output()
        .expect("unbundled environment delete should run");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["environment"], "development");
    assert_eq!(value["operation"], "delete");
    assert!(!workspace.join("environments/development.yml").exists());
    assert!(workspace.join("opencollection.yml").exists());

    let list = probe()
        .args(["environment", "list"])
        .arg(&workspace)
        .arg("--json")
        .output()
        .unwrap();
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert!(listed["environments"].as_array().unwrap().is_empty());

    fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn environment_delete_and_rename_have_stable_errors() {
    let workspace = temporary_path("phase-env-delete-rename-errors.yml");
    fs::copy(fixture("phase4-environments.yml"), &workspace).unwrap();
    let original = fs::read(&workspace).unwrap();

    let missing = probe()
        .args(["environment", "delete"])
        .arg(&workspace)
        .args(["--environment", "production", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(5));
    assert!(missing.stderr.is_empty());
    let value: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(value["error"]["category"], "environment_not_found");
    assert_eq!(value["error"]["exitCode"], 5);

    let missing_rename = probe()
        .args(["environment", "rename"])
        .arg(&workspace)
        .args(["--environment", "production", "--name", "staging", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing_rename.status.code(), Some(5));
    assert!(missing_rename.stderr.is_empty());
    let value: Value = serde_json::from_slice(&missing_rename.stdout).unwrap();
    assert_eq!(value["error"]["category"], "environment_not_found");
    assert_eq!(value["error"]["exitCode"], 5);

    let duplicate = probe()
        .args(["environment", "rename"])
        .arg(&workspace)
        .args(["--environment", "development", "--name", "base", "--json"])
        .output()
        .unwrap();
    assert_eq!(duplicate.status.code(), Some(5));
    assert!(duplicate.stderr.is_empty());
    let value: Value = serde_json::from_slice(&duplicate.stdout).unwrap();
    assert_eq!(value["error"]["category"], "duplicate_environment");
    assert_eq!(value["error"]["exitCode"], 5);

    for (action, extra) in [("delete", None), ("rename", Some(("--name", "shared")))] {
        let mut command = probe();
        command.args(["environment", action]).arg(&workspace).args([
            "--environment",
            "base",
            "--json",
        ]);
        if let Some((flag, value)) = extra {
            command.args([flag, value]);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(5), "{action}");
        assert!(output.stderr.is_empty(), "{action}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["category"], "environment_in_use");
        assert_eq!(value["error"]["exitCode"], 5);
        assert_eq!(
            value["error"]["message"],
            "environment 'base' is extended by another environment"
        );
        assert_eq!(fs::read(&workspace).unwrap(), original);
    }

    let missing_environment = probe()
        .args(["environment", "delete"])
        .arg(&workspace)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(missing_environment.status.code(), Some(2));
    assert!(missing_environment.stderr.is_empty());
    let value: Value = serde_json::from_slice(&missing_environment.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_arguments");
    assert_eq!(value["error"]["exitCode"], 2);

    let missing_name = probe()
        .args(["environment", "rename"])
        .arg(&workspace)
        .args(["--environment", "development", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing_name.status.code(), Some(2));
    assert!(missing_name.stderr.is_empty());
    let value: Value = serde_json::from_slice(&missing_name.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_arguments");
    assert_eq!(value["error"]["exitCode"], 2);

    let missing_rename_environment = probe()
        .args(["environment", "rename"])
        .arg(&workspace)
        .args(["--name", "staging", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing_rename_environment.status.code(), Some(2));
    assert!(missing_rename_environment.stderr.is_empty());
    let value: Value = serde_json::from_slice(&missing_rename_environment.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_arguments");
    assert_eq!(value["error"]["exitCode"], 2);

    fs::remove_file(workspace).unwrap();
}
