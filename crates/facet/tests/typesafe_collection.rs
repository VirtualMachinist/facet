//! Offline checks for the TypeSafe Jev example collection.
//!
//! Parses the shipped YAML and dry-runs every selector. No live TypeSafe
//! key: missing hydration is fail-closed; a dummy `--var` is enough to
//! preview resolve.

#[allow(dead_code, unused_imports)]
mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use probe_core::{
    Body, CollectionItem, EnvironmentVariable, RawBodyKind, RequestBody, VariableValue,
    VariableValueSet,
};
use probe_opencollection::parse;

fn typesafe_yaml_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples/typesafe/opencollection.yml")
}

fn typesafe_source() -> String {
    fs::read_to_string(typesafe_yaml_path()).expect("TypeSafe example collection should exist")
}

fn raw_json_body(request: &probe_core::HttpRequest) -> &str {
    match &request.body {
        Some(RequestBody::Single(Body::Raw(raw))) if raw.kind == RawBodyKind::Json => &raw.data,
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

#[test]
fn typesafe_example_parses_as_choice_and_noul_gates() {
    let source = typesafe_source();
    assert!(
        !source.contains("sk-") && !source.contains("Bearer ts_"),
        "example YAML must not bake an API key"
    );

    let parsed = parse(&source).expect("TypeSafe example should parse");
    let collection = parsed.collection();
    assert_eq!(
        collection.metadata.name.as_deref(),
        Some("TypeSafe Jev (Facet native)")
    );
    assert_eq!(collection.metadata.version.as_deref(), Some("0.2.0"));
    assert_eq!(collection.environments.len(), 1);
    assert_eq!(collection.environments[0].name, "typesafe");

    let mut saw_secret_key = false;
    let mut saw_shadow = false;
    for variable in &collection.environments[0].variables {
        match variable {
            EnvironmentVariable::Secret(secret) => {
                assert_eq!(secret.name.as_deref(), Some("typesafeApiKey"));
                saw_secret_key = true;
            }
            EnvironmentVariable::Plain(plain) if plain.name.as_deref() == Some("jevShadow") => {
                assert_eq!(
                    plain.value,
                    Some(VariableValueSet::Single(VariableValue::String(
                        "true".to_owned()
                    )))
                );
                saw_shadow = true;
            }
            EnvironmentVariable::Plain(plain)
                if plain.name.as_deref() == Some("typesafeApiKey") =>
            {
                panic!("typesafeApiKey must be secret: true with no YAML value");
            }
            _ => {}
        }
    }
    assert!(saw_secret_key, "typesafeApiKey must be declared secret");
    assert!(saw_shadow, "jevShadow=true is the collection default");

    let CollectionItem::Folder(folder) = &collection.items[0] else {
        panic!("first item should be the System One folder");
    };
    assert_eq!(folder.metadata.name.as_deref(), Some("System One"));
    assert_eq!(folder.items.len(), 4);

    let expected = [
        ("Smoke noul", "noul", "reachable"),
        ("Tool gate", "choice", "allow"),
        ("Loop stop escalate", "choice", "next"),
        ("Recipe vs adhoc", "choice", "path"),
    ];
    for (index, (name, primitive, question)) in expected.into_iter().enumerate() {
        let CollectionItem::HttpRequest(request) = &folder.items[index] else {
            panic!("{name} should be an HTTP request");
        };
        assert_eq!(request.metadata.name.as_deref(), Some(name));
        assert_eq!(request.method.as_deref(), Some("POST"));
        assert_eq!(request.url.as_deref(), Some("{{typesafeApi}}/v1/systemone"));
        assert!(
            request
                .headers
                .iter()
                .any(|header| header.name == "Authorization"
                    && header.value == "Bearer {{typesafeApiKey}}")
        );
        let body = raw_json_body(request);
        assert!(body.contains(&format!("\"type\": \"{primitive}\"")));
        assert!(body.contains(&format!("\"{question}\":")));
        assert!(!body.contains("TYPESAFE_API_KEY"));
    }
}

#[test]
fn typesafe_example_is_fail_closed_without_a_key() {
    let sandbox = Sandbox::new();
    let path = sandbox.root().join("opencollection.yml");
    fs::write(&path, typesafe_source()).unwrap();
    let ws = path.to_str().unwrap();

    let (code, error) = sandbox.run_error_json(&[
        "request",
        "run",
        ws,
        "items/0/items/0",
        "--environment",
        "typesafe",
    ]);
    assert_eq!(code, 5);
    assert_eq!(error["error"]["category"], "secret_variable_unavailable");
}

#[test]
fn typesafe_example_dry_runs_every_selector_without_network() {
    let sandbox = Sandbox::new();
    let path = sandbox.root().join("opencollection.yml");
    fs::write(&path, typesafe_source()).unwrap();
    let ws = path.to_str().unwrap();

    let selectors = [
        "items/0/items/0",
        "items/0/items/1",
        "items/0/items/2",
        "items/0/items/3",
    ];
    for selector in selectors {
        let preview = sandbox.run_json(&[
            "request",
            "run",
            ws,
            selector,
            "--environment",
            "typesafe",
            "--dry-run",
            "--var",
            "typesafeApiKey=not-a-live-key",
        ]);
        assert_eq!(preview["dryRun"], true, "{selector}");
        assert_eq!(preview["request"]["method"], "POST", "{selector}");
        assert_eq!(
            preview["request"]["url"], "https://api.typesafe.ai/v1/systemone",
            "{selector}"
        );
        let headers = preview["request"]["headers"].as_array().unwrap();
        assert!(
            headers
                .iter()
                .any(|header| header["name"] == "Authorization" && header["value"] == "<redacted>"),
            "{selector}: {headers:?}"
        );
        assert_eq!(preview["lattice"]["recorded"], false, "{selector}");
        assert_eq!(preview["lattice"]["reason"], "dry_run", "{selector}");
        assert!(preview.get("response").is_none(), "{selector}");
        let body = preview["request"]["body"]["content"].as_str().unwrap();
        assert!(!body.contains("not-a-live-key"), "{selector}");
        assert!(!body.contains("TYPESAFE_API_KEY"), "{selector}");
    }
}
