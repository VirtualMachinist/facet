//! Yaak export and directory-sync import adapter.
//!
//! The adapter validates Yaak's portable formats and projects one selected
//! workspace into Probe's serialization-independent domain model. It never
//! writes files; OpenCollection persistence remains a separate adapter.

#![forbid(unsafe_code)]

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, Collection, CollectionItem,
    CollectionMetadata, Environment, EnvironmentVariable, FileReference, Folder, FormField, Header,
    HttpRequest, ItemMetadata, MultipartPart, MultipartPartKind, MultipartValue, QueryParameter,
    RawBody, RawBodyKind, RequestBody, RequestSettings, Variable, VariableValue, VariableValueSet,
};
pub use probe_core::{ImportDiagnostic, ImportDiagnosticSeverity};
use serde::Deserialize;
use serde_json::{Map, Value};

/// Yaak portable format detected at the source path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YaakSourceFormat {
    /// A single Yaak JSON export document.
    ExportJson,
    /// A directory containing Yaak sync model files.
    SyncDirectory,
}

impl YaakSourceFormat {
    /// Stable machine-readable source-format name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExportJson => "yaak_export",
            Self::SyncDirectory => "yaak_sync",
        }
    }
}

/// Workspace available inside a Yaak source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YaakWorkspaceSummary {
    /// Yaak workspace ID used for deterministic selection.
    pub id: String,
    /// Human-readable workspace name.
    pub name: String,
}

/// Parsed Yaak source that can be previewed before conversion.
#[derive(Clone, Debug)]
pub struct YaakImportPreview {
    format: YaakSourceFormat,
    resources: Resources,
    diagnostics: Vec<ImportDiagnostic>,
}

impl YaakImportPreview {
    /// Returns the detected Yaak source format.
    #[must_use]
    pub const fn format(&self) -> YaakSourceFormat {
        self.format
    }

    /// Returns selectable workspaces in deterministic source order.
    #[must_use]
    pub fn workspaces(&self) -> Vec<YaakWorkspaceSummary> {
        self.resources
            .workspaces
            .iter()
            .map(|workspace| YaakWorkspaceSummary {
                id: workspace.id.clone(),
                name: workspace.name.clone(),
            })
            .collect()
    }

    /// Converts one workspace into Probe's domain model.
    ///
    /// When `allow_partial` is false, any lossy diagnostic rejects the conversion.
    pub fn convert(
        &self,
        workspace_id: Option<&str>,
        allow_partial: bool,
    ) -> Result<ImportedYaakWorkspace, YaakImportError> {
        convert_preview(self, workspace_id, allow_partial)
    }
}

/// Successfully converted Yaak workspace and its compatibility report.
#[derive(Clone, Debug)]
pub struct ImportedYaakWorkspace {
    /// Selected Yaak workspace.
    pub workspace: YaakWorkspaceSummary,
    /// Canonical domain collection ready for OpenCollection persistence.
    pub collection: Collection,
    /// Deterministically sorted compatibility diagnostics.
    pub diagnostics: Vec<ImportDiagnostic>,
    /// Whether lossy conversion was explicitly enabled and required.
    pub partial: bool,
}

/// Failure to inspect or convert a Yaak source.
#[derive(Debug)]
pub enum YaakImportError {
    /// Source or a contained model is invalid.
    Invalid(String),
    /// Reading the source failed.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A source contains multiple workspaces and no selection was supplied.
    WorkspaceSelectionRequired(Vec<YaakWorkspaceSummary>),
    /// The selected Yaak workspace does not exist.
    WorkspaceNotFound(String),
    /// Strict conversion found data that cannot be represented losslessly.
    Unsupported(Vec<ImportDiagnostic>),
}

impl YaakImportError {
    /// Returns structured compatibility diagnostics, when present.
    #[must_use]
    pub fn diagnostics(&self) -> Option<&[ImportDiagnostic]> {
        match self {
            Self::Unsupported(diagnostics) => Some(diagnostics),
            _ => None,
        }
    }
}

impl fmt::Display for YaakImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::Io { path, source } => {
                write!(formatter, "cannot read {}: {source}", path.display())
            }
            Self::WorkspaceSelectionRequired(workspaces) => write!(
                formatter,
                "Yaak source contains {} workspaces; select one by ID",
                workspaces.len()
            ),
            Self::WorkspaceNotFound(id) => write!(formatter, "Yaak workspace not found: {id}"),
            Self::Unsupported(diagnostics) => write!(
                formatter,
                "Yaak workspace requires partial import because {} item(s) cannot be represented losslessly",
                diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.severity == ImportDiagnosticSeverity::Lossy)
                    .count()
            ),
        }
    }
}

impl Error for YaakImportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Inspects an official Yaak export JSON file or directory-sync workspace.
pub fn inspect_yaak_source(path: impl AsRef<Path>) -> Result<YaakImportPreview, YaakImportError> {
    let path = path.as_ref();
    if path.is_dir() {
        inspect_sync_directory(path)
    } else {
        inspect_export_file(path)
    }
}

fn inspect_export_file(path: &Path) -> Result<YaakImportPreview, YaakImportError> {
    let source = fs::read_to_string(path).map_err(|source| YaakImportError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut document: ExportDocument = serde_json::from_str(&source)
        .map_err(|error| YaakImportError::Invalid(format!("invalid Yaak export JSON: {error}")))?;
    if !(1..=4).contains(&document.yaak_schema) {
        return Err(YaakImportError::Invalid(format!(
            "unsupported Yaak export schema {}; supported schemas are 1 through 4",
            document.yaak_schema
        )));
    }
    migrate_export(&mut document);
    let mut diagnostics = Vec::new();
    diagnose_extra_fields("export", None, &document.extra, &mut diagnostics);
    diagnose_extra_fields(
        "resources",
        None,
        &document.resources.extra,
        &mut diagnostics,
    );
    validate_resources(&document.resources)?;
    Ok(YaakImportPreview {
        format: YaakSourceFormat::ExportJson,
        resources: document.resources,
        diagnostics,
    })
}

fn inspect_sync_directory(path: &Path) -> Result<YaakImportPreview, YaakImportError> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| YaakImportError::Io {
            path: path.to_owned(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| YaakImportError::Io {
            path: path.to_owned(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut resources = Resources::default();
    for entry in entries {
        if !entry
            .file_type()
            .map_err(|source| YaakImportError::Io {
                path: entry.path(),
                source,
            })?
            .is_file()
        {
            continue;
        }
        let extension = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "yaml" | "yml" | "json") {
            continue;
        }
        let source = fs::read_to_string(entry.path()).map_err(|source| YaakImportError::Io {
            path: entry.path(),
            source,
        })?;
        let value: Value = if extension == "json" {
            serde_json::from_str(&source).map_err(|error| {
                YaakImportError::Invalid(format!(
                    "invalid Yaak sync model {}: {error}",
                    entry.path().display()
                ))
            })?
        } else {
            let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(&source).map_err(|error| {
                YaakImportError::Invalid(format!(
                    "invalid Yaak sync model {}: {error}",
                    entry.path().display()
                ))
            })?;
            serde_json::to_value(yaml).map_err(|error| {
                YaakImportError::Invalid(format!(
                    "invalid Yaak sync model {}: {error}",
                    entry.path().display()
                ))
            })?
        };
        let Some(model) = value.get("model").and_then(Value::as_str) else {
            continue;
        };
        if value.get("id").and_then(Value::as_str).is_none() {
            return Err(YaakImportError::Invalid(format!(
                "Yaak sync model {} is missing id",
                entry.path().display()
            )));
        }
        match model {
            "workspace" => resources.workspaces.push(from_value(value, &entry.path())?),
            "environment" => resources
                .environments
                .push(from_value(value, &entry.path())?),
            "folder" => resources.folders.push(from_value(value, &entry.path())?),
            "http_request" => resources
                .http_requests
                .push(from_value(value, &entry.path())?),
            "grpc_request" => resources
                .grpc_requests
                .push(from_value(value, &entry.path())?),
            "websocket_request" => resources
                .websocket_requests
                .push(from_value(value, &entry.path())?),
            _ => resources
                .unsupported_resources
                .push(from_value(value, &entry.path())?),
        }
    }
    migrate_resources(&mut resources);
    validate_resources(&resources)?;
    Ok(YaakImportPreview {
        format: YaakSourceFormat::SyncDirectory,
        resources,
        diagnostics: Vec::new(),
    })
}

fn from_value<T: for<'de> Deserialize<'de>>(
    value: Value,
    path: &Path,
) -> Result<T, YaakImportError> {
    serde_json::from_value(value).map_err(|error| {
        YaakImportError::Invalid(format!(
            "invalid Yaak sync model {}: {error}",
            path.display()
        ))
    })
}

fn migrate_export(document: &mut ExportDocument) {
    if !document.resources.requests.is_empty() {
        document
            .resources
            .http_requests
            .append(&mut document.resources.requests);
    }
    migrate_resources(&mut document.resources);
}

fn migrate_resources(resources: &mut Resources) {
    let mut generated = Vec::new();
    for workspace in &mut resources.workspaces {
        if !workspace.variables.is_empty() {
            let id = format!("GENERATE_ID::base_env_{}", workspace.id);
            generated.push(YaakEnvironment {
                model: "environment".to_owned(),
                id,
                workspace_id: workspace.id.clone(),
                name: "Global Variables".to_owned(),
                parent_model: "workspace".to_owned(),
                variables: std::mem::take(&mut workspace.variables),
                ..YaakEnvironment::default()
            });
        }
    }
    resources.environments.extend(generated);
    for environment in &mut resources.environments {
        if environment.parent_model.is_empty() {
            environment.parent_model = match environment.base {
                Some(true) => "workspace",
                _ => "environment",
            }
            .to_owned();
        }
    }
}

fn validate_resources(resources: &Resources) -> Result<(), YaakImportError> {
    if resources.workspaces.is_empty() {
        return Err(YaakImportError::Invalid(
            "Yaak source does not contain a workspace".to_owned(),
        ));
    }
    let mut ids = BTreeSet::new();
    for (model, id) in resources.identities() {
        if id.trim().is_empty() {
            return Err(YaakImportError::Invalid(format!(
                "Yaak {model} has an empty id"
            )));
        }
        if !ids.insert(id) {
            return Err(YaakImportError::Invalid(format!(
                "duplicate Yaak resource id: {id}"
            )));
        }
    }
    Ok(())
}

fn convert_preview(
    preview: &YaakImportPreview,
    workspace_id: Option<&str>,
    allow_partial: bool,
) -> Result<ImportedYaakWorkspace, YaakImportError> {
    let summary = select_workspace(preview, workspace_id)?;
    let workspace = preview
        .resources
        .workspaces
        .iter()
        .find(|workspace| workspace.id == summary.id)
        .expect("selected workspace must exist");
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
    )?;

    for resource in &preview.resources.grpc_requests {
        if resource.workspace_id == workspace.id {
            diagnostics.push(lossy(
                "unsupported_resource",
                "grpc_request",
                Some(&resource.id),
                None,
                "gRPC requests are not supported by the current Probe domain",
            ));
        }
    }
    for resource in &preview.resources.websocket_requests {
        if resource.workspace_id == workspace.id {
            diagnostics.push(lossy(
                "unsupported_resource",
                "websocket_request",
                Some(&resource.id),
                None,
                "WebSocket requests are not supported by the current Probe domain",
            ));
        }
    }
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
    sort_diagnostics(&mut diagnostics);
    let requires_partial = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == ImportDiagnosticSeverity::Lossy);
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

fn select_workspace(
    preview: &YaakImportPreview,
    workspace_id: Option<&str>,
) -> Result<YaakWorkspaceSummary, YaakImportError> {
    let summaries = preview.workspaces();
    if let Some(id) = workspace_id {
        return summaries
            .into_iter()
            .find(|workspace| workspace.id == id)
            .ok_or_else(|| YaakImportError::WorkspaceNotFound(id.to_owned()));
    }
    if summaries.len() == 1 {
        return Ok(summaries.into_iter().next().expect("one workspace exists"));
    }
    Err(YaakImportError::WorkspaceSelectionRequired(summaries))
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
) -> Result<Vec<CollectionItem>, YaakImportError> {
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
                Ok(CollectionItem::Folder(Folder {
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
                    )?,
                }))
            }
            SourceItem::Request(request) => Ok(CollectionItem::HttpRequest(convert_request(
                workspace,
                request,
                folders,
                diagnostics,
            )?)),
        })
        .collect()
}

fn convert_request(
    workspace: &YaakWorkspace,
    request: &YaakHttpRequest,
    folders: &BTreeMap<&str, &YaakFolder>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Result<HttpRequest, YaakImportError> {
    diagnose_extra_fields(
        "http_request",
        Some(&request.id),
        &request.extra,
        diagnostics,
    );
    if !request.description.trim().is_empty() {
        diagnostics.push(lossy(
            "unsupported_field",
            "http_request",
            Some(&request.id),
            Some("description"),
            "request descriptions cannot be represented by the current Probe domain",
        ));
    }
    let ancestors = folder_ancestors(request.folder_id.as_deref(), folders)?;
    let headers = effective_headers(workspace, &ancestors, request, diagnostics);
    let authentication = effective_authentication(workspace, &ancestors, request, diagnostics);
    let settings = effective_settings(workspace, &ancestors, request, diagnostics);
    let mut query_parameters = Vec::new();
    let mut path_parameters = Vec::new();
    for parameter in &request.url_parameters {
        let name = convert_templates(
            &parameter.name,
            "http_request",
            &request.id,
            "urlParameters.name",
            diagnostics,
        );
        let converted = QueryParameter {
            name: name.strip_prefix(':').unwrap_or(&name).to_owned(),
            value: convert_templates(
                &parameter.value,
                "http_request",
                &request.id,
                "urlParameters.value",
                diagnostics,
            ),
            disabled: !parameter.enabled.unwrap_or(true),
        };
        if name.starts_with(':') {
            path_parameters.push(converted);
        } else {
            query_parameters.push(converted);
        }
        diagnose_extra_fields(
            "http_request",
            Some(&request.id),
            &parameter.extra,
            diagnostics,
        );
    }
    Ok(HttpRequest {
        metadata: ItemMetadata {
            name: nonempty(&request.name),
            sequence: Some(request.sort_priority),
        },
        method: nonempty(&request.method),
        url: Some(convert_templates(
            &request.url,
            "http_request",
            &request.id,
            "url",
            diagnostics,
        )),
        headers,
        query_parameters,
        path_parameters,
        body: convert_body(request, diagnostics),
        authentication,
        settings,
    })
}

fn folder_ancestors<'a>(
    folder_id: Option<&str>,
    folders: &'a BTreeMap<&str, &'a YaakFolder>,
) -> Result<Vec<&'a YaakFolder>, YaakImportError> {
    let mut chain = Vec::new();
    let mut current = folder_id;
    while let Some(id) = current {
        let folder = folders.get(id).copied().ok_or_else(|| {
            YaakImportError::Invalid(format!("Yaak request references missing folder '{id}'"))
        })?;
        chain.push(folder);
        current = folder.folder_id.as_deref();
    }
    chain.reverse();
    Ok(chain)
}

fn effective_headers(
    workspace: &YaakWorkspace,
    ancestors: &[&YaakFolder],
    request: &YaakHttpRequest,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Vec<Header> {
    let mut headers = Vec::<Header>::new();
    let iter = workspace
        .headers
        .iter()
        .chain(ancestors.iter().flat_map(|folder| folder.headers.iter()))
        .chain(request.headers.iter());
    for header in iter {
        let converted = Header {
            name: convert_templates(
                &header.name,
                "http_request",
                &request.id,
                "headers.name",
                diagnostics,
            ),
            value: convert_templates(
                &header.value,
                "http_request",
                &request.id,
                "headers.value",
                diagnostics,
            ),
            disabled: !header.enabled.unwrap_or(true),
        };
        let key = converted.name.to_ascii_lowercase();
        if let Some(existing) = headers
            .iter_mut()
            .find(|existing| existing.name.to_ascii_lowercase() == key)
        {
            *existing = converted;
        } else {
            headers.push(converted);
        }
        diagnose_extra_fields("header", header.id.as_deref(), &header.extra, diagnostics);
    }
    headers
}

fn effective_authentication(
    workspace: &YaakWorkspace,
    ancestors: &[&YaakFolder],
    request: &YaakHttpRequest,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Option<Authentication> {
    let mut owner_type = workspace.authentication_type.as_deref();
    let mut owner = &workspace.authentication;
    for folder in ancestors {
        if folder.authentication_type.is_some() {
            owner_type = folder.authentication_type.as_deref();
            owner = &folder.authentication;
        }
    }
    if request.authentication_type.is_some() {
        owner_type = request.authentication_type.as_deref();
        owner = &request.authentication;
    }
    let auth_type = owner_type?;
    if auth_type == "none" {
        return None;
    }
    let kind = match auth_type {
        "awsv4" => AuthenticationKind::AwsV4,
        "basic" => AuthenticationKind::Basic,
        "bearer" => AuthenticationKind::Bearer,
        "digest" => AuthenticationKind::Digest,
        "ntlm" => AuthenticationKind::Ntlm,
        "apikey" => AuthenticationKind::ApiKey,
        "oauth1" => AuthenticationKind::OAuth1,
        "oauth2" => AuthenticationKind::OAuth2,
        other => {
            diagnostics.push(lossy(
                "unsupported_authentication",
                "http_request",
                Some(&request.id),
                Some("authenticationType"),
                &format!("Yaak authentication type '{other}' is not defined by OpenCollection"),
            ));
            return None;
        }
    };
    if !matches!(kind, AuthenticationKind::Basic | AuthenticationKind::Bearer) {
        diagnostics.push(warning(
            "execution_unsupported",
            "http_request",
            Some(&request.id),
            Some("authenticationType"),
            &format!(
                "authentication type '{}' is preserved but the current Probe HTTP engine cannot execute it",
                kind.as_str()
            ),
        ));
    }
    let properties = owner
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                json_auth_value(
                    value,
                    request,
                    &format!("authentication.{name}"),
                    diagnostics,
                ),
            )
        })
        .collect();
    Some(Authentication { kind, properties })
}

fn json_auth_value(
    value: &Value,
    request: &YaakHttpRequest,
    field: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> AuthenticationValue {
    match value {
        Value::Null => AuthenticationValue::Null,
        Value::Bool(value) => AuthenticationValue::Boolean(*value),
        Value::Number(value) => AuthenticationValue::Number(value.to_string()),
        Value::String(value) => AuthenticationValue::String(convert_templates(
            value,
            "http_request",
            &request.id,
            field,
            diagnostics,
        )),
        Value::Array(values) => AuthenticationValue::Sequence(
            values
                .iter()
                .map(|value| json_auth_value(value, request, field, diagnostics))
                .collect(),
        ),
        Value::Object(values) => AuthenticationValue::Object(
            values
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        json_auth_value(value, request, field, diagnostics),
                    )
                })
                .collect(),
        ),
    }
}

fn effective_settings(
    workspace: &YaakWorkspace,
    ancestors: &[&YaakFolder],
    request: &YaakHttpRequest,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> RequestSettings {
    let mut timeout = workspace.setting_request_timeout.unwrap_or(0);
    let mut follow_redirects = workspace.setting_follow_redirects.unwrap_or(true);
    let mut validate_certificates = workspace.setting_validate_certificates.unwrap_or(true);
    let mut send_cookies = workspace.setting_send_cookies.unwrap_or(true);
    let mut store_cookies = workspace.setting_store_cookies.unwrap_or(true);
    for folder in ancestors {
        override_setting(&folder.setting_request_timeout, &mut timeout);
        override_setting(&folder.setting_follow_redirects, &mut follow_redirects);
        override_setting(
            &folder.setting_validate_certificates,
            &mut validate_certificates,
        );
        override_setting(&folder.setting_send_cookies, &mut send_cookies);
        override_setting(&folder.setting_store_cookies, &mut store_cookies);
    }
    override_setting(&request.setting_request_timeout, &mut timeout);
    override_setting(&request.setting_follow_redirects, &mut follow_redirects);
    override_setting(
        &request.setting_validate_certificates,
        &mut validate_certificates,
    );
    override_setting(&request.setting_send_cookies, &mut send_cookies);
    override_setting(&request.setting_store_cookies, &mut store_cookies);
    if !validate_certificates {
        diagnostics.push(lossy(
            "unsupported_setting",
            "http_request",
            Some(&request.id),
            Some("settingValidateCertificates"),
            "disabling certificate validation cannot be represented by the current Probe domain",
        ));
    }
    if !send_cookies || !store_cookies {
        diagnostics.push(lossy(
            "unsupported_setting",
            "http_request",
            Some(&request.id),
            Some("cookieSettings"),
            "Yaak cookie-jar settings cannot be represented by the current Probe domain",
        ));
    }
    RequestSettings {
        timeout: (timeout > 0).then(|| Duration::from_millis(timeout as u64)),
        follow_redirects: Some(follow_redirects),
        max_redirects: None,
    }
}

fn override_setting<T: Copy + Default>(setting: &Option<InheritedSetting<T>>, value: &mut T) {
    if let Some(setting) = setting
        && setting.enabled.unwrap_or(false)
    {
        *value = setting.value;
    }
}

fn convert_body(
    request: &YaakHttpRequest,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> Option<RequestBody> {
    let body_type = request.body_type.as_deref()?;
    let body = match body_type {
        "application/json" | "graphql" => Body::Raw(RawBody {
            kind: RawBodyKind::Json,
            data: body_text(request, diagnostics),
        }),
        "application/xml" | "text/xml" => Body::Raw(RawBody {
            kind: RawBodyKind::Xml,
            data: body_text(request, diagnostics),
        }),
        "application/sparql-query" => Body::Raw(RawBody {
            kind: RawBodyKind::Sparql,
            data: body_text(request, diagnostics),
        }),
        "text/plain" | "other" => Body::Raw(RawBody {
            kind: RawBodyKind::Text,
            data: body_text(request, diagnostics),
        }),
        "application/x-www-form-urlencoded" => Body::FormUrlEncoded(
            body_forms(request)
                .into_iter()
                .map(|field| FormField {
                    name: convert_templates(
                        &field.name,
                        "http_request",
                        &request.id,
                        "body.form.name",
                        diagnostics,
                    ),
                    value: convert_templates(
                        field.value.as_deref().unwrap_or_default(),
                        "http_request",
                        &request.id,
                        "body.form.value",
                        diagnostics,
                    ),
                    disabled: !field.enabled.unwrap_or(true),
                })
                .collect(),
        ),
        "multipart/form-data" => Body::Multipart(
            body_forms(request)
                .into_iter()
                .map(|field| {
                    let file = field.file.as_deref();
                    MultipartPart {
                        name: convert_templates(
                            &field.name,
                            "http_request",
                            &request.id,
                            "body.form.name",
                            diagnostics,
                        ),
                        kind: if file.is_some() {
                            MultipartPartKind::File
                        } else {
                            MultipartPartKind::Text
                        },
                        value: MultipartValue::Single(convert_templates(
                            file.or(field.value.as_deref()).unwrap_or_default(),
                            "http_request",
                            &request.id,
                            "body.form.value",
                            diagnostics,
                        )),
                        content_type: field.content_type.clone(),
                        disabled: !field.enabled.unwrap_or(true),
                    }
                })
                .collect(),
        ),
        "binary" => {
            let file_path = request
                .body
                .get("filePath")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Body::File(vec![FileReference {
                file_path: convert_templates(
                    file_path,
                    "http_request",
                    &request.id,
                    "body.filePath",
                    diagnostics,
                ),
                content_type: String::new(),
                selected: true,
            }])
        }
        other => {
            diagnostics.push(lossy(
                "unsupported_body_type",
                "http_request",
                Some(&request.id),
                Some("bodyType"),
                &format!("Yaak body type '{other}' is not supported"),
            ));
            Body::Raw(RawBody {
                kind: RawBodyKind::Text,
                data: body_text(request, diagnostics),
            })
        }
    };
    Some(RequestBody::Single(body))
}

fn body_text(request: &YaakHttpRequest, diagnostics: &mut Vec<ImportDiagnostic>) -> String {
    convert_templates(
        request
            .body
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "http_request",
        &request.id,
        "body.text",
        diagnostics,
    )
}

fn body_forms(request: &YaakHttpRequest) -> Vec<YaakFormField> {
    request
        .body
        .get("form")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

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
    let base = selected
        .iter()
        .filter(|environment| environment.parent_model == "workspace")
        .copied()
        .collect::<Vec<_>>();
    if base.len() > 1 {
        return Err(YaakImportError::Invalid(format!(
            "Yaak workspace '{}' contains multiple global environments",
            workspace.id
        )));
    }
    let base_environment = base.first().copied();
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
        if !names.insert(environment.name.clone()) {
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

fn diagnose_workspace(workspace: &YaakWorkspace, diagnostics: &mut Vec<ImportDiagnostic>) {
    diagnose_extra_fields(
        "workspace",
        Some(&workspace.id),
        &workspace.extra,
        diagnostics,
    );
    if workspace.encryption_key_challenge.is_some() {
        diagnostics.push(lossy(
            "unsupported_field",
            "workspace",
            Some(&workspace.id),
            Some("encryptionKeyChallenge"),
            "Yaak workspace encryption metadata cannot be represented by OpenCollection",
        ));
    }
    if !workspace.setting_dns_overrides.is_empty() {
        diagnostics.push(lossy(
            "unsupported_setting",
            "workspace",
            Some(&workspace.id),
            Some("settingDnsOverrides"),
            "Yaak DNS overrides cannot be represented by the current Probe domain",
        ));
    }
}

fn diagnose_folder(folder: &YaakFolder, diagnostics: &mut Vec<ImportDiagnostic>) {
    diagnose_extra_fields("folder", Some(&folder.id), &folder.extra, diagnostics);
    if !folder.description.trim().is_empty() {
        diagnostics.push(lossy(
            "unsupported_field",
            "folder",
            Some(&folder.id),
            Some("description"),
            "folder descriptions cannot be represented by the current Probe domain",
        ));
    }
}

fn diagnose_extra_fields(
    resource_type: &str,
    resource_id: Option<&str>,
    extra: &BTreeMap<String, Value>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    diagnostics.extend(extra.keys().map(|field| {
        lossy(
            "unknown_field",
            resource_type,
            resource_id,
            Some(field),
            &format!("unknown Yaak field '{field}' cannot be guaranteed to survive import"),
        )
    }));
}

fn convert_templates(
    input: &str,
    resource_type: &str,
    resource_id: &str,
    field: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${[") {
        output.push_str(&rest[..start]);
        let expression = &rest[start + 3..];
        let Some(end) = expression.find("]}") else {
            output.push_str(&rest[start..]);
            diagnostics.push(lossy(
                "unsupported_template",
                resource_type,
                Some(resource_id),
                Some(field),
                "unterminated Yaak template expression was preserved literally",
            ));
            return output;
        };
        let raw = expression[..end].trim();
        let name = raw.strip_prefix("env.").unwrap_or(raw).trim();
        let simple = !name.is_empty()
            && !name.contains('(')
            && !name.contains(')')
            && !name.contains(' ')
            && (!raw.contains('.') || raw.starts_with("env."));
        if simple {
            output.push_str("{{");
            output.push_str(name);
            output.push_str("}}");
        } else {
            output.push_str(&rest[start..start + 3 + end + 2]);
            diagnostics.push(lossy(
                "unsupported_template",
                resource_type,
                Some(resource_id),
                Some(field),
                &format!("Yaak template '${{[{raw}]}}' was preserved literally"),
            ));
        }
        rest = &expression[end + 2..];
    }
    output.push_str(rest);
    output
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

fn sort_diagnostics(diagnostics: &mut Vec<ImportDiagnostic>) {
    diagnostics.sort_by(|left, right| {
        left.resource_type
            .cmp(&right.resource_type)
            .then_with(|| left.resource_id.cmp(&right.resource_id))
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.code.cmp(right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    diagnostics.dedup();
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ExportDocument {
    yaak_version: String,
    yaak_schema: i64,
    timestamp: Option<String>,
    resources: Resources,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Resources {
    workspaces: Vec<YaakWorkspace>,
    environments: Vec<YaakEnvironment>,
    folders: Vec<YaakFolder>,
    http_requests: Vec<YaakHttpRequest>,
    requests: Vec<YaakHttpRequest>,
    grpc_requests: Vec<UnsupportedRequest>,
    websocket_requests: Vec<UnsupportedRequest>,
    unsupported_resources: Vec<UnsupportedResource>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl Resources {
    fn identities(&self) -> Vec<(&'static str, &str)> {
        self.workspaces
            .iter()
            .map(|value| ("workspace", value.id.as_str()))
            .chain(
                self.environments
                    .iter()
                    .map(|value| ("environment", value.id.as_str())),
            )
            .chain(
                self.folders
                    .iter()
                    .map(|value| ("folder", value.id.as_str())),
            )
            .chain(
                self.http_requests
                    .iter()
                    .map(|value| ("http_request", value.id.as_str())),
            )
            .chain(
                self.grpc_requests
                    .iter()
                    .map(|value| ("grpc_request", value.id.as_str())),
            )
            .chain(
                self.websocket_requests
                    .iter()
                    .map(|value| ("websocket_request", value.id.as_str())),
            )
            .chain(
                self.unsupported_resources
                    .iter()
                    .map(|value| ("unsupported_resource", value.id.as_str())),
            )
            .collect()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakWorkspace {
    #[serde(rename = "type")]
    resource_type: String,
    model: String,
    id: String,
    created_at: Option<String>,
    updated_at: Option<String>,
    name: String,
    description: String,
    authentication_type: Option<String>,
    authentication: Map<String, Value>,
    headers: Vec<YaakHeader>,
    variables: Vec<YaakVariable>,
    encryption_key_challenge: Option<String>,
    setting_validate_certificates: Option<bool>,
    setting_follow_redirects: Option<bool>,
    setting_request_timeout: Option<i64>,
    setting_request_message_size: Option<i64>,
    setting_dns_overrides: Vec<Value>,
    setting_send_cookies: Option<bool>,
    setting_store_cookies: Option<bool>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakFolder {
    #[serde(rename = "type")]
    resource_type: String,
    model: String,
    id: String,
    workspace_id: String,
    folder_id: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    name: String,
    description: String,
    authentication_type: Option<String>,
    authentication: Map<String, Value>,
    headers: Vec<YaakHeader>,
    sort_priority: f64,
    setting_validate_certificates: Option<InheritedSetting<bool>>,
    setting_follow_redirects: Option<InheritedSetting<bool>>,
    setting_request_timeout: Option<InheritedSetting<i64>>,
    setting_request_message_size: Option<InheritedSetting<i64>>,
    setting_send_cookies: Option<InheritedSetting<bool>>,
    setting_store_cookies: Option<InheritedSetting<bool>>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakHttpRequest {
    #[serde(rename = "type")]
    resource_type: String,
    model: String,
    id: String,
    workspace_id: String,
    folder_id: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    name: String,
    description: String,
    method: String,
    url: String,
    headers: Vec<YaakHeader>,
    url_parameters: Vec<YaakParameter>,
    body_type: Option<String>,
    body: Map<String, Value>,
    authentication_type: Option<String>,
    authentication: Map<String, Value>,
    sort_priority: f64,
    setting_validate_certificates: Option<InheritedSetting<bool>>,
    setting_follow_redirects: Option<InheritedSetting<bool>>,
    setting_request_timeout: Option<InheritedSetting<i64>>,
    setting_send_cookies: Option<InheritedSetting<bool>>,
    setting_store_cookies: Option<InheritedSetting<bool>>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakEnvironment {
    #[serde(rename = "type")]
    resource_type: String,
    model: String,
    id: String,
    workspace_id: String,
    created_at: Option<String>,
    updated_at: Option<String>,
    name: String,
    public: bool,
    base: Option<bool>,
    parent_model: String,
    parent_id: Option<String>,
    variables: Vec<YaakVariable>,
    color: Option<String>,
    sort_priority: f64,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakVariable {
    enabled: Option<bool>,
    name: String,
    value: String,
    id: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakHeader {
    enabled: Option<bool>,
    name: String,
    value: String,
    id: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakParameter {
    enabled: Option<bool>,
    name: String,
    value: String,
    id: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct YaakFormField {
    enabled: Option<bool>,
    name: String,
    value: Option<String>,
    file: Option<String>,
    content_type: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct UnsupportedRequest {
    #[serde(rename = "type")]
    resource_type: String,
    model: String,
    id: String,
    workspace_id: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct UnsupportedResource {
    model: String,
    id: String,
    workspace_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct InheritedSetting<T: Default> {
    enabled: Option<bool>,
    value: T,
}

impl<T: Default> Default for InheritedSetting<T> {
    fn default() -> Self {
        Self {
            enabled: None,
            value: T::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ImportDiagnosticSeverity, YaakImportError, YaakSourceFormat, inspect_yaak_source};
    use probe_core::{AuthenticationKind, Body, CollectionItem, RequestBody};
    use std::{fs, path::PathBuf, time::SystemTime};

    fn temporary_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("probe-yaak-{}-{nanos}-{name}", std::process::id()))
    }

    #[test]
    fn converts_export_http_hierarchy_and_environment() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/yaak/export-v4.json");

        let preview = inspect_yaak_source(&path).unwrap();
        assert_eq!(preview.format(), YaakSourceFormat::ExportJson);
        let imported = preview.convert(None, false).unwrap();
        assert_eq!(imported.collection.metadata.name.as_deref(), Some("Pets"));
        assert_eq!(imported.collection.environments[0].name, "Global Variables");
        let CollectionItem::Folder(folder) = &imported.collection.items[0] else {
            panic!("expected folder");
        };
        let CollectionItem::HttpRequest(request) = &folder.items[0] else {
            panic!("expected request");
        };
        assert_eq!(request.path_parameters[0].name, "id");
        assert_eq!(request.query_parameters[0].name, "page");
        assert_eq!(request.headers[0].value, "{{TOKEN}}");
        assert_eq!(
            request.authentication.as_ref().unwrap().kind,
            AuthenticationKind::Bearer
        );
        let Some(RequestBody::Single(Body::Raw(body))) = &request.body else {
            panic!("expected raw body");
        };
        assert!(body.data.contains("{{TOKEN}}"));
    }

    #[test]
    fn converts_directory_sync_fixture() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/yaak/sync");
        let preview = inspect_yaak_source(path).unwrap();
        assert_eq!(preview.format(), YaakSourceFormat::SyncDirectory);
        let imported = preview.convert(None, false).unwrap();
        assert_eq!(
            imported.collection.metadata.name.as_deref(),
            Some("Sync Pets")
        );
        assert_eq!(imported.collection.environments.len(), 1);
        assert_eq!(imported.collection.items.len(), 1);
    }

    #[test]
    fn accepts_every_supported_export_schema() {
        for schema in 1..=4 {
            let path = temporary_path(&format!("schema-{schema}.json"));
            fs::write(
                &path,
                format!(
                    r#"{{"yaakSchema":{schema},"resources":{{"workspaces":[{{"model":"workspace","id":"wk_{schema}","name":"Schema {schema}"}}]}}}}"#
                ),
            )
            .unwrap();
            let imported = inspect_yaak_source(&path)
                .unwrap()
                .convert(None, false)
                .unwrap();
            assert_eq!(
                imported.collection.metadata.name.as_deref(),
                Some(format!("Schema {schema}").as_str())
            );
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn strict_mode_rejects_lossy_resources_and_partial_reports_them() {
        let path = temporary_path("lossy.json");
        fs::write(
            &path,
            r#"{
  "yaakSchema":4,
  "resources":{
    "workspaces":[{"model":"workspace","id":"wk_1","name":"Mixed"}],
    "grpcRequests":[{"model":"grpc_request","id":"gr_1","workspaceId":"wk_1"}]
  }
}"#,
        )
        .unwrap();
        let preview = inspect_yaak_source(&path).unwrap();
        assert!(matches!(
            preview.convert(None, false),
            Err(YaakImportError::Unsupported(_))
        ));
        let imported = preview.convert(None, true).unwrap();
        assert!(imported.partial);
        assert!(imported.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "unsupported_resource"
                && diagnostic.severity == ImportDiagnosticSeverity::Lossy
        }));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sync_directory_requires_valid_relationships() {
        let root = temporary_path("sync");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("yaak.wk_1.yaml"),
            "model: workspace\nid: wk_1\nname: Sync\n",
        )
        .unwrap();
        fs::write(
            root.join("yaak.rq_1.yaml"),
            "model: http_request\nid: rq_1\nworkspaceId: wk_1\nfolderId: missing\nname: Broken\n",
        )
        .unwrap();
        let preview = inspect_yaak_source(&root).unwrap();
        assert!(matches!(
            preview.convert(None, false),
            Err(YaakImportError::Invalid(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sync_directory_allows_partial_import_with_unsupported_resources() {
        let root = temporary_path("sync-partial");
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("yaak.wk_1.yaml"),
            "model: workspace\nid: wk_1\nname: Sync\n",
        )
        .unwrap();
        fs::write(
            root.join("yaak.sse_1.yaml"),
            "model: sse_request\nid: sse_1\nworkspaceId: wk_1\nname: Events\n",
        )
        .unwrap();

        let preview = inspect_yaak_source(&root).unwrap();
        assert!(matches!(
            preview.convert(None, false),
            Err(YaakImportError::Unsupported(_))
        ));
        let imported = preview.convert(None, true).unwrap();
        assert!(imported.partial);
        assert!(imported.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "unsupported_resource"
                && diagnostic.resource_type == "sse_request"
                && diagnostic.resource_id.as_deref() == Some("sse_1")
        }));
        fs::remove_dir_all(root).unwrap();
    }
}
