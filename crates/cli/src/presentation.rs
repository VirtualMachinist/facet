use std::path::Path;

use probe_core::{
    AuthenticationValue, Body, GraphqlOperation, GraphqlRequestError, HttpRequest,
    MultipartPartKind, MultipartValue, RawBodyKind, RequestBody, VariableValueType,
};
use probe_http::{HttpResponse, MAX_IN_MEMORY_RESPONSE_BYTES};
use serde_json::{Map, Value, json};

pub(super) fn response_human(
    request: &HttpRequest,
    response: &HttpResponse,
    output: Option<&Path>,
) -> String {
    let method = request.method.as_deref().unwrap_or("<unset>");
    let url = request.url.as_deref().unwrap_or("<unset>");
    let mut rendered = format!(
        "{method} {url}\n\n{} {}\n{} ms\n{}\nFinal URL: {}\nHeaders:\n",
        response.status,
        response.reason,
        response.duration.as_millis(),
        human_size(response.size),
        response.url,
    );
    if response.headers.is_empty() {
        rendered.push_str("  (none)\n");
    } else {
        for header in &response.headers {
            rendered.push_str(&format!("  {}: {}\n", header.name, header.value));
        }
    }
    rendered.push('\n');
    if let Some(output) = output {
        rendered.push_str(&format!("Response body written to {}\n", output.display()));
    } else if !response.body_complete {
        rendered.push_str(&format!(
            "Response body omitted because it exceeds {} bytes; use --output <file>.\n",
            MAX_IN_MEMORY_RESPONSE_BYTES
        ));
    } else if let Ok(body) = std::str::from_utf8(&response.body) {
        rendered.push_str(body);
        if !body.ends_with('\n') {
            rendered.push('\n');
        }
    } else {
        rendered.push_str("Binary response body omitted; use --output <file>.\n");
    }
    rendered
}

pub(super) fn response_json(
    request: &HttpRequest,
    response: &HttpResponse,
    output: Option<&Path>,
) -> Result<Value, GraphqlRequestError> {
    let output_path = output.map(|path| path.to_string_lossy().into_owned());
    let (content, encoding, omitted, omission_reason) = if output.is_some() {
        (None, None, false, None)
    } else if !response.body_complete {
        (None, None, true, Some("too_large"))
    } else if let Ok(body) = std::str::from_utf8(&response.body) {
        (Some(body), Some("utf8"), false, None)
    } else {
        (None, None, true, Some("binary"))
    };
    Ok(json!({
        "request": run_request_json(request)?,
        "response": {
            "body": {
                "content": content,
                "encoding": encoding,
                "omissionReason": omission_reason,
                "omitted": omitted,
                "outputPath": output_path,
            },
            "durationMs": response.duration.as_millis(),
            "headers": response.headers.iter().map(|header| json!({
                "name": header.name,
                "value": header.value,
            })).collect::<Vec<_>>(),
            "reason": response.reason,
            "sizeBytes": response.size,
            "status": response.status,
            "url": response.url,
        }
    }))
}

pub(super) fn dry_run_human(request: &HttpRequest) -> String {
    format!(
        "{} {}\n",
        request.method.as_deref().unwrap_or("<unset>"),
        request.url.as_deref().unwrap_or("<unset>"),
    )
}

pub(super) fn dry_run_json(request: &HttpRequest) -> Result<Value, GraphqlRequestError> {
    Ok(json!({
        "dryRun": true,
        "request": run_request_json(request)?,
    }))
}

fn run_request_json(request: &HttpRequest) -> Result<Value, GraphqlRequestError> {
    Ok(json!({
        "type": request.protocol.as_str(),
        "graphql": request.selected_graphql()?.map(graphql_json),
        "method": request.method,
        "url": request.url,
    }))
}

fn human_size(size: usize) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    }
}

pub(super) fn request_human(
    selector: &str,
    environment: Option<&str>,
    request: &HttpRequest,
) -> Result<String, GraphqlRequestError> {
    let mut output = String::new();
    output.push_str(&format!(
        "Name: {}\nSelector: {selector}\nType: {}\nEnvironment: {}\nMethod: {}\nURL: {}\n",
        request.metadata.name.as_deref().unwrap_or("<unnamed>"),
        request.protocol.as_str(),
        environment.unwrap_or("<none>"),
        request.method.as_deref().unwrap_or("<unset>"),
        request.url.as_deref().unwrap_or("<unset>"),
    ));

    output.push_str("Headers:\n");
    if request.headers.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for header in &request.headers {
            let state = if header.disabled { " [disabled]" } else { "" };
            output.push_str(&format!("  {}: {}{state}\n", header.name, header.value));
        }
    }

    output.push_str("Path parameters:\n");
    if request.path_parameters.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for parameter in &request.path_parameters {
            let state = if parameter.disabled {
                " [disabled]"
            } else {
                ""
            };
            output.push_str(&format!(
                "  :{}={}{state}\n",
                parameter.name, parameter.value
            ));
        }
    }

    output.push_str("Query parameters:\n");
    if request.query_parameters.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for parameter in &request.query_parameters {
            let state = if parameter.disabled {
                " [disabled]"
            } else {
                ""
            };
            output.push_str(&format!(
                "  {}={}{state}\n",
                parameter.name, parameter.value
            ));
        }
    }

    if request.protocol.as_str() == "graphql" {
        output.push_str("Body: graphql\n");
        match request.selected_graphql()? {
            None => output.push_str("GraphQL: <unset>\n"),
            Some(graphql) => {
                output.push_str(&format!(
                    "GraphQL query: {}\nGraphQL variables: {}\nGraphQL operation name: {}\nGraphQL extensions: {}\n",
                    graphql.query.as_deref().unwrap_or("<unset>"),
                    graphql_object_human(graphql.variables.as_ref()),
                    graphql.operation_name.as_deref().unwrap_or("<unset>"),
                    graphql_object_human(graphql.extensions.as_ref()),
                ));
            }
        }
    } else {
        output.push_str(&format!("Body: {}\n", body_summary(request.body.as_ref())));
    }
    output.push_str(&format!(
        "Authentication: {}\n",
        request
            .authentication
            .as_ref()
            .map(|auth| auth.kind.as_str())
            .unwrap_or("<unset>"),
    ));
    Ok(output)
}

pub(super) fn request_json(
    selector: &str,
    environment: Option<&str>,
    request: &HttpRequest,
) -> Result<Value, GraphqlRequestError> {
    let headers: Vec<_> = request
        .headers
        .iter()
        .map(|header| {
            json!({
                "disabled": header.disabled,
                "name": header.name,
                "value": header.value,
            })
        })
        .collect();
    let query_parameters: Vec<_> = request
        .query_parameters
        .iter()
        .map(|parameter| {
            json!({
                "disabled": parameter.disabled,
                "name": parameter.name,
                "value": parameter.value,
            })
        })
        .collect();
    let path_parameters: Vec<_> = request
        .path_parameters
        .iter()
        .map(|parameter| {
            json!({
                "disabled": parameter.disabled,
                "name": parameter.name,
                "value": parameter.value,
            })
        })
        .collect();
    let authentication = request.authentication.as_ref().map(|auth| {
        let properties: Map<_, _> = auth
            .properties
            .iter()
            .map(|(name, value)| (name.clone(), authentication_value(value)))
            .collect();
        json!({
            "properties": properties,
            "type": auth.kind.as_str(),
        })
    });

    Ok(json!({
        "authentication": authentication,
        "body": request.body.as_ref().map(request_body_json),
        "environment": environment,
        "graphql": request.selected_graphql()?.map(graphql_json),
        "headers": headers,
        "method": request.method,
        "name": request.metadata.name,
        "pathParameters": path_parameters,
        "queryParameters": query_parameters,
        "selector": selector,
        "type": request.protocol.as_str(),
        "url": request.url,
    }))
}

fn body_summary(body: Option<&RequestBody>) -> &'static str {
    match body {
        None => "<unset>",
        Some(RequestBody::Single(Body::Raw(raw))) => raw_body_kind(&raw.kind),
        Some(RequestBody::Single(Body::FormUrlEncoded(_))) => "form-urlencoded",
        Some(RequestBody::Single(Body::Multipart(_))) => "multipart-form",
        Some(RequestBody::Single(Body::File(_))) => "file",
        Some(RequestBody::Variants(_)) => "variants",
    }
}

fn graphql_json(graphql: &GraphqlOperation) -> Value {
    json!({
        "extensions": graphql.extensions,
        "operationName": graphql.operation_name,
        "query": graphql.query,
        "variables": graphql.variables,
    })
}

fn graphql_object_human(value: Option<&Map<String, Value>>) -> String {
    value.map_or_else(
        || "<unset>".to_owned(),
        |value| Value::Object(value.clone()).to_string(),
    )
}

fn request_body_json(body: &RequestBody) -> Value {
    match body {
        RequestBody::Single(body) => json!({
            "mode": "single",
            "value": body_json(body),
        }),
        RequestBody::Variants(variants) => json!({
            "mode": "variants",
            "variants": variants.iter().map(|variant| json!({
                "body": body_json(&variant.body),
                "selected": variant.selected,
                "title": variant.title,
            })).collect::<Vec<_>>(),
        }),
    }
}

fn body_json(body: &Body) -> Value {
    match body {
        Body::Raw(body) => json!({
            "data": body.data,
            "type": raw_body_kind(&body.kind),
        }),
        Body::FormUrlEncoded(fields) => json!({
            "data": fields.iter().map(|field| json!({
                "disabled": field.disabled,
                "name": field.name,
                "value": field.value,
            })).collect::<Vec<_>>(),
            "type": "form-urlencoded",
        }),
        Body::Multipart(parts) => json!({
            "data": parts.iter().map(|part| json!({
                "contentType": part.content_type,
                "disabled": part.disabled,
                "name": part.name,
                "type": match part.kind {
                    MultipartPartKind::Text => "text",
                    MultipartPartKind::File => "file",
                },
                "value": match &part.value {
                    MultipartValue::Single(value) => json!(value),
                    MultipartValue::Multiple(values) => json!(values),
                },
            })).collect::<Vec<_>>(),
            "type": "multipart-form",
        }),
        Body::File(files) => json!({
            "data": files.iter().map(|file| json!({
                "contentType": file.content_type,
                "filePath": file.file_path,
                "selected": file.selected,
            })).collect::<Vec<_>>(),
            "type": "file",
        }),
    }
}

const fn raw_body_kind(kind: &RawBodyKind) -> &'static str {
    match kind {
        RawBodyKind::Json => "json",
        RawBodyKind::Text => "text",
        RawBodyKind::Xml => "xml",
        RawBodyKind::Sparql => "sparql",
    }
}

fn authentication_value(value: &AuthenticationValue) -> Value {
    match value {
        AuthenticationValue::String(value) => json!(value),
        AuthenticationValue::Boolean(value) => json!(value),
        AuthenticationValue::Number(value) => json!({
            "data": value,
            "type": VariableValueType::Number.as_str(),
        }),
        AuthenticationValue::Null => Value::Null,
        AuthenticationValue::Sequence(values) => {
            Value::Array(values.iter().map(authentication_value).collect())
        }
        AuthenticationValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(name, value)| (name.clone(), authentication_value(value)))
                .collect(),
        ),
    }
}
