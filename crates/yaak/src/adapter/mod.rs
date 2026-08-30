use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use probe_core::{Collection, ImportDiagnostic, ImportDiagnosticSeverity};
use serde::Deserialize;
use serde_json::{Map, Value};

mod convert;
mod source;

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
        convert::convert_preview(self, workspace_id, allow_partial)
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
    source::inspect(path.as_ref())
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
    fn identities(&self) -> impl Iterator<Item = (&'static str, &str)> {
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
