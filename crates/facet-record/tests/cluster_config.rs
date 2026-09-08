use base64::{Engine as _, prelude::BASE64_STANDARD};
use facet_record::http_engine_from_kubeconfig;
use serde_json::{Value, json};

#[path = "../../../tests/support/cluster_pki.rs"]
#[allow(dead_code)]
mod cluster_pki;
use cluster_pki::Pki;

fn kube(pki: &Pki) -> Value {
    json!({"apiVersion":"v1", "kind":"Config", "current-context":"m1",
        "clusters":[{"name":"cluster", "cluster":{"server":"https://127.0.0.1:6443", "certificate-authority-data":BASE64_STANDARD.encode(&pki.ca)}}],
        "users":[{"name":"facet", "user":{"client-certificate-data":BASE64_STANDARD.encode(&pki.client_cert), "client-key-data":BASE64_STANDARD.encode(&pki.client_key)}}],
        "contexts":[{"name":"m1", "context":{"cluster":"cluster", "user":"facet", "namespace":"api-smoke"}}]})
}
fn load(
    doc: &Value,
    selected: Option<&str>,
) -> Result<probe_http::HttpEngine, probe_http::HttpError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, serde_json::to_vec(doc).unwrap()).unwrap();
    http_engine_from_kubeconfig(&path, selected)
}

#[test]
fn loads_embedded_and_relative_file_credentials_with_explicit_context() {
    let pki = Pki::new(&["127.0.0.1"]);
    let doc = kube(&pki);
    assert!(load(&doc, None).is_ok());
    let mut explicit = doc.clone();
    explicit["current-context"] = json!("absent");
    assert!(load(&explicit, Some("m1")).is_ok());
    assert!(load(&explicit, None).is_err());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ca.pem"), &pki.ca).unwrap();
    std::fs::write(dir.path().join("client.pem"), &pki.client_cert).unwrap();
    std::fs::write(dir.path().join("key.pem"), &pki.client_key).unwrap();
    let mut files = doc;
    files["clusters"][0]["cluster"] =
        json!({"server":"https://127.0.0.1:6443", "certificate-authority":"ca.pem"});
    files["users"][0]["user"] = json!({"client-certificate":"client.pem", "client-key":"key.pem"});
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, serde_yaml_ng::to_string(&files).unwrap()).unwrap();
    assert!(http_engine_from_kubeconfig(&path, None).is_ok());
}

#[test]
fn rejects_ambiguous_insecure_and_executable_credentials_without_leaking_material() {
    let pki = Pki::new(&["127.0.0.1"]);
    let original = kube(&pki);
    let mut cases = Vec::new();
    for server in [
        "http://127.0.0.1:6443",
        "https://user:secret@127.0.0.1",
        "https://127.0.0.1/prefix",
        "https://127.0.0.1?token=secret",
    ] {
        let mut doc = original.clone();
        doc["clusters"][0]["cluster"]["server"] = json!(server);
        cases.push(doc);
    }
    for key in [
        "exec",
        "auth-provider",
        "token",
        "tokenFile",
        "username",
        "password",
        "as",
    ] {
        let mut doc = original.clone();
        doc["users"][0]["user"][key] = json!("credential-canary-do-not-print");
        cases.push(doc);
    }
    for key in ["tls-server-name", "proxy-url"] {
        let mut doc = original.clone();
        doc["clusters"][0]["cluster"][key] = json!("credential-canary-do-not-print");
        cases.push(doc);
    }
    let mut doc = original.clone();
    doc["clusters"][0]["cluster"]["insecure-skip-tls-verify"] = json!(true);
    cases.push(doc);
    let mut doc = original.clone();
    doc["users"][0]["user"]["client-key"] = json!("extra.pem");
    cases.push(doc);
    let mut doc = original.clone();
    doc["users"][0]["user"]["client-key-data"] =
        json!("invalid-base64 credential-canary-do-not-print");
    cases.push(doc);
    let mut doc = original.clone();
    let entry = doc["contexts"][0].clone();
    doc["contexts"].as_array_mut().unwrap().push(entry);
    cases.push(doc);
    let mut doc = original.clone();
    doc["contexts"][0]["context"]["user"] = json!("missing");
    cases.push(doc);
    for doc in cases {
        let error = load(&doc, None).unwrap_err();
        assert!(error.is_configuration());
        assert!(!format!("{error:?}").contains("credential-canary-do-not-print"));
        assert!(!error.to_string().contains(&pki.client_key));
    }
}

#[test]
fn malformed_or_oversized_input_has_safe_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, "client-key: [credential-canary-do-not-print").unwrap();
    let error = http_engine_from_kubeconfig(&path, None).unwrap_err();
    assert!(!error.to_string().contains("credential-canary-do-not-print"));
    std::fs::write(&path, vec![b'x'; 4 * 1024 * 1024 + 1]).unwrap();
    assert!(
        http_engine_from_kubeconfig(&path, None)
            .unwrap_err()
            .to_string()
            .contains("exceeds 4 MiB")
    );
}
