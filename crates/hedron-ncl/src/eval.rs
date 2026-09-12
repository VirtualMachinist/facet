//! Thin wrapper over the Nickel VM: Nickel source in, frozen JSON out.
//!
//! This is the export boundary. `eval_full_for_export` skips fields marked
//! `| not_exported`, which is how plaintext secret fields stay out of every
//! frozen projection.

use nickel_lang_core::error::report::{ColorOpt, report_as_str};
use nickel_lang_core::eval::cache::CacheImpl;
use nickel_lang_core::program::{Program, ProgramBuilder};
use nickel_lang_core::serialize::{ExportFormat, to_string};

/// Errors from building, evaluating or exporting a Nickel program.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The program could not be assembled (I/O, no input).
    #[error("nickel build: {0}")]
    Build(String),
    /// Parse, typecheck, import or contract (blame) failure. Carries the rendered
    /// Nickel diagnostic so callers can surface blame text verbatim.
    #[error("nickel: {0}")]
    Nickel(String),
    /// The evaluated value is not JSON-exportable (functions, opaque values).
    #[error("nickel export: {0}")]
    Export(String),
    /// The exported JSON text failed to parse back (should not happen).
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Evaluate `source` (named `name` in diagnostics) for export and return the
/// frozen value as JSON.
pub fn eval_export(name: &str, source: &str) -> Result<serde_json::Value, Error> {
    let mut program: Program<CacheImpl> = ProgramBuilder::new()
        .add_source_string(source, name)
        .build()
        .map_err(|e| Error::Build(format!("{e:?}")))?;

    let value = program.eval_full_for_export().map_err(|e| {
        let mut files = program.files();
        Error::Nickel(report_as_str(&mut files, e, ColorOpt::Never))
    })?;

    let json = to_string(ExportFormat::Json, &value).map_err(|e| Error::Export(format!("{e:?}")))?;
    Ok(serde_json::from_str(&json)?)
}

/// Like [`eval_export`], with `platform` and `overlay` bound in scope
/// (see [`crate::overlay::prelude`]).
pub fn eval_export_with_prelude(name: &str, body: &str) -> Result<serde_json::Value, Error> {
    let source = format!("{}{body}", crate::overlay::prelude());
    eval_export(name, &source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_json() {
        let v = eval_export("t", r#"{ a = 1, b = "x", c | not_exported = "hidden" }"#).unwrap();
        assert_eq!(v, serde_json::json!({ "a": 1, "b": "x" }));
    }

    #[test]
    fn blame_is_an_error() {
        let err = eval_export("t", r#"("x" | Number)"#).unwrap_err();
        assert!(matches!(err, Error::Nickel(_)), "{err}");
    }
}
