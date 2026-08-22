//! Shared domain and application layer for Probe.
//!
//! These models describe application concepts and deliberately contain no YAML or
//! serialization concerns.

#![forbid(unsafe_code)]

use std::{collections::BTreeMap, time::Duration};

mod environment;
mod path_parameters;
mod workspace;

pub use environment::{
    EnvironmentResolutionError, ResolvedEnvironment, create_environment, resolve_environment,
    resolve_request, revert_created_environment, set_environment_variable,
    unset_environment_variable, validate_environments,
};
pub use path_parameters::{
    add_path_parameter, apply_path_parameters, ensure_path_parameters_from_url,
    path_variable_ranges, remove_path_parameter_at, rename_path_parameter_at,
    synchronize_path_parameters,
};
pub use workspace::{
    FolderKey, RequestKey, Workspace, WorkspaceEditError, WorkspaceFolder, WorkspaceItemRef,
    WorkspaceParent,
};

/// A parsed API collection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collection {
    /// Collection-level metadata.
    pub metadata: CollectionMetadata,
    /// Requests and folders at the collection root.
    pub items: Vec<CollectionItem>,
    /// Environments embedded in the collection configuration.
    pub environments: Vec<Environment>,
}

/// Collection-level metadata from OpenCollection's `info` object.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollectionMetadata {
    /// Human-readable collection name.
    pub name: Option<String>,
    /// Short collection summary.
    pub summary: Option<String>,
    /// User-defined collection version.
    pub version: Option<String>,
    /// Collection authors.
    pub authors: Vec<Author>,
}

/// A collection author.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Author {
    /// Author name.
    pub name: Option<String>,
    /// Author email address.
    pub email: Option<String>,
    /// Author URL.
    pub url: Option<String>,
}

/// An item supported by the current domain reader.
#[allow(clippy::large_enum_variant)] // Keep workspace items simply owned and allocation-free.
#[derive(Clone, Debug, PartialEq)]
pub enum CollectionItem {
    /// A folder containing more collection items.
    Folder(Folder),
    /// An HTTP request.
    HttpRequest(HttpRequest),
}

/// Metadata shared by folders and HTTP requests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemMetadata {
    /// Human-readable item name.
    pub name: Option<String>,
    /// User-interface ordering value.
    pub sequence: Option<f64>,
}

/// A folder in a collection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Folder {
    /// Folder metadata.
    pub metadata: ItemMetadata,
    /// Supported child items.
    pub items: Vec<CollectionItem>,
}

/// An HTTP request definition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HttpRequest {
    /// Request metadata.
    pub metadata: ItemMetadata,
    /// HTTP method as written in the collection.
    pub method: Option<String>,
    /// Request URL, which may contain unresolved collection variables.
    pub url: Option<String>,
    /// HTTP request headers.
    pub headers: Vec<Header>,
    /// Query parameters appended to the request URL.
    pub query_parameters: Vec<QueryParameter>,
    /// Path parameters referenced in the URL as `:variableName`.
    pub path_parameters: Vec<QueryParameter>,
    /// Request body definition, including selectable variants when present.
    pub body: Option<RequestBody>,
    /// Request authentication configuration.
    pub authentication: Option<Authentication>,
    /// Execution settings shared by every interface.
    pub settings: RequestSettings,
}

/// A non-interactive update to editable request metadata.
///
/// `None` leaves a field unchanged. Persistence adapters apply this update to the
/// in-memory request before attempting to save its repository representation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequestUpdate {
    /// Replacement request name.
    pub name: Option<String>,
    /// Replacement HTTP method.
    pub method: Option<String>,
    /// Replacement request URL.
    pub url: Option<String>,
    /// Replacement headers.
    pub headers: Option<Vec<Header>>,
    /// Replacement query parameters.
    pub query_parameters: Option<Vec<QueryParameter>>,
    /// Replacement path parameters.
    pub path_parameters: Option<Vec<QueryParameter>>,
    /// Replacement body. The inner `None` removes the body.
    pub body: Option<Option<RequestBody>>,
    /// Replacement authentication. The inner `None` removes authentication.
    pub authentication: Option<Option<Authentication>>,
}

impl RequestUpdate {
    /// Returns whether the update leaves every field unchanged.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.method.is_none()
            && self.url.is_none()
            && self.headers.is_none()
            && self.query_parameters.is_none()
            && self.path_parameters.is_none()
            && self.body.is_none()
            && self.authentication.is_none()
    }

    /// Applies the update to a domain request.
    pub fn apply(&self, request: &mut HttpRequest) {
        if let Some(name) = &self.name {
            request.metadata.name = Some(name.clone());
        }
        if let Some(method) = &self.method {
            request.method = Some(method.clone());
        }
        if let Some(url) = &self.url {
            request.url = Some(url.clone());
        }
        if let Some(headers) = &self.headers {
            request.headers.clone_from(headers);
        }
        if let Some(query_parameters) = &self.query_parameters {
            request.query_parameters.clone_from(query_parameters);
        }
        if let Some(path_parameters) = &self.path_parameters {
            request.path_parameters.clone_from(path_parameters);
        }
        if let Some(body) = &self.body {
            request.body.clone_from(body);
        }
        if let Some(authentication) = &self.authentication {
            request.authentication.clone_from(authentication);
        }
    }
}

/// HTTP execution settings projected from OpenCollection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RequestSettings {
    /// Total request timeout. Zero means no timeout.
    pub timeout: Option<Duration>,
    /// Whether redirects should be followed.
    pub follow_redirects: Option<bool>,
    /// Maximum redirect hops when redirects are enabled.
    pub max_redirects: Option<usize>,
}

/// An HTTP request header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Header {
    /// Header name.
    pub name: String,
    /// Header value.
    pub value: String,
    /// Whether this header is disabled.
    pub disabled: bool,
}

/// An HTTP query or path parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryParameter {
    /// Parameter name.
    pub name: String,
    /// Parameter value.
    pub value: String,
    /// Whether this parameter is disabled.
    pub disabled: bool,
}

/// A request body represented either directly or as selectable variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestBody {
    /// One body definition.
    Single(Body),
    /// Multiple named body definitions.
    Variants(Vec<BodyVariant>),
}

/// A named request-body variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodyVariant {
    /// Variant title.
    pub title: String,
    /// Whether the variant is selected.
    pub selected: bool,
    /// Variant body.
    pub body: Body,
}

/// A supported OpenCollection HTTP body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Body {
    /// JSON, text, XML, or SPARQL data.
    Raw(RawBody),
    /// URL-encoded form fields.
    FormUrlEncoded(Vec<FormField>),
    /// Multipart form parts.
    Multipart(Vec<MultipartPart>),
    /// One or more file-body variants.
    File(Vec<FileReference>),
}

/// A raw request body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawBody {
    /// Raw body content type.
    pub kind: RawBodyKind,
    /// Body data.
    pub data: String,
}

/// Raw body types defined by OpenCollection v1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawBodyKind {
    /// JSON text.
    Json,
    /// Plain text.
    Text,
    /// XML text.
    Xml,
    /// SPARQL text.
    Sparql,
}

/// A URL-encoded form field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormField {
    /// Field name.
    pub name: String,
    /// Field value.
    pub value: String,
    /// Whether the field is disabled.
    pub disabled: bool,
}

/// A multipart form part.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultipartPart {
    /// Part name.
    pub name: String,
    /// Part kind.
    pub kind: MultipartPartKind,
    /// Text value or file paths.
    pub value: MultipartValue,
    /// Optional content type.
    pub content_type: Option<String>,
    /// Whether the part is disabled.
    pub disabled: bool,
}

/// Multipart part types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MultipartPartKind {
    /// A text part.
    Text,
    /// A file part.
    File,
}

/// A multipart part value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MultipartValue {
    /// A single text value or file path.
    Single(String),
    /// Multiple file paths.
    Multiple(Vec<String>),
}

/// A file-body choice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileReference {
    /// File path.
    pub file_path: String,
    /// File media type.
    pub content_type: String,
    /// Whether this file is selected.
    pub selected: bool,
}

/// Authentication configuration for a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Authentication {
    /// Authentication scheme.
    pub kind: AuthenticationKind,
    /// Scheme-specific configuration, kept independent of serialization formats.
    pub properties: BTreeMap<String, AuthenticationValue>,
}

/// Authentication schemes defined by OpenCollection v1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthenticationKind {
    /// Inherit authentication from the containing scope.
    Inherit,
    /// AWS Signature Version 4.
    AwsV4,
    /// HTTP Basic authentication.
    Basic,
    /// WS-Security UsernameToken authentication.
    Wsse,
    /// Bearer token authentication.
    Bearer,
    /// HTTP Digest authentication.
    Digest,
    /// NTLM authentication.
    Ntlm,
    /// API-key authentication.
    ApiKey,
    /// OAuth 1.0.
    OAuth1,
    /// OAuth 2.0.
    OAuth2,
    /// A future or extension authentication scheme.
    Other(String),
}

impl AuthenticationKind {
    /// OpenCollection scheme name used by YAML, CLI JSON, and HTTP diagnostics.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Inherit => "inherit",
            Self::AwsV4 => "awsv4",
            Self::Basic => "basic",
            Self::Wsse => "wsse",
            Self::Bearer => "bearer",
            Self::Digest => "digest",
            Self::Ntlm => "ntlm",
            Self::ApiKey => "apikey",
            Self::OAuth1 => "oauth1",
            Self::OAuth2 => "oauth2",
            Self::Other(kind) => kind,
        }
    }
}

/// A serialization-independent authentication property value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthenticationValue {
    /// A string value.
    String(String),
    /// A boolean value.
    Boolean(bool),
    /// A number retained in its string form.
    Number(String),
    /// A null value.
    Null,
    /// A sequence of values.
    Sequence(Vec<AuthenticationValue>),
    /// A string-keyed object.
    Object(BTreeMap<String, AuthenticationValue>),
}

/// A bundled OpenCollection environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Environment {
    /// Environment name.
    pub name: String,
    /// Optional display color.
    pub color: Option<String>,
    /// Parent environment name.
    pub extends: Option<String>,
    /// Optional dotenv file path.
    pub dot_env_file_path: Option<String>,
    /// Environment variables.
    pub variables: Vec<EnvironmentVariable>,
}

/// A normal or secret environment variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvironmentVariable {
    /// A value stored in the collection.
    Plain(Variable),
    /// A secret whose value is stored outside the collection.
    Secret(SecretVariable),
}

/// A non-secret environment variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Variable {
    /// Variable name.
    pub name: Option<String>,
    /// Variable value or selectable values.
    pub value: Option<VariableValueSet>,
    /// Whether the variable is disabled.
    pub disabled: bool,
}

/// A secret environment-variable declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretVariable {
    /// Variable name.
    pub name: Option<String>,
    /// Declared value type.
    pub value_type: Option<VariableValueType>,
    /// Whether the variable is disabled.
    pub disabled: bool,
}

/// A single variable value or a set of selectable variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariableValueSet {
    /// One value.
    Single(VariableValue),
    /// Named selectable values.
    Variants(Vec<VariableValueVariant>),
}

/// A named variable-value variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariableValueVariant {
    /// Variant title.
    pub title: String,
    /// Whether the variant is selected.
    pub selected: bool,
    /// Variant value.
    pub value: VariableValue,
}

/// An environment variable value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariableValue {
    /// The shorthand string form.
    String(String),
    /// An explicitly typed value whose data remains in string form.
    Typed {
        /// Declared value type.
        kind: VariableValueType,
        /// String representation of the value.
        data: String,
    },
}

/// Types available for explicitly typed and secret variables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariableValueType {
    /// String data.
    String,
    /// Numeric data.
    Number,
    /// Boolean data.
    Boolean,
    /// Null data.
    Null,
    /// Object data.
    Object,
}

impl VariableValueType {
    /// OpenCollection type name used by YAML and CLI JSON.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Null => "null",
            Self::Object => "object",
        }
    }
}
