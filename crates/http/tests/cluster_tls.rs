use probe_core::{Header, HttpRequest};
use probe_http::{ClusterTls, ExecutionOptions, HttpEngine};
use rustls::{
    RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::WebPkiClientVerifier,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use tokio_rustls::TlsAcceptor;

#[path = "../../../tests/support/cluster_pki.rs"]
mod cluster_pki;
use cluster_pki::Pki;

type Captures = Vec<(Vec<u8>, String)>;
async fn server(
    pki: &Pki,
    responses: Vec<String>,
) -> (String, JoinHandle<Result<Captures, String>>) {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(pki.ca.as_bytes()).unwrap())
        .unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .unwrap();
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![CertificateDer::from_pem_slice(pki.server_cert.as_bytes()).unwrap()],
            PrivateKeyDer::from_pem_slice(pki.server_key.as_bytes()).unwrap(),
        )
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async {
            let acceptor = TlsAcceptor::from(Arc::new(config));
            let mut captured = Vec::new();
            for response in responses {
                let (socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
                let mut stream = acceptor.accept(socket).await.map_err(|e| e.to_string())?;
                let peer = stream.get_ref().1.peer_certificates().unwrap()[0].to_vec();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let byte = stream.read_u8().await.map_err(|e| e.to_string())?;
                    request.push(byte);
                    if request.len() > 8192 {
                        return Err("oversized request".into());
                    }
                }
                captured.push((peer, String::from_utf8(request).unwrap()));
                stream
                    .write_all(response.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
                stream.shutdown().await.map_err(|e| e.to_string())?;
            }
            Ok(captured)
        })
        .await
        .map_err(|e| e.to_string())?
    });
    (url, task)
}
fn ok() -> String {
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into()
}
fn request(url: String) -> HttpRequest {
    HttpRequest {
        method: Some("GET".into()),
        url: Some(url),
        ..Default::default()
    }
}
fn engine(url: &str, pki: &Pki) -> HttpEngine {
    HttpEngine::with_cluster_tls(
        ClusterTls::from_pem(url, pki.ca.as_bytes(), pki.client_pem().as_bytes()).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn verifies_server_and_sends_the_client_certificate_without_request_secrets() {
    let pki = Pki::new(&["127.0.0.1"]);
    let (url, task) = server(&pki, vec![ok()]).await;
    let client = engine(&url, &pki);
    let debug = format!("{client:?}");
    assert!(!debug.contains("PRIVATE KEY"));
    assert!(!debug.contains(&pki.client_key));
    let response = client
        .execute(&request(format!("{url}/api")), &ExecutionOptions::default())
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    let requests = task.await.unwrap().unwrap();
    assert_eq!(
        requests[0].0,
        CertificateDer::from_pem_slice(pki.client_cert.as_bytes())
            .unwrap()
            .to_vec()
    );
    assert!(requests[0].1.starts_with("GET /api HTTP/1.1"));
    assert!(!requests[0].1.to_lowercase().contains("authorization"));
    assert!(!requests[0].1.contains("PRIVATE KEY"));
}

#[tokio::test]
async fn rejects_foreign_ca_client_and_wrong_hostname() {
    for case in ["ca", "client", "hostname"] {
        let pki = Pki::new(if case == "hostname" {
            &["localhost"]
        } else {
            &["127.0.0.1"]
        });
        let other = Pki::new(&["127.0.0.1"]);
        let (url, task) = server(&pki, vec![ok()]).await;
        let tls = ClusterTls::from_pem(
            &url,
            if case == "ca" {
                other.ca.as_bytes()
            } else {
                pki.ca.as_bytes()
            },
            if case == "client" {
                other.client_pem()
            } else {
                pki.client_pem()
            }
            .as_bytes(),
        )
        .unwrap();
        let client = HttpEngine::with_cluster_tls(tls).unwrap();
        assert!(
            client
                .execute(&request(url), &ExecutionOptions::default())
                .await
                .is_err(),
            "{case}"
        );
        assert!(task.await.unwrap().is_err(), "{case}");
    }
}

#[tokio::test]
async fn refuses_out_of_scope_urls_and_conflicting_headers_before_connecting() {
    let pki = Pki::new(&["127.0.0.1"]);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap());
    let client = engine(&url, &pki);
    for foreign in [
        url.replace("https:", "http:"),
        format!(
            "https://localhost:{}/",
            listener.local_addr().unwrap().port()
        ),
        url.replace("https://", "https://name:password@"),
    ] {
        assert!(
            client
                .execute(&request(foreign), &ExecutionOptions::default())
                .await
                .unwrap_err()
                .is_configuration()
        );
    }
    for name in [
        "Authorization",
        "Host",
        "Proxy-Authorization",
        "Impersonate-User",
    ] {
        let mut req = request(url.clone());
        req.headers.push(Header {
            name: name.into(),
            value: "not-sent".into(),
            disabled: false,
        });
        assert!(
            client
                .execute(&req, &ExecutionOptions::default())
                .await
                .unwrap_err()
                .is_configuration()
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn permits_same_origin_redirects_but_never_connects_to_a_foreign_target() {
    let pki = Pki::new(&["127.0.0.1"]);
    let (url, task) = server(&pki, vec!["HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ok()]).await;
    let client = engine(&url, &pki);
    let mut req = request(url.clone());
    req.settings.max_redirects = Some(2); // alternate client preserves credentials/policy
    assert_eq!(
        client
            .execute(&req, &ExecutionOptions::default())
            .await
            .unwrap()
            .status,
        200
    );
    assert!(
        task.await.unwrap().unwrap()[1]
            .1
            .starts_with("GET /next HTTP/1.1")
    );
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (url, task) = server(&pki, vec![format!("HTTP/1.1 302 Found\r\nLocation: https://{}/escape\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", target.local_addr().unwrap())]).await;
    let client = engine(&url, &pki);
    assert!(
        client
            .execute(&request(url), &ExecutionOptions::default())
            .await
            .is_err()
    );
    assert_eq!(task.await.unwrap().unwrap().len(), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target.accept())
            .await
            .is_err()
    );
}
