use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, Collection, CollectionItem,
    CollectionMetadata, Environment, EnvironmentVariable, FileReference, Folder, FormField, Header,
    HttpRequest, ItemMetadata, MultipartPart, MultipartPartKind, MultipartValue, QueryParameter,
    RawBody, RawBodyKind, RequestBody, RequestSettings, Variable, VariableValue, VariableValueSet,
    lossy_import_diagnostic_count, sort_import_diagnostics,
};
use serde_json::Value;

use super::*;

pub(super) fn convert_preview(
    preview: &YaakImportPreview,
    workspace_id: Option<&str>,
    allow_partial: bool,
) -> Result<ImportedYaakWorkspace, YaakImportError> {
    let workspace = select_workspace(preview, workspace_id)?;
    let summary = YaakWorkspaceSummary {
        id: workspace.id.clone(),
        name: workspace.name.clone(),
    };
    let mut diagnostics = preview.diagnostics.clone();
    diagnose_workspace(workspace, &mut diagnostics);

    let folders: BTreeMap<&str, &YaakFolder> = preview
        .resources
        .folders
        .iter()
        .filter(|folder| folder.workspace_id == workspace.id)
        .map(|folder| (folder.id.as_str(), folder))
        .collect();
    validate_folder_graph(workspace, &folders, &preview.resources)?;

    let mut collection = Collection {
        metadata: CollectionMetadata {
            name: Some(workspace.name.clone()),
            summary: nonempty(&workspace.description),
            ..CollectionMetadata::default()
        },
        environments: convert_environments(workspace, &preview.resources, &mut diagnostics)?,
        ..Collection::default()
    };
    collection.items = convert_items(
        workspace,
        None,
        &folders,
        &preview.resources,
        &mut diagnostics,
    );

    diagnose_unsupported_requests(
        &preview.resources.grpc_requests,
        workspace,
        "grpc_request",
        "gRPC requests are not supported by the current Probe domain",
        &mut diagnostics,
    );
    diagnose_unsupported_requests(
        &preview.resources.websocket_requests,
        workspace,
        "websocket_request",
        "WebSocket requests are not supported by the current Probe domain",
        &mut diagnostics,
    );
    for resource in &preview.resources.unsupported_resources {
        if resource.workspace_id.as_deref() == Some(workspace.id.as_str()) {
            diagnostics.push(lossy(
                "unsupported_resource",
                &resource.model,
                Some(&resource.id),
                None,
                &format!(
                    "Yaak resource model '{}' is not supported by the current Probe domain",
                    resource.model
                ),
            ));
        }
    }
    sort_import_diagnostics(&mut diagnostics);
    let requires_partial = lossy_import_diagnostic_count(&diagnostics) > 0;
    if requires_partial && !allow_partial {
        return Err(YaakImportError::Unsupported(diagnostics));
    }
    Ok(ImportedYaakWorkspace {
        workspace: summary,
        collection,
        diagnostics,
        partial: requires_partial,
    })
}

fn diagnose_unsupported_requests(
    resources: &[UnsupportedRequest],
    workspace: &YaakWorkspace,
    resource_type: &str,
    message: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    diagnostics.extend(
        resources
            .iter()
            .filter(|resource| resource.workspace_id == workspace.id)
            .map(|resource| {
                lossy(
                    "unsupported_resource",
                    resource_type,
                    Some(&resource.id),
                    None,
                    message,
                )
            }),
    );
}

fn select_workspace<'a>(
    preview: &'a YaakImportPreview,
    workspace_id: Option<&str>,
) -> Result<&'a YaakWorkspace, YaakImportError> {
    if let Some(id) = workspace_id {
        return preview
            .resources
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .ok_or_else(|| YaakImportError::WorkspaceNotFound(id.to_owned()));
    }
    if let [workspace] = preview.resources.workspaces.as_slice() {
        return Ok(workspace);
    }
    Err(YaakImportError::WorkspaceSelectionRequired(
        preview.workspaces(),
    ))
}

fn validate_folder_graph(
    workspace: &YaakWorkspace,
    folders: &BTreeMap<&str, &YaakFolder>,
    resources: &Resources,
) -> Result<(), YaakImportError> {
    for folder in folders.values() {
        if let Some(parent) = folder.folder_id.as_deref()
            && !folders.contains_key(parent)
        {
            return Err(YaakImportError::Invalid(format!(
                "Yaak folder '{}' references missing parent '{parent}'",
                folder.id
            )));
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(folder.id.as_str());
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err(YaakImportError::Invalid(format!(
                    "Yaak folder hierarchy contains a cycle at '{id}'"
                )));
            }
            current = folders
                .get(id)
                .and_then(|folder| folder.folder_id.as_deref());
        }
    }
    for request in resources
        .http_requests
        .iter()
        .filter(|request| request.workspace_id == workspace.id)
    {
        if let Some(parent) = request.folder_id.as_deref()
            && !folders.contains_key(parent)
        {
            return Err(YaakImportError::Invalid(format!(
                "Yaak request '{}' references missing folder '{parent}'",
                request.id
            )));
        }
    }
    Ok(())
}

fn convert_items(
    workspace: &YaakWorkspace,
    parent_id: Option<&str>,
    folders: &BTreeMap<&str, &YaakFolder>,
    resources: &Resources,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Vec<CollectionItem> {
    enum SourceItem<'a> {
        Folder(&'a YaakFolder),
        Request(&'a YaakHttpRequest),
    }
    impl SourceItem<'_> {
        fn priority(&self) -> f64 {
            match self {
                Self::Folder(folder) => folder.sort_priority,
                Self::Request(request) => request.sort_priority,
            }
        }
        fn id(&self) -> &str {
            match self {
                Self::Folder(folder) => &folder.id,
                Self::Request(request) => &request.id,
            }
        }
    }

    let mut source_items = folders
        .values()
        .filter(|folder| folder.folder_id.as_deref() == parent_id)
        .copied()
        .map(SourceItem::Folder)
        .chain(
            resources
                .http_requests
                .iter()
                .filter(|request| {
                    request.workspace_id == workspace.id
                        && request.folder_id.as_deref() == parent_id
                })
                .map(SourceItem::Request),
        )
        .collect::<Vec<_>>();
    source_items.sort_by(|left, right| {
        left.priority()
            .partial_cmp(&right.priority())
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.id().cmp(right.id()))
    });

    source_items
        .into_iter()
        .map(|item| match item {
            SourceItem::Folder(folder) => {
                diagnose_folder(folder, diagnostics);
                CollectionItem::Folder(Folder {
                    metadata: ItemMetadata {
                        name: nonempty(&folder.name),
                        sequence: Some(folder.sort_priority),
                    },
                    items: convert_items(
                        workspace,
                        Some(&folder.id),
                        folders,
                        resources,
                        diagnostics,
                    ),
                })
            }
            SourceItem::Request(request) => CollectionItem::HttpRequest(convert_request(
                workspace,
                request,
                folders,
                diagnostics,
            )),
        })
        .collect()
}

mod diagnostics;
mod request;

pub(super) use diagnostics::diagnose_extra_fields;
use diagnostics::{
    convert_templates, diagnose_folder, diagnose_workspace, lossy, nonempty, warning,
};
use request::convert_request;

fn convert_environments(
    workspace: &YaakWorkspace,
    resources: &Resources,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<Vec<Environment>, YaakImportError> {
    let selected = resources
        .environments
        .iter()
        .filter(|environment| environment.workspace_id == workspace.id)
        .collect::<Vec<_>>();
    let by_id: BTreeMap<&str, &YaakEnvironment> = selected
        .iter()
        .map(|environment| (environment.id.as_str(), *environment))
        .collect();
    let mut base = selected
        .iter()
        .filter(|environment| environment.parent_model == "workspace")
        .copied();
    let base_environment = base.next();
    if base.next().is_some() {
        return Err(YaakImportError::Invalid(format!(
            "Yaak workspace '{}' contains multiple global environments",
            workspace.id
        )));
    }
    let mut names = BTreeSet::new();
    let mut converted = Vec::new();
    for environment in selected {
        diagnose_extra_fields(
            "environment",
            Some(&environment.id),
            &environment.extra,
            diagnostics,
        );
        if environment.parent_model == "folder" {
            diagnostics.push(lossy(
                "unsupported_environment_scope",
                "environment",
                Some(&environment.id),
                Some("parentModel"),
                "folder-scoped environments cannot be represented by OpenCollection",
            ));
            continue;
        }
        if !names.insert(environment.name.as_str()) {
            return Err(YaakImportError::Invalid(format!(
                "Yaak workspace contains duplicate environment name '{}'",
                environment.name
            )));
        }
        let extends = if environment.parent_model == "workspace" {
            None
        } else if let Some(parent_id) = environment.parent_id.as_deref() {
            Some(
                by_id
                    .get(parent_id)
                    .ok_or_else(|| {
                        YaakImportError::Invalid(format!(
                            "Yaak environment '{}' references missing parent '{parent_id}'",
                            environment.id
                        ))
                    })?
                    .name
                    .clone(),
            )
        } else {
            base_environment.map(|environment| environment.name.clone())
        };
        let variables = environment
            .variables
            .iter()
            .map(|variable| {
                diagnose_extra_fields(
                    "environment_variable",
                    variable.id.as_deref(),
                    &variable.extra,
                    diagnostics,
                );
                EnvironmentVariable::Plain(Variable {
                    name: nonempty(&variable.name),
                    value: Some(VariableValueSet::Single(VariableValue::String(
                        convert_templates(
                            &variable.value,
                            "environment",
                            &environment.id,
                            "variables.value",
                            diagnostics,
                        ),
                    ))),
                    disabled: !variable.enabled.unwrap_or(true),
                })
            })
            .collect();
        converted.push(Environment {
            name: environment.name.clone(),
            color: environment.color.clone(),
            extends,
            dot_env_file_path: None,
            variables,
        });
    }
    Ok(converted)
}
