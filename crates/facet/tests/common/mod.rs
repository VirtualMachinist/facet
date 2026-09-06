pub(crate) use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    thread::{self, JoinHandle},
};

pub(crate) use serde_json::{Value, json};

/// A throwaway workspace: the collection, its `.facet/`, and a private
/// machine store, all under one temporary directory.
pub(crate) struct Sandbox {
    pub(crate) dir: tempfile::TempDir,
}

impl Sandbox {
    pub(crate) fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temporary directory"),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Writes the phase-5 HTTP fixture with `__SERVER_URL__` replaced.
    pub(crate) fn workspace(&self, server_url: &str) -> PathBuf {
        let source = fs::read_to_string(fixture("phase5-http.yml")).unwrap();
        let path = self.root().join("workspace.yml");
        fs::write(&path, source.replace("__SERVER_URL__", server_url)).unwrap();
        path
    }

    /// A `facet` command whose machine store and config live in the sandbox.
    pub(crate) fn facet(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_facet"));
        command
            .env("FACET_DATA_DIR", self.root().join("machine"))
            .env("FACET_CONFIG_DIR", self.root().join("config"))
            .env_remove("FACET_ACTOR")
            .env_remove("FACET_SESSION")
            .env_remove("FACET_NO_RECORD");
        command
    }

    pub(crate) fn run_json(&self, arguments: &[&str]) -> Value {
        let output = self
            .facet()
            .args(arguments)
            .arg("--json")
            .output()
            .expect("facet should run");
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

    /// Runs `facet … --json` and returns the exit code plus parsed stdout (errors
    /// are JSON on stdout, not stderr).
    pub(crate) fn run_error_json(&self, arguments: &[&str]) -> (u8, Value) {
        let output = self
            .facet()
            .args(arguments)
            .arg("--json")
            .output()
            .expect("facet should run");
        let exit_code = output.status.code().unwrap_or(255) as u8;
        let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "stdout should be JSON (exit {exit_code}): {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (exit_code, value)
    }

    /// Rewrites `started_at` for one run in the workspace store (test helper).
    pub(crate) fn backdate_run(&self, run_id: &str, started_at_ms: i64) {
        let db = std::path::absolute(self.root().join(".facet/lattice.db"))
            .expect("workspace store path");
        assert!(db.is_file(), "missing {}", db.display());
        let conn = rusqlite::Connection::open(&db).expect("open workspace store");
        let updated = conn
            .execute(
                "UPDATE runs SET started_at = ?1 WHERE id = ?2",
                rusqlite::params![started_at_ms, run_id],
            )
            .expect("backdate run");
        assert_eq!(updated, 1, "backdate one run in {}", db.display());
    }
}

pub(crate) fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection")
        .join(path)
}

pub(crate) fn serve_once(body: Vec<u8>, content_type: &str) -> (String, JoinHandle<Vec<u8>>) {
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
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        request[header_end..header_end + content_length].to_vec()
    });
    (format!("http://{address}"), handle)
}

/// Error envelopes carry volatile `message` text; category and exitCode are the contract.
pub(crate) fn normalize_error(value: Value) -> Value {
    let mut normalized = normalize(value);
    if let Value::Object(root) = &mut normalized {
        if let Some(Value::Object(error)) = root.get_mut("error") {
            error.insert("message".to_owned(), json!("<message>"));
        }
    }
    normalized
}

/// Replaces values that legitimately differ between runs (ids, times,
/// hashes, paths, ports) so JSON can be compared against a golden file.
pub(crate) fn normalize(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let replaced = match key.as_str() {
                        "version" | "probeVersion" => json!("<version>"),
                        "id" | "runId" | "workspaceId" => json!("<ulid>"),
                        "startedAt" | "durationMs" => json!("<int>"),
                        "requestHash" => json!("<sha256>"),
                        "hash" if value.is_string() => json!("<sha256>"),
                        "path" | "outputPath" if value.is_string() => json!("<path>"),
                        "url" if value.is_string() => json!("<url>"),
                        _ => normalize(value),
                    };
                    (key, replaced)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        other => other,
    }
}

/// Pretty JSON with sorted keys regardless of serde_json's map feature.
pub(crate) fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), sorted(&map[key])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string_pretty(&sorted(value)).unwrap()
}

/// Compares against `tests/golden/<name>`; set `UPDATE_GOLDEN=1` to rewrite.
pub(crate) fn assert_golden(name: &str, value: &Value) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let actual = canonical(value);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&path, format!("{actual}\n")).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing golden {}: {error}", path.display()));
    assert_eq!(
        actual.trim_end(),
        expected.trim_end(),
        "golden mismatch for {name}; run with UPDATE_GOLDEN=1 to accept"
    );
}
