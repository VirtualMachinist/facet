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

/// Builds a portable-import warning diagnostic.
#[must_use]
pub fn warning_import_diagnostic(
    code: &'static str,
    resource_type: &str,
    resource_id: Option<&str>,
    field: Option<&str>,
    message: &str,
) -> ImportDiagnostic {
    import_diagnostic(
        code,
        ImportDiagnosticSeverity::Warning,
        resource_type,
        resource_id,
        field,
        message,
    )
}

/// Builds a portable-import diagnostic for data that cannot be retained losslessly.
#[must_use]
pub fn lossy_import_diagnostic(
    code: &'static str,
    resource_type: &str,
    resource_id: Option<&str>,
    field: Option<&str>,
    message: &str,
) -> ImportDiagnostic {
    import_diagnostic(
        code,
        ImportDiagnosticSeverity::Lossy,
        resource_type,
        resource_id,
        field,
        message,
    )
}

fn import_diagnostic(
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

/// Sorts and deduplicates portable-import diagnostics into their canonical order.
pub fn sort_import_diagnostics(diagnostics: &mut Vec<ImportDiagnostic>) {
    diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.resource_type.cmp(&right.resource_type))
            .then_with(|| left.resource_id.cmp(&right.resource_id))
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.code.cmp(right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    diagnostics.dedup();
}

/// Counts diagnostics that require explicit partial-import permission.
#[must_use]
pub fn lossy_import_diagnostic_count(diagnostics: &[ImportDiagnostic]) -> usize {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == ImportDiagnosticSeverity::Lossy)
        .count()
}

/// Returns an owned string unless its value is empty or whitespace-only.
#[must_use]
pub fn nonempty_string(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        ImportDiagnosticSeverity, lossy_import_diagnostic, sort_import_diagnostics,
        warning_import_diagnostic,
    };

    #[test]
    fn import_diagnostics_have_one_canonical_order_and_no_duplicates() {
        let warning =
            warning_import_diagnostic("warning", "z-resource", Some("id"), None, "warning");
        let lossy = lossy_import_diagnostic("lossy", "a-resource", Some("id"), None, "lossy");
        let mut diagnostics = vec![lossy, warning.clone(), warning];

        sort_import_diagnostics(&mut diagnostics);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].severity, ImportDiagnosticSeverity::Warning);
        assert_eq!(diagnostics[1].severity, ImportDiagnosticSeverity::Lossy);
    }
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
