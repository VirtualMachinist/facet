//! `facet request run`: execute through the shared core and HTTP engine
//! exactly as `probe request run` does, then record the run in Lattice.

use std::{
    borrow::Cow,
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};

use lattice::{BodyInput, MachineStore, NewRun, RunRow, WorkspaceStore, now_ms, sha256_hex};
use probe_core::{
    Body, FormField, Header, HttpRequest, RequestBody, resolve_environment_with_overrides,
    resolve_request, resolve_request_strict,
};
use probe_http::{ExecutionOptions, HttpEngine, HttpError, HttpResponse, ResponseHeader};
use probe_opencollection::LoadedWorkspace;
use serde_json::{Value, json};

use crate::{
    CommandOutput, FacetError, args,
    presentation::{response_human, response_json},
    workspace::{ConfigOverrides, WorkspaceInput, load, open_store},
};

const VALUE_FLAGS: &[&str] = &[
    "--environment",
    "--output",
    "--var",
    "--tag",
    "--inline-body-max",
    "--history-retention",
];
const SWITCH_FLAGS: &[&str] = &["--strict-variables", "--no-record"];

/// Header names whose values are never written to Lattice.
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "api-key",
    "x-amz-security-token",
];
const REDACTED: &str = "<redacted>";

pub(crate) fn run(args: &[String], stdin: &mut impl Read) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, VALUE_FLAGS, SWITCH_FLAGS)?;
    let [path, selector] = parsed.positionals() else {
        return Err(FacetError::invalid_arguments(
            "request run requires <path> and <selector>",
        ));
    };
    let input = WorkspaceInput::from_argument(path);
    let environment = parsed.value("--environment")?.map(str::to_owned);
    let output = parsed.value("--output")?.map(PathBuf::from);
    let strict_variables = parsed.switch("--strict-variables");
    let variables = parsed
        .values("--var")
        .into_iter()
        .map(|entry| {
            entry
                .split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .ok_or_else(|| {
                    FacetError::invalid_arguments(format!(
                        "--var expects NAME=VALUE, got {entry:?}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let tags: Vec<String> = parsed
        .values("--tag")
        .into_iter()
        .map(str::to_owned)
        .collect();
    let record = !parsed.switch("--no-record") && !env_flag("FACET_NO_RECORD");
    let overrides = ConfigOverrides::from_parsed(&parsed)?;

    let loaded = load(&input, stdin)?;
    let request = selected_request(
        &loaded,
        selector,
        environment.as_deref(),
        &variables,
        strict_variables,
    )?;
    let options = ExecutionOptions {
        base_directory: input.base_directory(),
        ..ExecutionOptions::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| FacetError::runtime(&error))?;
    let engine = HttpEngine::new().map_err(|error| FacetError::http(&error))?;

    let started_at = now_ms();
    let clock = Instant::now();
    let result = runtime.block_on(async {
        if let Some(output) = &output {
            engine
                .execute_cancellable_to_file(&request, &options, output, tokio::signal::ctrl_c())
                .await
        } else {
            engine
                .execute_cancellable(&request, &options, tokio::signal::ctrl_c())
                .await
        }
    });
    let elapsed_ms = i64::try_from(clock.elapsed().as_millis()).unwrap_or(i64::MAX);

    let recording = if record {
        record_run(
            &input,
            &overrides,
            selector,
            environment.as_deref(),
            &request,
            started_at,
            elapsed_ms,
            &result,
            output.as_deref(),
            &tags,
        )
    } else {
        Recording::Skipped("disabled")
    };

    match result {
        Ok(response) => {
            let output = output.as_deref();
            let human = format!(
                "{}\nLattice: {}\n",
                response_human(&request, &response, output),
                recording.human()
            );
            let mut json = response_json(&request, &response, output);
            json["lattice"] = recording.json();
            let mut command = CommandOutput::new(human, json);
            if let Some(warning) = recording.warning() {
                command = command.warn(warning);
            }
            Ok(command)
        }
        Err(error) => {
            Err(FacetError::http(&error).with_details(json!({ "lattice": recording.json() })))
        }
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0")
}

/// Mirrors `probe-cli`'s selection and interpolation rules.
fn selected_request<'a>(
    loaded: &'a LoadedWorkspace,
    selector: &str,
    environment: Option<&str>,
    variables: &[(String, String)],
    strict_variables: bool,
) -> Result<Cow<'a, HttpRequest>, FacetError> {
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| FacetError::request_not_found(selector))?;
    let workspace = loaded.workspace();
    let request = workspace
        .request(key)
        .expect("repository request key must resolve");
    if environment.is_some() || !variables.is_empty() || strict_variables {
        let environment =
            resolve_environment_with_overrides(workspace.environments(), environment, variables)
                .map_err(FacetError::configuration)?;
        if strict_variables {
            resolve_request_strict(request, &environment)
        } else {
            resolve_request(request, &environment)
        }
        .map(Cow::Owned)
        .map_err(FacetError::configuration)
    } else {
        Ok(Cow::Borrowed(request))
    }
}

/// What happened to the Lattice write for this run.
enum Recording {
    Recorded {
        run: RunRow,
        workspace_id: String,
        indexed: Result<(), String>,
    },
    Skipped(&'static str),
    Failed(String),
}

impl Recording {
    fn json(&self) -> Value {
        match self {
            Self::Recorded {
                run,
                workspace_id,
                indexed,
            } => json!({
                "recorded": true,
                "runId": run.id,
                "workspaceId": workspace_id,
                "indexed": indexed.is_ok(),
                "requestHash": run.request_hash,
                "responseBody": {
                    "sizeBytes": run.res_body.len,
                    "hash": run.res_body.hash,
                    "retention": run.res_body.retention(),
                },
            }),
            Self::Skipped(reason) => json!({ "recorded": false, "reason": reason }),
            Self::Failed(message) => json!({
                "recorded": false,
                "reason": "error",
                "message": message,
            }),
        }
    }

    fn human(&self) -> String {
        match self {
            Self::Recorded {
                run, workspace_id, ..
            } => format!("run {} recorded (workspace {workspace_id})", run.id),
            Self::Skipped(reason) => format!("not recorded ({reason})"),
            Self::Failed(message) => format!("not recorded ({message})"),
        }
    }

    fn warning(&self) -> Option<String> {
        match self {
            Self::Recorded {
                indexed: Err(message),
                ..
            } => Some(format!(
                "run recorded but not indexed in the machine store: {message}"
            )),
            Self::Failed(message) => Some(format!("run not recorded in Lattice: {message}")),
            _ => None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_run(
    input: &WorkspaceInput,
    overrides: &ConfigOverrides,
    selector: &str,
    environment: Option<&str>,
    request: &HttpRequest,
    started_at: i64,
    elapsed_ms: i64,
    result: &Result<HttpResponse, HttpError>,
    output: Option<&Path>,
    tags: &[String],
) -> Recording {
    let Some(root) = input.base_directory() else {
        return Recording::Skipped("stdin_workspace");
    };
    let store = match open_store(&root, overrides) {
        Ok(store) => store,
        Err(error) => return Recording::Failed(error.message),
    };

    let req_body_bytes = request_body_bytes(request);
    let request_hash = request_hash(request, req_body_bytes.as_deref());
    let req_headers = request_headers_json(&request.headers).to_string();
    let error_text = result.as_ref().err().map(ToString::to_string);
    let res_headers = result
        .as_ref()
        .ok()
        .map(|response| response_headers_json(&response.headers).to_string());
    let content_type = result.as_ref().ok().and_then(|response| {
        response
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case("content-type"))
            .map(|header| header.value.clone())
    });
    let tags_json = if tags.is_empty() {
        None
    } else {
        Some(json!(tags).to_string())
    };
    let actor = std::env::var("FACET_ACTOR")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "human".to_owned());
    let session = std::env::var("FACET_SESSION")
        .ok()
        .filter(|value| !value.is_empty());
    let redacted_url = redact_url(request.url.as_deref().unwrap_or(""));

    let res_body = match result {
        Ok(response) => {
            if let Some(output) = output {
                BodyInput::File(output)
            } else if response.body_complete {
                BodyInput::Bytes(&response.body)
            } else if let Some(file) = &response.body_file {
                BodyInput::File(file.path())
            } else {
                BodyInput::Unretained {
                    len: response.size as u64,
                }
            }
        }
        Err(_) => BodyInput::None,
    };
    let req_body = req_body_bytes
        .as_deref()
        .map_or(BodyInput::None, BodyInput::Bytes);

    let new_run = NewRun {
        started_at,
        duration_ms: Some(elapsed_ms),
        request_path: selector,
        request_hash: &request_hash,
        environment,
        method: request.method.as_deref().unwrap_or(""),
        url: &redacted_url,
        status: result
            .as_ref()
            .ok()
            .map(|response| i64::from(response.status)),
        error: error_text.as_deref(),
        req_headers: Some(&req_headers),
        res_headers: res_headers.as_deref(),
        req_body,
        res_body,
        res_content_type: content_type.as_deref(),
        session_id: session.as_deref(),
        actor: &actor,
        tags: tags_json.as_deref(),
    };

    match store.record_run(&new_run) {
        Ok(run) => {
            let indexed = index_run(&store, &run);
            Recording::Recorded {
                run,
                workspace_id: store.workspace_id().to_owned(),
                indexed,
            }
        }
        Err(error) => Recording::Failed(FacetError::lattice(error).message),
    }
}

/// Best-effort pointer row in the machine store. The workspace store is the
/// record of truth; the index is a convenience and never fails the run.
fn index_run(store: &WorkspaceStore, run: &RunRow) -> Result<(), String> {
    let machine = MachineStore::open(store.config()).map_err(|error| error.to_string())?;
    machine
        .touch_workspace(store.workspace_id(), store.root(), None, now_ms())
        .map_err(|error| error.to_string())?;
    machine
        .index_run(run, store.workspace_id())
        .map_err(|error| error.to_string())
}

fn request_body_bytes(request: &HttpRequest) -> Option<Vec<u8>> {
    match request.body.as_ref()? {
        RequestBody::Single(body) => single_body_bytes(body),
        RequestBody::Variants(variants) => variants
            .iter()
            .find(|variant| variant.selected)
            .and_then(|variant| single_body_bytes(&variant.body)),
    }
}

fn single_body_bytes(body: &Body) -> Option<Vec<u8>> {
    match body {
        Body::Raw(raw) => Some(raw.data.clone().into_bytes()),
        Body::FormUrlEncoded(fields) => Some(form_encode(fields).into_bytes()),
        Body::Multipart(_) | Body::File(_) => None,
    }
}

fn form_encode(fields: &[FormField]) -> String {
    fields
        .iter()
        .filter(|field| !field.disabled)
        .map(|field| format!("{}={}", url_encode(&field.name), url_encode(&field.value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// SHA-256 of a canonical JSON view of the resolved request. Bodies enter by
/// hash so the digest is small and stable; authentication enters by scheme
/// only so secrets never influence a value stored beside the run.
fn request_hash(request: &HttpRequest, body: Option<&[u8]>) -> String {
    let canonical = json!({
        "auth": request.authentication.as_ref().map(|auth| auth.kind.as_str()),
        "body": body.map(sha256_hex),
        "headers": request.headers.iter().map(|header| json!([header.name, header.value, header.disabled])).collect::<Vec<_>>(),
        "method": request.method,
        "path": request.path_parameters.iter().map(|parameter| json!([parameter.name, parameter.value, parameter.disabled])).collect::<Vec<_>>(),
        "query": request.query_parameters.iter().map(|parameter| json!([parameter.name, parameter.value, parameter.disabled])).collect::<Vec<_>>(),
        "url": request.url,
    });
    sha256_hex(canonical.to_string().as_bytes())
}

fn is_sensitive(name: &str) -> bool {
    SENSITIVE_HEADERS
        .iter()
        .any(|sensitive| name.eq_ignore_ascii_case(sensitive))
}

fn request_headers_json(headers: &[Header]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|header| {
                json!({
                    "disabled": header.disabled,
                    "name": header.name,
                    "value": if is_sensitive(&header.name) { REDACTED } else { header.value.as_str() },
                })
            })
            .collect(),
    )
}

fn response_headers_json(headers: &[ResponseHeader]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|header| {
                json!({
                    "name": header.name,
                    "value": if is_sensitive(&header.name) { REDACTED } else { header.value.as_str() },
                })
            })
            .collect(),
    )
}

/// Strips `user:password@` from a URL's authority.
fn redact_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_owned();
    };
    let authority_start = scheme_end + 3;
    let authority_end = url[authority_start..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |offset| authority_start + offset);
    let authority = &url[authority_start..authority_end];
    match authority.rfind('@') {
        Some(at) => format!(
            "{}{REDACTED}@{}{}",
            &url[..authority_start],
            &authority[at + 1..],
            &url[authority_end..]
        ),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use probe_core::{Header, HttpRequest};

    use super::{form_encode, redact_url, request_hash, request_headers_json, url_encode};

    fn request(url: &str) -> HttpRequest {
        HttpRequest {
            method: Some("GET".to_owned()),
            url: Some(url.to_owned()),
            headers: vec![
                Header {
                    name: "Authorization".to_owned(),
                    value: "Bearer secret".to_owned(),
                    disabled: false,
                },
                Header {
                    name: "Accept".to_owned(),
                    value: "application/json".to_owned(),
                    disabled: false,
                },
            ],
            ..HttpRequest::default()
        }
    }

    #[test]
    fn redacts_sensitive_headers_case_insensitively() {
        let headers = request_headers_json(&request("http://x/").headers);
        assert_eq!(headers[0]["value"], "<redacted>");
        assert_eq!(headers[1]["value"], "application/json");
    }

    #[test]
    fn redacts_url_userinfo_only() {
        assert_eq!(
            redact_url("https://user:pw@api.example.com/v1?x=1"),
            "https://<redacted>@api.example.com/v1?x=1"
        );
        assert_eq!(
            redact_url("https://api.example.com/v1"),
            "https://api.example.com/v1"
        );
        assert_eq!(redact_url("not a url"), "not a url");
    }

    #[test]
    fn request_hash_is_deterministic_and_url_sensitive() {
        let first = request_hash(&request("http://a/"), Some(b"body"));
        let same = request_hash(&request("http://a/"), Some(b"body"));
        let other = request_hash(&request("http://b/"), Some(b"body"));
        assert_eq!(first, same);
        assert_ne!(first, other);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn encodes_forms() {
        assert_eq!(url_encode("a b&c"), "a+b%26c");
        let fields = vec![
            probe_core::FormField {
                name: "q".to_owned(),
                value: "x y".to_owned(),
                disabled: false,
            },
            probe_core::FormField {
                name: "skip".to_owned(),
                value: "me".to_owned(),
                disabled: true,
            },
        ];
        assert_eq!(form_encode(&fields), "q=x+y");
    }
}
