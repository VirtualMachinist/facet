use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection")
        .join(path)
}

fn yaak_fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/yaak")
        .join(path)
}

fn probe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_probe"))
}

#[derive(Debug)]
struct CapturedRequest {
    head: String,
    body: Vec<u8>,
}

fn serve_once(body: Vec<u8>, content_type: &str) -> (String, JoinHandle<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("mock server should bind");
    let address = listener.local_addr().unwrap();
    let content_type = content_type.to_owned();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 8 * 1024];
        let header_end = loop {
            if let Some(position) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break position + 4;
            }
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        };
        let head = String::from_utf8_lossy(&request[..header_end]).into_owned();
        let content_length = head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
            .unwrap_or(0);
        while request.len() - header_end < content_length {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
        }
        let captured = CapturedRequest {
            head,
            body: request[header_end..header_end + content_length].to_vec(),
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        captured
    });
    (format!("http://{address}"), handle)
}

fn runtime_fixture(server_url: &str) -> PathBuf {
    let source = fs::read_to_string(fixture("phase5-http.yml")).unwrap();
    let path = temporary_path("workspace.yml");
    fs::write(&path, source.replace("__SERVER_URL__", server_url)).unwrap();
    path
}

fn temporary_path(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "probe-cli-{}-{unique}-{suffix}",
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

fn run_json(arguments: &[&str]) -> Value {
    let output = probe().args(arguments).arg("--json").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

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
    let mut child = probe()
        .args(["request", "list", "-", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("list command should start");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&source)
        .expect("fixture should be written to stdin");
    let output = child
        .wait_with_output()
        .expect("list command should finish");

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
fn set_rejects_stdin_as_read_only() {
    let source = fs::read(fixture("phase1-round-trip.yml")).unwrap();
    let mut child = probe()
        .args([
            "request",
            "set",
            "-",
            "items/0",
            "--url",
            "https://example.com/updated",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("set command should start");
    child.stdin.take().unwrap().write_all(&source).unwrap();
    let output = child.wait_with_output().expect("set command should finish");

    assert_eq!(output.status.code(), Some(7));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["exitCode"], 7);
    assert_eq!(value["error"]["category"], "persistence_read_only");
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
fn reports_environment_and_missing_variable_errors() {
    let missing_environment = probe()
        .args(["request", "get"])
        .arg(fixture("phase4-environments.yml"))
        .arg("items/0")
        .args(["--environment", "production", "--json"])
        .output()
        .expect("get command should run");
    assert_eq!(missing_environment.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&missing_environment.stdout).unwrap();
    assert_eq!(value["error"]["category"], "environment_not_found");

    let missing_variable = probe()
        .args(["request", "get"])
        .arg(fixture("phase4-environments.yml"))
        .arg("items/1")
        .args(["--environment", "development", "--json"])
        .output()
        .expect("get command should run");
    assert_eq!(missing_variable.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&missing_variable.stdout).unwrap();
    assert_eq!(value["error"]["category"], "missing_variable");
}

#[test]
fn reports_unavailable_collection_secret() {
    let output = probe()
        .args(["request", "get"])
        .arg(fixture("phase4-environments.yml"))
        .arg("items/2")
        .args(["--environment", "development", "--json"])
        .output()
        .expect("get command should run");

    assert_eq!(output.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["category"], "secret_variable_unavailable");
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
    let response_body = vec![b'x'; 1024 * 1024 + 1];
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

#[test]
fn structural_cli_commands_share_repository_operations() {
    let workspace = temporary_path("phase16-cli.yml");
    fs::copy(fixture("phase16-bundled.yml"), &workspace).unwrap();
    let path = workspace.to_str().unwrap();

    let folder = run_json(&["folder", "create", path, "--name", "Destination"]);
    assert_eq!(folder["operation"], "create");
    assert_eq!(folder["itemType"], "folder");
    assert_eq!(folder["selector"], "items/2");
    let folders = run_json(&["folder", "list", path]);
    assert_eq!(folders["folders"][0]["selector"], "items/1");
    assert_eq!(folders["folders"][1]["selector"], "items/2");
    assert!(folders["folders"][1]["parent"].is_null());

    let request = run_json(&[
        "request",
        "create",
        path,
        "--parent",
        "items/2",
        "--index",
        "0",
        "--name",
        "CLI Request",
        "--method",
        "PATCH",
        "--url",
        "https://example.com/cli",
    ]);
    assert_eq!(request["selector"], "items/2/items/0");
    let renamed = run_json(&[
        "request",
        "rename",
        path,
        "items/2/items/0",
        "--name",
        "Renamed CLI Request",
    ]);
    assert_eq!(renamed["previousSelector"], "items/2/items/0");

    let moved = run_json(&["request", "move", path, "items/2/items/0", "--index", "0"]);
    assert_eq!(moved["selector"], "items/0");
    let reordered = run_json(&["request", "reorder", path, "items/0", "--index", "2"]);
    assert_eq!(reordered["operation"], "reorder");
    assert_eq!(reordered["selector"], "items/2");
    run_json(&["request", "delete", path, "items/2"]);

    let reordered_folder = run_json(&["folder", "reorder", path, "items/2", "--index", "0"]);
    assert_eq!(reordered_folder["selector"], "items/0");
    let moved_folder = run_json(&[
        "folder", "move", path, "items/0", "--parent", "items/2", "--index", "0",
    ]);
    assert_eq!(moved_folder["selector"], "items/1/items/0");
    let renamed_folder = run_json(&[
        "folder",
        "rename",
        path,
        "items/1/items/0",
        "--name",
        "Renamed Destination",
    ]);
    assert_eq!(renamed_folder["selector"], "items/1/items/0");
    run_json(&["folder", "delete", path, "items/1/items/0"]);

    let saved = fs::read_to_string(&workspace).unwrap();
    assert!(saved.contains("vendor.example"));
    assert!(!saved.contains("Renamed CLI Request"));
    fs::remove_file(workspace).unwrap();
}

#[test]
fn structural_cli_errors_have_stable_categories() {
    let workspace = temporary_path("phase16-cli-errors");
    copy_directory(&fixture("phase16-unbundled"), &workspace);
    let output = probe()
        .args(["request", "create"])
        .arg(&workspace)
        .args(["--name", "Alpha", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["category"], "duplicate_destination");

    let missing = probe()
        .args(["folder", "move"])
        .arg(&workspace)
        .args(["missing", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(4));
    let value: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(value["error"]["category"], "folder_not_found");

    let missing_parent = probe()
        .args(["request", "create"])
        .arg(&workspace)
        .args(["--name", "Child", "--parent", "missing", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing_parent.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&missing_parent.stdout).unwrap();
    assert_eq!(value["error"]["category"], "destination_not_found");

    let invalid_destination = probe()
        .args(["folder", "move"])
        .arg(&workspace)
        .args(["group", "--parent", "group", "--json"])
        .output()
        .unwrap();
    assert_eq!(invalid_destination.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&invalid_destination.stdout).unwrap();
    assert_eq!(value["error"]["category"], "invalid_destination");
    fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn structural_cli_rejects_stdin_as_read_only() {
    let source = fs::read(fixture("phase16-bundled.yml")).unwrap();
    let mut child = probe()
        .args(["request", "create", "-", "--name", "Read only", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("structure command should start");
    child.stdin.take().unwrap().write_all(&source).unwrap();
    let output = child
        .wait_with_output()
        .expect("structure command should finish");

    assert_eq!(output.status.code(), Some(7));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["category"], "persistence_read_only");
}

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
fn environment_set_rejects_stdin_as_read_only() {
    let source = fs::read(fixture("phase4-environments.yml")).unwrap();
    let mut child = probe()
        .args([
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
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("environment set should start");
    child.stdin.take().unwrap().write_all(&source).unwrap();
    let output = child
        .wait_with_output()
        .expect("environment set should finish");

    assert_eq!(output.status.code(), Some(7));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["exitCode"], 7);
    assert_eq!(value["error"]["category"], "persistence_read_only");
}
