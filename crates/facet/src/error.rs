//! Facet error contract. Categories and exit codes for delegated behavior
//! mirror `probe-cli`'s `error.rs` exactly; Facet adds `lattice_*`,
//! `blob_not_found`, `invalid_sql`, and `sql_read_only`.

use lattice::LatticeError;
use probe_core::EnvironmentResolutionError;
use probe_http::HttpError;
use serde_json::Value;

use crate::{
    CONFIGURATION_EXIT_CODE, EXECUTION_EXIT_CODE, INVALID_ARGUMENTS_EXIT_CODE,
    INVALID_WORKSPACE_EXIT_CODE, LATTICE_EXIT_CODE, REQUEST_NOT_FOUND_EXIT_CODE,
};

/// A structured command failure.
#[derive(Debug)]
pub struct FacetError {
    /// Stable programmatic category.
    pub category: &'static str,
    /// Human diagnostic; not part of the stable contract.
    pub message: String,
    /// Process exit code.
    pub exit_code: u8,
    /// Optional structured details.
    pub details: Option<Value>,
}

impl FacetError {
    fn new(category: &'static str, message: impl Into<String>, exit_code: u8) -> Self {
        Self {
            category,
            message: message.into(),
            exit_code,
            details: None,
        }
    }

    /// Attaches structured details.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub(crate) fn invalid_arguments(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message, INVALID_ARGUMENTS_EXIT_CODE)
    }

    pub(crate) fn invalid_workspace(message: impl Into<String>) -> Self {
        Self::new("invalid_workspace", message, INVALID_WORKSPACE_EXIT_CODE)
    }

    pub(crate) fn request_not_found(selector: &str) -> Self {
        Self::new(
            "request_not_found",
            format!("request selector not found: {selector}"),
            REQUEST_NOT_FOUND_EXIT_CODE,
        )
    }

    pub(crate) fn blob_not_found(hash: &str) -> Self {
        Self::new(
            "blob_not_found",
            format!("blob not found: {hash}"),
            REQUEST_NOT_FOUND_EXIT_CODE,
        )
    }

    pub(crate) fn lattice_not_found(path: &std::path::Path) -> Self {
        Self::new(
            "lattice_not_found",
            format!(
                "no Lattice store found at or above {}; run a request with facet first",
                path.display()
            ),
            LATTICE_EXIT_CODE,
        )
    }

    pub(crate) fn lattice(error: LatticeError) -> Self {
        match error {
            LatticeError::ReadOnlyQuery => Self::new(
                "sql_read_only",
                error.to_string(),
                INVALID_ARGUMENTS_EXIT_CODE,
            ),
            error if error.is_query_error() => Self::new(
                "invalid_sql",
                error.to_string(),
                INVALID_ARGUMENTS_EXIT_CODE,
            ),
            LatticeError::Config(error) => Self::new(
                "invalid_arguments",
                error.to_string(),
                INVALID_ARGUMENTS_EXIT_CODE,
            ),
            error => Self::new("lattice_error", error.to_string(), LATTICE_EXIT_CODE),
        }
    }

    pub(crate) fn output(path: &std::path::Path, error: &std::io::Error) -> Self {
        Self::new(
            "output_error",
            format!("cannot write {}: {error}", path.display()),
            EXECUTION_EXIT_CODE,
        )
    }

    /// Mirrors `probe-cli`'s environment-resolution mapping.
    pub(crate) fn configuration(error: EnvironmentResolutionError) -> Self {
        if matches!(
            error,
            EnvironmentResolutionError::InvalidVariableName
                | EnvironmentResolutionError::InvalidEnvironmentName
        ) {
            return Self::invalid_arguments(error.to_string());
        }
        let category = match error {
            EnvironmentResolutionError::EnvironmentNotFound(_) => "environment_not_found",
            EnvironmentResolutionError::DuplicateEnvironment(_) => "duplicate_environment",
            EnvironmentResolutionError::ParentEnvironmentNotFound { .. } => {
                "parent_environment_not_found"
            }
            EnvironmentResolutionError::EnvironmentInheritanceCycle(_) => {
                "environment_inheritance_cycle"
            }
            EnvironmentResolutionError::MissingVariable(_) => "missing_variable",
            EnvironmentResolutionError::VariableNotFound { .. } => "variable_not_found",
            EnvironmentResolutionError::SecretVariableUnavailable(_) => {
                "secret_variable_unavailable"
            }
            EnvironmentResolutionError::DuplicateVariable { .. } => "duplicate_variable",
            EnvironmentResolutionError::EnvironmentInUse(_) => "environment_in_use",
            _ => "environment_resolution",
        };
        Self::new(category, error.to_string(), CONFIGURATION_EXIT_CODE)
    }

    /// Mirrors `probe-cli`'s HTTP mapping.
    pub(crate) fn http(error: &HttpError) -> Self {
        let category = if error.is_configuration() {
            "request_configuration"
        } else {
            match error {
                HttpError::Timeout => "request_timeout",
                HttpError::Cancelled => "request_cancelled",
                HttpError::ResponseOutput { .. } => "output_error",
                _ => "network_execution",
            }
        };
        let exit_code = if error.is_configuration() {
            CONFIGURATION_EXIT_CODE
        } else {
            EXECUTION_EXIT_CODE
        };
        Self::new(category, error.to_string(), exit_code)
    }

    pub(crate) fn runtime(error: &std::io::Error) -> Self {
        Self::new(
            "runtime_error",
            format!("cannot start asynchronous HTTP runtime: {error}"),
            EXECUTION_EXIT_CODE,
        )
    }

    pub(crate) fn stdin(error: &std::io::Error) -> Self {
        Self::new(
            "stdin_error",
            format!("cannot read OpenCollection YAML from stdin: {error}"),
            INVALID_WORKSPACE_EXIT_CODE,
        )
    }
}
