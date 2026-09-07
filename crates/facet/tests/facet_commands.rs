//! Golden-file JSON contract for every Facet-owned command (Surface 6), plus
//! the behaviors an agent depends on: recording, the reader rule from the CLI
//! side, read-only SQL, and a dry-run-by-default gc.

#[allow(dead_code, unused_imports)]
mod common;

use common::*;

const ECHO_BODY: &[u8] = br#"{"users":[]}"#;

fn record_one_run(sandbox: &Sandbox, extra: &[&str]) -> Value {
    record_run_with(sandbox, &[], extra)
}

/// A run whose mock server answers with `body` instead of [`ECHO_BODY`].
fn record_run_body(sandbox: &Sandbox, body: &[u8]) -> Value {
    let (url, server) = serve_once(body.to_vec(), "application/json");
    let workspace = sandbox.workspace(&url);
    let value = sandbox.run_json(&[
        "request",
        "run",
        workspace.to_str().unwrap(),
        "items/0",
        "--environment",
        "local",
    ]);
    server.join().unwrap();
    value
}

/// `request run … --json` against a one-shot echo server, with extra
/// environment variables (`FACET_SESSION`, `FACET_ACTOR`) as a harness sets them.
fn record_run_with(sandbox: &Sandbox, env: &[(&str, &str)], extra: &[&str]) -> Value {
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
    let mut command = sandbox.facet();
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.args(&arguments).arg("--json").output().unwrap();
    let sent = server.join().unwrap();
    assert_eq!(sent, br#"{"source":"cli"}"#);
    assert!(
        output.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout should be JSON")
}

fn run_count(sandbox: &Sandbox, arguments: &[&str]) -> usize {
    sandbox.run_json(arguments)["runs"]
        .as_array()
        .expect("runs array")
        .len()
}

/// A well-formed ULID no store has ever minted.
const UNKNOWN_ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

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

    // Payloads are explicit pulls. Response bodies under the inline
    // threshold hydrate via --bodies; request bodies are hash-only (v2), so
    // --bodies shows the hash and omits the content (hydrate via `facet blob`).
    let with_bodies = sandbox.run_json(&["history", sandbox.root().to_str().unwrap(), "--bodies"]);
    assert_eq!(
        with_bodies["runs"][0]["response"]["body"]["content"],
        r#"{"users":[]}"#
    );
    assert_eq!(
        with_bodies["runs"][0]["request"]["body"]["retention"],
        "blob"
    );
    assert!(with_bodies["runs"][0]["request"]["body"]["content"].is_null());
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

    let (write_code, write_error) =
        sandbox.run_error_json(&["history", root, "--sql", "DELETE FROM runs"]);
    assert_eq!(write_code, 2);
    assert_eq!(write_error["error"]["category"], "sql_read_only");
    assert_golden("error_sql_read_only.json", &normalize_error(write_error));

    let (bad_code, bad_error) = sandbox.run_error_json(&["history", root, "--sql", "SELEC nope"]);
    assert_eq!(bad_code, 2);
    assert_eq!(bad_error["error"]["category"], "invalid_sql");
    assert_golden("error_invalid_sql.json", &normalize_error(bad_error));

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
fn request_body_reader_rule_at_cli() {
    let sandbox = Sandbox::new();
    record_one_run(&sandbox, &["--inline-body-max", "0"]);
    let root = sandbox.root().to_str().unwrap();

    let history = sandbox.run_json(&["history", root]);
    let req_hash = history["runs"][0]["request"]["body"]["hash"]
        .as_str()
        .expect("request body hash when blobbed");
    assert_eq!(req_hash.len(), 64);
    assert!(sandbox.root().join(".facet/blobs").join(req_hash).is_file());

    let blob = sandbox.run_json(&["blob", req_hash, root]);
    assert_eq!(blob["blob"]["sizeBytes"], 16);
    assert_eq!(blob["blob"]["body"]["content"], r#"{"source":"cli"}"#);
    assert_golden("blob_request.json", &normalize(blob));

    let with_bodies = sandbox.run_json(&["history", root, "--bodies"]);
    let req = &with_bodies["runs"][0]["request"]["body"];
    assert_eq!(req["retention"], "blob");
    assert_eq!(req["hash"], req_hash);
    assert_eq!(req["omitted"], true);
    assert_eq!(req["omissionReason"], "blob");
    assert!(req["content"].is_null());
}

#[test]
fn blob_not_found_uses_exit_code_4() {
    let sandbox = Sandbox::new();
    record_one_run(&sandbox, &[]);
    let missing = "0".repeat(64);
    let (code, error) =
        sandbox.run_error_json(&["blob", &missing, sandbox.root().to_str().unwrap()]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "blob_not_found");
    assert_golden("error_blob_not_found.json", &normalize_error(error));
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
fn gc_expires_runs_by_retention_at_cli() {
    let sandbox = Sandbox::new();
    let first = record_one_run(&sandbox, &["--inline-body-max", "0"]);
    let run_id = first["lattice"]["runId"].as_str().expect("recorded run id");
    sandbox.backdate_run(run_id, lattice::now_ms() - 3 * 86_400_000);
    record_one_run(&sandbox, &["--inline-body-max", "0"]);
    let root = sandbox.root().to_str().unwrap();

    let stamps = sandbox.run_json(&[
        "history",
        root,
        "--sql",
        "SELECT started_at FROM runs ORDER BY started_at ASC",
    ]);
    assert_eq!(stamps["rows"].as_array().unwrap().len(), 2);
    let gap = stamps["rows"][1][0].as_i64().unwrap() - stamps["rows"][0][0].as_i64().unwrap();
    assert!(
        gap > 86_400_000,
        "backdated run should be more than a day older"
    );

    let dry = sandbox.run_json(&["gc", root, "--history-retention", "1d"]);
    assert_eq!(dry["applied"], false);
    assert_eq!(dry["retention"], "1d");
    assert_eq!(dry["runsExpired"], 1);
    // Identical bodies share blob files; the fresh run keeps them referenced.
    assert_eq!(dry["orphans"].as_array().unwrap().len(), 0);
    assert_golden("gc_retention_expired.json", &normalize(dry));

    let applied = sandbox.run_json(&["gc", root, "--history-retention", "1d", "--yes"]);
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["runsExpired"], 1);

    let history = sandbox.run_json(&["history", root]);
    assert_eq!(history["runs"].as_array().unwrap().len(), 1);
    assert_eq!(
        fs::read_dir(sandbox.root().join(".facet/blobs"))
            .unwrap()
            .count(),
        2,
        "fresh run keeps req and res blobs"
    );
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

#[test]
fn session_lifecycle_is_golden_and_start_prints_a_bare_ulid() {
    let sandbox = Sandbox::new();

    // Human mode prints the id alone so `$(facet session start)` needs no jq.
    let output = sandbox.facet().args(["session", "start"]).output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let bare = String::from_utf8(output.stdout).unwrap();
    assert!(bare.ends_with('\n'));
    let bare_id = bare.trim().to_owned();
    assert!(lattice::is_ulid(&bare_id), "not a ULID: {bare_id:?}");

    let started = sandbox.run_json(&[
        "session",
        "start",
        "--actor",
        "halo-qa",
        "--meta",
        r#"{"note":"golden"}"#,
    ]);
    let session = &started["session"];
    assert!(lattice::is_ulid(session["id"].as_str().unwrap()));
    assert_eq!(session["actor"], "halo-qa");
    assert!(session["startedAt"].is_number());
    assert!(session["endedAt"].is_null());
    assert_eq!(session["meta"]["note"], "golden");
    assert!(session["meta"]["cwd"].is_string());
    assert!(session["meta"].get("herdr").is_none(), "env is scrubbed");
    assert!(session.get("runs").is_none(), "no workspace store yet");
    assert_golden("session_start.json", &normalize(started.clone()));
    let id = session["id"].as_str().unwrap().to_owned();

    let shown = sandbox.run_json(&["session", "show", &id]);
    assert_eq!(shown["session"], started["session"]);
    let current = sandbox.run_json_in_session(&id, &["session", "show", "current"]);
    assert_eq!(current["session"]["id"], id);

    let open = sandbox.run_json(&["session", "list", "--open"]);
    let open_ids: Vec<&str> = open["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(open_ids.len(), 2);
    assert!(open_ids.contains(&id.as_str()) && open_ids.contains(&bare_id.as_str()));
    let by_actor = sandbox.run_json(&["session", "list", "--actor", "halo-qa"]);
    assert_eq!(by_actor["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(by_actor["sessions"][0]["id"], id);
    assert_golden("session_list.json", &normalize(by_actor));

    let ended = sandbox.run_json(&["session", "end", &id]);
    assert_eq!(ended["alreadyEnded"], false);
    assert!(ended["session"]["endedAt"].is_number());
    assert_golden("session_end.json", &normalize(ended.clone()));
    // Idempotent: the second end changes nothing and says so.
    let again = sandbox.run_json(&["session", "end", &id]);
    assert_eq!(again["alreadyEnded"], true);
    assert_eq!(again["session"]["endedAt"], ended["session"]["endedAt"]);
    // No id: FACET_SESSION.
    let from_env = sandbox.run_json_in_session(&bare_id, &["session", "end"]);
    assert_eq!(from_env["session"]["id"], bare_id);
    assert_eq!(from_env["alreadyEnded"], false);
    let none_open = sandbox.run_json(&["session", "list", "--open"]);
    assert_eq!(none_open["sessions"], json!([]));
    assert_eq!(
        sandbox.run_json(&["session", "list"])["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let (code, error) = sandbox.run_error_json(&["session", "end"]);
    assert_eq!(code, 5);
    assert_eq!(error["error"]["category"], "session_not_set");
    let (code, error) = sandbox.run_error_json(&["session", "show", UNKNOWN_ULID]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "session_not_found");
    assert_golden("error_session_not_found.json", &normalize_error(error));
    let (code, error) = sandbox.run_error_json(&["session", "end", UNKNOWN_ULID]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "session_not_found");
    let (code, error) = sandbox.run_error_json(&["session", "start", "--meta", "[1]"]);
    assert_eq!(code, 2);
    assert_eq!(error["error"]["category"], "invalid_arguments");
    let (code, _) = sandbox.run_error_json(&["session", "frobnicate"]);
    assert_eq!(code, 2);
}

#[test]
fn history_id_is_golden_and_exclusive_with_filters() {
    let sandbox = Sandbox::new();
    let run = record_one_run(&sandbox, &[]);
    let run_id = run["lattice"]["runId"].as_str().unwrap();
    let root = sandbox.root().to_str().unwrap();

    let one = sandbox.run_json(&["history", root, "--id", run_id]);
    assert!(one.get("runs").is_none(), "--id is one object, not a list");
    assert_eq!(one["run"]["id"], run_id);
    assert_eq!(one["run"]["requestPath"], "items/0");
    assert_eq!(one["workspace"]["id"], run["lattice"]["workspaceId"]);
    assert!(one["run"]["response"]["body"].get("content").is_none());
    assert_golden("history_id.json", &normalize(one));

    // --bodies behaves exactly as on the list.
    let with_bodies = sandbox.run_json(&["history", root, "--id", run_id, "--bodies"]);
    assert_eq!(
        with_bodies["run"]["response"]["body"]["content"],
        r#"{"users":[]}"#
    );

    let human = sandbox
        .facet()
        .args(["history", root, "--id", run_id])
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8(human.stdout).unwrap();
    assert_eq!(text.lines().count(), 2, "header plus one row");
    assert!(text.lines().nth(1).unwrap().starts_with(run_id));

    let (code, error) = sandbox.run_error_json(&["history", root, "--id", UNKNOWN_ULID]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "run_not_found");
    assert_golden("error_run_not_found.json", &normalize_error(error));

    let (code, error) =
        sandbox.run_error_json(&["history", root, "--id", run_id, "--status", "200"]);
    assert_eq!(code, 2);
    assert_eq!(error["error"]["category"], "invalid_arguments");
    let (code, _) = sandbox.run_error_json(&["history", root, "--id", run_id, "--tag", "x"]);
    assert_eq!(code, 2);
    let (code, _) = sandbox.run_error_json(&["history", root, "--id", run_id, "--sql", "SELECT 1"]);
    assert_eq!(code, 2);

    // No store at all: the run cannot exist.
    let empty = Sandbox::new();
    let (code, error) =
        empty.run_error_json(&["history", empty.root().to_str().unwrap(), "--id", run_id]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "run_not_found");
}

#[test]
fn history_session_filter_and_mint_if_missing() {
    let sandbox = Sandbox::new();
    let root = sandbox.root().to_str().unwrap();
    let started = sandbox.run_json(&["session", "start", "--actor", "claude.halo-fullstack"]);
    let id = started["session"]["id"].as_str().unwrap().to_owned();

    let first = record_run_with(&sandbox, &[("FACET_SESSION", &id)], &[]);
    let second = record_run_with(&sandbox, &[("FACET_SESSION", &id)], &["--tag", "smoke"]);
    let outside = record_one_run(&sandbox, &[]);
    assert_eq!(run_count(&sandbox, &["history", root]), 3);

    let in_session = sandbox.run_json(&["history", root, "--session", &id]);
    let rows = in_session["runs"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row["sessionId"] == id));
    let ids: Vec<&Value> = rows.iter().map(|row| &row["id"]).collect();
    assert!(ids.contains(&&first["lattice"]["runId"]));
    assert!(ids.contains(&&second["lattice"]["runId"]));
    assert!(!ids.contains(&&outside["lattice"]["runId"]));
    assert_golden("history_session.json", &normalize(in_session));

    // `current` reads FACET_SESSION; without it the agent gets a clear exit 5.
    let current = sandbox.run_json_in_session(&id, &["history", root, "--session", "current"]);
    assert_eq!(current["runs"].as_array().unwrap().len(), 2);
    let (code, error) = sandbox.run_error_json(&["history", root, "--session", "current"]);
    assert_eq!(code, 5);
    assert_eq!(error["error"]["category"], "session_not_set");
    assert_golden("error_session_not_set.json", &normalize_error(error));

    // With a workspace store under the cwd, sessions report their run count.
    let shown = sandbox.run_json(&["session", "show", &id]);
    assert_eq!(shown["session"]["runs"], 2);
    let listed = sandbox.run_json(&["session", "list"]);
    assert_eq!(listed["sessions"][0]["runs"], 2);

    // Mint-if-missing: a harness that invents its own FACET_SESSION gets a
    // session row on the first recorded run, owned by that run's actor.
    let (code, error) = sandbox.run_error_json(&["session", "show", UNKNOWN_ULID]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "session_not_found");
    record_run_with(
        &sandbox,
        &[("FACET_SESSION", UNKNOWN_ULID), ("FACET_ACTOR", "halo-qa")],
        &[],
    );
    let minted = sandbox.run_json(&["session", "show", UNKNOWN_ULID]);
    assert_eq!(minted["session"]["id"], UNKNOWN_ULID);
    assert_eq!(minted["session"]["actor"], "halo-qa");
    assert!(minted["session"]["endedAt"].is_null());
    assert!(minted["session"]["meta"].is_null());
    assert_eq!(minted["session"]["runs"], 1);
    // A later run under the same id does not rewrite the row.
    record_run_with(
        &sandbox,
        &[
            ("FACET_SESSION", UNKNOWN_ULID),
            ("FACET_ACTOR", "someone-else"),
        ],
        &[],
    );
    let again = sandbox.run_json(&["session", "show", UNKNOWN_ULID]);
    assert_eq!(again["session"]["actor"], "halo-qa");
    assert_eq!(
        again["session"]["startedAt"],
        minted["session"]["startedAt"]
    );
    assert_eq!(again["session"]["runs"], 2);
    assert_golden("session_show.json", &normalize(again));
    assert_eq!(
        run_count(&sandbox, &["history", root, "--session", UNKNOWN_ULID]),
        2
    );
}

#[test]
fn history_environment_tag_and_hash_filters() {
    let sandbox = Sandbox::new();
    let tagged = record_one_run(
        &sandbox,
        &[
            "--tag",
            "smoke",
            "--tag",
            "golden",
            "--inline-body-max",
            "0",
        ],
    );
    record_one_run(&sandbox, &[]);
    let root = sandbox.root().to_str().unwrap();
    let tagged_id = tagged["lattice"]["runId"].as_str().unwrap();
    let response_hash = tagged["lattice"]["responseBody"]["hash"].as_str().unwrap();
    let request_hash = tagged["lattice"]["requestHash"].as_str().unwrap();

    assert_eq!(
        run_count(&sandbox, &["history", root, "--environment", "local"]),
        2
    );
    assert_eq!(
        run_count(&sandbox, &["history", root, "--environment", "prod"]),
        0
    );

    // --tag repeats and ANDs; untagged runs never match.
    assert_eq!(run_count(&sandbox, &["history", root, "--tag", "smoke"]), 1);
    assert_eq!(
        run_count(
            &sandbox,
            &["history", root, "--tag", "smoke", "--tag", "golden"]
        ),
        1
    );
    assert_eq!(
        run_count(
            &sandbox,
            &["history", root, "--tag", "smoke", "--tag", "nope"]
        ),
        0
    );

    // --hash is column-agnostic; the row says which column hit.
    let by_response =
        sandbox.run_json(&["history", root, "--tag", "smoke", "--hash", response_hash]);
    assert_eq!(by_response["runs"].as_array().unwrap().len(), 1);
    assert_eq!(by_response["runs"][0]["id"], tagged_id);
    assert_eq!(by_response["runs"][0]["matchedHash"], "responseBody");
    assert_golden("history_tag_hash.json", &normalize(by_response));

    // Each run hit a fresh mock server (different port, different URL), so
    // the resolved-request hash is unique per run; the request body bytes
    // are identical, so the request-body hash hits both.
    let by_request = sandbox.run_json(&["history", root, "--hash", request_hash]);
    let rows = by_request["runs"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], tagged_id);
    assert_eq!(rows[0]["matchedHash"], "request");
    let request_body_hash = rows[0]["request"]["body"]["hash"].as_str().unwrap();
    let by_request_body = sandbox.run_json(&["history", root, "--hash", request_body_hash]);
    let rows = by_request_body["runs"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row["matchedHash"] == "requestBody"));

    // Hashes compare case-insensitively, like `blob`.
    let upper = sandbox.run_json(&["history", root, "--hash", &response_hash.to_uppercase()]);
    assert_eq!(upper["runs"].as_array().unwrap().len(), 1);
    let (code, _) =
        sandbox.run_error_json(&["history", root, "--hash", "nope", "--sql", "SELECT 1"]);
    assert_eq!(code, 2);

    // Without --hash the row shape is unchanged.
    let plain = sandbox.run_json(&["history", root]);
    assert!(plain["runs"][0].get("matchedHash").is_none());
    assert_eq!(
        run_count(&sandbox, &["history", root, "--hash", &"0".repeat(64)]),
        0
    );
}

#[test]
fn replay_uses_current_yaml_and_records_lineage() {
    let sandbox = Sandbox::new();
    let (url, server) = serve_once(ECHO_BODY.to_vec(), "application/json");
    let workspace = sandbox.workspace(&url);
    let ws = workspace.to_str().unwrap();
    let original = sandbox.run_json(&[
        "request",
        "run",
        ws,
        "items/0",
        "--environment",
        "local",
        "--tag",
        "smoke",
    ]);
    server.join().unwrap();
    let run_id = original["lattice"]["runId"].as_str().unwrap().to_owned();

    // Same URL live again: the current YAML resolves to the recorded hash.
    let again = serve_again(&url, ECHO_BODY.to_vec());
    let replayed = sandbox.run_json(&["replay", &run_id, ws, "--tag", "again"]);
    assert_eq!(again.join().unwrap(), br#"{"source":"cli"}"#);
    assert_eq!(replayed["response"]["status"], 200);
    assert_eq!(replayed["lattice"]["recorded"], true);
    assert_eq!(replayed["lattice"]["replayedFrom"], run_id);
    assert_eq!(replayed["lattice"]["hashChanged"], false);
    assert_eq!(
        replayed["lattice"]["requestHash"],
        original["lattice"]["requestHash"]
    );
    assert_golden("replay.json", &normalize(replayed.clone()));
    let replay_id = replayed["lattice"]["runId"].as_str().unwrap().to_owned();

    let row = sandbox.run_json(&["history", ws, "--id", &replay_id]);
    assert_eq!(row["run"]["replayedFrom"], run_id);
    assert_eq!(row["run"]["tags"], json!(["smoke", "again"]));
    assert_eq!(row["run"]["varNames"], json!([]));
    assert_eq!(row["run"]["environment"], "local");
    let source = sandbox.run_json(&["history", ws, "--id", &run_id]);
    assert!(source["run"]["replayedFrom"].is_null());
    let lineage = sandbox.run_json(&[
        "history",
        ws,
        "--sql",
        &format!("SELECT count(*) FROM runs WHERE replayed_from = '{run_id}'"),
    ]);
    assert_eq!(lineage["rows"][0][0], 1);

    // Human mode names the lineage.
    let human = sandbox
        .facet()
        .args(["replay", &run_id, ws, "--no-record"])
        .output();
    // (no server: transport failure is fine, we only care that it parsed)
    assert!(human.is_ok());

    // A changed request: --var moves the URL, so the hash differs. --frozen
    // refuses before any network and writes no row.
    let (moved_url, moved) = serve_once(ECHO_BODY.to_vec(), "application/json");
    let var = format!("serverUrl={moved_url}");
    let before = run_count(&sandbox, &["history", ws]);
    let (code, error) = sandbox.run_error_json(&["replay", &run_id, ws, "--var", &var, "--frozen"]);
    assert_eq!(code, 1);
    assert_eq!(error["error"]["category"], "replay_changed");
    assert_eq!(error["error"]["details"]["replayedFrom"], run_id);
    assert_ne!(
        error["error"]["details"]["recordedHash"],
        error["error"]["details"]["currentHash"]
    );
    assert_golden("error_replay_changed.json", &normalize_error(error));
    assert_eq!(
        run_count(&sandbox, &["history", ws]),
        before,
        "no row on refuse"
    );

    // Without --frozen it sends, warns, and records varNames (names only).
    let output = sandbox
        .facet()
        .args(["replay", &run_id, ws, "--var", &var, "--json"])
        .output()
        .unwrap();
    moved.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("warning: request changed since"),
        "{stderr}"
    );
    let changed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(changed["lattice"]["hashChanged"], true);
    assert_eq!(changed["lattice"]["replayedFrom"], run_id);
    let changed_id = changed["lattice"]["runId"].as_str().unwrap().to_owned();
    let changed_row = sandbox.run_json(&["history", ws, "--id", &changed_id]);
    assert_eq!(changed_row["run"]["varNames"], json!(["serverUrl"]));
    let dump = sandbox.run_json(&["history", ws, "--sql", "SELECT var_names FROM runs"]);
    let text = dump.to_string();
    assert!(
        !text.contains(&moved_url),
        "--var values must never persist"
    );

    // Replaying the changed run without its --var warns about the names.
    let again = serve_again(&url, ECHO_BODY.to_vec());
    let output = sandbox
        .facet()
        .args(["replay", &changed_id, ws, "--json"])
        .output()
        .unwrap();
    again.join().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("was recorded with --var serverUrl"),
        "{stderr}"
    );
    assert!(stderr.contains("request changed since"), "{stderr}");

    // Unknown run, and a run that lives in another workspace (breadcrumb).
    let (code, error) = sandbox.run_error_json(&["replay", UNKNOWN_ULID, ws]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "run_not_found");
    assert!(error["error"].get("details").is_none());
    let other_dir = sandbox.root().join("other");
    fs::create_dir_all(&other_dir).unwrap();
    let source = fs::read_to_string(fixture("phase5-http.yml")).unwrap();
    let other_ws = other_dir.join("workspace.yml");
    let (other_url, other_server) = serve_once(ECHO_BODY.to_vec(), "application/json");
    fs::write(&other_ws, source.replace("__SERVER_URL__", &other_url)).unwrap();
    let other_run = sandbox.run_json(&[
        "request",
        "run",
        other_ws.to_str().unwrap(),
        "items/0",
        "--environment",
        "local",
    ]);
    other_server.join().unwrap();
    let other_id = other_run["lattice"]["runId"].as_str().unwrap();
    let (code, error) = sandbox.run_error_json(&["replay", other_id, ws]);
    assert_eq!(code, 4);
    assert_eq!(
        error["error"]["details"]["workspaceId"],
        other_run["lattice"]["workspaceId"]
    );
    assert_golden("error_replay_run_elsewhere.json", &normalize_error(error));

    // No store beside the collection at all.
    let empty = Sandbox::new();
    let empty_ws = empty.workspace("http://127.0.0.1:9");
    let (code, error) = empty.run_error_json(&["replay", &run_id, empty_ws.to_str().unwrap()]);
    assert_eq!(code, 9);
    assert_eq!(error["error"]["category"], "lattice_not_found");
    let (code, _) = sandbox.run_error_json(&["replay"]);
    assert_eq!(code, 2);
}

#[test]
fn diff_is_hash_first_and_exit_1_when_different() {
    let sandbox = Sandbox::new();
    let (url, server) = serve_once(ECHO_BODY.to_vec(), "application/json");
    let workspace = sandbox.workspace(&url);
    let ws = workspace.to_str().unwrap();
    let a = sandbox.run_json(&["request", "run", ws, "items/0", "--environment", "local"]);
    server.join().unwrap();
    let id_a = a["lattice"]["runId"].as_str().unwrap().to_owned();
    let again = serve_again(&url, ECHO_BODY.to_vec());
    let b = sandbox.run_json(&["replay", &id_a, ws, "--tag", "retry"]);
    again.join().unwrap();
    let id_b = b["lattice"]["runId"].as_str().unwrap().to_owned();
    sandbox.set_duration_ms(&id_a, 10);
    sandbox.set_duration_ms(&id_b, 20);

    // Same request, same bytes: equal, even though duration and tags
    // (provenance) differ; both are still reported.
    let equal = sandbox.run_json(&["diff", &id_a, &id_b, ws]);
    assert_eq!(equal["equal"], true);
    assert_eq!(
        equal["changes"],
        json!([
            { "field": "durationMs", "a": 10, "b": 20 },
            { "field": "tags", "a": [], "b": ["retry"] },
        ])
    );
    assert_eq!(equal["request"]["hash"]["equal"], true);
    assert_eq!(equal["request"]["body"]["equal"], true);
    assert_eq!(equal["response"]["body"]["equal"], true);
    assert_eq!(equal["response"]["body"]["a"]["retention"], "inline");
    assert!(
        equal["response"]["body"]["a"]["hash"].is_string(),
        "inline bodies are hashed"
    );
    assert_golden("diff_equal.json", &normalize(equal));
    let status = sandbox
        .facet()
        .args(["diff", &id_a, &id_b, ws])
        .output()
        .unwrap();
    assert_eq!(status.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&status.stdout).contains("response body: equal"));

    // A different response (and a different port, so url + requestHash move).
    let c = record_run_body(&sandbox, br#"{"users":[1]}"#);
    let id_c = c["lattice"]["runId"].as_str().unwrap().to_owned();
    sandbox.set_duration_ms(&id_c, 30);
    let output = sandbox
        .facet()
        .args(["diff", &id_a, &id_c, ws, "--bodies", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let changed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(changed["equal"], false);
    let fields: Vec<&str> = changed["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["field"].as_str().unwrap())
        .collect();
    assert!(
        fields.contains(&"url") && fields.contains(&"requestHash"),
        "{fields:?}"
    );
    assert!(
        fields.contains(&"response.headers"),
        "content-length moved: {fields:?}"
    );
    assert!(!fields.contains(&"status"));
    assert_eq!(changed["request"]["hash"]["equal"], false);
    assert_eq!(
        changed["request"]["body"]["equal"], true,
        "same request body blob"
    );
    assert_eq!(changed["response"]["body"]["equal"], false);
    let text = changed["response"]["body"]["text"]
        .as_str()
        .expect("unified diff");
    assert!(text.starts_with("--- a\n+++ b\n@@ "), "{text}");
    assert!(
        text.contains("-{\"users\":[]}\n+{\"users\":[1]}\n"),
        "{text}"
    );
    assert_golden("diff_changed.json", &normalize(changed));

    // Without --bodies there is no text; the human view still says differs.
    let terse = sandbox
        .facet()
        .args(["diff", &id_a, &id_c, ws, "--json"])
        .output()
        .unwrap();
    assert_eq!(terse.status.code(), Some(1));
    let terse: Value = serde_json::from_slice(&terse.stdout).unwrap();
    assert!(terse["response"]["body"]["text"].is_null());
    let human = sandbox
        .facet()
        .args(["diff", &id_a, &id_c, ws])
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(1));
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("response body: differs"), "{text}");
    let quiet = sandbox
        .facet()
        .args(["diff", &id_a, &id_c, ws, "--quiet"])
        .output()
        .unwrap();
    assert_eq!(quiet.status.code(), Some(1));
    assert!(quiet.stdout.is_empty());

    // Missing runs are exit 4 with the ids listed; no store is exit 9.
    let (code, error) = sandbox.run_error_json(&["diff", &id_a, UNKNOWN_ULID, ws]);
    assert_eq!(code, 4);
    assert_eq!(error["error"]["category"], "run_not_found");
    assert_eq!(error["error"]["details"]["missing"], json!([UNKNOWN_ULID]));
    assert_golden("error_diff_run_not_found.json", &normalize_error(error));
    let empty = Sandbox::new();
    let (code, error) =
        empty.run_error_json(&["diff", &id_a, &id_b, empty.root().to_str().unwrap()]);
    assert_eq!(code, 9);
    assert_eq!(error["error"]["category"], "lattice_not_found");
    let (code, _) = sandbox.run_error_json(&["diff", &id_a]);
    assert_eq!(code, 2);
}
