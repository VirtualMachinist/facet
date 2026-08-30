//! Yaak export and directory-sync import adapter.
//!
//! The adapter validates Yaak's portable formats and projects one selected
//! workspace into Probe's serialization-independent domain model. It never
//! writes files; OpenCollection persistence remains a separate adapter.

#![forbid(unsafe_code)]

mod adapter;

pub use adapter::{
    ImportedYaakWorkspace, YaakImportError, YaakImportPreview, YaakSourceFormat,
    YaakWorkspaceSummary, inspect_yaak_source,
};
pub use probe_core::{ImportDiagnostic, ImportDiagnosticSeverity};

#[cfg(test)]
mod tests;
