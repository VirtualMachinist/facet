use std::collections::BTreeMap;

use probe_core::{Authentication, AuthenticationKind, AuthenticationValue, ImportDiagnostic};
use serde_json::Value;

use super::convert_string;
use crate::{
    PostmanImportError, PostmanSourceFormat,
    diagnostics::{lossy, meaningful, value_string, warning},
};

pub(super) fn convert_authentication(
    auth: Option<&Value>,
    format: PostmanSourceFormat,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Option<Authentication>, PostmanImportError> {
    let Some(auth) = auth.filter(|auth| !auth.is_null()) else {
        return Ok(None);
    };
    let object = auth.as_object().ok_or_else(|| {
        PostmanImportError::Invalid(format!(
            "Postman authentication for request '{resource_id}' must be an object"
        ))
    })?;
    let auth_type = object.get("type").and_then(Value::as_str).ok_or_else(|| {
        PostmanImportError::Invalid(format!(
            "Postman authentication for request '{resource_id}' is missing type"
        ))
    })?;
    if auth_type == "noauth" {
        return Ok(None);
    }
    for (field, value) in object {
        if field != "type" && field != auth_type && meaningful(value) {
            diagnostics.push(lossy(
                "inactive_auth_configuration",
                "request",
                Some(resource_id),
                Some(field),
                "inactive Postman authentication configuration cannot be represented on the selected authentication method",
            ));
        }
    }
    let kind = authentication_kind(auth_type);
    if !matches!(kind, AuthenticationKind::Basic | AuthenticationKind::Bearer) {
        diagnostics.push(warning(
            "execution_unsupported",
            "request",
            Some(resource_id),
            Some("auth.type"),
            &format!(
                "authentication type '{}' is preserved but the current Probe HTTP engine cannot execute it",
                kind.as_str()
            ),
        ));
    }
    let properties = convert_properties(
        format,
        object.get(auth_type).unwrap_or(&Value::Null),
        resource_id,
        diagnostics,
    )?;
    Ok(Some(Authentication { kind, properties }))
}

fn authentication_kind(auth_type: &str) -> AuthenticationKind {
    match auth_type {
        "apikey" => AuthenticationKind::ApiKey,
        "awsv4" => AuthenticationKind::AwsV4,
        "basic" => AuthenticationKind::Basic,
        "bearer" => AuthenticationKind::Bearer,
        "digest" => AuthenticationKind::Digest,
        "ntlm" => AuthenticationKind::Ntlm,
        "oauth1" => AuthenticationKind::OAuth1,
        "oauth2" => AuthenticationKind::OAuth2,
        other => AuthenticationKind::Other(other.to_owned()),
    }
}

fn convert_properties(
    format: PostmanSourceFormat,
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<BTreeMap<String, AuthenticationValue>, PostmanImportError> {
    let properties = match (format, value) {
        (_, Value::Null) => BTreeMap::new(),
        (PostmanSourceFormat::CollectionV2, Value::Object(properties)) => properties
            .iter()
            .map(|(name, value)| (name.clone(), auth_value(value, resource_id, diagnostics)))
            .collect(),
        (PostmanSourceFormat::CollectionV2_1, Value::Array(attributes)) => attributes
            .iter()
            .map(|attribute| convert_attribute(attribute, resource_id, diagnostics))
            .collect::<Result<_, _>>()?,
        (PostmanSourceFormat::CollectionV2, _) => {
            return Err(PostmanImportError::Invalid(format!(
                "Postman v2 authentication properties for request '{resource_id}' must be an object"
            )));
        }
        (PostmanSourceFormat::CollectionV2_1, _) => {
            return Err(PostmanImportError::Invalid(format!(
                "Postman v2.1 authentication properties for request '{resource_id}' must be an array"
            )));
        }
    };
    Ok(properties)
}

fn convert_attribute(
    attribute: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<(String, AuthenticationValue), PostmanImportError> {
    let attribute = attribute.as_object().ok_or_else(|| {
        PostmanImportError::Invalid(format!(
            "Postman authentication attribute for request '{resource_id}' must be an object"
        ))
    })?;
    let key = attribute
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            PostmanImportError::Invalid(format!(
                "Postman authentication attribute for request '{resource_id}' is missing key"
            ))
        })?
        .to_owned();
    for (field, value) in attribute {
        if !matches!(field.as_str(), "key" | "value" | "type") && meaningful(value) {
            diagnostics.push(lossy(
                "unknown_field",
                "authentication_attribute",
                Some(resource_id),
                Some(field),
                &format!(
                    "unknown Postman authentication field '{field}' cannot be guaranteed to survive import"
                ),
            ));
        }
    }
    let value = attribute.get("value").unwrap_or(&Value::Null);
    let value_type = attribute.get("type").and_then(Value::as_str);
    Ok((
        key,
        typed_auth_value(value, value_type, resource_id, diagnostics),
    ))
}

fn auth_value(
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> AuthenticationValue {
    match value {
        Value::Null => AuthenticationValue::Null,
        Value::Bool(value) => AuthenticationValue::Boolean(*value),
        Value::Number(value) => AuthenticationValue::Number(value.to_string()),
        Value::String(value) => AuthenticationValue::String(convert_string(
            value,
            "request",
            resource_id,
            "auth.value",
            diagnostics,
        )),
        Value::Array(values) => AuthenticationValue::Sequence(
            values
                .iter()
                .map(|value| auth_value(value, resource_id, diagnostics))
                .collect(),
        ),
        Value::Object(values) => AuthenticationValue::Object(
            values
                .iter()
                .map(|(name, value)| (name.clone(), auth_value(value, resource_id, diagnostics)))
                .collect(),
        ),
    }
}

fn typed_auth_value(
    value: &Value,
    value_type: Option<&str>,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> AuthenticationValue {
    match value_type {
        Some("boolean") => match value {
            Value::Bool(value) => AuthenticationValue::Boolean(*value),
            Value::String(value) if value.eq_ignore_ascii_case("true") => {
                AuthenticationValue::Boolean(true)
            }
            Value::String(value) if value.eq_ignore_ascii_case("false") => {
                AuthenticationValue::Boolean(false)
            }
            _ => auth_value(value, resource_id, diagnostics),
        },
        Some("number") => AuthenticationValue::Number(convert_string(
            &value_string(value),
            "request",
            resource_id,
            "auth.value",
            diagnostics,
        )),
        Some("string") => AuthenticationValue::String(convert_string(
            &value_string(value),
            "request",
            resource_id,
            "auth.value",
            diagnostics,
        )),
        Some("any" | "default") | None => auth_value(value, resource_id, diagnostics),
        Some(kind) => {
            diagnostics.push(lossy(
                "unsupported_auth_value_type",
                "authentication_attribute",
                Some(resource_id),
                Some("type"),
                &format!("Postman authentication value type '{kind}' is not supported"),
            ));
            auth_value(value, resource_id, diagnostics)
        }
    }
}
