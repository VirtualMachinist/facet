#[allow(dead_code, unused_imports)]
mod common;

use common::*;

#[test]
fn lists_requests_deterministically_as_json() {
    let path = fixture("unbundled");
    let first = probe()
        .args(["request", "list"])
        .arg(&path)
        .arg("--json")
        .output()
        .expect("first list command should run");
    let second = probe()
        .args(["request", "list"])
        .arg(&path)
        .arg("--json")
        .output()
        .expect("second list command should run");

    assert!(first.status.success());
    assert!(first.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    let value: Value = serde_json::from_slice(&first.stdout).expect("stdout should be JSON");
    assert_eq!(value["requests"][0]["selector"], "health.yml");
    assert_eq!(value["requests"][1]["selector"], "users/list-users.yml");
}

#[test]
fn reads_a_bundled_workspace_from_stdin() {
    let source = fs::read(fixture("phase1-bundled.yml")).unwrap();
    let output = run_with_stdin(&["request", "list", "-", "--json"], &source);

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["requests"][0]["selector"], "items/0/items/0");
    assert_eq!(value["requests"][1]["selector"], "items/1");
}

#[test]
fn quiet_mode_suppresses_success_output() {
    let output = probe()
        .args(["collection", "validate"])
        .arg(fixture("unbundled"))
        .arg("--quiet")
        .output()
        .expect("validate command should run");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn gets_request_by_repository_selector() {
    let output = probe()
        .args(["request", "get"])
        .arg(fixture("unbundled"))
        .arg("users/list-users.yml")
        .arg("--json")
        .output()
        .expect("get command should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["name"], "List users");
    assert_eq!(value["method"], "GET");
    assert_eq!(value["headers"][0]["name"], "Accept");
    assert_eq!(value["queryParameters"][0]["name"], "limit");
    assert!(value["environment"].is_null());
}

#[test]
fn gets_path_parameters_in_json_output() {
    let output = probe()
        .args(["request", "get"])
        .arg(fixture("phase1-bundled.yml"))
        .arg("items/0/items/0")
        .arg("--json")
        .output()
        .expect("get command should run");

    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["pathParameters"][0]["name"], "ownerId");
    assert_eq!(value["pathParameters"][0]["value"], "42");
}

#[test]
fn sets_and_persists_request_fields_as_json() {
    let workspace = temporary_path("phase7-workspace.yml");
    fs::copy(fixture("phase1-round-trip.yml"), &workspace).unwrap();
    let output = probe()
        .args(["request", "set"])
        .arg(&workspace)
        .arg("items/0")
        .args([
            "--name",
            "Replace pet",
            "--method",
            "PUT",
            "--url",
            "https://api.example.com/pets/42",
            "--json",
        ])
        .output()
        .expect("set command should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["selector"], "items/0");
    assert_eq!(value["name"], "Replace pet");
    assert_eq!(value["method"], "PUT");
    assert_eq!(value["url"], "https://api.example.com/pets/42");

    let inspect = probe()
        .args(["request", "get"])
        .arg(&workspace)
        .args(["items/0", "--json"])
        .output()
        .expect("saved request should be inspectable");
    assert!(inspect.status.success());
    let reloaded: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(reloaded["name"], "Replace pet");
    assert_eq!(reloaded["method"], "PUT");
    assert_eq!(reloaded["url"], "https://api.example.com/pets/42");
    let saved = fs::read_to_string(&workspace).unwrap();
    assert!(saved.contains("vendor.example"));
    assert!(saved.contains("runtime:"));
    fs::remove_file(workspace).unwrap();
}

#[test]
fn set_requires_at_least_one_explicit_field() {
    let output = probe()
        .args(["request", "set"])
        .arg(fixture("phase1-round-trip.yml"))
        .args(["items/0", "--json"])
        .output()
        .expect("set command should run");

    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_arguments");
}

#[test]
fn mutating_commands_reject_stdin_as_read_only() {
    let request_set = [
        "request",
        "set",
        "-",
        "items/0",
        "--url",
        "https://example.com/updated",
        "--json",
    ];
    let request_create = ["request", "create", "-", "--name", "Read only", "--json"];
    let environment_set = [
        "environment",
        "set",
        "-",
        "--environment",
        "development",
        "--name",
        "token",
        "--value",
        "rotated",
        "--json",
    ];
    let environment_create = ["environment", "create", "-", "--name", "staging", "--json"];
    let environment_delete = [
        "environment",
        "delete",
        "-",
        "--environment",
        "development",
        "--json",
    ];
    let environment_rename = [
        "environment",
        "rename",
        "-",
        "--environment",
        "development",
        "--name",
        "staging",
        "--json",
    ];
    let cases: &[(&str, &[&str])] = &[
        ("phase1-round-trip.yml", request_set.as_slice()),
        ("phase16-bundled.yml", request_create.as_slice()),
        ("phase4-environments.yml", environment_set.as_slice()),
        ("phase4-environments.yml", environment_create.as_slice()),
        ("phase4-environments.yml", environment_delete.as_slice()),
        ("phase4-environments.yml", environment_rename.as_slice()),
    ];

    for (fixture_name, arguments) in cases {
        let source = fs::read(fixture(fixture_name)).unwrap();
        let output = run_with_stdin(arguments, &source);
        assert_eq!(
            output.status.code(),
            Some(7),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty(), "{arguments:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["exitCode"], 7);
        assert_eq!(value["error"]["category"], "persistence_read_only");
    }
}

#[test]
fn gets_request_resolved_with_selected_environment() {
    let output = probe()
        .args(["request", "get"])
        .arg(fixture("phase4-environments.yml"))
        .arg("items/0")
        .args(["--environment", "development", "--json"])
        .output()
        .expect("resolved get command should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["environment"], "development");
    assert_eq!(value["url"], "https://dev.example.com/au/users");
    assert_eq!(value["headers"][0]["value"], "Bearer development-token");
    assert_eq!(value["queryParameters"][0]["value"], "au");
    assert_eq!(value["body"]["value"]["data"], "{\"tenant\":\"au\"}");
    assert_eq!(
        value["authentication"]["properties"]["token"],
        "development-token"
    );
}

#[test]
fn reports_environment_resolution_errors() {
    for (selector, environment, category) in [
        ("items/0", "production", "environment_not_found"),
        ("items/1", "development", "missing_variable"),
        ("items/2", "development", "secret_variable_unavailable"),
    ] {
        let output = probe()
            .args(["request", "get"])
            .arg(fixture("phase4-environments.yml"))
            .arg(selector)
            .args(["--environment", environment, "--json"])
            .output()
            .expect("get command should run");
        assert_eq!(output.status.code(), Some(5), "{category}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["category"], category);
    }
}

#[test]
fn reports_request_not_found_as_structured_error() {
    let output = probe()
        .args(["request", "get"])
        .arg(fixture("unbundled"))
        .arg("missing.yml")
        .arg("--json")
        .output()
        .expect("get command should run");

    assert_eq!(output.status.code(), Some(4));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["error"]["exitCode"], 4);
    assert_eq!(value["error"]["category"], "request_not_found");
}

#[test]
fn quiet_mode_preserves_failure_diagnostics_and_status() {
    let output = probe()
        .args(["request", "get"])
        .arg(fixture("unbundled"))
        .arg("missing.yml")
        .arg("--quiet")
        .output()
        .expect("get command should run");

    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("request_not_found"));
}

#[test]
fn executes_request_as_deterministic_json() {
    let (server_url, server) = serve_once(b"{\"result\":\"ok\"}".to_vec(), "application/json");
    let workspace = runtime_fixture(&server_url);
    let output = probe()
        .args(["request", "run"])
        .arg(&workspace)
        .arg("items/0")
        .args(["--environment", "local", "--json"])
        .output()
        .expect("run command should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["request"]["method"], "POST");
    assert_eq!(value["response"]["status"], 200);
    assert_eq!(value["response"]["sizeBytes"], 15);
    assert_eq!(value["response"]["body"]["encoding"], "utf8");
    assert_eq!(value["response"]["body"]["content"], "{\"result\":\"ok\"}");
    let captured = server.join().unwrap();
    assert!(
        captured
            .head
            .starts_with("POST /echo?mode=cli HTTP/1.1\r\n")
    );
    assert!(
        captured
            .head
            .contains("authorization: Bearer cli-token\r\n")
    );
    assert!(captured.head.contains("x-probe: phase-five\r\n"));
    assert_eq!(captured.body, b"{\"source\":\"cli\"}");
    fs::remove_file(workspace).unwrap();
}

#[test]
fn writes_response_body_to_an_explicit_file() {
    let response_body = vec![0, 159, 146, 150];
    let (server_url, server) = serve_once(response_body.clone(), "application/octet-stream");
    let workspace = runtime_fixture(&server_url);
    let body_output = temporary_path("response.bin");
    let output = probe()
        .args(["request", "run"])
        .arg(&workspace)
        .arg("items/0")
        .args(["--environment", "local", "--output"])
        .arg(&body_output)
        .arg("--json")
        .output()
        .expect("run command should run");

    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["response"]["body"]["content"], Value::Null);
    assert_eq!(value["response"]["body"]["omitted"], false);
    assert_eq!(
        value["response"]["body"]["outputPath"],
        body_output.to_string_lossy().as_ref()
    );
    assert_eq!(fs::read(&body_output).unwrap(), response_body);
    server.join().unwrap();
    fs::remove_file(workspace).unwrap();
    fs::remove_file(body_output).unwrap();
}

#[test]
fn omits_large_response_body_from_stdout() {
    let response_body = vec![b'x'; MAX_IN_MEMORY_RESPONSE_BYTES + 1];
    let (server_url, server) = serve_once(response_body, "text/plain");
    let workspace = runtime_fixture(&server_url);
    let output = probe()
        .args(["request", "run"])
        .arg(&workspace)
        .arg("items/0")
        .args(["--environment", "local", "--json"])
        .output()
        .expect("run command should run");

    assert!(output.status.success());
    assert!(output.stdout.len() < 10_000);
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["response"]["body"]["content"], Value::Null);
    assert_eq!(value["response"]["body"]["omitted"], true);
    assert_eq!(value["response"]["body"]["omissionReason"], "too_large");
    server.join().unwrap();
    fs::remove_file(workspace).unwrap();
}

#[test]
fn run_preflights_environment_resolution_before_http() {
    let output = probe()
        .args(["request", "run"])
        .arg(fixture("phase4-environments.yml"))
        .arg("items/1")
        .args(["--environment", "development", "--json"])
        .output()
        .expect("run command should run");

    assert_eq!(output.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["category"], "missing_variable");
}

#[test]
fn distinguishes_invalid_workspace() {
    let output = probe()
        .args(["collection", "validate"])
        .arg(fixture("does-not-exist.yml"))
        .arg("--json")
        .output()
        .expect("validate command should run");

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["error"]["category"], "invalid_workspace");
}

#[test]
fn distinguishes_invalid_arguments() {
    let output = probe()
        .args(["unknown", "command", "--json"])
        .output()
        .expect("invalid command should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["error"]["exitCode"], 2);
    assert_eq!(value["error"]["category"], "invalid_arguments");
}

#[test]
fn rejects_json_and_quiet_together_as_structured_error() {
    let output = probe()
        .args(["collection", "validate"])
        .arg(fixture("unbundled"))
        .args(["--json", "--quiet"])
        .output()
        .expect("validate command should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["error"]["category"], "invalid_arguments");
}
