use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, FileReference, FormField,
    Header, MultipartPart, MultipartPartKind, MultipartValue, QueryParameter, RawBodyKind,
    RequestBody,
};
use serde_yaml_ng::Value;

pub(super) fn string_key(name: &str) -> Value {
    Value::String(name.to_owned())
}

pub(super) fn set_optional(mapping: &mut serde_yaml_ng::Mapping, name: &str, value: Option<Value>) {
    let key = string_key(name);
    if let Some(value) = value {
        mapping.insert(key, value);
    } else {
        mapping.remove(&key);
    }
}

pub(super) fn set_optional_merged(
    mapping: &mut serde_yaml_ng::Mapping,
    name: &str,
    value: Option<Value>,
) {
    let key = string_key(name);
    if let Some(mut value) = value {
        if let Some(existing) = mapping.remove(&key) {
            merge_yaml(&mut value, existing);
        }
        mapping.insert(key, value);
    } else {
        mapping.remove(&key);
    }
}

pub(super) fn merge_yaml(replacement: &mut Value, existing: Value) {
    match (replacement, existing) {
        (Value::Mapping(replacement), Value::Mapping(existing)) => {
            for (key, old_value) in existing {
                if let Some(new_value) = replacement.get_mut(&key) {
                    merge_yaml(new_value, old_value);
                } else {
                    replacement.insert(key, old_value);
                }
            }
        }
        (Value::Sequence(replacement), Value::Sequence(existing)) => {
            for (new_value, old_value) in replacement.iter_mut().zip(existing) {
                merge_yaml(new_value, old_value);
            }
        }
        _ => {}
    }
}

pub(super) fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Mapping(
        entries
            .into_iter()
            .map(|(key, value)| (string_key(key), value))
            .collect(),
    )
}

pub(super) fn merge_sequence_preserving(
    parent: &mut serde_yaml_ng::Mapping,
    name: &str,
    replacements: Vec<Value>,
    preserved_keys: &[&str],
) {
    let key = string_key(name);
    let existing = parent
        .get(&key)
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let merged = replacements
        .into_iter()
        .enumerate()
        .map(|(index, replacement)| {
            let Some(Value::Mapping(mut old)) = existing.get(index).cloned() else {
                return replacement;
            };
            let Value::Mapping(new) = replacement else {
                return replacement;
            };
            for (key, value) in new {
                if preserved_keys
                    .iter()
                    .any(|preserved| key == string_key(preserved))
                    && old.contains_key(&key)
                {
                    continue;
                }
                old.insert(key, value);
            }
            Value::Mapping(old)
        })
        .collect();
    parent.insert(key, Value::Sequence(merged));
}

pub(super) fn header_value(header: &Header) -> Value {
    map([
        ("name", Value::String(header.name.clone())),
        ("value", Value::String(header.value.clone())),
        ("disabled", Value::Bool(header.disabled)),
    ])
}

pub(super) fn query_parameter_value(parameter: &QueryParameter) -> Value {
    parameter_value(parameter, "query")
}

pub(super) fn path_parameter_value(parameter: &QueryParameter) -> Value {
    parameter_value(parameter, "path")
}

pub(super) fn parameter_value(parameter: &QueryParameter, parameter_type: &str) -> Value {
    map([
        ("name", Value::String(parameter.name.clone())),
        ("value", Value::String(parameter.value.clone())),
        ("type", Value::String(parameter_type.to_owned())),
        ("disabled", Value::Bool(parameter.disabled)),
    ])
}

pub(super) fn merge_parameters(
    parent: &mut serde_yaml_ng::Mapping,
    query: Option<&[QueryParameter]>,
    path: Option<&[QueryParameter]>,
) {
    let key = string_key("params");
    let existing = parent
        .get(&key)
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let mut query = query.map(|values| values.iter().map(query_parameter_value));
    let mut path = path.map(|values| values.iter().map(path_parameter_value));
    let mut merged = Vec::new();

    for old in existing {
        let parameter_type = old
            .as_mapping()
            .and_then(|mapping| mapping.get(string_key("type")))
            .and_then(Value::as_str);
        let replacement = match parameter_type {
            Some("query") => query.as_mut().map(Iterator::next),
            Some("path") => path.as_mut().map(Iterator::next),
            _ => None,
        };
        match replacement {
            Some(Some(mut replacement)) => {
                merge_yaml(&mut replacement, old);
                merged.push(replacement);
            }
            Some(None) => {}
            None => merged.push(old),
        }
    }
    if let Some(values) = query {
        merged.extend(values);
    }
    if let Some(values) = path {
        merged.extend(values);
    }
    parent.insert(key, Value::Sequence(merged));
}

pub(super) fn request_body_value(body: &RequestBody) -> Value {
    match body {
        RequestBody::Single(body) => body_value(body),
        RequestBody::Variants(variants) => Value::Sequence(
            variants
                .iter()
                .map(|variant| {
                    map([
                        ("title", Value::String(variant.title.clone())),
                        ("selected", Value::Bool(variant.selected)),
                        ("body", body_value(&variant.body)),
                    ])
                })
                .collect(),
        ),
    }
}

pub(super) fn body_value(body: &Body) -> Value {
    match body {
        Body::Raw(body) => map([
            (
                "type",
                Value::String(
                    match body.kind {
                        RawBodyKind::Json => "json",
                        RawBodyKind::Text => "text",
                        RawBodyKind::Xml => "xml",
                        RawBodyKind::Sparql => "sparql",
                    }
                    .to_owned(),
                ),
            ),
            ("data", Value::String(body.data.clone())),
        ]),
        Body::FormUrlEncoded(fields) => map([
            ("type", Value::String("form-urlencoded".to_owned())),
            (
                "data",
                Value::Sequence(fields.iter().map(form_field_value).collect()),
            ),
        ]),
        Body::Multipart(parts) => map([
            ("type", Value::String("multipart-form".to_owned())),
            (
                "data",
                Value::Sequence(parts.iter().map(multipart_part_value).collect()),
            ),
        ]),
        Body::File(files) => map([
            ("type", Value::String("file".to_owned())),
            (
                "data",
                Value::Sequence(files.iter().map(file_reference_value).collect()),
            ),
        ]),
    }
}

pub(super) fn form_field_value(field: &FormField) -> Value {
    map([
        ("name", Value::String(field.name.clone())),
        ("value", Value::String(field.value.clone())),
        ("disabled", Value::Bool(field.disabled)),
    ])
}

pub(super) fn multipart_part_value(part: &MultipartPart) -> Value {
    let mut value = match map([
        ("name", Value::String(part.name.clone())),
        (
            "type",
            Value::String(
                match part.kind {
                    MultipartPartKind::Text => "text",
                    MultipartPartKind::File => "file",
                }
                .to_owned(),
            ),
        ),
        (
            "value",
            match &part.value {
                MultipartValue::Single(value) => Value::String(value.clone()),
                MultipartValue::Multiple(values) => {
                    Value::Sequence(values.iter().cloned().map(Value::String).collect())
                }
            },
        ),
        ("disabled", Value::Bool(part.disabled)),
    ]) {
        Value::Mapping(value) => value,
        _ => unreachable!(),
    };
    value.insert(
        string_key("contentType"),
        part.content_type
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    Value::Mapping(value)
}

pub(super) fn file_reference_value(file: &FileReference) -> Value {
    map([
        ("filePath", Value::String(file.file_path.clone())),
        ("contentType", Value::String(file.content_type.clone())),
        ("selected", Value::Bool(file.selected)),
    ])
}

pub(super) fn authentication_value(authentication: &Authentication) -> Value {
    if authentication.kind == AuthenticationKind::Inherit {
        return Value::String("inherit".to_owned());
    }
    let mut value = serde_yaml_ng::Mapping::new();
    value.insert(
        string_key("type"),
        Value::String(authentication.kind.as_str().to_owned()),
    );
    value.extend(
        authentication
            .properties
            .iter()
            .map(|(name, value)| (Value::String(name.clone()), auth_property_value(value))),
    );
    Value::Mapping(value)
}

pub(super) fn auth_property_value(value: &AuthenticationValue) -> Value {
    match value {
        AuthenticationValue::String(value) => Value::String(value.clone()),
        AuthenticationValue::Number(value) => {
            serde_yaml_ng::from_str(value).unwrap_or_else(|_| Value::String(value.clone()))
        }
        AuthenticationValue::Boolean(value) => Value::Bool(*value),
        AuthenticationValue::Null => Value::Null,
        AuthenticationValue::Sequence(values) => {
            Value::Sequence(values.iter().map(auth_property_value).collect())
        }
        AuthenticationValue::Object(values) => Value::Mapping(
            values
                .iter()
                .map(|(name, value)| (Value::String(name.clone()), auth_property_value(value)))
                .collect(),
        ),
    }
}
