//! `facet ncl check|export`: CLI freeze shape and secret-refusal contract.

#[allow(dead_code, unused_imports)]
mod common;

use std::fs;

use common::*;

const LIB: &str = r#"
{
  pod = fun args => {
    apiVersion = "v1",
    kind = "Pod",
    metadata.name = args.name,
    metadata.annotations.replicas = std.to_string args.n,
  },
}
"#;

const WORLD: &str = r#"
let lib = import "lib.ncl" in
{
  replicas = 1,
  cluster = [ lib.pod { name = "web", n = replicas } ],
  intent = [
    {
      name = "test-docs-eod",
      importance = 0.5,
      spec = { kind = "docs_eod", date = "2026-09-12", required_briefs = [] },
    },
  ],
  calls = {
    opencollection = "1.0.0",
    environments = [ { name = "dev", variables = [ { name = "TOKEN", secret_ref = "kr:me" } ] } ],
    plaintext_token | not_exported = "hunter2",
  },
}
"#;

fn write_world(root: &std::path::Path) -> std::path::PathBuf {
    fs::write(root.join("lib.ncl"), LIB).unwrap();
    let world = root.join("world.ncl");
    fs::write(&world, WORLD).unwrap();
    world
}

fn normalize_ncl(value: Value) -> Value {
    let mut v = normalize(value);
    if let Value::Object(map) = &mut v {
        for key in ["moduleHash", "exportHash"] {
            if map.contains_key(key) {
                map.insert(key.to_owned(), json!("<sha256>"));
            }
        }
    }
    v
}

#[test]
fn ncl_check_json_is_golden() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let value = sandbox.run_json(&[
        "ncl",
        "check",
        world.to_str().unwrap(),
    ]);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["contractSet"], "k8s-1.34-h3s-0.9.1");
    assert!(value["moduleHash"].as_str().unwrap().len() == 64);
    assert_golden("ncl_check.json", &normalize_ncl(value));
}

#[test]
fn ncl_export_json_is_golden_and_drops_not_exported() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let value = sandbox.run_json(&[
        "ncl",
        "export",
        world.to_str().unwrap(),
    ]);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["contractSet"], "k8s-1.34-h3s-0.9.1");
    assert!(value.get("plaintext_token").is_none());
    assert_eq!(
        value["calls"]["environments"][0]["variables"][0]["secret_ref"],
        "kr:me"
    );
    let text = value.to_string();
    assert!(!text.contains("hunter2"), "not_exported leaked into export JSON");
    assert_golden("ncl_export.json", &normalize_ncl(value));
}

#[test]
fn ncl_export_respects_var_override() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let base = sandbox.run_json(&["ncl", "export", world.to_str().unwrap()]);
    let overridden = sandbox.run_json(&[
        "ncl",
        "export",
        world.to_str().unwrap(),
        "--var",
        "replicas=3",
    ]);
    assert_eq!(base["moduleHash"], overridden["moduleHash"]);
    assert_ne!(base["exportHash"], overridden["exportHash"]);
    assert_eq!(
        overridden["cluster"][0]["metadata"]["annotations"]["replicas"],
        "3"
    );
}

#[test]
fn ncl_rejects_secret_var() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let (code, value) = sandbox.run_error_json(&[
        "ncl",
        "export",
        world.to_str().unwrap(),
        "--var",
        "calls.token=\"leak\"",
    ]);
    assert_eq!(code, 2);
    assert_eq!(value["error"]["category"], "invalid_arguments");
}

#[test]
fn ncl_export_records_ledger_tags_and_blobs() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let export = sandbox.run_json(&["ncl", "export", world.to_str().unwrap()]);
    let module_hash = export["moduleHash"].as_str().unwrap();
    let export_hash = export["exportHash"].as_str().unwrap();
    let contract_set = export["contractSet"].as_str().unwrap();

    let root = sandbox.root().to_str().unwrap();
    let history = sandbox.run_json(&["history", root, "--bodies"]);
    let runs = history["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "expected one ledger row");
    let row = &runs[0];
    assert_eq!(row["requestPath"], "ncl:export");
    assert_eq!(row["method"], "NCL");

    let tags: Vec<&str> = row["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| tag.as_str().unwrap())
        .collect();
    assert!(tags.contains(&format!("ncl:module:{module_hash}").as_str()));
    assert!(tags.contains(&format!("ncl:export:{export_hash}").as_str()));
    assert!(tags.contains(&format!("ncl:contracts:{contract_set}").as_str()));

    let source_hash = row["request"]["body"]["hash"].as_str().unwrap();
    let freeze_hash = row["response"]["body"]["hash"].as_str().unwrap();
    assert!(sandbox.root().join(".facet/blobs").join(source_hash).is_file());
    assert!(sandbox.root().join(".facet/blobs").join(freeze_hash).is_file());

    let source_blob = sandbox.run_json(&["blob", source_hash, root]);
    let content = &source_blob["blob"]["body"]["content"];
    let snapshot: Value = if content.is_string() {
        serde_json::from_str(content.as_str().unwrap()).expect("source snapshot JSON")
    } else {
        content.clone()
    };
    let sources = &snapshot["sources"];
    assert!(sources.get("world.ncl").is_some());
    assert!(sources.get("lib.ncl").is_some());

    let freeze_blob = sandbox.run_json(&["blob", freeze_hash, root]);
    let freeze_content = &freeze_blob["blob"]["body"]["content"];
    let frozen: Value = if freeze_content.is_string() {
        serde_json::from_str(freeze_content.as_str().unwrap()).expect("freeze artifact JSON")
    } else {
        freeze_content.clone()
    };
    assert_eq!(frozen["exportHash"], export_hash);
    assert_eq!(frozen["moduleHash"], module_hash);
    assert_eq!(frozen["contractSet"], contract_set);
    assert!(frozen.get("cluster").is_some());
    assert!(frozen.get("intent").is_some());
    assert!(frozen.get("calls").is_some());
    assert!(frozen.get("plaintext_token").is_none());
}

fn run_count(sandbox: &common::Sandbox, root: &str) -> usize {
    sandbox
        .run_json(&["history", root])["runs"]
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn ncl_export_frozen_passes_when_unchanged() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let path = world.to_str().unwrap();
    let root = sandbox.root().to_str().unwrap();
    let first = sandbox.run_json(&["ncl", "export", path]);
    let second = sandbox.run_json(&["ncl", "export", path, "--frozen"]);
    assert_eq!(first["exportHash"], second["exportHash"]);
    assert_eq!(run_count(&sandbox, root), 2, "frozen pass still records the export");
}

#[test]
fn ncl_export_frozen_refuses_on_drift() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let path = world.to_str().unwrap();
    let root = sandbox.root().to_str().unwrap();
    let recorded = sandbox.run_json(&["ncl", "export", path]);
    let run_id = sandbox.run_json(&["history", root, "--bodies"])["runs"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    std::fs::write(
        &world,
        WORLD.replace("replicas = 1", "replicas = 2"),
    )
    .unwrap();

    let before = run_count(&sandbox, root);
    let (code, error) = sandbox.run_error_json(&["ncl", "export", path, "--frozen"]);
    assert_eq!(code, 1);
    assert_eq!(error["error"]["category"], "replay_changed");
    assert_eq!(error["error"]["details"]["replayedFrom"], run_id);
    assert_eq!(
        error["error"]["details"]["recordedHash"],
        recorded["exportHash"]
    );
    assert_ne!(
        error["error"]["details"]["recordedHash"],
        error["error"]["details"]["currentHash"]
    );
    assert_eq!(
        run_count(&sandbox, root),
        before,
        "no ledger row when --frozen refuses"
    );
}

#[test]
fn ncl_apply_records_one_lattice_row_and_skips_cluster_without_kubeconfig() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let path = world.to_str().unwrap();
    let root = sandbox.root().to_str().unwrap();
    let value = sandbox.run_json(&["ncl", "apply", path]);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["contractSet"], "k8s-1.34-h3s-0.9.1");
    assert_eq!(value["action"]["cluster"]["skipped"], true);
    assert_eq!(value["action"]["intent"]["skipped"], true);

    let history = sandbox.run_json(&["history", root, "--bodies"]);
    let runs = history["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "expected one ncl:apply ledger row");
    let row = &runs[0];
    assert_eq!(row["requestPath"], "ncl:apply");
    assert_eq!(row["method"], "NCL");

    let tags: Vec<&str> = row["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| tag.as_str().unwrap())
        .collect();
    assert!(tags.iter().any(|tag| tag.starts_with("ncl:module:")));
    assert!(tags.iter().any(|tag| tag.starts_with("ncl:export:")));
    assert!(tags.iter().any(|tag| tag.starts_with("ncl:contracts:")));
}

#[test]
fn ncl_apply_puts_intent_when_hedron_db_present() {
    let sandbox = Sandbox::new();
    let world = write_world(sandbox.root());
    let path = world.to_str().unwrap();
    let db_path = sandbox.root().join("hedron.db");
    hedron_core::Store::open(&db_path).expect("create empty hedrondb schema");

    let output = sandbox
        .facet()
        .env("FACET_HEDRON_DB", &db_path)
        .args(["ncl", "apply", path, "--json"])
        .output()
        .expect("facet ncl apply");
    assert!(
        output.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("apply JSON");
    assert_eq!(value["action"]["intent"]["skipped"], false);
    assert_eq!(value["action"]["intent"]["put"], 1);
    assert_eq!(value["action"]["intent"]["names"], json!(["test-docs-eod"]));
}
