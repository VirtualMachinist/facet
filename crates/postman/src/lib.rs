//! Postman Collection v2.0/v2.1 JSON import adapter.
//!
//! This crate validates one exported Postman collection and projects it into
//! Probe's serialization-independent domain model. Persistence remains owned by
//! the `OpenCollection` repository.

#![forbid(unsafe_code)]

use std::{fs, path::Path};

use probe_core::{Collection, ImportDiagnostic};
use serde_json::Value;

mod conversion;
mod diagnostics;
mod errors;
mod schema;

#[cfg(test)]
mod tests;

use conversion::convert_preview;
use diagnostics::extra_fields as diagnose_extra_fields;
use schema::PostmanDocument;

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
    ///
    /// # Errors
    ///
    /// Returns [`PostmanImportError::Unsupported`] when strict conversion would be lossy, or
    /// [`PostmanImportError::Invalid`] when a supported Postman structure is malformed.
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
    /// Canonical domain collection ready for `OpenCollection` persistence.
    pub collection: Collection,
    /// Deterministically sorted compatibility diagnostics.
    pub diagnostics: Vec<ImportDiagnostic>,
    /// Whether lossy conversion was explicitly enabled and required.
    pub partial: bool,
    /// Environment containing collection variables, when one was created.
    pub collection_variables_environment: Option<String>,
}

/// Inspects one official Postman Collection v2.0/v2.1 JSON export.
///
/// # Errors
///
/// Returns [`PostmanImportError::Io`] when the source cannot be read and
/// [`PostmanImportError::Invalid`] when it is not a valid supported collection export.
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
