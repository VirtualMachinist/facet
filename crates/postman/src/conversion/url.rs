use probe_core::{ImportDiagnostic, QueryParameter};
use serde::Deserialize;
use serde_json::Value;

use super::{
    convert_string,
    variables::{diagnose_variable_metadata, variable_name},
};
use crate::{
    PostmanImportError,
    diagnostics::{
        extra_fields as diagnose_extra_fields, meaningful, nonempty,
        nonempty_description as diagnose_nonempty_description, value_string,
    },
    schema::PostmanUrl,
};

pub(super) fn convert_url(
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<(String, Vec<QueryParameter>, Vec<QueryParameter>), PostmanImportError> {
    if let Some(url) = value.as_str() {
        return Ok((
            convert_string(url, "request", resource_id, "url", diagnostics),
            Vec::new(),
            Vec::new(),
        ));
    }
    let url = PostmanUrl::deserialize(value).map_err(|error| {
        PostmanImportError::Invalid(format!(
            "invalid Postman URL for request '{resource_id}': {error}"
        ))
    })?;
    diagnose_extra_fields("url", Some(resource_id), &url.extra, diagnostics);
    let query_parameters = url
        .query
        .iter()
        .map(|parameter| {
            diagnose_nonempty_description(
                "query_parameter",
                resource_id,
                &parameter.description,
                diagnostics,
            );
            diagnose_extra_fields(
                "query_parameter",
                Some(resource_id),
                &parameter.extra,
                diagnostics,
            );
            QueryParameter {
                name: value_string(&parameter.key),
                value: convert_string(
                    &value_string(&parameter.value),
                    "request",
                    resource_id,
                    "url.query.value",
                    diagnostics,
                ),
                disabled: parameter.disabled,
            }
        })
        .collect::<Vec<_>>();
    let path_parameters = url
        .variable
        .iter()
        .map(|variable| {
            diagnose_variable_metadata(variable, "path_variable", resource_id, diagnostics);
            QueryParameter {
                name: variable_name(variable).unwrap_or_default(),
                value: convert_string(
                    &value_string(&variable.value),
                    "request",
                    resource_id,
                    "url.variable.value",
                    diagnostics,
                ),
                disabled: variable.disabled,
            }
        })
        .collect::<Vec<_>>();

    let raw = url
        .raw
        .as_deref()
        .filter(|raw| !raw.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_default();
    let structured = url
        .protocol
        .as_deref()
        .is_some_and(|value| !value.is_empty())
        || meaningful(&url.host)
        || meaningful(&url.path)
        || !url.port.is_empty()
        || !url.hash.is_empty();
    let base = if structured {
        reconstruct_url(&url)
    } else if query_parameters.is_empty() {
        raw
    } else {
        strip_query_preserving_fragment(&raw)
    };
    if base.trim().is_empty() {
        return Err(PostmanImportError::Invalid(format!(
            "Postman request '{resource_id}' has no usable URL"
        )));
    }
    Ok((
        convert_string(&base, "request", resource_id, "url", diagnostics),
        query_parameters,
        path_parameters,
    ))
}

fn reconstruct_url(url: &PostmanUrl) -> String {
    let host = match &url.host {
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("."),
        _ => String::new(),
    };
    let path = match &url.path {
        Value::String(value) => value.trim_start_matches('/').to_owned(),
        Value::Array(values) => values
            .iter()
            .filter_map(|value| {
                value.as_str().map(str::to_owned).or_else(|| {
                    value
                        .get("value")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
            })
            .collect::<Vec<_>>()
            .join("/"),
        _ => String::new(),
    };
    let protocol = url.protocol.as_deref().unwrap_or_default();
    let mut result = if protocol.is_empty() {
        host
    } else {
        format!("{protocol}://{host}")
    };
    if let Some(port) = nonempty(&url.port) {
        result.push(':');
        result.push_str(&port);
    }
    if !path.is_empty() {
        result.push('/');
        result.push_str(&path);
    }
    if let Some(hash) = nonempty(&url.hash) {
        result.push('#');
        result.push_str(&hash);
    }
    result
}

fn strip_query_preserving_fragment(raw: &str) -> String {
    let Some(query) = raw.find('?') else {
        return raw.to_owned();
    };
    let fragment = raw[query..]
        .find('#')
        .map(|offset| &raw[query + offset..])
        .unwrap_or_default();
    format!("{}{}", &raw[..query], fragment)
}
