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
  intent = [ { kind = "docs_eod", date = "2026-09-12", required_briefs = [] } ],
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
