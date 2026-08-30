use std::{error::Error, fmt, path::PathBuf};

use probe_core::{ImportDiagnostic, lossy_import_diagnostic_count};

/// Failure to inspect or convert a Postman collection.
#[derive(Debug)]
pub enum PostmanImportError {
    /// Source JSON or collection structure is invalid.
    Invalid(String),
    /// Reading the source failed.
    Io {
        /// Source path.
        path: PathBuf,
        /// Underlying I/O failure.
        source: std::io::Error,
    },
    /// Strict conversion found data that cannot be represented losslessly.
    Unsupported(Vec<ImportDiagnostic>),
}

impl PostmanImportError {
    /// Returns structured compatibility diagnostics, when present.
    #[must_use]
    pub fn diagnostics(&self) -> Option<&[ImportDiagnostic]> {
        match self {
            Self::Unsupported(diagnostics) => Some(diagnostics),
            _ => None,
        }
    }
}

impl fmt::Display for PostmanImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::Io { path, source } => {
                write!(formatter, "cannot read {}: {source}", path.display())
            }
            Self::Unsupported(diagnostics) => write!(
                formatter,
                "Postman collection requires partial import because {} item(s) cannot be represented losslessly",
                lossy_import_diagnostic_count(diagnostics)
            ),
        }
    }
}

impl Error for PostmanImportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
