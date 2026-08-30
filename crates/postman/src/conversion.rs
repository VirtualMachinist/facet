mod authentication;
mod body;
mod url;
mod variables;

use probe_core::{
    Collection, CollectionItem, CollectionMetadata, Folder, Header, HttpRequest, ImportDiagnostic,
    ItemMetadata, RequestSettings, lossy_import_diagnostic_count, sort_import_diagnostics,
};
use serde::Deserialize;
use serde_json::Value;

use self::{
    authentication::convert_authentication, body::convert_body, url::convert_url,
    variables::convert_collection_variables,
};
use crate::{
    COLLECTION_VARIABLES_ENVIRONMENT, ImportedPostmanCollection, PostmanImportError,
    PostmanImportPreview, PostmanSourceFormat,
    diagnostics::{
        convert_string, description_text, events as diagnose_events,
        extra_fields as diagnose_extra_fields, lossy,
        meaningful_value as diagnose_meaningful_value, nonempty,
        nonempty_description as diagnose_nonempty_description, value_string, version_text,
    },
    schema::{PostmanItem, PostmanParameter, PostmanRequest},
};

pub(super) fn convert_preview(
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

    let items = convert_items(
        &document.item,
        document.auth.as_ref(),
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

    sort_import_diagnostics(&mut diagnostics);
    let partial = lossy_import_diagnostic_count(&diagnostics) > 0;
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
                        metadata: item_metadata(item, index),
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
            metadata: item_metadata(item, index),
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
                metadata: item_metadata(item, index),
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

#[allow(clippy::cast_precision_loss)]
fn item_metadata(item: &PostmanItem, index: usize) -> ItemMetadata {
    ItemMetadata {
        name: nonempty(&item.name),
        sequence: Some(index as f64),
    }
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
    let headers = Vec::<PostmanParameter>::deserialize(value).map_err(|error| {
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
