//! Collection structure and portable-import diagnostics.

use crate::{Environment, HttpRequest};

/// A parsed API collection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collection {
    /// Collection-level metadata.
    pub metadata: CollectionMetadata,
    /// Requests and folders at the collection root.
    pub items: Vec<CollectionItem>,
    /// Environments embedded in collection configuration.
    pub environments: Vec<Environment>,
}

/// Severity of a portable-import compatibility diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ImportDiagnosticSeverity {
    /// Data was preserved, but Probe cannot use all of it.
    Warning,
    /// Data would be omitted or changed without partial mode.
    Lossy,
}

impl ImportDiagnosticSeverity {
    /// Returns the stable machine-readable severity name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Lossy => "lossy",
        }
    }
}

/// A deterministic portable-import compatibility issue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportDiagnostic {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Diagnostic severity.
    pub severity: ImportDiagnosticSeverity,
    /// Source-format resource type.
    pub resource_type: String,
    /// Source-format resource identifier, when available.
    pub resource_id: Option<String>,
    /// Affected source field, when available.
    pub field: Option<String>,
    /// Human-readable explanation.
    pub message: String,
}

/// Collection-level metadata.
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

/// An item supported by the domain reader.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum CollectionItem {
    /// A folder containing more items.
    Folder(Folder),
    /// An HTTP request.
    HttpRequest(HttpRequest),
}

/// Metadata shared by folders and requests.
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
