//! Shared asynchronous HTTP construction and execution for Probe.

#![forbid(unsafe_code)]

use std::{
    error::Error,
    ffi::OsString,
    fmt,
    future::{Future, pending},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, HttpRequest, MultipartPartKind,
    MultipartValue, RawBody, RawBodyKind, RequestBody, RequestSettings,
};
use reqwest::{
    Client, Method, RequestBuilder, Response,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    multipart::{Form, Part},
    redirect::Policy,
};
use tokio::io::AsyncWriteExt;

const DEFAULT_MAX_REDIRECTS: usize = 10;
/// Maximum response body retained by the default in-memory execution methods.
pub const MAX_IN_MEMORY_RESPONSE_BYTES: usize = 1024 * 1024;
static RESPONSE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Filesystem context used while constructing request bodies.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionOptions {
    /// Base directory for relative request-body file paths.
    pub base_directory: Option<PathBuf>,
}

/// A response header retained independently of the HTTP implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseHeader {
    /// Header name.
    pub name: String,
    /// Header value. Invalid UTF-8 bytes are represented lossily.
    pub value: String,
}

/// A completed HTTP response and its execution metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    /// Numeric HTTP status code.
    pub status: u16,
    /// Canonical reason phrase when one is defined.
    pub reason: String,
    /// Final URL after redirects.
    pub url: String,
    /// Total time through receipt of the complete response body.
    pub duration: Duration,
    /// Decoded response-body size in bytes.
    pub size: usize,
    /// Response headers sorted deterministically by name and value.
    pub headers: Vec<ResponseHeader>,
    /// Raw response body.
    pub body: Vec<u8>,
    /// Whether `body` contains the complete response body.
    pub body_complete: bool,
}

/// A request-construction, cancellation, timeout, or transport failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpError {
    /// Request method is absent.
    MissingMethod,
    /// Request URL is absent.
    MissingUrl,
    /// Method is outside the Phase 5 HTTP subset.
    UnsupportedMethod(String),
    /// A request header name is invalid.
    InvalidHeaderName(String),
    /// A request header value is invalid.
    InvalidHeaderValue(String),
    /// A body variant could not be selected unambiguously.
    InvalidBodySelection(String),
    /// A body definition is inconsistent with its declared kind.
    InvalidBody(String),
    /// A required authentication property is absent or not a string.
    MissingAuthenticationProperty {
        /// Authentication scheme.
        scheme: &'static str,
        /// Required property.
        property: &'static str,
    },
    /// The parsed authentication kind is not supported in Phase 5.
    UnsupportedAuthentication(String),
    /// A request-body file could not be opened or configured.
    File {
        /// Resolved file path.
        path: PathBuf,
        /// Underlying diagnostic.
        message: String,
    },
    /// The HTTP client could not be configured.
    ClientConfiguration(String),
    /// Reqwest rejected the method, URL, headers, or another request component.
    InvalidRequest(String),
    /// The configured total timeout elapsed.
    Timeout,
    /// Execution was cancelled by the caller.
    Cancelled,
    /// Connection, protocol, or response-body failure.
    Transport(String),
    /// A response body could not be written to its requested destination.
    ResponseOutput {
        /// Requested final output path.
        path: PathBuf,
        /// Underlying diagnostic.
        message: String,
    },
}

impl HttpError {
    /// Returns whether this is a deterministic request-configuration failure.
    #[must_use]
    pub const fn is_configuration(&self) -> bool {
        matches!(
            self,
            Self::MissingMethod
                | Self::MissingUrl
                | Self::UnsupportedMethod(_)
                | Self::InvalidHeaderName(_)
                | Self::InvalidHeaderValue(_)
                | Self::InvalidBodySelection(_)
                | Self::InvalidBody(_)
                | Self::MissingAuthenticationProperty { .. }
                | Self::UnsupportedAuthentication(_)
                | Self::File { .. }
                | Self::ClientConfiguration(_)
                | Self::InvalidRequest(_)
        )
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMethod => write!(formatter, "HTTP request method is not set"),
            Self::MissingUrl => write!(formatter, "HTTP request URL is not set"),
            Self::UnsupportedMethod(method) => {
                write!(formatter, "unsupported HTTP method: {method}")
            }
            Self::InvalidHeaderName(name) => write!(formatter, "invalid header name: {name}"),
            Self::InvalidHeaderValue(name) => {
                write!(formatter, "invalid value for header: {name}")
            }
            Self::InvalidBodySelection(message) | Self::InvalidBody(message) => {
                formatter.write_str(message)
            }
            Self::MissingAuthenticationProperty { scheme, property } => {
                write!(formatter, "{scheme} authentication requires '{property}'")
            }
            Self::UnsupportedAuthentication(scheme) => {
                write!(formatter, "unsupported authentication scheme: {scheme}")
            }
            Self::File { path, message } => {
                write!(formatter, "cannot read '{}': {message}", path.display())
            }
            Self::ClientConfiguration(message) => {
                write!(formatter, "cannot configure HTTP client: {message}")
            }
            Self::InvalidRequest(message) => {
                write!(formatter, "cannot construct HTTP request: {message}")
            }
            Self::Timeout => write!(formatter, "HTTP request timed out"),
            Self::Cancelled => write!(formatter, "HTTP request was cancelled"),
            Self::Transport(message) => write!(formatter, "HTTP execution failed: {message}"),
            Self::ResponseOutput { path, message } => write!(
                formatter,
                "cannot write response body to '{}': {message}",
                path.display()
            ),
        }
    }
}

impl Error for HttpError {}

/// Reusable asynchronous HTTP engine shared by every interface.
#[derive(Clone, Debug)]
pub struct HttpEngine {
    default_client: Client,
}

impl HttpEngine {
    /// Creates an engine with the default redirect policy.
    pub fn new() -> Result<Self, HttpError> {
        let default_client = build_client(true, DEFAULT_MAX_REDIRECTS)?;
        Ok(Self { default_client })
    }

    /// Executes a request until completion.
    pub async fn execute(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
    ) -> Result<HttpResponse, HttpError> {
        self.execute_cancellable(request, options, pending::<()>())
            .await
    }

    /// Executes a request while streaming its response body to a file.
    ///
    /// The destination is replaced only after the full response body has been written.
    pub async fn execute_to_file(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        output: &Path,
    ) -> Result<HttpResponse, HttpError> {
        self.execute_cancellable_to_file(request, options, output, pending::<()>())
            .await
    }

    /// Executes a request, cancelling it when `cancellation` completes.
    ///
    /// Dropping the execution future also cancels the underlying reqwest request.
    pub async fn execute_cancellable<C>(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        cancellation: C,
    ) -> Result<HttpResponse, HttpError>
    where
        C: Future + Send,
    {
        tokio::pin!(cancellation);
        tokio::select! {
            biased;
            _ = &mut cancellation => Err(HttpError::Cancelled),
            response = self.execute_inner(request, options, None) => response,
        }
    }

    /// Executes a cancellable request while streaming its response body to a file.
    pub async fn execute_cancellable_to_file<C>(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        output: &Path,
        cancellation: C,
    ) -> Result<HttpResponse, HttpError>
    where
        C: Future + Send,
    {
        tokio::pin!(cancellation);
        tokio::select! {
            biased;
            _ = &mut cancellation => Err(HttpError::Cancelled),
            response = self.execute_inner(request, options, Some(output)) => response,
        }
    }

    async fn execute_inner(
        &self,
        request: &HttpRequest,
        options: &ExecutionOptions,
        output: Option<&Path>,
    ) -> Result<HttpResponse, HttpError> {
        let client = self.client_for(&request.settings)?;
        let builder = build_request(&client, request, options).await?;
        let started = Instant::now();
        let mut response = builder.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        let url = response.url().to_string();
        let mut headers = response_headers(response.headers());
        headers.sort_by(|left, right| (&left.name, &left.value).cmp(&(&right.name, &right.value)));
        let (body, size, body_complete) = if let Some(output) = output {
            let size = stream_response_to_file(&mut response, output).await?;
            (Vec::new(), size, false)
        } else {
            collect_bounded_response(&mut response).await?
        };
        Ok(HttpResponse {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or_default().to_owned(),
            url,
            duration: started.elapsed(),
            size,
            headers,
            body,
            body_complete,
        })
    }

    fn client_for(&self, settings: &RequestSettings) -> Result<Client, HttpError> {
        let follow = settings.follow_redirects.unwrap_or(true);
        let maximum = settings.max_redirects.unwrap_or(DEFAULT_MAX_REDIRECTS);
        if follow && maximum == DEFAULT_MAX_REDIRECTS {
            Ok(self.default_client.clone())
        } else {
            build_client(follow, maximum)
        }
    }
}

async fn collect_bounded_response(
    response: &mut Response,
) -> Result<(Vec<u8>, usize, bool), HttpError> {
    let mut body = Vec::new();
    let mut size = 0_usize;
    let mut complete = true;
    while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
        size = size.checked_add(chunk.len()).ok_or_else(|| {
            HttpError::Transport("response body size exceeds platform limits".to_owned())
        })?;
        if complete && size <= MAX_IN_MEMORY_RESPONSE_BYTES {
            body.extend_from_slice(&chunk);
        } else if complete {
            body.clear();
            complete = false;
        }
    }
    Ok((body, size, complete))
}

async fn stream_response_to_file(
    response: &mut Response,
    output: &Path,
) -> Result<usize, HttpError> {
    let (mut file, mut temporary) = create_response_temp_file(output).await?;
    let mut size = 0_usize;
    while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
        size = size.checked_add(chunk.len()).ok_or_else(|| {
            HttpError::Transport("response body size exceeds platform limits".to_owned())
        })?;
        file.write_all(&chunk)
            .await
            .map_err(|error| response_output_error(output, error))?;
    }
    file.flush()
        .await
        .map_err(|error| response_output_error(output, error))?;
    file.sync_all()
        .await
        .map_err(|error| response_output_error(output, error))?;
    drop(file);
    replace_response_output(temporary.path(), output).await?;
    temporary.committed = true;
    Ok(size)
}

async fn create_response_temp_file(
    output: &Path,
) -> Result<(tokio::fs::File, TemporaryResponseFile), HttpError> {
    let file_name = output
        .file_name()
        .ok_or_else(|| HttpError::ResponseOutput {
            path: output.to_owned(),
            message: "output path has no file name".to_owned(),
        })?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let sequence = RESPONSE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".probe-{}-{sequence}.part", std::process::id()));
        let temporary = parent.join(temporary_name);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
        {
            Ok(file) => {
                return Ok((
                    file,
                    TemporaryResponseFile {
                        path: temporary,
                        committed: false,
                    },
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(response_output_error(output, error)),
        }
    }
}

async fn replace_response_output(temporary: &Path, output: &Path) -> Result<(), HttpError> {
    #[cfg(windows)]
    if tokio::fs::try_exists(output)
        .await
        .map_err(|error| response_output_error(output, error))?
    {
        tokio::fs::remove_file(output)
            .await
            .map_err(|error| response_output_error(output, error))?;
    }
    tokio::fs::rename(temporary, output)
        .await
        .map_err(|error| response_output_error(output, error))
}

fn response_output_error(output: &Path, error: std::io::Error) -> HttpError {
    HttpError::ResponseOutput {
        path: output.to_owned(),
        message: error.to_string(),
    }
}

struct TemporaryResponseFile {
    path: PathBuf,
    committed: bool,
}

impl TemporaryResponseFile {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryResponseFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn build_client(follow_redirects: bool, maximum: usize) -> Result<Client, HttpError> {
    let policy = if follow_redirects {
        Policy::limited(maximum)
    } else {
        Policy::none()
    };
    Client::builder()
        .redirect(policy)
        .build()
        .map_err(|error| HttpError::ClientConfiguration(error.to_string()))
}

async fn build_request(
    client: &Client,
    request: &HttpRequest,
    options: &ExecutionOptions,
) -> Result<RequestBuilder, HttpError> {
    let method = supported_method(request.method.as_deref())?;
    let url = request.url.as_deref().ok_or(HttpError::MissingUrl)?;
    let url = probe_core::apply_path_parameters(url, &request.path_parameters);
    let headers = request_headers(request)?;
    let has_content_type = headers.contains_key(CONTENT_TYPE);
    let mut builder = client.request(method, url).headers(headers);

    let query: Vec<_> = request
        .query_parameters
        .iter()
        .filter(|parameter| !parameter.disabled && has_parameter_name(&parameter.name))
        .map(|parameter| (parameter.name.as_str(), parameter.value.as_str()))
        .collect();
    if !query.is_empty() {
        builder = builder.query(&query);
    }
    if let Some(timeout) = request
        .settings
        .timeout
        .filter(|value| *value > Duration::ZERO)
    {
        builder = builder.timeout(timeout);
    }
    if let Some(body) = selected_body(request.body.as_ref())? {
        builder = apply_body(builder, body, has_content_type, options).await?;
    }
    if let Some(authentication) = request.authentication.as_ref() {
        builder = apply_authentication(builder, authentication)?;
    }
    Ok(builder)
}

fn has_parameter_name(name: &str) -> bool {
    !name.trim().is_empty()
}

fn supported_method(method: Option<&str>) -> Result<Method, HttpError> {
    let method = method
        .ok_or(HttpError::MissingMethod)?
        .trim()
        .to_uppercase();
    match method.as_str() {
        "GET" => Ok(Method::GET),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        _ => Err(HttpError::UnsupportedMethod(method)),
    }
}

fn request_headers(request: &HttpRequest) -> Result<HeaderMap, HttpError> {
    let mut headers = HeaderMap::new();
    for header in request
        .headers
        .iter()
        .filter(|header| !header.disabled && has_parameter_name(&header.name))
    {
        let name = HeaderName::from_bytes(header.name.as_bytes())
            .map_err(|_| HttpError::InvalidHeaderName(header.name.clone()))?;
        let value = HeaderValue::from_str(&header.value)
            .map_err(|_| HttpError::InvalidHeaderValue(header.name.clone()))?;
        headers.append(name, value);
    }
    Ok(headers)
}

fn selected_body(body: Option<&RequestBody>) -> Result<Option<&Body>, HttpError> {
    match body {
        None => Ok(None),
        Some(RequestBody::Single(body)) => Ok(Some(body)),
        Some(RequestBody::Variants(variants)) => {
            let mut selected = variants.iter().filter(|variant| variant.selected);
            let body = selected.next().ok_or_else(|| {
                HttpError::InvalidBodySelection(
                    "request body variants have no selected value".to_owned(),
                )
            })?;
            if selected.next().is_some() {
                return Err(HttpError::InvalidBodySelection(
                    "request body variants have multiple selected values".to_owned(),
                ));
            }
            Ok(Some(&body.body))
        }
    }
}

async fn apply_body(
    mut builder: RequestBuilder,
    body: &Body,
    has_content_type: bool,
    options: &ExecutionOptions,
) -> Result<RequestBuilder, HttpError> {
    match body {
        Body::Raw(body) => {
            if !has_content_type {
                builder = builder.header(CONTENT_TYPE, raw_content_type(&body.kind));
            }
            Ok(builder.body(raw_body_data(body)))
        }
        Body::FormUrlEncoded(fields) => {
            let fields: Vec<_> = fields
                .iter()
                .filter(|field| !field.disabled && has_parameter_name(&field.name))
                .map(|field| (field.name.as_str(), field.value.as_str()))
                .collect();
            Ok(builder.form(&fields))
        }
        Body::Multipart(parts) => {
            let mut form = Form::new();
            for part in parts
                .iter()
                .filter(|part| !part.disabled && has_parameter_name(&part.name))
            {
                match part.kind {
                    MultipartPartKind::Text => {
                        let MultipartValue::Single(value) = &part.value else {
                            return Err(HttpError::InvalidBody(format!(
                                "multipart text field '{}' must have one value",
                                part.name
                            )));
                        };
                        let mut value = Part::text(value.clone());
                        if let Some(content_type) = part.content_type.as_deref() {
                            value = value.mime_str(content_type).map_err(|error| {
                                HttpError::InvalidBody(format!(
                                    "invalid content type for multipart field '{}': {error}",
                                    part.name
                                ))
                            })?;
                        }
                        form = form.part(part.name.clone(), value);
                    }
                    MultipartPartKind::File => {
                        let paths: Vec<&str> = match &part.value {
                            MultipartValue::Single(path) => vec![path],
                            MultipartValue::Multiple(paths) => {
                                paths.iter().map(String::as_str).collect()
                            }
                        };
                        for path in paths {
                            let resolved = resolve_path(path, options);
                            let mut value =
                                Part::file(&resolved)
                                    .await
                                    .map_err(|error| HttpError::File {
                                        path: resolved.clone(),
                                        message: error.to_string(),
                                    })?;
                            if let Some(content_type) = part.content_type.as_deref() {
                                value = value.mime_str(content_type).map_err(|error| {
                                    HttpError::InvalidBody(format!(
                                        "invalid content type for multipart field '{}': {error}",
                                        part.name
                                    ))
                                })?;
                            }
                            form = form.part(part.name.clone(), value);
                        }
                    }
                }
            }
            Ok(builder.multipart(form))
        }
        Body::File(files) => {
            let mut selected = files.iter().filter(|file| file.selected);
            let file = selected.next().ok_or_else(|| {
                HttpError::InvalidBodySelection("file body has no selected file".to_owned())
            })?;
            if selected.next().is_some() {
                return Err(HttpError::InvalidBodySelection(
                    "file body has multiple selected files".to_owned(),
                ));
            }
            let path = resolve_path(&file.file_path, options);
            let handle = tokio::fs::File::open(&path)
                .await
                .map_err(|error| HttpError::File {
                    path: path.clone(),
                    message: error.to_string(),
                })?;
            if !has_content_type {
                builder = builder.header(CONTENT_TYPE, file.content_type.as_str());
            }
            Ok(builder.body(reqwest::Body::from(handle)))
        }
    }
}

fn resolve_path(path: &str, options: &ExecutionOptions) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_owned()
    } else if let Some(base) = options.base_directory.as_ref() {
        base.join(path)
    } else {
        path.to_owned()
    }
}

fn apply_authentication(
    builder: RequestBuilder,
    authentication: &Authentication,
) -> Result<RequestBuilder, HttpError> {
    match &authentication.kind {
        AuthenticationKind::Basic => {
            let username = authentication_string(authentication, "username", "basic")?;
            let password = authentication_string(authentication, "password", "basic")?;
            Ok(builder.basic_auth(username, Some(password)))
        }
        AuthenticationKind::Bearer => {
            let token = authentication_string(authentication, "token", "bearer")?;
            Ok(builder.bearer_auth(token))
        }
        kind => Err(HttpError::UnsupportedAuthentication(
            kind.as_str().to_owned(),
        )),
    }
}

fn authentication_string<'a>(
    authentication: &'a Authentication,
    property: &'static str,
    scheme: &'static str,
) -> Result<&'a str, HttpError> {
    match authentication.properties.get(property) {
        Some(AuthenticationValue::String(value)) => Ok(value),
        _ => Err(HttpError::MissingAuthenticationProperty { scheme, property }),
    }
}

const fn raw_content_type(kind: &RawBodyKind) -> &'static str {
    match kind {
        RawBodyKind::Json => "application/json",
        RawBodyKind::Text => "text/plain; charset=utf-8",
        RawBodyKind::Xml => "application/xml",
        RawBodyKind::Sparql => "application/sparql-query",
    }
}

fn raw_body_data(body: &RawBody) -> String {
    match body.kind {
        RawBodyKind::Json => strip_json_comments(&body.data),
        RawBodyKind::Text | RawBodyKind::Xml | RawBodyKind::Sparql => body.data.clone(),
    }
}

fn strip_json_comments(source: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Normal,
        String { escaped: bool },
        LineComment,
        BlockComment { previous_was_star: bool },
    }

    let mut stripped = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut state = State::Normal;

    while let Some(character) = chars.next() {
        match state {
            State::Normal => match character {
                '"' => {
                    stripped.push('"');
                    state = State::String { escaped: false };
                }
                '/' => match chars.peek() {
                    Some('/') => {
                        let _ = chars.next();
                        stripped.push(' ');
                        stripped.push(' ');
                        state = State::LineComment;
                    }
                    Some('*') => {
                        let _ = chars.next();
                        stripped.push(' ');
                        stripped.push(' ');
                        state = State::BlockComment {
                            previous_was_star: false,
                        };
                    }
                    _ => stripped.push('/'),
                },
                _ => stripped.push(character),
            },
            State::String { escaped } => {
                stripped.push(character);
                state = match (escaped, character) {
                    (true, _) => State::String { escaped: false },
                    (false, '\\') => State::String { escaped: true },
                    (false, '"') => State::Normal,
                    (false, _) => State::String { escaped: false },
                };
            }
            State::LineComment => {
                if character == '\n' {
                    stripped.push('\n');
                    state = State::Normal;
                } else if character == '\r' {
                    stripped.push('\r');
                } else {
                    stripped.push(' ');
                }
            }
            State::BlockComment { previous_was_star } => {
                if character == '\n' {
                    stripped.push('\n');
                } else if character == '\r' {
                    stripped.push('\r');
                } else {
                    stripped.push(' ');
                }
                state = if previous_was_star && character == '/' {
                    State::Normal
                } else {
                    State::BlockComment {
                        previous_was_star: character == '*',
                    }
                };
            }
        }
    }

    stripped
}

fn response_headers(headers: &HeaderMap) -> Vec<ResponseHeader> {
    headers
        .iter()
        .map(|(name, value)| ResponseHeader {
            name: name.as_str().to_owned(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect()
}

fn map_reqwest_error(error: reqwest::Error) -> HttpError {
    if error.is_timeout() {
        HttpError::Timeout
    } else if error.is_builder() {
        HttpError::InvalidRequest(error.to_string())
    } else {
        HttpError::Transport(error.to_string())
    }
}
