use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, FileReference, FormField,
    Header, HttpRequest, MultipartPart, MultipartPartKind, MultipartValue, QueryParameter, RawBody,
    RawBodyKind, RequestBody, RequestSettings,
};
use probe_http::{ExecutionOptions, HttpEngine, HttpError, MAX_IN_MEMORY_RESPONSE_BYTES};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

#[derive(Debug)]
struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

async fn serve_once(
    status: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> io::Result<(String, JoinHandle<io::Result<CapturedRequest>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let status = status.to_owned();
    let headers: Vec<_> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let body = body.to_vec();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let captured = read_request(&mut stream).await?;
        write_response(&mut stream, &status, &headers, &body).await?;
        Ok(captured)
    });
    Ok((format!("http://{address}"), handle))
}

async fn read_request(stream: &mut TcpStream) -> io::Result<CapturedRequest> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
        read_more(stream, &mut bytes).await?;
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let headers: Vec<_> = lines
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect();
    let mut encoded_body = bytes[header_end..].to_vec();
    let body = if let Some(length) = header(&headers, "content-length") {
        let length = length.parse::<usize>().map_err(io::Error::other)?;
        while encoded_body.len() < length {
            read_more(stream, &mut encoded_body).await?;
        }
        encoded_body.truncate(length);
        encoded_body
    } else if header(&headers, "transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        decode_chunked(stream, encoded_body).await?
    } else {
        Vec::new()
    };
    Ok(CapturedRequest {
        request_line,
        headers,
        body,
    })
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

async fn decode_chunked(stream: &mut TcpStream, mut encoded: Vec<u8>) -> io::Result<Vec<u8>> {
    let mut decoded = Vec::new();
    loop {
        let line_end = loop {
            if let Some(position) = find_bytes(&encoded, b"\r\n") {
                break position;
            }
            read_more(stream, &mut encoded).await?;
        };
        let line = String::from_utf8_lossy(&encoded[..line_end]);
        let size_text = line.split(';').next().unwrap_or_default();
        let size = usize::from_str_radix(size_text.trim(), 16).map_err(io::Error::other)?;
        encoded.drain(..line_end + 2);
        if size == 0 {
            return Ok(decoded);
        }
        while encoded.len() < size + 2 {
            read_more(stream, &mut encoded).await?;
        }
        decoded.extend_from_slice(&encoded[..size]);
        encoded.drain(..size + 2);
    }
}

async fn read_more(stream: &mut TcpStream, bytes: &mut Vec<u8>) -> io::Result<()> {
    let mut buffer = [0_u8; 8 * 1024];
    let count = stream.read(&mut buffer).await?;
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "connection closed while reading request",
        ));
    }
    bytes.extend_from_slice(&buffer[..count]);
    Ok(())
}

async fn write_response(
    stream: &mut TcpStream,
    status: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> io::Result<()> {
    let mut response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("Connection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn request(method: &str, url: String) -> HttpRequest {
    HttpRequest {
        method: Some(method.to_owned()),
        url: Some(url),
        ..HttpRequest::default()
    }
}

#[tokio::test]
async fn ignores_enabled_parameters_without_names() {
    let (base_url, captured) = serve_once("200 OK", &[], b"ok").await.unwrap();
    let mut request = request("POST", format!("{base_url}/unnamed"));
    request.headers = vec![
        Header {
            name: String::new(),
            value: "ignored".to_owned(),
            disabled: false,
        },
        Header {
            name: "   ".to_owned(),
            value: "ignored".to_owned(),
            disabled: false,
        },
        Header {
            name: "X-Valid".to_owned(),
            value: "sent".to_owned(),
            disabled: false,
        },
    ];
    request.query_parameters = vec![
        QueryParameter {
            name: String::new(),
            value: "ignored".to_owned(),
            disabled: false,
        },
        QueryParameter {
            name: "   ".to_owned(),
            value: "ignored".to_owned(),
            disabled: false,
        },
        QueryParameter {
            name: "limit".to_owned(),
            value: "10".to_owned(),
            disabled: false,
        },
        QueryParameter {
            name: "flag".to_owned(),
            value: String::new(),
            disabled: false,
        },
    ];
    request.body = Some(RequestBody::Single(Body::FormUrlEncoded(vec![
        FormField {
            name: String::new(),
            value: "ignored".to_owned(),
            disabled: false,
        },
        FormField {
            name: "name".to_owned(),
            value: "Probe".to_owned(),
            disabled: false,
        },
    ])));

    HttpEngine::new()
        .unwrap()
        .execute(&request, &ExecutionOptions::default())
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();

    assert_eq!(
        captured.request_line,
        "POST /unnamed?limit=10&flag= HTTP/1.1"
    );
    assert_eq!(captured.header("x-valid"), Some("sent"));
    assert!(
        captured
            .headers
            .iter()
            .all(|(name, _)| !name.trim().is_empty())
    );
    assert_eq!(captured.body, b"name=Probe");
}

#[tokio::test]
async fn ignores_multipart_parts_without_names() {
    let (base_url, captured) = serve_once("200 OK", &[], b"ok").await.unwrap();
    let mut request = request("POST", format!("{base_url}/upload"));
    request.body = Some(RequestBody::Single(Body::Multipart(vec![
        MultipartPart {
            name: String::new(),
            kind: MultipartPartKind::Text,
            value: MultipartValue::Single("ignored".to_owned()),
            content_type: None,
            disabled: false,
        },
        MultipartPart {
            name: "caption".to_owned(),
            kind: MultipartPartKind::Text,
            value: MultipartValue::Single("hello".to_owned()),
            content_type: None,
            disabled: false,
        },
    ])));

    HttpEngine::new()
        .unwrap()
        .execute(&request, &ExecutionOptions::default())
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();
    let body = String::from_utf8_lossy(&captured.body);
    assert!(body.contains("name=\"caption\""));
    assert!(body.contains("hello"));
    assert!(!body.contains("name=\"\""));
    assert!(!body.contains("ignored"));
}

#[tokio::test]
async fn executes_json_with_headers_query_bearer_auth_and_response_metadata() {
    let (base_url, captured) = serve_once(
        "201 Created",
        &[("X-Zeta", "last"), ("X-Alpha", "first")],
        b"created",
    )
    .await
    .unwrap();
    let mut request = request("POST", format!("{base_url}/users/:userId?existing=yes"));
    request.headers = vec![
        Header {
            name: "X-Probe".to_owned(),
            value: "enabled".to_owned(),
            disabled: false,
        },
        Header {
            name: "X-Skipped".to_owned(),
            value: "disabled".to_owned(),
            disabled: true,
        },
    ];
    request.query_parameters = vec![QueryParameter {
        name: "search".to_owned(),
        value: "hello world".to_owned(),
        disabled: false,
    }];
    request.path_parameters = vec![QueryParameter {
        name: "userId".to_owned(),
        value: "probe/user".to_owned(),
        disabled: false,
    }];
    request.body = Some(RequestBody::Single(Body::Raw(RawBody {
        kind: RawBodyKind::Json,
        data: "{\"name\":\"Milo\"}".to_owned(),
    })));
    request.authentication = Some(Authentication {
        kind: AuthenticationKind::Bearer,
        properties: BTreeMap::from([(
            "token".to_owned(),
            AuthenticationValue::String("token-value".to_owned()),
        )]),
    });

    let response = HttpEngine::new()
        .unwrap()
        .execute(&request, &ExecutionOptions::default())
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();

    assert_eq!(
        captured.request_line,
        "POST /users/probe%2Fuser?existing=yes&search=hello+world HTTP/1.1"
    );
    assert_eq!(captured.header("x-probe"), Some("enabled"));
    assert_eq!(captured.header("x-skipped"), None);
    assert_eq!(captured.header("authorization"), Some("Bearer token-value"));
    assert_eq!(captured.header("content-type"), Some("application/json"));
    assert_eq!(captured.body, br#"{"name":"Milo"}"#);
    assert_eq!(response.status, 201);
    assert_eq!(response.reason, "Created");
    assert_eq!(response.body, b"created");
    assert_eq!(response.size, 7);
    assert_eq!(response.headers[0].name, "connection");
    assert!(response.duration > Duration::ZERO);
}

#[tokio::test]
async fn strips_json_body_comments_before_sending() {
    let (base_url, captured) = serve_once("200 OK", &[], b"ok").await.unwrap();
    let mut request = request("POST", format!("{base_url}/comments"));
    request.body = Some(RequestBody::Single(Body::Raw(RawBody {
        kind: RawBodyKind::Json,
        data: r#"{
  // editor-only line comment
  "url": "https://example.com/a//b",
  "literal": "/* not a comment */",
  /* editor-only block
     comment */
  "name": "Café"
}"#
        .to_owned(),
    })));

    HttpEngine::new()
        .unwrap()
        .execute(&request, &ExecutionOptions::default())
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();
    let body = String::from_utf8(captured.body).unwrap();

    assert!(!body.contains("editor-only line comment"));
    assert!(!body.contains("editor-only block"));
    assert!(body.contains(r#""url": "https://example.com/a//b""#));
    assert!(body.contains(r#""literal": "/* not a comment */""#));
    assert!(body.contains(r#""name": "Café""#));
}

#[tokio::test]
async fn supports_all_phase_five_methods() {
    for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
        let (base_url, captured) = serve_once("204 No Content", &[], b"").await.unwrap();
        let response = HttpEngine::new()
            .unwrap()
            .execute(
                &request(method, format!("{base_url}/method")),
                &ExecutionOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(response.status, 204);
        assert_eq!(
            captured.await.unwrap().unwrap().request_line,
            format!("{method} /method HTTP/1.1")
        );
    }
}

#[tokio::test]
async fn sends_urlencoded_forms_and_basic_auth() {
    let (base_url, captured) = serve_once("200 OK", &[], b"ok").await.unwrap();
    let mut request = request("POST", format!("{base_url}/form"));
    request.body = Some(RequestBody::Single(Body::FormUrlEncoded(vec![
        FormField {
            name: "name".to_owned(),
            value: "Probe Client".to_owned(),
            disabled: false,
        },
        FormField {
            name: "skip".to_owned(),
            value: "true".to_owned(),
            disabled: true,
        },
    ])));
    request.authentication = Some(Authentication {
        kind: AuthenticationKind::Basic,
        properties: BTreeMap::from([
            (
                "username".to_owned(),
                AuthenticationValue::String("demo".to_owned()),
            ),
            (
                "password".to_owned(),
                AuthenticationValue::String("secret".to_owned()),
            ),
        ]),
    });

    HttpEngine::new()
        .unwrap()
        .execute(&request, &ExecutionOptions::default())
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();
    assert_eq!(
        captured.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        captured.header("authorization"),
        Some("Basic ZGVtbzpzZWNyZXQ=")
    );
    assert_eq!(captured.body, b"name=Probe+Client");
}

#[tokio::test]
async fn sends_text_body_with_default_content_type() {
    let (base_url, captured) = serve_once("200 OK", &[], b"ok").await.unwrap();
    let mut request = request("PATCH", format!("{base_url}/text"));
    request.body = Some(RequestBody::Single(Body::Raw(RawBody {
        kind: RawBodyKind::Text,
        data: "plain text // keep this".to_owned(),
    })));

    HttpEngine::new()
        .unwrap()
        .execute(&request, &ExecutionOptions::default())
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();
    assert_eq!(
        captured.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(captured.body, b"plain text // keep this");
}

#[tokio::test]
async fn sends_multipart_text_and_file_parts_from_the_execution_base_directory() {
    let path = temporary_file("multipart.txt");
    std::fs::write(&path, b"file contents").unwrap();
    let directory = path.parent().unwrap().to_owned();
    let filename = path.file_name().unwrap().to_string_lossy().into_owned();
    let (base_url, captured) = serve_once("200 OK", &[], b"uploaded").await.unwrap();
    let mut request = request("POST", format!("{base_url}/upload"));
    request.body = Some(RequestBody::Single(Body::Multipart(vec![
        MultipartPart {
            name: "caption".to_owned(),
            kind: MultipartPartKind::Text,
            value: MultipartValue::Single("hello".to_owned()),
            content_type: None,
            disabled: false,
        },
        MultipartPart {
            name: "asset".to_owned(),
            kind: MultipartPartKind::File,
            value: MultipartValue::Single(filename),
            content_type: Some("text/plain".to_owned()),
            disabled: false,
        },
    ])));

    HttpEngine::new()
        .unwrap()
        .execute(
            &request,
            &ExecutionOptions {
                base_directory: Some(directory),
            },
        )
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();
    let body = String::from_utf8_lossy(&captured.body);
    assert!(
        captured
            .header("content-type")
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    assert!(body.contains("name=\"caption\""));
    assert!(body.contains("hello"));
    assert!(body.contains("name=\"asset\""));
    assert!(body.contains("file contents"));
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn streams_selected_file_body() {
    let path = temporary_file("body.txt");
    std::fs::write(&path, b"streamed body").unwrap();
    let directory = path.parent().unwrap().to_owned();
    let filename = path.file_name().unwrap().to_string_lossy().into_owned();
    let (base_url, captured) = serve_once("200 OK", &[], b"ok").await.unwrap();
    let mut request = request("PUT", format!("{base_url}/file"));
    request.body = Some(RequestBody::Single(Body::File(vec![FileReference {
        file_path: filename,
        content_type: "text/plain".to_owned(),
        selected: true,
    }])));

    HttpEngine::new()
        .unwrap()
        .execute(
            &request,
            &ExecutionOptions {
                base_directory: Some(directory),
            },
        )
        .await
        .unwrap();
    let captured = captured.await.unwrap().unwrap();
    assert_eq!(captured.header("content-type"), Some("text/plain"));
    assert_eq!(captured.body, b"streamed body");
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn follows_or_returns_redirects_according_to_request_settings() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let redirect_server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await?;
        read_request(&mut first).await?;
        write_response(
            &mut first,
            "302 Found",
            &[("Location".to_owned(), format!("http://{address}/final"))],
            b"",
        )
        .await?;
        let (mut second, _) = listener.accept().await?;
        let captured = read_request(&mut second).await?;
        write_response(&mut second, "200 OK", &[], b"final").await?;
        Ok::<_, io::Error>(captured)
    });
    let mut following = request("GET", format!("http://{address}/redirect"));
    following.settings.max_redirects = Some(3);
    let response = HttpEngine::new()
        .unwrap()
        .execute(&following, &ExecutionOptions::default())
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"final");
    assert_eq!(
        redirect_server.await.unwrap().unwrap().request_line,
        "GET /final HTTP/1.1"
    );

    let (base_url, captured) = serve_once(
        "302 Found",
        &[("Location", "http://127.0.0.1:1/unreachable")],
        b"redirect",
    )
    .await
    .unwrap();
    let mut not_following = request("GET", format!("{base_url}/redirect"));
    not_following.settings.follow_redirects = Some(false);
    let response = HttpEngine::new()
        .unwrap()
        .execute(&not_following, &ExecutionOptions::default())
        .await
        .unwrap();
    assert_eq!(response.status, 302);
    captured.await.unwrap().unwrap();
}

#[tokio::test]
async fn reports_timeout_and_cancellation_separately() {
    let (timeout_url, timeout_server) = delayed_server().await;
    let mut timed = request("GET", timeout_url);
    timed.settings = RequestSettings {
        timeout: Some(Duration::from_millis(20)),
        ..RequestSettings::default()
    };
    let error = HttpEngine::new()
        .unwrap()
        .execute(&timed, &ExecutionOptions::default())
        .await
        .unwrap_err();
    assert_eq!(error, HttpError::Timeout);
    timeout_server.abort();

    let (cancel_url, cancel_server) = delayed_server().await;
    let error = HttpEngine::new()
        .unwrap()
        .execute_cancellable(
            &request("GET", cancel_url),
            &ExecutionOptions::default(),
            tokio::time::sleep(Duration::from_millis(20)),
        )
        .await
        .unwrap_err();
    assert_eq!(error, HttpError::Cancelled);
    cancel_server.abort();
}

#[tokio::test]
async fn bounds_in_memory_responses_and_streams_file_output() {
    let body = vec![b'x'; MAX_IN_MEMORY_RESPONSE_BYTES + 32 * 1024];
    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let response = HttpEngine::new()
        .unwrap()
        .execute(
            &request("GET", format!("{base_url}/bounded")),
            &ExecutionOptions::default(),
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    assert_eq!(response.size, body.len());
    assert!(response.body.is_empty());
    assert!(!response.body_complete);

    let output = temporary_file("streamed-response.bin");
    let (base_url, captured) = serve_once("200 OK", &[], &body).await.unwrap();
    let response = HttpEngine::new()
        .unwrap()
        .execute_to_file(
            &request("GET", format!("{base_url}/file")),
            &ExecutionOptions::default(),
            &output,
        )
        .await
        .unwrap();
    captured.await.unwrap().unwrap();
    assert_eq!(response.size, body.len());
    assert!(response.body.is_empty());
    assert!(!response.body_complete);
    assert_eq!(std::fs::read(&output).unwrap(), body);
    std::fs::remove_file(output).unwrap();
}

async fn delayed_server() -> (String, JoinHandle<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        read_request(&mut stream).await?;
        tokio::time::sleep(Duration::from_secs(10)).await;
        write_response(&mut stream, "200 OK", &[], b"late").await
    });
    (format!("http://{address}/slow"), handle)
}

fn temporary_file(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "probe-http-{}-{unique}-{suffix}",
        std::process::id()
    ))
}
