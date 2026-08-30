pub(crate) use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) use probe_http::MAX_IN_MEMORY_RESPONSE_BYTES;
pub(crate) use serde_json::Value;

pub(crate) fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection")
        .join(path)
}

pub(crate) fn yaak_fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/yaak")
        .join(path)
}

pub(crate) fn postman_fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/postman")
        .join(path)
}

pub(crate) fn probe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_probe"))
}

#[derive(Debug)]
pub(crate) struct CapturedRequest {
    pub(crate) head: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) fn serve_once(
    body: Vec<u8>,
    content_type: &str,
) -> (String, JoinHandle<CapturedRequest>) {
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

pub(crate) fn runtime_fixture(server_url: &str) -> PathBuf {
    let source = fs::read_to_string(fixture("phase5-http.yml")).unwrap();
    let path = temporary_path("workspace.yml");
    fs::write(&path, source.replace("__SERVER_URL__", server_url)).unwrap();
    path
}

pub(crate) fn temporary_path(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "probe-cli-{}-{unique}-{suffix}",
        std::process::id()
    ))
}

pub(crate) fn copy_directory(source: &std::path::Path, destination: &std::path::Path) {
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

pub(crate) fn run_json(arguments: &[&str]) -> Value {
    let output = probe().args(arguments).arg("--json").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}

pub(crate) fn run_with_stdin(arguments: &[&str], stdin: &[u8]) -> std::process::Output {
    let mut child = probe()
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("command should start");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin)
        .expect("stdin should be writable");
    child.wait_with_output().expect("command should finish")
}
