use probe_core::{EnvironmentResolutionError, ImportDiagnostic};
use probe_http::HttpError;
use probe_opencollection::{CreateError, SaveError, StructureError};
use probe_postman::PostmanImportError;
use probe_yaak::YaakImportError;
use serde_json::{Value, json};

use crate::{
    CONFIGURATION_EXIT_CODE, EXECUTION_EXIT_CODE, IMPORT_EXIT_CODE, INVALID_ARGUMENTS_EXIT_CODE,
    INVALID_WORKSPACE_EXIT_CODE, PERSISTENCE_EXIT_CODE, REQUEST_NOT_FOUND_EXIT_CODE,
};

#[derive(Debug)]
pub(crate) struct CliError {
    pub(crate) category: &'static str,
    pub(crate) message: String,
    pub(crate) exit_code: u8,
    pub(crate) details: Option<Value>,
}

impl CliError {
    pub(crate) fn invalid_arguments(message: impl Into<String>) -> Self {
        Self {
            category: "invalid_arguments",
            message: message.into(),
            exit_code: INVALID_ARGUMENTS_EXIT_CODE,
            details: None,
        }
    }

    pub(crate) fn invalid_workspace(message: impl Into<String>) -> Self {
        Self {
            category: "invalid_workspace",
            message: message.into(),
            exit_code: INVALID_WORKSPACE_EXIT_CODE,
            details: None,
        }
    }

    pub(crate) fn request_not_found(selector: &str) -> Self {
        Self {
            category: "request_not_found",
            message: format!("request selector not found: {selector}"),
            exit_code: REQUEST_NOT_FOUND_EXIT_CODE,
            details: None,
        }
    }

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
        Self {
            category,
            message: error.to_string(),
            exit_code: CONFIGURATION_EXIT_CODE,
            details: None,
        }
    }

    pub(crate) fn http(error: HttpError) -> Self {
        let category = if error.is_configuration() {
            "request_configuration"
        } else {
            match &error {
                HttpError::Timeout => "request_timeout",
                HttpError::Cancelled => "request_cancelled",
                HttpError::ResponseOutput { .. } => "output_error",
                _ => "network_execution",
            }
        };
        Self {
            category,
            message: error.to_string(),
            exit_code: if error.is_configuration() {
                CONFIGURATION_EXIT_CODE
            } else {
                EXECUTION_EXIT_CODE
            },
            details: None,
        }
    }

    pub(crate) fn runtime(error: &std::io::Error) -> Self {
        Self {
            category: "runtime_error",
            message: format!("cannot start asynchronous HTTP runtime: {error}"),
            exit_code: EXECUTION_EXIT_CODE,
            details: None,
        }
    }

    pub(crate) fn stdin(error: &std::io::Error) -> Self {
        Self {
            category: "stdin_error",
            message: format!("cannot read OpenCollection YAML from stdin: {error}"),
            exit_code: INVALID_WORKSPACE_EXIT_CODE,
            details: None,
        }
    }

    pub(crate) fn persistence(error: SaveError) -> Self {
        let (category, exit_code) = match &error {
            SaveError::RequestNotFound(_) => ("request_not_found", REQUEST_NOT_FOUND_EXIT_CODE),
            SaveError::EmptyUpdate => ("invalid_arguments", INVALID_ARGUMENTS_EXIT_CODE),
            SaveError::ReadOnlySource => ("persistence_read_only", PERSISTENCE_EXIT_CODE),
            SaveError::ConcurrentModification(_) => ("workspace_modified", PERSISTENCE_EXIT_CODE),
            SaveError::Environment(error) => return Self::configuration(error.clone()),
            SaveError::InvalidDocument(_) | SaveError::Serialize(_) | SaveError::Io { .. } => {
                ("persistence_error", PERSISTENCE_EXIT_CODE)
            }
        };
        Self {
            category,
            message: error.to_string(),
            exit_code,
            details: None,
        }
    }

    pub(crate) fn create(error: CreateError) -> Self {
        match error {
            CreateError::AlreadyExists(_) | CreateError::IsDirectory(_) => {
                Self::invalid_arguments(error.to_string())
            }
            CreateError::Load(error) => Self::invalid_workspace(error.to_string()),
            CreateError::Serialize(_) | CreateError::Io { .. } => Self {
                category: "persistence_error",
                message: error.to_string(),
                exit_code: PERSISTENCE_EXIT_CODE,
                details: None,
            },
        }
    }

    pub(crate) fn structure(error: StructureError) -> Self {
        let category = error.category();
        let exit_code = match category {
            "request_not_found" | "folder_not_found" => REQUEST_NOT_FOUND_EXIT_CODE,
            "duplicate_destination"
            | "destination_not_found"
            | "invalid_destination"
            | "invalid_name"
            | "invalid_index" => INVALID_ARGUMENTS_EXIT_CODE,
            _ => PERSISTENCE_EXIT_CODE,
        };
        Self {
            category,
            message: error.to_string(),
            exit_code,
            details: None,
        }
    }

    pub(crate) fn yaak(error: YaakImportError) -> Self {
        match error {
            YaakImportError::WorkspaceSelectionRequired(workspaces) => Self {
                category: "workspace_selection_required",
                message: format!(
                    "Yaak source contains {} workspaces; select one with --workspace <id>",
                    workspaces.len()
                ),
                exit_code: INVALID_ARGUMENTS_EXIT_CODE,
                details: Some(json!({
                    "workspaces": workspaces.into_iter().map(|workspace| json!({
                        "id": workspace.id,
                        "name": workspace.name,
                    })).collect::<Vec<_>>()
                })),
            },
            YaakImportError::WorkspaceNotFound(id) => Self {
                category: "workspace_not_found",
                message: format!("Yaak workspace not found: {id}"),
                exit_code: INVALID_ARGUMENTS_EXIT_CODE,
                details: None,
            },
            YaakImportError::Unsupported(diagnostics) => {
                unsupported_import("Yaak workspace", diagnostics)
            }
            YaakImportError::Invalid(message) => invalid_import(message),
            YaakImportError::Io { path, source } => {
                invalid_import(format!("cannot read {}: {source}", path.display()))
            }
        }
    }

    pub(crate) fn postman(error: PostmanImportError) -> Self {
        match error {
            PostmanImportError::Unsupported(diagnostics) => {
                unsupported_import("Postman collection", diagnostics)
            }
            PostmanImportError::Invalid(message) => invalid_import(message),
            PostmanImportError::Io { path, source } => {
                invalid_import(format!("cannot read {}: {source}", path.display()))
            }
        }
    }
}

fn invalid_import(message: String) -> CliError {
    CliError {
        category: "invalid_import",
        message,
        exit_code: INVALID_WORKSPACE_EXIT_CODE,
        details: None,
    }
}

fn unsupported_import(source: &str, diagnostics: Vec<ImportDiagnostic>) -> CliError {
    let lossy_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity.as_str() == "lossy")
        .count();
    CliError {
        category: "unsupported_import",
        message: format!(
            "{source} contains {lossy_count} lossy item(s); inspect diagnostics or pass --allow-partial"
        ),
        exit_code: IMPORT_EXIT_CODE,
        details: Some(json!({
            "diagnostics": diagnostics.iter().map(import_diagnostic_json).collect::<Vec<_>>()
        })),
    }
}

pub(crate) fn import_diagnostic_json(diagnostic: &ImportDiagnostic) -> Value {
    json!({
        "code": diagnostic.code,
        "severity": diagnostic.severity.as_str(),
        "resourceType": diagnostic.resource_type,
        "resourceId": diagnostic.resource_id,
        "field": diagnostic.field,
        "message": diagnostic.message,
    })
}
