//! Golden-file JSON contract for every Facet-owned command (Surface 6), plus
//! the behaviors an agent depends on: recording, the reader rule from the CLI
//! side, read-only SQL, and a dry-run-by-default gc.

#[allow(dead_code, unused_imports)]
mod common;

use common::*;

const ECHO_BODY: &[u8] = br#"{"users":[]}"#;

fn record_one_run(sandbox: &Sandbox, extra: &[&str]) -> Value {
    let (url, server) = serve_once(ECHO_BODY.to_vec(), "application/json");
    let workspace = sandbox.workspace(&url);
    let mut arguments = vec![
        "request",
        "run",
        workspace.to_str().unwrap(),
        "items/0",
        "--environment",
        "local",
    ];
    arguments.extend_from_slice(extra);
    let value = sandbox.run_json(&arguments);
    let sent = server.join().unwrap();
    assert_eq!(sent, br#"{"source":"cli"}"#);
    value
}

#[test]
fn version_json_is_golden_and_names_probe() {
    let sandbox = Sandbox::new();
    let value = sandbox.run_json(&["--version"]);
    assert_eq!(value["name"], "facet");
    assert_golden("version.json", &normalize(value));
}

#[test]
fn request_run_keeps_the_upstream_envelope_and_records() {
    let sandbox = Sandbox::new();
    let value = record_one_run(&sandbox, &[]);

    // Upstream fields, byte for byte in shape.
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["response"]["status"], 200);
    assert_eq!(value["response"]["body"]["content"], r#"{"users":[]}"#);
    assert_eq!(value["response"]["body"]["omitted"], false);
    // Facet's additive field.
    assert_eq!(value["lattice"]["recorded"], true);
    assert_eq!(value["lattice"]["indexed"], true);
    assert_eq!(value["lattice"]["responseBody"]["retention"], "inline");
    assert_golden("run.json", &normalize(value));

    let facet = sandbox.root().join(".facet");
    assert!(facet.join("lattice.db").is_file());
    assert!(facet.join("workspace.toml").is_file());
    assert!(facet.join(".gitignore").is_file());
    assert!(sandbox.root().join("machine/lattice.db").is_file());
}

#[test]
fn history_json_is_golden_and_carries_no_bodies_by_default() {
    let sandbox = Sandbox::new();
    let run = record_one_run(&sandbox, &[]);

    let history = sandbox.run_json(&["history", sandbox.root().to_str().unwrap()]);
    assert_eq!(history["runs"].as_array().unwrap().len(), 1);
    let row = &history["runs"][0];
    assert_eq!(row["id"], run["lattice"]["runId"]);
    assert_eq!(row["requestPath"], "items/0");
    assert_eq!(row["environment"], "local");
    assert_eq!(row["status"], 200);
    assert_eq!(row["actor"], "human");
    assert_eq!(row["response"]["body"]["sizeBytes"], 12);
    assert!(row["response"]["body"].get("content").is_none());
    assert_golden("history.json", &normalize(history));

    // Payloads are explicit pulls.
    let with_bodies = sandbox.run_json(&["history", sandbox.root().to_str().unwrap(), "--bodies"]);
    assert_eq!(
        with_bodies["runs"][0]["response"]["body"]["content"],
        r#"{"users":[]}"#
    );
    assert_eq!(
        with_bodies["runs"][0]["request"]["body"]["content"],
        r#"{"source":"cli"}"#
    );
    assert_golden("history_bodies.json", &normalize(with_bodies));

    // Filters.
    let none = sandbox.run_json(&[
        "history",
        sandbox.root().to_str().unwrap(),
        "--status",
        "500",
    ]);
    assert_eq!(none["runs"].as_array().unwrap().len(), 0);
}

#[test]
fn history_without_a_store_is_empty_not_an_error() {
    let sandbox = Sandbox::new();
    let history = sandbox.run_json(&["history", sandbox.root().to_str().unwrap()]);
    assert!(history["workspace"].is_null());
    assert_eq!(history["runs"], json!([]));
}

#[test]
fn history_sql_is_golden_and_read_only() {
    let sandbox = Sandbox::new();
    record_one_run(&sandbox, &["--tag", "smoke"]);
    let root = sandbox.root().to_str().unwrap();

    let result = sandbox.run_json(&[
        "history",
        root,
        "--sql",
        "SELECT request_path, status, actor, tags FROM runs",
    ]);
    assert_eq!(
        result["columns"],
        json!(["request_path", "status", "actor", "tags"])
    );
    assert_eq!(
        result["rows"][0],
        json!(["items/0", 200, "human", "[\"smoke\"]"])
    );
    assert_golden("history_sql.json", &normalize(result));

    let write = sandbox
        .facet()
        .args(["history", root, "--sql", "DELETE FROM runs", "--json"])
        .output()
        .unwrap();
    assert_eq!(write.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&write.stdout).unwrap();
    assert_eq!(error["error"]["category"], "sql_read_only");

    let bad = sandbox
        .facet()
        .args(["history", root, "--sql", "SELEC nope", "--json"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&bad.stdout).unwrap();
    assert_eq!(error["error"]["category"], "invalid_sql");

    let still_there = sandbox.run_json(&["history", root, "--sql", "SELECT count(*) FROM runs"]);
    assert_eq!(still_there["rows"][0][0], 1);
}

#[test]
fn history_sql_without_a_store_is_lattice_not_found() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .facet()
        .args([
            "history",
            sandbox.root().to_str().unwrap(),
            "--sql",
            "SELECT 1",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(9));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["category"], "lattice_not_found");
    assert_eq!(error["error"]["exitCode"], 9);
}

#[test]
fn blob_json_is_golden_and_raw_mode_streams_bytes() {
    let sandbox = Sandbox::new();
    // A zero threshold makes every body a blob.
    let run = record_one_run(&sandbox, &["--inline-body-max", "0"]);
    assert_eq!(run["lattice"]["responseBody"]["retention"], "blob");
    let hash = run["lattice"]["responseBody"]["hash"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(hash.len(), 64);
    let root = sandbox.root().to_str().unwrap();
    assert!(sandbox.root().join(".facet/blobs").join(&hash).is_file());

    let blob = sandbox.run_json(&["blob", &hash, root]);
    assert_eq!(blob["blob"]["sizeBytes"], 12);
    assert_eq!(blob["blob"]["contentType"], "application/json");
    assert_eq!(blob["blob"]["body"]["content"], r#"{"users":[]}"#);
    assert_golden("blob.json", &normalize(blob));

    let raw = sandbox
        .facet()
        .args(["blob", &hash, root])
        .output()
        .unwrap();
    assert!(raw.status.success());
    assert_eq!(raw.stdout, ECHO_BODY);

    let target = sandbox.root().join("body.json");
    let written = sandbox.run_json(&["blob", &hash, root, "--output", target.to_str().unwrap()]);
    assert_eq!(written["blob"]["body"]["content"], Value::Null);
    assert_eq!(fs::read(&target).unwrap(), ECHO_BODY);

    // History never carries a blob body inline, even when asked.
    let history = sandbox.run_json(&["history", root, "--bodies"]);
    let body = &history["runs"][0]["response"]["body"];
    assert_eq!(body["retention"], "blob");
    assert_eq!(body["hash"], hash);
    assert_eq!(body["omitted"], true);
    assert_eq!(body["omissionReason"], "blob");
}

#[test]
fn blob_not_found_uses_exit_code_4() {
    let sandbox = Sandbox::new();
    record_one_run(&sandbox, &[]);
    let missing = "0".repeat(64);
    let output = sandbox
        .facet()
        .args(["blob", &missing, sandbox.root().to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["category"], "blob_not_found");
}

#[test]
fn gc_is_a_dry_run_unless_yes() {
    let sandbox = Sandbox::new();
    record_one_run(&sandbox, &["--inline-body-max", "0"]);
    let root = sandbox.root().to_str().unwrap();
    let blobs = sandbox.root().join(".facet/blobs");
    let orphan = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    fs::write(blobs.join(orphan), b"abc").unwrap();

    let dry = sandbox.run_json(&["gc", root]);
    assert_eq!(dry["applied"], false);
    assert_eq!(dry["orphans"][0]["hash"], orphan);
    assert_eq!(dry["bytesReclaimable"], 3);
    assert!(blobs.join(orphan).is_file());

    let applied = sandbox.run_json(&["gc", root, "--yes"]);
    assert_eq!(applied["applied"], true);
    assert!(!blobs.join(orphan).exists());
    // Threshold 0 blobbed both request and response; gc must keep both referenced files.
    assert_eq!(
        fs::read_dir(&blobs).unwrap().count(),
        2,
        "referenced req and res blobs survive"
    );

    let clean = sandbox.run_json(&["gc", root]);
    assert_eq!(clean["orphans"], json!([]));
    assert_golden("gc.json", &normalize(clean));
}

#[test]
fn no_record_flag_skips_lattice() {
    let sandbox = Sandbox::new();
    let value = record_one_run(&sandbox, &["--no-record"]);
    assert_eq!(value["lattice"]["recorded"], false);
    assert_eq!(value["lattice"]["reason"], "disabled");
    assert!(!sandbox.root().join(".facet").exists());
}

#[test]
fn actor_and_tags_are_recorded() {
    let sandbox = Sandbox::new();
    let (url, server) = serve_once(ECHO_BODY.to_vec(), "application/json");
    let workspace = sandbox.workspace(&url);
    let output = sandbox
        .facet()
        .env("FACET_ACTOR", "halo-qa")
        .args([
            "request",
            "run",
            workspace.to_str().unwrap(),
            "items/0",
            "--environment",
            "local",
            "--tag",
            "a",
            "--tag",
            "b",
            "--json",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(output.status.success());

    let history = sandbox.run_json(&[
        "history",
        sandbox.root().to_str().unwrap(),
        "--actor",
        "halo-qa",
    ]);
    assert_eq!(history["runs"][0]["actor"], "halo-qa");
    assert_eq!(history["runs"][0]["tags"], json!(["a", "b"]));
}

#[test]
fn missing_selector_matches_upstream_category_and_exit_code() {
    let sandbox = Sandbox::new();
    let workspace = sandbox.workspace("http://127.0.0.1:9");
    let output = sandbox
        .facet()
        .args([
            "request",
            "run",
            workspace.to_str().unwrap(),
            "items/99",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["category"], "request_not_found");
    assert!(!sandbox.root().join(".facet").exists());
}

#[test]
fn transport_failure_is_recorded_with_null_status() {
    let sandbox = Sandbox::new();
    // Port 9 (discard) is closed on a normal host; the connection is refused.
    let workspace = sandbox.workspace("http://127.0.0.1:9");
    let output = sandbox
        .facet()
        .args([
            "request",
            "run",
            workspace.to_str().unwrap(),
            "items/0",
            "--environment",
            "local",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(6));
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["category"], "network_execution");
    assert_eq!(error["error"]["details"]["lattice"]["recorded"], true);

    let history = sandbox.run_json(&["history", sandbox.root().to_str().unwrap()]);
    let row = &history["runs"][0];
    assert!(row["status"].is_null());
    assert!(row["error"].as_str().unwrap().len() > 0);
    assert_eq!(row["response"]["body"]["retention"], "none");
}

#[test]
fn delegated_commands_are_untouched() {
    let sandbox = Sandbox::new();
    let workspace = sandbox.workspace("http://127.0.0.1:9");
    let value = sandbox.run_json(&["request", "list", workspace.to_str().unwrap()]);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["requests"][0]["selector"], "items/0");
    assert!(!sandbox.root().join(".facet").exists());
}
