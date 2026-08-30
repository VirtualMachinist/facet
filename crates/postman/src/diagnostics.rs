use std::collections::BTreeMap;

use probe_core::{ImportDiagnostic, ImportDiagnosticSeverity};
use serde_json::Value;

pub(super) fn convert_string(
    value: &str,
    resource_type: &str,
    resource_id: &str,
    field: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> String {
    if value.contains("{{$") {
        diagnostics.push(warning(
            "dynamic_variable_unsupported",
            resource_type,
            Some(resource_id),
            Some(field),
            "Postman dynamic variables are preserved literally but Probe cannot resolve them",
        ));
    }
    value.to_owned()
}

pub(super) fn events(
    resource_type: &str,
    resource_id: Option<&str>,
    events: &[Value],
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    if !events.is_empty() {
        diagnostics.push(lossy(
            "unsupported_scripts",
            resource_type,
            resource_id,
            Some("event"),
            "Postman pre-request and test scripts cannot be represented by the current Probe domain",
        ));
    }
}

pub(super) fn nonempty_description(
    resource_type: &str,
    resource_id: &str,
    description: &Value,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    if description_text(description).is_some() {
        diagnostics.push(lossy(
            "unsupported_description",
            resource_type,
            Some(resource_id),
            Some("description"),
            "this Postman description cannot be represented by the current Probe domain",
        ));
    }
}

pub(super) fn meaningful_value(
    code: &'static str,
    resource_type: &str,
    resource_id: Option<&str>,
    field: &str,
    value: &Value,
    message: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    if meaningful(value) {
        diagnostics.push(lossy(
            code,
            resource_type,
            resource_id,
            Some(field),
            message,
        ));
    }
}

pub(super) fn extra_fields(
    resource_type: &str,
    resource_id: Option<&str>,
    extra: &BTreeMap<String, Value>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    for field in extra
        .iter()
        .filter_map(|(field, value)| meaningful(value).then_some(field))
    {
        diagnostics.push(lossy(
            "unknown_field",
            resource_type,
            resource_id,
            Some(field),
            &format!("unknown Postman field '{field}' cannot be guaranteed to survive import"),
        ));
    }
}

pub(super) fn meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(value) => *value,
        Value::Number(_) => true,
    }
}

pub(super) fn description_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => nonempty(value),
        Value::Object(value) => value
            .get("content")
            .and_then(Value::as_str)
            .and_then(nonempty),
        _ => None,
    }
}

pub(super) fn version_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => nonempty(value),
        Value::Object(value) => {
            let major = value.get("major")?.as_u64()?;
            let minor = value.get("minor")?.as_u64()?;
            let patch = value.get("patch")?.as_u64()?;
            let suffix = value
                .get("identifier")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| format!("-{value}"))
                .unwrap_or_default();
            Some(format!("{major}.{minor}.{patch}{suffix}"))
        }
        _ => None,
    }
}

pub(super) fn value_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).expect("JSON values must serialize")
        }
    }
}

pub(super) fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

pub(super) fn warning(
    code: &'static str,
    resource_type: &str,
    resource_id: Option<&str>,
    field: Option<&str>,
    message: &str,
) -> ImportDiagnostic {
    diagnostic(
        code,
        ImportDiagnosticSeverity::Warning,
        resource_type,
        resource_id,
        field,
        message,
    )
}

pub(super) fn lossy(
    code: &'static str,
    resource_type: &str,
    resource_id: Option<&str>,
    field: Option<&str>,
    message: &str,
) -> ImportDiagnostic {
    diagnostic(
        code,
        ImportDiagnosticSeverity::Lossy,
        resource_type,
        resource_id,
        field,
        message,
    )
}

fn diagnostic(
    code: &'static str,
    severity: ImportDiagnosticSeverity,
    resource_type: &str,
    resource_id: Option<&str>,
    field: Option<&str>,
    message: &str,
) -> ImportDiagnostic {
    ImportDiagnostic {
        code,
        severity,
        resource_type: resource_type.to_owned(),
        resource_id: resource_id.map(str::to_owned),
        field: field.map(str::to_owned),
        message: message.to_owned(),
    }
}

pub(super) fn sort(diagnostics: &mut [ImportDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.resource_type.cmp(&right.resource_type))
            .then_with(|| left.resource_id.cmp(&right.resource_id))
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.code.cmp(right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
}
