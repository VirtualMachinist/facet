//! Postman Collection v2.0/v2.1 JSON import adapter.
//!
//! This crate validates one exported Postman collection and projects it into
//! Probe's serialization-independent domain model. Persistence remains owned by
//! the OpenCollection repository.

#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, Collection, CollectionItem,
    CollectionMetadata, Environment, EnvironmentVariable, FileReference, Folder, FormField, Header,
    HttpRequest, ImportDiagnostic, ImportDiagnosticSeverity, ItemMetadata, MultipartPart,
    MultipartPartKind, MultipartValue, QueryParameter, RawBody, RawBodyKind, RequestBody,
    RequestSettings, Variable, VariableValue, VariableValueSet, VariableValueType,
};
use serde::Deserialize;
use serde_json::Value;

mod errors;

pub use errors::PostmanImportError;

/// Environment used to retain Postman collection-scoped variables.
pub const COLLECTION_VARIABLES_ENVIRONMENT: &str = "Postman Collection Variables";

/// Supported Postman collection JSON format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostmanSourceFormat {
    /// Postman Collection Format v2.0.0.
    CollectionV2,
    /// Postman Collection Format v2.1.0.
    CollectionV2_1,
}

impl PostmanSourceFormat {
    /// Stable machine-readable source-format name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CollectionV2 => "postman_collection_v2_0",
            Self::CollectionV2_1 => "postman_collection_v2_1",
        }
    }
}

/// Identity and display metadata for the imported Postman collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostmanCollectionSummary {
    /// Postman's collection ID, when exported.
    pub id: Option<String>,
    /// Human-readable collection name.
    pub name: String,
}

/// Parsed Postman collection that can be inspected before conversion.
#[derive(Clone, Debug)]
pub struct PostmanImportPreview {
    format: PostmanSourceFormat,
    document: PostmanDocument,
    diagnostics: Vec<ImportDiagnostic>,
}

impl PostmanImportPreview {
    /// Returns the detected Postman collection format.
    #[must_use]
    pub const fn format(&self) -> PostmanSourceFormat {
        self.format
    }

    /// Returns the source collection summary.
    #[must_use]
    pub fn collection(&self) -> PostmanCollectionSummary {
        PostmanCollectionSummary {
            id: self.document.info.postman_id.clone(),
            name: self.document.info.name.clone(),
        }
    }

    /// Converts the source into Probe's domain collection.
    pub fn convert(
        &self,
        allow_partial: bool,
    ) -> Result<ImportedPostmanCollection, PostmanImportError> {
        convert_preview(self, allow_partial)
    }
}

/// Converted Postman collection and its compatibility report.
#[derive(Clone, Debug)]
pub struct ImportedPostmanCollection {
    /// Source collection identity.
    pub source: PostmanCollectionSummary,
    /// Canonical domain collection ready for OpenCollection persistence.
    pub collection: Collection,
    /// Deterministically sorted compatibility diagnostics.
    pub diagnostics: Vec<ImportDiagnostic>,
    /// Whether lossy conversion was explicitly enabled and required.
    pub partial: bool,
    /// Environment containing collection variables, when one was created.
    pub collection_variables_environment: Option<String>,
}

/// Inspects one official Postman Collection v2.0/v2.1 JSON export.
pub fn inspect_postman_source(
    path: impl AsRef<Path>,
) -> Result<PostmanImportPreview, PostmanImportError> {
    let path = path.as_ref();
    if path.is_dir() {
        return Err(PostmanImportError::Invalid(
            "Postman import requires a Collection v2 or v2.1 JSON file".to_owned(),
        ));
    }
    let source = fs::read_to_string(path).map_err(|source| PostmanImportError::Io {
        path: path.to_owned(),
        source,
    })?;
    let value: Value = serde_json::from_str(&source).map_err(|error| {
        PostmanImportError::Invalid(format!("invalid Postman collection JSON: {error}"))
    })?;
    let schema = value
        .pointer("/info/schema")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            PostmanImportError::Invalid("Postman collection info.schema is required".to_owned())
        })?;
    let format = detect_format(schema)?;
    let document: PostmanDocument = serde_json::from_value(value).map_err(|error| {
        PostmanImportError::Invalid(format!("invalid Postman collection: {error}"))
    })?;
    if document.info.name.trim().is_empty() {
        return Err(PostmanImportError::Invalid(
            "Postman collection name cannot be empty".to_owned(),
        ));
    }
    let mut diagnostics = Vec::new();
    diagnose_extra_fields(
        "collection",
        document.info.postman_id.as_deref(),
        &document.extra,
        &mut diagnostics,
    );
    diagnose_extra_fields(
        "collection_info",
        document.info.postman_id.as_deref(),
        &document.info.extra,
        &mut diagnostics,
    );
    Ok(PostmanImportPreview {
        format,
        document,
        diagnostics,
    })
}

fn detect_format(schema: &str) -> Result<PostmanSourceFormat, PostmanImportError> {
    let normalized = schema.to_ascii_lowercase();
    if normalized.contains("/v2.1.0/") {
        Ok(PostmanSourceFormat::CollectionV2_1)
    } else if normalized.contains("/v2.0.0/") {
        Ok(PostmanSourceFormat::CollectionV2)
    } else {
        Err(PostmanImportError::Invalid(format!(
            "unsupported Postman collection schema '{schema}'; supported schemas are v2.0.0 and v2.1.0"
        )))
    }
}

fn convert_preview(
    preview: &PostmanImportPreview,
    allow_partial: bool,
) -> Result<ImportedPostmanCollection, PostmanImportError> {
    let document = &preview.document;
    let summary = preview.collection();
    let mut diagnostics = preview.diagnostics.clone();
    diagnose_events(
        "collection",
        summary.id.as_deref(),
        &document.event,
        &mut diagnostics,
    );
    diagnose_meaningful_value(
        "unsupported_setting",
        "collection",
        summary.id.as_deref(),
        "protocolProfileBehavior",
        &document.protocol_profile_behavior,
        "Postman protocol profile behavior cannot be represented by the current Probe domain",
        &mut diagnostics,
    );

    let collection_auth = document.auth.as_ref();
    let items = convert_items(
        &document.item,
        collection_auth,
        "items",
        preview.format,
        &mut diagnostics,
    )?;
    let environments =
        convert_collection_variables(&document.variable, summary.id.as_deref(), &mut diagnostics)?;
    let collection_variables_environment =
        (!environments.is_empty()).then(|| COLLECTION_VARIABLES_ENVIRONMENT.to_owned());
    let collection = Collection {
        metadata: CollectionMetadata {
            name: Some(summary.name.clone()),
            summary: description_text(&document.info.description),
            version: version_text(&document.info.version),
            ..CollectionMetadata::default()
        },
        items,
        environments,
    };

    sort_diagnostics(&mut diagnostics);
    let partial = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == ImportDiagnosticSeverity::Lossy);
    if partial && !allow_partial {
        return Err(PostmanImportError::Unsupported(diagnostics));
    }
    Ok(ImportedPostmanCollection {
        source: summary,
        collection,
        diagnostics,
        partial,
        collection_variables_environment,
    })
}

fn convert_items(
    items: &[PostmanItem],
    inherited_auth: Option<&Value>,
    path: &str,
    format: PostmanSourceFormat,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Vec<CollectionItem>, PostmanImportError> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let item_path = format!("{path}/{index}");
            let resource_id = item.id.as_deref().unwrap_or(&item_path);
            diagnose_extra_fields("item", Some(resource_id), &item.extra, diagnostics);
            diagnose_nonempty_description(
                "item",
                resource_id,
                &item.description,
                diagnostics,
            );
            diagnose_events("item", Some(resource_id), &item.event, diagnostics);
            diagnose_meaningful_value(
                "unsupported_setting",
                "item",
                Some(resource_id),
                "protocolProfileBehavior",
                &item.protocol_profile_behavior,
                "Postman protocol profile behavior cannot be represented by the current Probe domain",
                diagnostics,
            );
            if !item.variable.is_empty() {
                diagnostics.push(lossy(
                    "unsupported_variable_scope",
                    "item",
                    Some(resource_id),
                    Some("variable"),
                    "folder/request-scoped Postman variables cannot be represented by OpenCollection environments",
                ));
            }

            match (&item.item, &item.request) {
                (Some(children), None) => {
                    let effective_auth = item.auth.as_ref().or(inherited_auth);
                    Ok(CollectionItem::Folder(Folder {
                        metadata: ItemMetadata {
                            name: nonempty(&item.name),
                            sequence: Some(index as f64),
                        },
                        items: convert_items(
                            children,
                            effective_auth,
                            &format!("{item_path}/item"),
                            format,
                            diagnostics,
                        )?,
                    }))
                }
                (None, Some(request)) => {
                    if !item.response.is_empty() {
                        diagnostics.push(lossy(
                            "unsupported_examples",
                            "request",
                            Some(resource_id),
                            Some("response"),
                            "saved Postman response examples cannot be represented by the current Probe domain",
                        ));
                    }
                    Ok(CollectionItem::HttpRequest(convert_request(
                        item,
                        request,
                        inherited_auth,
                        resource_id,
                        index,
                        format,
                        diagnostics,
                    )?))
                }
                (Some(_), Some(_)) => Err(PostmanImportError::Invalid(format!(
                    "Postman item '{resource_id}' cannot contain both request and child items"
                ))),
                (None, None) => Err(PostmanImportError::Invalid(format!(
                    "Postman item '{resource_id}' contains neither a request nor child items"
                ))),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn convert_request(
    item: &PostmanItem,
    request: &PostmanRequest,
    inherited_auth: Option<&Value>,
    resource_id: &str,
    index: usize,
    format: PostmanSourceFormat,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<HttpRequest, PostmanImportError> {
    match request {
        PostmanRequest::Url(url) => Ok(HttpRequest {
            metadata: ItemMetadata {
                name: nonempty(&item.name),
                sequence: Some(index as f64),
            },
            method: Some("GET".to_owned()),
            url: Some(convert_string(
                url,
                "request",
                resource_id,
                "url",
                diagnostics,
            )),
            authentication: convert_authentication(
                inherited_auth,
                format,
                resource_id,
                diagnostics,
            )?,
            settings: RequestSettings::default(),
            ..HttpRequest::default()
        }),
        PostmanRequest::Object(request) => {
            diagnose_extra_fields("request", Some(resource_id), &request.extra, diagnostics);
            diagnose_nonempty_description(
                "request",
                resource_id,
                &request.description,
                diagnostics,
            );
            diagnose_meaningful_value(
                "unsupported_setting",
                "request",
                Some(resource_id),
                "proxy",
                &request.proxy,
                "Postman proxy configuration cannot be represented by the current Probe domain",
                diagnostics,
            );
            diagnose_meaningful_value(
                "unsupported_setting",
                "request",
                Some(resource_id),
                "certificate",
                &request.certificate,
                "Postman certificate configuration cannot be represented by the current Probe domain",
                diagnostics,
            );
            let (url, query_parameters, path_parameters) =
                convert_url(&request.url, resource_id, diagnostics)?;
            let auth = request.auth.as_ref().or(inherited_auth);
            Ok(HttpRequest {
                metadata: ItemMetadata {
                    name: nonempty(&item.name),
                    sequence: Some(index as f64),
                },
                method: nonempty(&request.method),
                url: Some(url),
                headers: convert_headers(&request.header, resource_id, diagnostics)?,
                query_parameters,
                path_parameters,
                body: convert_body(&request.body, resource_id, diagnostics)?,
                authentication: convert_authentication(auth, format, resource_id, diagnostics)?,
                settings: RequestSettings::default(),
            })
        }
    }
}

fn convert_url(
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
    let url: PostmanUrl = serde_json::from_value(value.clone()).map_err(|error| {
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

fn convert_headers(
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Vec<Header>, PostmanImportError> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(lines) = value.as_str() {
        return lines
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (name, value) = line.split_once(':').ok_or_else(|| {
                    PostmanImportError::Invalid(format!(
                        "invalid Postman header line for request '{resource_id}': {line}"
                    ))
                })?;
                Ok(Header {
                    name: name.trim().to_owned(),
                    value: convert_string(
                        value.trim(),
                        "request",
                        resource_id,
                        "header.value",
                        diagnostics,
                    ),
                    disabled: false,
                })
            })
            .collect();
    }
    let headers: Vec<PostmanParameter> =
        serde_json::from_value(value.clone()).map_err(|error| {
            PostmanImportError::Invalid(format!(
                "invalid Postman headers for request '{resource_id}': {error}"
            ))
        })?;
    Ok(headers
        .into_iter()
        .map(|header| {
            diagnose_nonempty_description("header", resource_id, &header.description, diagnostics);
            diagnose_extra_fields("header", Some(resource_id), &header.extra, diagnostics);
            Header {
                name: value_string(&header.key),
                value: convert_string(
                    &value_string(&header.value),
                    "request",
                    resource_id,
                    "header.value",
                    diagnostics,
                ),
                disabled: header.disabled,
            }
        })
        .collect())
}

fn convert_body(
    value: &Value,
    resource_id: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Option<RequestBody>, PostmanImportError> {
    if value.is_null() {
        return Ok(None);
    }
    let body: PostmanBody = serde_json::from_value(value.clone()).map_err(|error| {
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
        "raw" => {
            let language = body
                .options
                .pointer("/raw/language")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let kind = match language {
                "json" => RawBodyKind::Json,
                "xml" => RawBodyKind::Xml,
                "sparql" => RawBodyKind::Sparql,
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
        "urlencoded" => Body::FormUrlEncoded(
            body.urlencoded
                .iter()
                .map(|field| {
                    diagnose_nonempty_description(
                        "form_field",
                        resource_id,
                        &field.description,
                        diagnostics,
                    );
                    diagnose_extra_fields(
                        "form_field",
                        Some(resource_id),
                        &field.extra,
                        diagnostics,
                    );
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
        ),
        "formdata" => Body::Multipart(
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
        ),
        "file" => {
            let file = body.file.unwrap_or_default();
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
            let file_path = value_string(&file.src);
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
                selected: !file.src.is_null(),
            }])
        }
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
    if values.len() == 1 {
        MultipartValue::Single(values.into_iter().next().unwrap_or_default())
    } else {
        MultipartValue::Multiple(values)
    }
}

fn convert_authentication(
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
    let kind = match auth_type {
        "apikey" => AuthenticationKind::ApiKey,
        "awsv4" => AuthenticationKind::AwsV4,
        "basic" => AuthenticationKind::Basic,
        "bearer" => AuthenticationKind::Bearer,
        "digest" => AuthenticationKind::Digest,
        "ntlm" => AuthenticationKind::Ntlm,
        "oauth1" => AuthenticationKind::OAuth1,
        "oauth2" => AuthenticationKind::OAuth2,
        other => AuthenticationKind::Other(other.to_owned()),
    };
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
    let properties_value = object.get(auth_type).unwrap_or(&Value::Null);
    let properties = match (format, properties_value) {
        (_, Value::Null) => BTreeMap::new(),
        (PostmanSourceFormat::CollectionV2, Value::Object(properties)) => properties
            .iter()
            .map(|(name, value)| {
                (name.clone(), auth_value(value, resource_id, diagnostics))
            })
            .collect(),
        (PostmanSourceFormat::CollectionV2_1, Value::Array(attributes)) => attributes
            .iter()
            .map(|attribute| {
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
                            &format!("unknown Postman authentication field '{field}' cannot be guaranteed to survive import"),
                        ));
                    }
                }
                let value = attribute.get("value").unwrap_or(&Value::Null);
                let value_type = attribute.get("type").and_then(Value::as_str);
                Ok((
                    key,
                    typed_auth_value(value, value_type, resource_id, diagnostics),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, PostmanImportError>>()?,
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
    Ok(Some(Authentication { kind, properties }))
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

fn convert_collection_variables(
    variables: &[PostmanVariable],
    collection_id: Option<&str>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Vec<Environment>, PostmanImportError> {
    if variables.is_empty() {
        return Ok(Vec::new());
    }
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
            diagnose_variable_metadata(
                variable,
                "collection_variable",
                collection_id.unwrap_or("collection"),
                diagnostics,
            );
            Ok(EnvironmentVariable::Plain(Variable {
                name: Some(name),
                value: variable_value(variable, collection_id, diagnostics),
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
    collection_id: Option<&str>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Option<VariableValueSet> {
    let resource_id = collection_id.unwrap_or("collection");
    let value = &variable.value;
    if value.is_null() {
        return Some(VariableValueSet::Single(VariableValue::Typed {
            kind: VariableValueType::Null,
            data: "null".to_owned(),
        }));
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
    Some(VariableValueSet::Single(converted))
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
        Value::Null => unreachable!(),
    }
}

fn diagnose_variable_metadata(
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

fn variable_name(variable: &PostmanVariable) -> Option<String> {
    nonempty(&variable.key).or_else(|| nonempty(&variable.id))
}

fn convert_string(
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

fn diagnose_events(
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

fn diagnose_nonempty_description(
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

#[allow(clippy::too_many_arguments)]
fn diagnose_meaningful_value(
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

fn diagnose_extra_fields(
    resource_type: &str,
    resource_id: Option<&str>,
    extra: &BTreeMap<String, Value>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    for (field, _) in extra.iter().filter(|(_, value)| meaningful(value)) {
        diagnostics.push(lossy(
            "unknown_field",
            resource_type,
            resource_id,
            Some(field),
            &format!("unknown Postman field '{field}' cannot be guaranteed to survive import"),
        ));
    }
}

fn meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Bool(value) => *value,
        Value::Number(_) => true,
    }
}

fn description_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => nonempty(value),
        Value::Object(value) => value
            .get("content")
            .and_then(Value::as_str)
            .and_then(nonempty),
        _ => None,
    }
}

fn version_text(value: &Value) -> Option<String> {
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

fn value_string(value: &Value) -> String {
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

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

fn warning(
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

fn lossy(
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

fn sort_diagnostics(diagnostics: &mut [ImportDiagnostic]) {
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

fn null_value() -> Value {
    Value::Null
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostmanDocument {
    info: PostmanInfo,
    #[serde(default)]
    item: Vec<PostmanItem>,
    #[serde(default)]
    event: Vec<Value>,
    #[serde(default)]
    variable: Vec<PostmanVariable>,
    #[serde(default)]
    auth: Option<Value>,
    #[serde(default = "null_value")]
    protocol_profile_behavior: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
struct PostmanInfo {
    name: String,
    #[serde(rename = "schema")]
    _schema: String,
    #[serde(default, rename = "_postman_id")]
    postman_id: Option<String>,
    #[serde(default = "null_value")]
    description: Value,
    #[serde(default = "null_value")]
    version: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostmanItem {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default = "null_value")]
    description: Value,
    #[serde(default)]
    variable: Vec<PostmanVariable>,
    #[serde(default)]
    event: Vec<Value>,
    #[serde(default)]
    auth: Option<Value>,
    #[serde(default)]
    item: Option<Vec<PostmanItem>>,
    #[serde(default)]
    request: Option<PostmanRequest>,
    #[serde(default)]
    response: Vec<Value>,
    #[serde(default = "null_value")]
    protocol_profile_behavior: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum PostmanRequest {
    Url(String),
    Object(Box<PostmanRequestObject>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostmanRequestObject {
    #[serde(default)]
    method: String,
    #[serde(default = "null_value")]
    url: Value,
    #[serde(default)]
    auth: Option<Value>,
    #[serde(default = "null_value")]
    proxy: Value,
    #[serde(default = "null_value")]
    certificate: Value,
    #[serde(default = "null_value")]
    description: Value,
    #[serde(default = "null_value")]
    header: Value,
    #[serde(default = "null_value")]
    body: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostmanUrl {
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    host: Value,
    #[serde(default)]
    path: Value,
    #[serde(default)]
    port: String,
    #[serde(default)]
    query: Vec<PostmanParameter>,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    variable: Vec<PostmanVariable>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
struct PostmanParameter {
    #[serde(default)]
    key: Value,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    disabled: bool,
    #[serde(default = "null_value")]
    description: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostmanBody {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    urlencoded: Vec<PostmanParameter>,
    #[serde(default)]
    formdata: Vec<PostmanFormParameter>,
    #[serde(default)]
    file: Option<PostmanFile>,
    #[serde(default)]
    graphql: Option<Value>,
    #[serde(default = "null_value")]
    options: Value,
    #[serde(default)]
    disabled: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostmanFormParameter {
    #[serde(default)]
    key: Value,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    src: Value,
    #[serde(default, rename = "type")]
    field_type: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    disabled: bool,
    #[serde(default = "null_value")]
    description: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PostmanFile {
    #[serde(default)]
    src: Value,
    #[serde(default)]
    content: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostmanVariable {
    #[serde(default)]
    id: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: Value,
    #[serde(default, rename = "type")]
    variable_type: Option<String>,
    #[serde(default = "null_value")]
    description: Value,
    #[serde(default)]
    system: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use probe_core::{
        AuthenticationKind, Body, CollectionItem, EnvironmentVariable, MultipartPartKind,
        RequestBody, VariableValue, VariableValueSet, VariableValueType, WorkspaceItemRef,
    };
    use probe_opencollection::create_bundled_workspace_from_collection;

    use super::{
        COLLECTION_VARIABLES_ENVIRONMENT, PostmanImportError, PostmanSourceFormat,
        inspect_postman_source,
    };

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/postman")
            .join(name)
    }

    fn temporary_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "probe-postman-{}-{unique}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn imports_v21_collection_into_domain_model() {
        let preview = inspect_postman_source(fixture("collection-v2.1.json")).unwrap();
        assert_eq!(preview.format(), PostmanSourceFormat::CollectionV2_1);
        let imported = preview.convert(false).unwrap();
        assert_eq!(imported.source.id.as_deref(), Some("pm_v21"));
        assert_eq!(
            imported.collection.metadata.name.as_deref(),
            Some("Postman Pets")
        );
        assert_eq!(
            imported.collection.metadata.version.as_deref(),
            Some("1.2.3-beta")
        );
        assert_eq!(imported.collection.items.len(), 1);
        let CollectionItem::Folder(folder) = &imported.collection.items[0] else {
            panic!("first item should be a folder");
        };
        let CollectionItem::HttpRequest(request) = &folder.items[0] else {
            panic!("folder should contain a request");
        };
        assert_eq!(request.url.as_deref(), Some("{{baseUrl}}/pets/:petId"));
        assert_eq!(request.query_parameters[0].name, "expand");
        assert_eq!(request.path_parameters[0].name, "petId");
        assert_eq!(
            request.authentication.as_ref().map(|auth| &auth.kind),
            Some(&AuthenticationKind::Bearer)
        );
        assert!(matches!(
            request.body,
            Some(RequestBody::Single(Body::Raw(_)))
        ));
        assert_eq!(
            imported.collection.environments[0].name,
            COLLECTION_VARIABLES_ENVIRONMENT
        );
        let probe_core::EnvironmentVariable::Plain(variable) =
            &imported.collection.environments[0].variables[0]
        else {
            panic!("collection variable should be plain");
        };
        assert!(matches!(
            variable.value,
            Some(VariableValueSet::Single(VariableValue::String(_)))
        ));
        let variables = &imported.collection.environments[0].variables;
        assert!(matches!(
            variables[2],
            EnvironmentVariable::Plain(probe_core::Variable {
                value: Some(VariableValueSet::Single(VariableValue::Typed {
                    kind: VariableValueType::Number,
                    ref data,
                })),
                ..
            }) if data == "25"
        ));
        assert!(matches!(
            variables[3],
            EnvironmentVariable::Plain(probe_core::Variable {
                value: Some(VariableValueSet::Single(VariableValue::Typed {
                    kind: VariableValueType::Boolean,
                    ref data,
                })),
                ..
            }) if data == "true"
        ));
        assert!(matches!(
            variables[4],
            EnvironmentVariable::Plain(probe_core::Variable {
                value: Some(VariableValueSet::Single(VariableValue::Typed {
                    kind: VariableValueType::Object,
                    ..
                })),
                ..
            })
        ));
        assert!(matches!(
            variables[5],
            EnvironmentVariable::Plain(probe_core::Variable {
                value: Some(VariableValueSet::Single(VariableValue::Typed {
                    kind: VariableValueType::Null,
                    ..
                })),
                ..
            })
        ));
        assert!(!imported.partial);
    }

    #[test]
    fn imports_v2_object_authentication() {
        let preview = inspect_postman_source(fixture("collection-v2.json")).unwrap();
        assert_eq!(preview.format(), PostmanSourceFormat::CollectionV2);
        let imported = preview.convert(false).unwrap();
        let CollectionItem::HttpRequest(request) = &imported.collection.items[0] else {
            panic!("item should be a request");
        };
        let authentication = request.authentication.as_ref().unwrap();
        assert_eq!(authentication.kind, AuthenticationKind::Basic);
        assert_eq!(
            authentication.properties.get("username"),
            Some(&probe_core::AuthenticationValue::String("demo".to_owned()))
        );
    }

    #[test]
    fn imports_all_supported_bodies_and_expands_authentication_inheritance() {
        let preview = inspect_postman_source(fixture("collection-bodies-v2.1.json")).unwrap();
        let imported = preview.convert(false).unwrap();
        assert!(!imported.partial);
        let CollectionItem::Folder(folder) = &imported.collection.items[0] else {
            panic!("first item should be a folder");
        };
        assert_eq!(folder.items.len(), 5);
        let requests = folder
            .items
            .iter()
            .map(|item| match item {
                CollectionItem::HttpRequest(request) => request,
                CollectionItem::Folder(_) => panic!("payload item should be a request"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            requests[0].url.as_deref(),
            Some("https://api.example.com/pets/:petId")
        );
        assert_eq!(requests[0].query_parameters[0].value, "right");
        assert!(requests[0].query_parameters[0].disabled);
        assert!(requests[0].path_parameters[0].disabled);
        assert_eq!(
            requests[0].authentication.as_ref().map(|auth| &auth.kind),
            Some(&AuthenticationKind::Bearer)
        );
        assert!(requests[1].authentication.is_none());
        assert!(matches!(
            requests[0].body,
            Some(RequestBody::Single(Body::Raw(_)))
        ));
        assert!(matches!(
            requests[1].body,
            Some(RequestBody::Single(Body::FormUrlEncoded(_)))
        ));
        let Some(RequestBody::Single(Body::Multipart(parts))) = &requests[2].body else {
            panic!("third request should be multipart");
        };
        assert_eq!(parts[1].kind, MultipartPartKind::File);
        assert!(matches!(
            requests[3].body,
            Some(RequestBody::Single(Body::File(_)))
        ));
        assert!(matches!(
            requests[4].body,
            Some(RequestBody::Single(Body::Raw(_)))
        ));
        let CollectionItem::HttpRequest(string_request) = &imported.collection.items[1] else {
            panic!("second root item should be a request");
        };
        assert_eq!(
            string_request
                .authentication
                .as_ref()
                .map(|auth| &auth.kind),
            Some(&AuthenticationKind::Basic)
        );
    }

    #[test]
    fn strict_mode_rejects_lossy_data_and_partial_is_explicit() {
        let preview = inspect_postman_source(fixture("collection-lossy.json")).unwrap();
        let diagnostics = match preview.convert(false) {
            Err(PostmanImportError::Unsupported(diagnostics)) => diagnostics,
            other => panic!("expected strict compatibility failure, got {other:?}"),
        };
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "unsupported_scripts")
        );
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "unsupported_examples")
        );
        assert!(diagnostics.iter().any(|item| item.code == "unknown_field"));
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "unsupported_variable_scope")
        );
        let imported = preview.convert(true).unwrap();
        assert!(imported.partial);
        assert!(
            imported
                .diagnostics
                .iter()
                .any(|item| item.code == "dynamic_variable_unsupported")
        );
        assert!(
            imported
                .diagnostics
                .iter()
                .any(|item| item.code == "execution_unsupported")
        );
    }

    #[test]
    fn rejects_malformed_json_and_unknown_schema() {
        let malformed = inspect_postman_source(fixture("malformed.json")).unwrap_err();
        assert!(matches!(malformed, PostmanImportError::Invalid(_)));
        let error = inspect_postman_source(fixture("collection-v3.json")).unwrap_err();
        assert!(matches!(error, PostmanImportError::Invalid(_)));
    }

    #[test]
    fn bundled_save_reload_preserves_imported_semantics_and_environment() {
        let imported = inspect_postman_source(fixture("collection-v2.1.json"))
            .unwrap()
            .convert(false)
            .unwrap();
        let destination = temporary_path("roundtrip.yml");
        let loaded =
            create_bundled_workspace_from_collection(&destination, &imported.collection).unwrap();
        let workspace = loaded.workspace();
        assert_eq!(workspace.metadata(), &imported.collection.metadata);
        assert_eq!(workspace.request_count(), 1);
        assert_eq!(workspace.folder_count(), 1);
        assert_eq!(workspace.environments(), imported.collection.environments);
        let WorkspaceItemRef::Folder(folder_key) = workspace.root_items()[0] else {
            panic!("round-tripped root item should be a folder");
        };
        let folder = workspace.folder(folder_key).unwrap();
        let WorkspaceItemRef::Request(request_key) = folder.children[0] else {
            panic!("round-tripped folder item should be a request");
        };
        let request = workspace.request(request_key).unwrap();
        let CollectionItem::Folder(source_folder) = &imported.collection.items[0] else {
            panic!("source item should be a folder");
        };
        let CollectionItem::HttpRequest(source_request) = &source_folder.items[0] else {
            panic!("source folder item should be a request");
        };
        assert_eq!(request, source_request);
        fs::remove_file(destination).unwrap();
    }
}
