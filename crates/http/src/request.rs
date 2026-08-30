use std::{path::Path, time::Duration};

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, HttpRequest, MultipartPart,
    MultipartPartKind, MultipartValue, RawBody, RawBodyKind, RequestBody,
};
use reqwest::{
    Client, Method, RequestBuilder,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
    multipart::{Form, Part},
};

use crate::{ExecutionOptions, HttpError};

pub(crate) async fn build_request(
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
        .filter(|parameter| is_enabled_and_named(parameter.disabled, &parameter.name))
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

fn is_enabled_and_named(disabled: bool, name: &str) -> bool {
    !disabled && !name.trim().is_empty()
}

fn supported_method(method: Option<&str>) -> Result<Method, HttpError> {
    let method = method.ok_or(HttpError::MissingMethod)?.trim();
    if method.eq_ignore_ascii_case("GET") {
        Ok(Method::GET)
    } else if method.eq_ignore_ascii_case("POST") {
        Ok(Method::POST)
    } else if method.eq_ignore_ascii_case("PUT") {
        Ok(Method::PUT)
    } else if method.eq_ignore_ascii_case("PATCH") {
        Ok(Method::PATCH)
    } else if method.eq_ignore_ascii_case("DELETE") {
        Ok(Method::DELETE)
    } else {
        Err(HttpError::UnsupportedMethod(method.to_uppercase()))
    }
}

fn request_headers(request: &HttpRequest) -> Result<HeaderMap, HttpError> {
    let mut headers = HeaderMap::new();
    for header in request
        .headers
        .iter()
        .filter(|header| is_enabled_and_named(header.disabled, &header.name))
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
                .filter(|field| is_enabled_and_named(field.disabled, &field.name))
                .map(|field| (field.name.as_str(), field.value.as_str()))
                .collect();
            Ok(builder.form(&fields))
        }
        Body::Multipart(parts) => {
            let mut form = Form::new();
            for part in parts
                .iter()
                .filter(|part| is_enabled_and_named(part.disabled, &part.name))
            {
                form = apply_multipart_part(form, part, options).await?;
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
                .map_err(|error| file_error(path.clone(), error))?;
            if !has_content_type {
                builder = builder.header(CONTENT_TYPE, file.content_type.as_str());
            }
            Ok(builder.body(reqwest::Body::from(handle)))
        }
    }
}

async fn apply_multipart_part(
    mut form: Form,
    part: &MultipartPart,
    options: &ExecutionOptions,
) -> Result<Form, HttpError> {
    match part.kind {
        MultipartPartKind::Text => {
            let MultipartValue::Single(value) = &part.value else {
                return Err(HttpError::InvalidBody(format!(
                    "multipart text field '{}' must have one value",
                    part.name
                )));
            };
            let value = apply_part_content_type(Part::text(value.clone()), part)?;
            Ok(form.part(part.name.clone(), value))
        }
        MultipartPartKind::File => {
            match &part.value {
                MultipartValue::Single(path) => {
                    form = add_multipart_file(form, part, path, options).await?;
                }
                MultipartValue::Multiple(paths) => {
                    for path in paths {
                        form = add_multipart_file(form, part, path, options).await?;
                    }
                }
            }
            Ok(form)
        }
    }
}

async fn add_multipart_file(
    form: Form,
    part: &MultipartPart,
    path: &str,
    options: &ExecutionOptions,
) -> Result<Form, HttpError> {
    let path = resolve_path(path, options);
    let value = Part::file(&path)
        .await
        .map_err(|error| file_error(path.clone(), error))?;
    let value = apply_part_content_type(value, part)?;
    Ok(form.part(part.name.clone(), value))
}

fn apply_part_content_type(value: Part, part: &MultipartPart) -> Result<Part, HttpError> {
    let Some(content_type) = part.content_type.as_deref() else {
        return Ok(value);
    };
    value.mime_str(content_type).map_err(|error| {
        HttpError::InvalidBody(format!(
            "invalid content type for multipart field '{}': {error}",
            part.name
        ))
    })
}

fn resolve_path(path: &str, options: &ExecutionOptions) -> std::path::PathBuf {
    let path = Path::new(path);
    match (&options.base_directory, path.is_absolute()) {
        (_, true) | (None, false) => path.to_owned(),
        (Some(base), false) => base.join(path),
    }
}

fn file_error(path: std::path::PathBuf, error: std::io::Error) -> HttpError {
    HttpError::File {
        path,
        message: error.to_string(),
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
                        chars.next();
                        stripped.push_str("  ");
                        state = State::LineComment;
                    }
                    Some('*') => {
                        chars.next();
                        stripped.push_str("  ");
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
            State::LineComment => match character {
                '\n' => {
                    stripped.push('\n');
                    state = State::Normal;
                }
                '\r' => stripped.push('\r'),
                _ => stripped.push(' '),
            },
            State::BlockComment { previous_was_star } => {
                stripped.push(if matches!(character, '\n' | '\r') {
                    character
                } else {
                    ' '
                });
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
