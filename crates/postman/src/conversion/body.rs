use probe_core::{
    Body, FileReference, FormField, ImportDiagnostic, MultipartPart, MultipartPartKind,
    MultipartValue, RawBody, RawBodyKind, RequestBody,
};
use serde::Deserialize;
use serde_json::Value;

use super::convert_string;
use crate::{
    PostmanImportError,
    diagnostics::{
        extra_fields as diagnose_extra_fields, lossy, meaningful,
        nonempty_description as diagnose_nonempty_description, value_string, warning,
    },
    schema::{PostmanBody, PostmanFile},
};

pub(super) fn convert_body(
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Option<RequestBody>, PostmanImportError> {
    if value.is_null() {
        return Ok(None);
    }
    let body = PostmanBody::deserialize(value).map_err(|error| {
        PostmanImportError::Invalid(format!(
            "invalid Postman body for request '{resource_id}': {error}"
        ))
    })?;
    diagnose_extra_fields("body", Some(resource_id), &body.extra, diagnostics);
    if body.disabled {
        diagnostics.push(lossy(
            "unsupported_body_state",
            "request",
            Some(resource_id),
            Some("body.disabled"),
            "a disabled Postman body cannot be represented by the current Probe domain",
        ));
        return Ok(None);
    }
    let Some(mode) = body.mode.as_deref() else {
        return Ok(None);
    };
    diagnose_body_options(mode, &body.options, resource_id, diagnostics);
    diagnose_inactive_body_data(&body, mode, resource_id, diagnostics);
    let converted = match mode {
        "raw" => convert_raw_body(&body, resource_id, diagnostics),
        "urlencoded" => convert_urlencoded_body(&body, resource_id, diagnostics),
        "formdata" => convert_multipart_body(&body, resource_id, diagnostics),
        "file" => convert_file_body(body.file.as_ref(), resource_id, diagnostics),
        "graphql" => Body::Raw(RawBody {
            kind: RawBodyKind::Json,
            data: serde_json::to_string(body.graphql.as_ref().unwrap_or(&Value::Null))
                .expect("JSON values must serialize"),
        }),
        other => {
            diagnostics.push(lossy(
                "unsupported_body_type",
                "request",
                Some(resource_id),
                Some("body.mode"),
                &format!("Postman body mode '{other}' is not supported"),
            ));
            return Ok(None);
        }
    };
    Ok(Some(RequestBody::Single(converted)))
}

fn convert_raw_body(
    body: &PostmanBody,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Body {
    let kind = match body
        .options
        .pointer("/raw/language")
        .and_then(Value::as_str)
    {
        Some("json") => RawBodyKind::Json,
        Some("xml") => RawBodyKind::Xml,
        Some("sparql") => RawBodyKind::Sparql,
        _ => RawBodyKind::Text,
    };
    Body::Raw(RawBody {
        kind,
        data: convert_string(
            body.raw.as_deref().unwrap_or_default(),
            "request",
            resource_id,
            "body.raw",
            diagnostics,
        ),
    })
}

fn convert_urlencoded_body(
    body: &PostmanBody,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Body {
    Body::FormUrlEncoded(
        body.urlencoded
            .iter()
            .map(|field| {
                diagnose_nonempty_description(
                    "form_field",
                    resource_id,
                    &field.description,
                    diagnostics,
                );
                diagnose_extra_fields("form_field", Some(resource_id), &field.extra, diagnostics);
                FormField {
                    name: value_string(&field.key),
                    value: convert_string(
                        &value_string(&field.value),
                        "request",
                        resource_id,
                        "body.urlencoded.value",
                        diagnostics,
                    ),
                    disabled: field.disabled,
                }
            })
            .collect(),
    )
}

fn convert_multipart_body(
    body: &PostmanBody,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Body {
    Body::Multipart(
        body.formdata
            .iter()
            .map(|field| {
                diagnose_nonempty_description(
                    "multipart_part",
                    resource_id,
                    &field.description,
                    diagnostics,
                );
                diagnose_extra_fields(
                    "multipart_part",
                    Some(resource_id),
                    &field.extra,
                    diagnostics,
                );
                let file = field.field_type.as_deref() == Some("file");
                let value = if file {
                    multipart_file_value(&field.src, resource_id, diagnostics)
                } else {
                    MultipartValue::Single(convert_string(
                        &value_string(&field.value),
                        "request",
                        resource_id,
                        "body.formdata.value",
                        diagnostics,
                    ))
                };
                MultipartPart {
                    name: value_string(&field.key),
                    kind: if file {
                        MultipartPartKind::File
                    } else {
                        MultipartPartKind::Text
                    },
                    value,
                    content_type: field.content_type.clone(),
                    disabled: field.disabled,
                }
            })
            .collect(),
    )
}

fn convert_file_body(
    file: Option<&PostmanFile>,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Body {
    if let Some(file) = file {
        diagnose_extra_fields("body_file", Some(resource_id), &file.extra, diagnostics);
        if file
            .content
            .as_deref()
            .is_some_and(|content| !content.is_empty())
        {
            diagnostics.push(lossy(
                "unsupported_embedded_file",
                "request",
                Some(resource_id),
                Some("body.file.content"),
                "embedded Postman file content cannot be represented by the current Probe domain",
            ));
        }
    }
    let src = file.map_or(&Value::Null, |file| &file.src);
    let file_path = value_string(src);
    if !file_path.is_empty() {
        diagnostics.push(warning(
            "file_relink_required",
            "request",
            Some(resource_id),
            Some("body.file.src"),
            "Postman exports may store only a file name; relink the file before execution",
        ));
    }
    Body::File(vec![FileReference {
        file_path,
        content_type: String::new(),
        selected: !src.is_null(),
    }])
}

fn diagnose_body_options(
    mode: &str,
    options: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    if !meaningful(options) {
        return;
    }
    if mode != "raw" {
        diagnostics.push(lossy(
            "unsupported_body_options",
            "request",
            Some(resource_id),
            Some("body.options"),
            "Postman body options for this mode cannot be represented by the current Probe domain",
        ));
        return;
    }
    let Some(options) = options.as_object() else {
        diagnostics.push(lossy(
            "unsupported_body_options",
            "request",
            Some(resource_id),
            Some("body.options"),
            "Postman raw-body options are not in the supported object form",
        ));
        return;
    };
    for (field, value) in options {
        if field != "raw" && meaningful(value) {
            diagnostics.push(lossy(
                "unknown_field",
                "body_options",
                Some(resource_id),
                Some(field),
                &format!(
                    "unknown Postman body option '{field}' cannot be guaranteed to survive import"
                ),
            ));
        }
    }
    let Some(raw) = options.get("raw").filter(|value| meaningful(value)) else {
        return;
    };
    let Some(raw) = raw.as_object() else {
        diagnostics.push(lossy(
            "unsupported_body_options",
            "request",
            Some(resource_id),
            Some("body.options.raw"),
            "Postman raw-body options are not in the supported object form",
        ));
        return;
    };
    for (field, value) in raw {
        if field != "language" && meaningful(value) {
            diagnostics.push(lossy(
                "unknown_field",
                "body_options",
                Some(resource_id),
                Some(field),
                &format!("unknown Postman raw-body option '{field}' cannot be guaranteed to survive import"),
            ));
        }
    }
    if let Some(language) = raw.get("language").and_then(Value::as_str)
        && !matches!(language, "" | "json" | "xml" | "sparql" | "text")
    {
        diagnostics.push(lossy(
            "unsupported_raw_language",
            "request",
            Some(resource_id),
            Some("body.options.raw.language"),
            &format!("Postman raw-body language '{language}' is imported as plain text"),
        ));
    }
}

fn diagnose_inactive_body_data(
    body: &PostmanBody,
    mode: &str,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    let inactive = [
        (
            "raw",
            mode != "raw" && body.raw.as_deref().is_some_and(|value| !value.is_empty()),
        ),
        (
            "urlencoded",
            mode != "urlencoded" && !body.urlencoded.is_empty(),
        ),
        ("formdata", mode != "formdata" && !body.formdata.is_empty()),
        ("file", mode != "file" && body.file.is_some()),
        (
            "graphql",
            mode != "graphql" && body.graphql.as_ref().is_some_and(meaningful),
        ),
    ];
    for (field, present) in inactive {
        if present {
            diagnostics.push(lossy(
                "inactive_body_data",
                "request",
                Some(resource_id),
                Some(field),
                "inactive Postman body data cannot be represented alongside the selected body mode",
            ));
        }
    }
}

fn multipart_file_value(
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> MultipartValue {
    let values = match value {
        Value::Array(values) => values
            .iter()
            .map(value_string)
            .map(|value| {
                convert_string(
                    &value,
                    "request",
                    resource_id,
                    "body.formdata.src",
                    diagnostics,
                )
            })
            .collect::<Vec<_>>(),
        Value::Null => Vec::new(),
        value => vec![convert_string(
            &value_string(value),
            "request",
            resource_id,
            "body.formdata.src",
            diagnostics,
        )],
    };
    match <Vec<String> as TryInto<[String; 1]>>::try_into(values) {
        Ok([value]) => MultipartValue::Single(value),
        Err(values) => MultipartValue::Multiple(values),
    }
}
