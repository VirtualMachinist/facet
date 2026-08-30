use std::collections::BTreeSet;

use probe_core::{
    Environment, EnvironmentVariable, ImportDiagnostic, Variable, VariableValue, VariableValueSet,
    VariableValueType,
};
use serde_json::Value;

use super::convert_string;
use crate::{
    COLLECTION_VARIABLES_ENVIRONMENT, PostmanImportError,
    diagnostics::{
        extra_fields as diagnose_extra_fields, lossy, nonempty,
        nonempty_description as diagnose_nonempty_description, value_string,
    },
    schema::PostmanVariable,
};

pub(super) fn convert_collection_variables(
    variables: &[PostmanVariable],
    collection_id: Option<&str>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Vec<Environment>, PostmanImportError> {
    if variables.is_empty() {
        return Ok(Vec::new());
    }
    let resource_id = collection_id.unwrap_or("collection");
    let mut names = BTreeSet::new();
    let variables = variables
        .iter()
        .map(|variable| {
            let name = variable_name(variable).ok_or_else(|| {
                PostmanImportError::Invalid(
                    "Postman collection variable is missing key and id".to_owned(),
                )
            })?;
            if !names.insert(name.clone()) {
                return Err(PostmanImportError::Invalid(format!(
                    "Postman collection contains duplicate variable '{name}'"
                )));
            }
            diagnose_variable_metadata(variable, "collection_variable", resource_id, diagnostics);
            Ok(EnvironmentVariable::Plain(Variable {
                name: Some(name),
                value: Some(variable_value(variable, resource_id, diagnostics)),
                disabled: variable.disabled,
            }))
        })
        .collect::<Result<Vec<_>, PostmanImportError>>()?;
    Ok(vec![Environment {
        name: COLLECTION_VARIABLES_ENVIRONMENT.to_owned(),
        color: None,
        extends: None,
        dot_env_file_path: None,
        variables,
    }])
}

fn variable_value(
    variable: &PostmanVariable,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> VariableValueSet {
    let value = &variable.value;
    if value.is_null() {
        return VariableValueSet::Single(VariableValue::Typed {
            kind: VariableValueType::Null,
            data: "null".to_owned(),
        });
    }
    let converted = match (variable.variable_type.as_deref(), value) {
        (Some("string"), value) => VariableValue::String(convert_string(
            &value_string(value),
            "collection_variable",
            resource_id,
            "value",
            diagnostics,
        )),
        (Some("number"), value) => VariableValue::Typed {
            kind: VariableValueType::Number,
            data: convert_string(
                &value_string(value),
                "collection_variable",
                resource_id,
                "value",
                diagnostics,
            ),
        },
        (Some("boolean"), value) => VariableValue::Typed {
            kind: VariableValueType::Boolean,
            data: convert_string(
                &value_string(value),
                "collection_variable",
                resource_id,
                "value",
                diagnostics,
            ),
        },
        (Some("object"), value) => VariableValue::Typed {
            kind: VariableValueType::Object,
            data: serde_json::to_string(value).expect("JSON values must serialize"),
        },
        (Some(kind), value) if !matches!(kind, "any" | "default") => {
            diagnostics.push(lossy(
                "unsupported_variable_type",
                "collection_variable",
                Some(resource_id),
                Some("type"),
                &format!("Postman variable type '{kind}' is not supported"),
            ));
            inferred_variable_value(value, resource_id, diagnostics)
        }
        (_, value) => inferred_variable_value(value, resource_id, diagnostics),
    };
    VariableValueSet::Single(converted)
}

fn inferred_variable_value(
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> VariableValue {
    match value {
        Value::String(value) => VariableValue::String(convert_string(
            value,
            "collection_variable",
            resource_id,
            "value",
            diagnostics,
        )),
        Value::Number(value) => VariableValue::Typed {
            kind: VariableValueType::Number,
            data: value.to_string(),
        },
        Value::Bool(value) => VariableValue::Typed {
            kind: VariableValueType::Boolean,
            data: value.to_string(),
        },
        Value::Array(_) | Value::Object(_) => VariableValue::Typed {
            kind: VariableValueType::Object,
            data: serde_json::to_string(value).expect("JSON values must serialize"),
        },
        Value::Null => VariableValue::Typed {
            kind: VariableValueType::Null,
            data: "null".to_owned(),
        },
    }
}

pub(super) fn diagnose_variable_metadata(
    variable: &PostmanVariable,
    resource_type: &str,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    diagnose_nonempty_description(
        resource_type,
        resource_id,
        &variable.description,
        diagnostics,
    );
    if variable.system {
        diagnostics.push(lossy(
            "unsupported_field",
            resource_type,
            Some(resource_id),
            Some("system"),
            "Postman's system-variable marker cannot be represented by the current Probe domain",
        ));
    }
    diagnose_extra_fields(
        resource_type,
        Some(resource_id),
        &variable.extra,
        diagnostics,
    );
}

pub(super) fn variable_name(variable: &PostmanVariable) -> Option<String> {
    nonempty(&variable.key).or_else(|| nonempty(&variable.id))
}
