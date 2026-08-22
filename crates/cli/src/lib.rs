//! Command-line presentation adapter for Probe.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    io::{self, Read},
    path::PathBuf,
};

use probe_core::{
    EnvironmentResolutionError, FolderKey, RequestUpdate, WorkspaceItemRef, resolve_environment,
    resolve_request,
};
use probe_http::{ExecutionOptions, HttpEngine, HttpError, HttpResponse};
use probe_opencollection::{
    CreateError, LoadedWorkspace, SaveError, StructureError, StructureOperation, StructureResult,
    create_bundled_workspace, create_bundled_workspace_from_collection, load_workspace,
    load_workspace_from_str,
};
use probe_yaak::{ImportDiagnostic, YaakImportError, inspect_yaak_source};
use serde_json::json;

mod presentation;

use presentation::{request_human, request_json, response_human, response_json};

/// Exit code used when command-line arguments are invalid.
pub const INVALID_ARGUMENTS_EXIT_CODE: u8 = 2;
/// Exit code used when a workspace cannot be loaded or parsed.
pub const INVALID_WORKSPACE_EXIT_CODE: u8 = 3;
/// Exit code used when a request selector does not exist.
pub const REQUEST_NOT_FOUND_EXIT_CODE: u8 = 4;
/// Exit code reserved for environment and configuration errors.
pub const CONFIGURATION_EXIT_CODE: u8 = 5;
/// Exit code used for request execution and output errors.
pub const EXECUTION_EXIT_CODE: u8 = 6;
/// Exit code used for persistence failures and external-modification conflicts.
pub const PERSISTENCE_EXIT_CODE: u8 = 7;
/// Exit code used when a source requires an explicitly lossy import.
pub const IMPORT_EXIT_CODE: u8 = 8;
/// Version of the documented machine-readable JSON contracts.
pub const JSON_SCHEMA_VERSION: u64 = 1;

/// Captured CLI output and process status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutput {
    /// Command output written to stdout.
    pub stdout: String,
    /// Diagnostics written to stderr.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: u8,
}

impl RunOutput {
    fn success(stdout: String) -> Self {
        Self {
            stdout,
            stderr: String::new(),
            exit_code: 0,
        }
    }

    fn failure(error: CliError, json_output: bool) -> Self {
        if json_output {
            let mut value = json!({
                "schemaVersion": JSON_SCHEMA_VERSION,
                "error": {
                    "category": error.category,
                    "exitCode": error.exit_code,
                    "message": error.message,
                }
            });
            if let Some(details) = error.details {
                value["error"]["details"] = details;
            }
            Self {
                stdout: pretty_json(&value),
                stderr: String::new(),
                exit_code: error.exit_code,
            }
        } else {
            Self {
                stdout: String::new(),
                stderr: format!("error[{}]: {}\n", error.category, error.message),
                exit_code: error.exit_code,
            }
        }
    }
}

/// Returns the stable top-level help text.
#[must_use]
pub const fn help() -> &'static str {
    concat!(
        "Probe - a fast, native, local-first API client\n",
        "\n",
        "Usage: probe [OPTIONS] <COMMAND>\n",
        "\n",
        "Commands:\n",
        "  collection create <path>            Create an empty bundled collection\n",
        "  collection import yaak <source> <destination>  Import a Yaak workspace\n",
        "  collection validate <path>          Validate an OpenCollection workspace\n",
        "  request list <path>                 List HTTP requests\n",
        "  request get <path> <selector>       Inspect an HTTP request\n",
        "  request run <path> <selector>       Execute an HTTP request\n",
        "  request set <path> <selector>       Set and persist HTTP request fields\n",
        "  request create|rename|delete|move|reorder   Edit request structure\n",
        "  folder list <path>                  List folders\n",
        "  folder create|rename|delete|move|reorder    Edit folder structure\n",
        "  environment create <path>           Create a new environment\n",
        "  environment list <path>             List environments\n",
        "  environment set <path>              Set and persist an environment variable\n",
        "  environment unset <path>            Remove an environment variable override\n",
        "\n",
        "Options:\n",
        "      --environment <name>  Environment used to resolve or mutate variables\n",
        "      --extends <name>       Parent environment for environment create\n",
        "      --output <file>        Write the response body to a file\n",
        "      --name <name>          Set a request, folder, collection, environment, or variable name\n",
        "      --method <method>      Set an HTTP method\n",
        "      --url <url>            Set a request URL\n",
        "      --value <value>        Set an environment variable value\n",
        "      --parent <selector>    Destination folder (omit for collection root)\n",
        "      --index <index>        Zero-based insertion position (omit to append)\n",
        "      --workspace <id>       Select a workspace from a multi-workspace import\n",
        "      --allow-partial        Explicitly allow lossy Yaak conversion\n",
        "      --json                Emit versioned deterministic JSON\n",
        "  -q, --quiet               Suppress successful command output\n",
        "  -h, --help                Print help\n",
    )
}

const COLLECTION_HELP: &str = concat!(
    "Usage: probe collection <COMMAND>\n",
    "\n",
    "Commands:\n",
    "  create <path> [--name <name>]  Create an empty bundled OpenCollection YAML file\n",
    "  import yaak <source> <destination> [--workspace <id>] [--allow-partial]\n",
    "  validate <path|->              Validate a bundled file, stdin (-), or unbundled directory\n",
    "\n",
    "create writes a new bundled collection and refuses to overwrite an existing path.\n",
    "When --name is omitted, the collection title is the file stem. A missing .yml\n",
    "extension is added. validate accepts a bundled YAML file, stdin (-), or an unbundled directory.\n",
    "Yaak import accepts an official export JSON file or Directory Sync folder and never\n",
    "overwrites an existing destination. Multi-workspace exports require --workspace.\n",
);

const REQUEST_HELP: &str = concat!(
    "Usage: probe request <COMMAND>\n",
    "\n",
    "Commands:\n",
    "  list <path|->                 List requests and repository selectors\n",
    "  get <path|-> <selector> [--environment <name>]  Inspect one request\n",
    "  run <path|-> <selector> [--environment <name>] [--output <file>]\n",
    "  set <path> <selector> [--name <name>] [--method <method>] [--url <url>]\n",
    "  create <path> --name <name> [--parent <folder>] [--index <index>] [--method <method>] [--url <url>]\n",
    "  rename <path> <selector> --name <name>\n",
    "  delete <path> <selector>\n",
    "  move <path> <selector> [--parent <folder>] [--index <index>]\n",
    "  reorder <path> <selector> --index <index>\n",
);

const FOLDER_HELP: &str = concat!(
    "Usage: probe folder <COMMAND>\n",
    "\n",
    "Commands:\n",
    "  list <path|->                 List folders and repository selectors\n",
    "  create <path> --name <name> [--parent <folder>] [--index <index>]\n",
    "  rename <path> <selector> --name <name>\n",
    "  delete <path> <selector>\n",
    "  move <path> <selector> [--parent <folder>] [--index <index>]\n",
    "  reorder <path> <selector> --index <index>\n",
);

const ENVIRONMENT_HELP: &str = concat!(
    "Usage: probe environment <COMMAND>\n",
    "\n",
    "Commands:\n",
    "  list <path|->                 List environments\n",
    "  create <path> --name <name> [--extends <parent>]\n",
    "  set <path> --environment <name> --name <var> --value <value>\n",
    "  unset <path> --environment <name> --name <var>\n",
    "\n",
    "Create adds a new environment with an optional parent. Set writes a plain variable on\n",
    "the named environment. A parent-only variable is overridden on the selected environment.\n",
    "Unset removes that environment's entry so a parent value can show through. Stdin\n",
    "workspaces cannot be persisted.\n",
);

/// Runs the CLI adapter for arguments that exclude the executable name.
#[must_use]
pub fn run<I, S>(args: I) -> RunOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    run_with_stdin(args, &mut io::empty())
}

/// Runs the CLI adapter with a reader used when the workspace path is `-`.
#[must_use]
pub fn run_with_stdin<I, S, R>(args: I, stdin: &mut R) -> RunOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    R: Read,
{
    let mut args: Vec<String> = args
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect();
    let json_count = args.iter().filter(|argument| *argument == "--json").count();
    let json_output = json_count == 1;
    args.retain(|argument| argument != "--json");

    let quiet_count = args
        .iter()
        .filter(|argument| matches!(argument.as_str(), "-q" | "--quiet"))
        .count();
    let quiet = quiet_count == 1;
    args.retain(|argument| !matches!(argument.as_str(), "-q" | "--quiet"));

    if json_count > 1 {
        return RunOutput::failure(
            CliError::invalid_arguments("--json may only be specified once"),
            true,
        );
    }
    if quiet_count > 1 {
        return RunOutput::failure(
            CliError::invalid_arguments("--quiet may only be specified once"),
            json_output,
        );
    }
    if json_output && quiet {
        return RunOutput::failure(
            CliError::invalid_arguments("--json and --quiet cannot be used together"),
            true,
        );
    }

    if args.is_empty() || args == ["-h"] || args == ["--help"] {
        return RunOutput::success(help().to_owned());
    }
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        let help = match args.first().map(String::as_str) {
            Some("collection") => COLLECTION_HELP,
            Some("request") => REQUEST_HELP,
            Some("folder") => FOLDER_HELP,
            Some("environment") => ENVIRONMENT_HELP,
            _ => help(),
        };
        return RunOutput::success(help.to_owned());
    }

    let options = match extract_options(&mut args) {
        Ok(options) => options,
        Err(error) => return RunOutput::failure(error, json_output),
    };
    match parse_command(&args, options).and_then(|command| execute(command, stdin)) {
        Ok(output) => RunOutput::success(output.render(json_output, quiet)),
        Err(error) => RunOutput::failure(error, json_output),
    }
}

#[derive(Debug)]
enum Command {
    CreateCollection {
        path: PathBuf,
        name: Option<String>,
    },
    ImportYaak {
        source: PathBuf,
        destination: PathBuf,
        workspace: Option<String>,
        allow_partial: bool,
    },
    Validate {
        input: WorkspaceInput,
    },
    ListRequests {
        input: WorkspaceInput,
    },
    ListFolders {
        input: WorkspaceInput,
    },
    ListEnvironments {
        input: WorkspaceInput,
    },
    Get {
        input: WorkspaceInput,
        selector: String,
        environment: Option<String>,
    },
    Run {
        input: WorkspaceInput,
        selector: String,
        environment: Option<String>,
        output: Option<PathBuf>,
    },
    Set {
        input: WorkspaceInput,
        selector: String,
        update: RequestUpdate,
    },
    Structure {
        input: WorkspaceInput,
        operation_name: &'static str,
        operation: StructureOperation,
    },
    EnvironmentSet {
        input: WorkspaceInput,
        environment: String,
        name: String,
        value: String,
    },
    EnvironmentUnset {
        input: WorkspaceInput,
        environment: String,
        name: String,
    },
    EnvironmentCreate {
        input: WorkspaceInput,
        name: String,
        extends: Option<String>,
    },
}

#[derive(Debug)]
enum WorkspaceInput {
    Path(PathBuf),
    Stdin,
}

impl WorkspaceInput {
    fn from_argument(argument: &str) -> Self {
        if argument == "-" {
            Self::Stdin
        } else {
            Self::Path(PathBuf::from(argument))
        }
    }

    fn base_directory(&self) -> Option<PathBuf> {
        match self {
            Self::Path(path) if path.is_dir() => Some(path.clone()),
            Self::Path(path) => path.parent().map(std::path::Path::to_owned),
            Self::Stdin => None,
        }
    }
}

#[derive(Debug)]
struct CommandOutput {
    human: String,
    json: serde_json::Value,
}

impl CommandOutput {
    fn render(self, json_output: bool, quiet: bool) -> String {
        if quiet {
            String::new()
        } else if json_output {
            pretty_json(&versioned_json(self.json))
        } else {
            self.human
        }
    }
}

#[derive(Debug)]
struct CliError {
    category: &'static str,
    message: String,
    exit_code: u8,
    details: Option<serde_json::Value>,
}

impl CliError {
    fn invalid_arguments(message: impl Into<String>) -> Self {
        Self {
            category: "invalid_arguments",
            message: message.into(),
            exit_code: INVALID_ARGUMENTS_EXIT_CODE,
            details: None,
        }
    }

    fn invalid_workspace(message: impl Into<String>) -> Self {
        Self {
            category: "invalid_workspace",
            message: message.into(),
            exit_code: INVALID_WORKSPACE_EXIT_CODE,
            details: None,
        }
    }

    fn request_not_found(selector: &str) -> Self {
        Self {
            category: "request_not_found",
            message: format!("request selector not found: {selector}"),
            exit_code: REQUEST_NOT_FOUND_EXIT_CODE,
            details: None,
        }
    }

    fn configuration(error: EnvironmentResolutionError) -> Self {
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
            _ => "environment_resolution",
        };
        Self {
            category,
            message: error.to_string(),
            exit_code: CONFIGURATION_EXIT_CODE,
            details: None,
        }
    }

    fn http(error: HttpError) -> Self {
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

    fn runtime(error: &std::io::Error) -> Self {
        Self {
            category: "runtime_error",
            message: format!("cannot start asynchronous HTTP runtime: {error}"),
            exit_code: EXECUTION_EXIT_CODE,
            details: None,
        }
    }

    fn stdin(error: &std::io::Error) -> Self {
        Self {
            category: "stdin_error",
            message: format!("cannot read OpenCollection YAML from stdin: {error}"),
            exit_code: INVALID_WORKSPACE_EXIT_CODE,
            details: None,
        }
    }

    fn persistence(error: SaveError) -> Self {
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

    fn create(error: CreateError) -> Self {
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

    fn structure(error: StructureError) -> Self {
        let category = error.category();
        let exit_code = match category {
            "request_not_found" | "folder_not_found" => REQUEST_NOT_FOUND_EXIT_CODE,
            "duplicate_destination"
            | "destination_not_found"
            | "invalid_destination"
            | "invalid_name"
            | "invalid_index" => INVALID_ARGUMENTS_EXIT_CODE,
            "persistence_read_only"
            | "workspace_modified"
            | "recovery_required"
            | "committed_refresh_failed"
            | "committed_cleanup_failed"
            | "persistence_error" => PERSISTENCE_EXIT_CODE,
            _ => PERSISTENCE_EXIT_CODE,
        };
        Self {
            category,
            message: error.to_string(),
            exit_code,
            details: None,
        }
    }

    fn yaak(error: YaakImportError) -> Self {
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
            YaakImportError::Unsupported(diagnostics) => Self {
                category: "unsupported_import",
                message: format!(
                    "Yaak workspace contains {} lossy item(s); inspect diagnostics or pass --allow-partial",
                    diagnostics
                        .iter()
                        .filter(|diagnostic| diagnostic.severity.as_str() == "lossy")
                        .count()
                ),
                exit_code: IMPORT_EXIT_CODE,
                details: Some(json!({
                    "diagnostics": diagnostics.iter().map(import_diagnostic_json).collect::<Vec<_>>()
                })),
            },
            YaakImportError::Invalid(message) => Self {
                category: "invalid_import",
                message,
                exit_code: INVALID_WORKSPACE_EXIT_CODE,
                details: None,
            },
            YaakImportError::Io { path, source } => Self {
                category: "invalid_import",
                message: format!("cannot read {}: {source}", path.display()),
                exit_code: INVALID_WORKSPACE_EXIT_CODE,
                details: None,
            },
        }
    }
}

struct ParsedOptions {
    environment: Option<String>,
    output: Option<PathBuf>,
    update: RequestUpdate,
    parent: Option<String>,
    index: Option<usize>,
    value: Option<String>,
    extends: Option<String>,
    workspace: Option<String>,
    allow_partial: bool,
}

fn extract_options(args: &mut Vec<String>) -> Result<ParsedOptions, CliError> {
    Ok(ParsedOptions {
        environment: extract_string_option(args, "--environment")?,
        output: extract_string_option(args, "--output")?.map(PathBuf::from),
        update: extract_request_update(args)?,
        parent: extract_string_option(args, "--parent")?,
        index: extract_index(args)?,
        value: extract_string_option(args, "--value")?,
        extends: extract_string_option(args, "--extends")?,
        workspace: extract_string_option(args, "--workspace")?,
        allow_partial: extract_flag(args, "--allow-partial")?,
    })
}

fn extract_flag(args: &mut Vec<String>, option: &'static str) -> Result<bool, CliError> {
    let count = args
        .iter()
        .filter(|argument| argument.as_str() == option)
        .count();
    if count > 1 {
        return Err(CliError::invalid_arguments(format!(
            "{option} may only be specified once"
        )));
    }
    args.retain(|argument| argument != option);
    Ok(count == 1)
}

fn extract_request_update(args: &mut Vec<String>) -> Result<RequestUpdate, CliError> {
    Ok(RequestUpdate {
        name: extract_string_option(args, "--name")?,
        method: extract_string_option(args, "--method")?,
        url: extract_string_option(args, "--url")?,
        ..RequestUpdate::default()
    })
}

fn extract_string_option(
    args: &mut Vec<String>,
    option: &'static str,
) -> Result<Option<String>, CliError> {
    let positions: Vec<_> = args
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == option).then_some(index))
        .collect();
    match positions.as_slice() {
        [] => Ok(None),
        [_first, _second, ..] => Err(CliError::invalid_arguments(format!(
            "{option} may only be specified once"
        ))),
        [position] => {
            if *position + 1 >= args.len()
                || args[*position + 1].is_empty()
                || args[*position + 1].starts_with('-')
            {
                return Err(CliError::invalid_arguments(format!(
                    "{option} requires a non-empty value"
                )));
            }
            let value = args.remove(*position + 1);
            args.remove(*position);
            Ok(Some(value))
        }
    }
}

fn extract_index(args: &mut Vec<String>) -> Result<Option<usize>, CliError> {
    let value = extract_string_option(args, "--index")?;
    value
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| CliError::invalid_arguments("--index requires a non-negative integer"))
        })
        .transpose()
}

fn parse_command(args: &[String], options: ParsedOptions) -> Result<Command, CliError> {
    let ParsedOptions {
        environment,
        output,
        update,
        parent,
        index,
        value,
        extends,
        workspace,
        allow_partial,
    } = options;
    let is_environment_set = matches!(
        args,
        [group, action, _] if group == "environment" && action == "set"
    );
    let is_environment_create = matches!(
        args,
        [group, action, _] if group == "environment" && action == "create"
    );
    if value.is_some() && !is_environment_set {
        return Err(CliError::invalid_arguments(
            "invalid command; run 'probe --help' for usage",
        ));
    }
    if extends.is_some() && !is_environment_create {
        return Err(CliError::invalid_arguments(
            "--extends is only valid for environment create",
        ));
    }
    let is_yaak_import = matches!(
        args,
        [group, action, format, _, _]
            if group == "collection" && action == "import" && format == "yaak"
    );
    if (workspace.is_some() || allow_partial) && !is_yaak_import {
        return Err(CliError::invalid_arguments(
            "--workspace and --allow-partial are only valid for Yaak import",
        ));
    }
    match args {
        [group, action, path]
            if group == "collection"
                && action == "create"
                && path != "-"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.method.is_none()
                && update.url.is_none() =>
        {
            Ok(Command::CreateCollection {
                path: PathBuf::from(path),
                name: update.name,
            })
        }
        [group, action, format, source, destination]
            if group == "collection"
                && action == "import"
                && format == "yaak"
                && source != "-"
                && destination != "-"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.is_empty()
                && value.is_none() =>
        {
            Ok(Command::ImportYaak {
                source: PathBuf::from(source),
                destination: PathBuf::from(destination),
                workspace,
                allow_partial,
            })
        }
        [group, action, path]
            if group == "collection"
                && action == "validate"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.is_empty() =>
        {
            Ok(Command::Validate {
                input: WorkspaceInput::from_argument(path),
            })
        }
        [group, action, path]
            if group == "request"
                && action == "list"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.is_empty() =>
        {
            Ok(Command::ListRequests {
                input: WorkspaceInput::from_argument(path),
            })
        }
        [group, action, path]
            if group == "folder"
                && action == "list"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.is_empty() =>
        {
            Ok(Command::ListFolders {
                input: WorkspaceInput::from_argument(path),
            })
        }
        [group, action, path, selector]
            if group == "request"
                && action == "get"
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.is_empty() =>
        {
            Ok(Command::Get {
                input: WorkspaceInput::from_argument(path),
                selector: selector.clone(),
                environment,
            })
        }
        [group, action, path, selector]
            if group == "request"
                && action == "run"
                && parent.is_none()
                && index.is_none()
                && update.is_empty() =>
        {
            Ok(Command::Run {
                input: WorkspaceInput::from_argument(path),
                selector: selector.clone(),
                environment,
                output,
            })
        }
        [group, action, path, selector]
            if group == "request"
                && action == "set"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && !update.is_empty() =>
        {
            Ok(Command::Set {
                input: WorkspaceInput::from_argument(path),
                selector: selector.clone(),
                update,
            })
        }
        [group, action, path]
            if group == "request"
                && action == "create"
                && environment.is_none()
                && output.is_none()
                && update.name.is_some() =>
        {
            Ok(Command::Structure {
                input: WorkspaceInput::from_argument(path),
                operation_name: "create",
                operation: StructureOperation::CreateRequest {
                    parent,
                    index,
                    name: update.name.expect("guarded request name"),
                    method: update.method,
                    url: update.url,
                },
            })
        }
        [group, action, path]
            if group == "folder"
                && action == "create"
                && environment.is_none()
                && output.is_none()
                && update.name.is_some()
                && update.method.is_none()
                && update.url.is_none() =>
        {
            Ok(Command::Structure {
                input: WorkspaceInput::from_argument(path),
                operation_name: "create",
                operation: StructureOperation::CreateFolder {
                    parent,
                    index,
                    name: update.name.expect("guarded folder name"),
                },
            })
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder")
                && action == "rename"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.name.is_some()
                && update.method.is_none()
                && update.url.is_none() =>
        {
            let operation = if group == "request" {
                StructureOperation::RenameRequest {
                    selector: selector.clone(),
                    name: update.name.expect("guarded rename name"),
                }
            } else {
                StructureOperation::RenameFolder {
                    selector: selector.clone(),
                    name: update.name.expect("guarded rename name"),
                }
            };
            Ok(Command::Structure {
                input: WorkspaceInput::from_argument(path),
                operation_name: "rename",
                operation,
            })
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder")
                && action == "delete"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.is_empty() =>
        {
            let operation = if group == "request" {
                StructureOperation::DeleteRequest {
                    selector: selector.clone(),
                }
            } else {
                StructureOperation::DeleteFolder {
                    selector: selector.clone(),
                }
            };
            Ok(Command::Structure {
                input: WorkspaceInput::from_argument(path),
                operation_name: "delete",
                operation,
            })
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder")
                && action == "move"
                && environment.is_none()
                && output.is_none()
                && update.is_empty() =>
        {
            let operation = if group == "request" {
                StructureOperation::MoveRequest {
                    selector: selector.clone(),
                    parent,
                    index,
                }
            } else {
                StructureOperation::MoveFolder {
                    selector: selector.clone(),
                    parent,
                    index,
                }
            };
            Ok(Command::Structure {
                input: WorkspaceInput::from_argument(path),
                operation_name: "move",
                operation,
            })
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder")
                && action == "reorder"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_some()
                && update.is_empty() =>
        {
            let index = index.expect("guarded reorder index");
            let operation = if group == "request" {
                StructureOperation::ReorderRequest {
                    selector: selector.clone(),
                    index,
                }
            } else {
                StructureOperation::ReorderFolder {
                    selector: selector.clone(),
                    index,
                }
            };
            Ok(Command::Structure {
                input: WorkspaceInput::from_argument(path),
                operation_name: "reorder",
                operation,
            })
        }
        [group, action, path]
            if group == "environment"
                && action == "list"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.is_empty()
                && value.is_none()
                && extends.is_none() =>
        {
            Ok(Command::ListEnvironments {
                input: WorkspaceInput::from_argument(path),
            })
        }
        [group, action, path]
            if group == "environment"
                && action == "create"
                && environment.is_none()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.name.is_some()
                && update.method.is_none()
                && update.url.is_none()
                && value.is_none() =>
        {
            Ok(Command::EnvironmentCreate {
                input: WorkspaceInput::from_argument(path),
                name: update.name.expect("guarded environment name"),
                extends,
            })
        }
        [group, action, path]
            if group == "environment"
                && action == "set"
                && environment.is_some()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.name.is_some()
                && update.method.is_none()
                && update.url.is_none()
                && value.is_some() =>
        {
            Ok(Command::EnvironmentSet {
                input: WorkspaceInput::from_argument(path),
                environment: environment.expect("guarded environment name"),
                name: update.name.expect("guarded variable name"),
                value: value.expect("guarded variable value"),
            })
        }
        [group, action, path]
            if group == "environment"
                && action == "unset"
                && environment.is_some()
                && output.is_none()
                && parent.is_none()
                && index.is_none()
                && update.name.is_some()
                && update.method.is_none()
                && update.url.is_none()
                && value.is_none() =>
        {
            Ok(Command::EnvironmentUnset {
                input: WorkspaceInput::from_argument(path),
                environment: environment.expect("guarded environment name"),
                name: update.name.expect("guarded variable name"),
            })
        }
        _ => Err(CliError::invalid_arguments(
            "invalid command; run 'probe --help' for usage",
        )),
    }
}

fn execute(command: Command, stdin: &mut impl Read) -> Result<CommandOutput, CliError> {
    match command {
        Command::CreateCollection { path, name } => create_collection(path, name),
        Command::ImportYaak {
            source,
            destination,
            workspace,
            allow_partial,
        } => import_yaak(source, destination, workspace.as_deref(), allow_partial),
        Command::Validate { input } => validate(&input, stdin),
        Command::ListRequests { input } => list_requests(&input, stdin),
        Command::ListFolders { input } => list_folders(&input, stdin),
        Command::ListEnvironments { input } => list_environments(&input, stdin),
        Command::Get {
            input,
            selector,
            environment,
        } => get_request(&input, &selector, environment.as_deref(), stdin),
        Command::Run {
            input,
            selector,
            environment,
            output,
        } => run_request(
            &input,
            &selector,
            environment.as_deref(),
            output.as_ref(),
            stdin,
        ),
        Command::Set {
            input,
            selector,
            update,
        } => update_request(&input, &selector, &update, stdin),
        Command::Structure {
            input,
            operation_name,
            operation,
        } => edit_structure(&input, operation_name, operation, stdin),
        Command::EnvironmentSet {
            input,
            environment,
            name,
            value,
        } => set_environment_variable(&input, &environment, &name, value, stdin),
        Command::EnvironmentUnset {
            input,
            environment,
            name,
        } => unset_environment_variable(&input, &environment, &name, stdin),
        Command::EnvironmentCreate {
            input,
            name,
            extends,
        } => create_environment(&input, &name, extends, stdin),
    }
}

fn create_environment(
    input: &WorkspaceInput,
    name: &str,
    extends: Option<String>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    loaded
        .create_environment(name.to_owned(), extends.clone())
        .map_err(CliError::persistence)?;
    Ok(environment_create_output(name, extends.as_deref()))
}

fn environment_create_output(name: &str, extends: Option<&str>) -> CommandOutput {
    let mut json = json!({
        "environment": name,
        "operation": "create",
    });
    if let Some(extends) = extends {
        json.as_object_mut()
            .expect("environment JSON output must be an object")
            .insert("extends".to_owned(), json!(extends));
    }
    CommandOutput {
        human: format!("Created environment {name}\n"),
        json,
    }
}

fn list_environments(
    input: &WorkspaceInput,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let mut lines = vec!["NAME\tEXTENDS".to_owned()];
    let mut environments = Vec::with_capacity(loaded.workspace().environments().len());
    for environment in loaded.workspace().environments() {
        let parent = environment.extends.as_deref().unwrap_or("");
        lines.push(format!("{}\t{parent}", environment.name));
        environments.push(json!({
            "extends": environment.extends,
            "name": environment.name,
        }));
    }
    Ok(CommandOutput {
        human: format!("{}\n", lines.join("\n")),
        json: json!({ "environments": environments }),
    })
}

fn set_environment_variable(
    input: &WorkspaceInput,
    environment: &str,
    name: &str,
    value: String,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    loaded
        .update_environment_variable(environment, name, value.clone())
        .map_err(CliError::persistence)?;
    Ok(environment_variable_output(
        "set",
        environment,
        name,
        Some(&value),
    ))
}

fn unset_environment_variable(
    input: &WorkspaceInput,
    environment: &str,
    name: &str,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    loaded
        .unset_environment_variable(environment, name)
        .map_err(CliError::persistence)?;
    Ok(environment_variable_output(
        "unset",
        environment,
        name,
        None,
    ))
}

fn environment_variable_output(
    operation: &str,
    environment: &str,
    name: &str,
    value: Option<&str>,
) -> CommandOutput {
    let mut json = json!({
        "environment": environment,
        "name": name,
        "operation": operation,
    });
    if let Some(value) = value {
        json.as_object_mut()
            .expect("environment JSON output must be an object")
            .insert("value".to_owned(), json!(value));
    }
    CommandOutput {
        human: format!(
            "{} environment variable {environment}.{name}\n",
            match operation {
                "set" => "Set",
                "unset" => "Unset",
                _ => operation,
            }
        ),
        json,
    }
}

fn edit_structure(
    input: &WorkspaceInput,
    operation_name: &str,
    operation: StructureOperation,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    let result = loaded
        .apply_structure(operation)
        .map_err(CliError::structure)?;
    Ok(structure_output(operation_name, &result))
}

fn structure_output(operation: &str, result: &StructureResult) -> CommandOutput {
    let selector = result.selector.as_deref().unwrap_or("<deleted>");
    CommandOutput {
        human: format!("{} {}: {selector}\n", operation, result.kind.as_str()),
        json: json!({
            "index": result.index,
            "itemType": result.kind.as_str(),
            "operation": operation,
            "parent": result.parent,
            "previousSelector": result.previous_selector,
            "selector": result.selector,
            "selectorRemaps": result.selector_remaps,
        }),
    }
}

fn update_request(
    input: &WorkspaceInput,
    selector: &str,
    update: &RequestUpdate,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    loaded
        .update_request(selector, update)
        .map_err(CliError::persistence)?;
    let key = loaded
        .request_key(selector)
        .expect("successfully updated selector must resolve");
    let request = loaded
        .workspace()
        .request(key)
        .expect("repository request key must resolve");
    Ok(CommandOutput {
        human: format!(
            "Updated request\n{}",
            request_human(selector, None, request)
        ),
        json: request_json(selector, None, request),
    })
}

fn run_request(
    input: &WorkspaceInput,
    selector: &str,
    environment: Option<&str>,
    output: Option<&PathBuf>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| CliError::request_not_found(selector))?;
    let request = if let Some(environment) = environment {
        resolve_loaded_request(&loaded, key, environment)?
    } else {
        loaded
            .workspace()
            .request(key)
            .expect("repository request key must resolve")
            .clone()
    };
    let base_directory = input.base_directory();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CliError::runtime(&error))?;
    let response = runtime.block_on(async {
        let engine = HttpEngine::new().map_err(CliError::http)?;
        let options = ExecutionOptions { base_directory };
        if let Some(output) = output {
            engine
                .execute_cancellable_to_file(&request, &options, output, tokio::signal::ctrl_c())
                .await
                .map_err(CliError::http)
        } else {
            engine
                .execute_cancellable(&request, &options, tokio::signal::ctrl_c())
                .await
                .map_err(CliError::http)
        }
    })?;
    Ok(response_output(&request, &response, output))
}

fn response_output(
    request: &probe_core::HttpRequest,
    response: &HttpResponse,
    output: Option<&PathBuf>,
) -> CommandOutput {
    CommandOutput {
        human: response_human(request, response, output.map(PathBuf::as_path)),
        json: response_json(request, response, output.map(PathBuf::as_path)),
    }
}

fn create_collection(path: PathBuf, name: Option<String>) -> Result<CommandOutput, CliError> {
    let loaded =
        create_bundled_workspace(&path, name.as_deref(), false).map_err(CliError::create)?;
    let workspace = loaded.workspace();
    let collection_name = workspace.metadata().name.as_deref().unwrap_or("<unnamed>");
    let created_path = loaded.source_path().map(PathBuf::from).unwrap_or(path);
    Ok(CommandOutput {
        human: format!(
            "Created bundled OpenCollection workspace\nName: {collection_name}\nPath: {}\n",
            created_path.display()
        ),
        json: json!({
            "collection": {
                "name": workspace.metadata().name,
            },
            "counts": {
                "environments": workspace.environments().len(),
                "folders": workspace.folder_count(),
                "requests": workspace.request_count(),
            },
            "created": true,
            "path": created_path,
        }),
    })
}

fn import_yaak(
    source: PathBuf,
    destination: PathBuf,
    workspace_id: Option<&str>,
    allow_partial: bool,
) -> Result<CommandOutput, CliError> {
    let preview = inspect_yaak_source(&source).map_err(CliError::yaak)?;
    let source_format = preview.format();
    let imported = preview
        .convert(workspace_id, allow_partial)
        .map_err(CliError::yaak)?;
    let loaded = create_bundled_workspace_from_collection(&destination, &imported.collection)
        .map_err(CliError::create)?;
    let path = loaded
        .source_path()
        .map(PathBuf::from)
        .unwrap_or(destination);
    let workspace = loaded.workspace();
    let warning_count = imported.diagnostics.len();
    Ok(CommandOutput {
        human: format!(
            "Imported Yaak workspace\nName: {}\nPath: {}\nRequests: {}\nFolders: {}\nEnvironments: {}\nWarnings: {warning_count}\n",
            imported.workspace.name,
            path.display(),
            workspace.request_count(),
            workspace.folder_count(),
            workspace.environments().len(),
        ),
        json: json!({
            "imported": true,
            "partial": imported.partial,
            "sourceFormat": source_format.as_str(),
            "workspace": {
                "id": imported.workspace.id,
                "name": imported.workspace.name,
            },
            "path": path,
            "counts": {
                "environments": workspace.environments().len(),
                "folders": workspace.folder_count(),
                "requests": workspace.request_count(),
            },
            "warnings": imported
                .diagnostics
                .iter()
                .map(import_diagnostic_json)
                .collect::<Vec<_>>(),
        }),
    })
}

fn import_diagnostic_json(diagnostic: &ImportDiagnostic) -> serde_json::Value {
    json!({
        "code": diagnostic.code,
        "severity": diagnostic.severity.as_str(),
        "resourceType": diagnostic.resource_type,
        "resourceId": diagnostic.resource_id,
        "field": diagnostic.field,
        "message": diagnostic.message,
    })
}

fn validate(input: &WorkspaceInput, stdin: &mut impl Read) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let workspace = loaded.workspace();
    let name = workspace.metadata().name.as_deref().unwrap_or("<unnamed>");
    Ok(CommandOutput {
        human: format!(
            "Valid OpenCollection workspace\nName: {name}\nRequests: {}\nFolders: {}\nEnvironments: {}\n",
            workspace.request_count(),
            workspace.folder_count(),
            workspace.environments().len()
        ),
        json: json!({
            "collection": {
                "name": workspace.metadata().name,
                "summary": workspace.metadata().summary,
                "version": workspace.metadata().version,
            },
            "counts": {
                "environments": workspace.environments().len(),
                "folders": workspace.folder_count(),
                "requests": workspace.request_count(),
            },
            "valid": true,
        }),
    })
}

fn list_requests(input: &WorkspaceInput, stdin: &mut impl Read) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let mut lines = vec!["SELECTOR\tMETHOD\tNAME\tURL".to_owned()];
    let mut requests = Vec::with_capacity(loaded.requests().len());
    for located in loaded.requests() {
        let request = loaded
            .workspace()
            .request(located.key())
            .expect("repository request key must resolve");
        let name = request.metadata.name.as_deref().unwrap_or("");
        let method = request.method.as_deref().unwrap_or("");
        let url = request.url.as_deref().unwrap_or("");
        lines.push(format!("{}\t{method}\t{name}\t{url}", located.selector()));
        requests.push(json!({
            "method": request.method,
            "name": request.metadata.name,
            "selector": located.selector(),
            "url": request.url,
        }));
    }
    Ok(CommandOutput {
        human: format!("{}\n", lines.join("\n")),
        json: json!({ "requests": requests }),
    })
}

fn list_folders(input: &WorkspaceInput, stdin: &mut impl Read) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let mut parents = BTreeMap::new();
    collect_folder_parents(&loaded, loaded.workspace().root_items(), None, &mut parents);
    let mut lines = vec!["SELECTOR\tNAME\tPARENT".to_owned()];
    let mut folders = Vec::with_capacity(loaded.folders().len());
    for located in loaded.folders() {
        let folder = loaded
            .workspace()
            .folder(located.key())
            .expect("repository folder key must resolve");
        let name = folder.metadata.name.as_deref().unwrap_or("");
        let parent = parents
            .get(&located.key())
            .and_then(|parent| parent.as_deref());
        lines.push(format!(
            "{}\t{name}\t{}",
            located.selector(),
            parent.unwrap_or("")
        ));
        folders.push(json!({
            "name": folder.metadata.name,
            "parent": parent,
            "selector": located.selector(),
        }));
    }
    Ok(CommandOutput {
        human: format!("{}\n", lines.join("\n")),
        json: json!({ "folders": folders }),
    })
}

fn collect_folder_parents(
    loaded: &LoadedWorkspace,
    items: &[WorkspaceItemRef],
    parent: Option<&str>,
    output: &mut BTreeMap<FolderKey, Option<String>>,
) {
    for item in items {
        if let WorkspaceItemRef::Folder(key) = item {
            output.insert(*key, parent.map(str::to_owned));
            let selector = loaded
                .folder_selector(*key)
                .expect("repository folder key must have a selector");
            let folder = loaded
                .workspace()
                .folder(*key)
                .expect("repository folder key must resolve");
            collect_folder_parents(loaded, &folder.children, Some(selector), output);
        }
    }
}

fn get_request(
    input: &WorkspaceInput,
    selector: &str,
    environment: Option<&str>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| CliError::request_not_found(selector))?;
    let resolved;
    let request = if let Some(environment) = environment {
        resolved = resolve_loaded_request(&loaded, key, environment)?;
        &resolved
    } else {
        loaded
            .workspace()
            .request(key)
            .expect("repository request key must resolve")
    };
    Ok(CommandOutput {
        human: request_human(selector, environment, request),
        json: request_json(selector, environment, request),
    })
}

fn resolve_loaded_request(
    loaded: &LoadedWorkspace,
    key: probe_core::RequestKey,
    environment: &str,
) -> Result<probe_core::HttpRequest, CliError> {
    let workspace = loaded.workspace();
    let environment = resolve_environment(workspace.environments(), environment)
        .map_err(CliError::configuration)?;
    let request = workspace
        .request(key)
        .expect("repository request key must resolve");
    resolve_request(request, &environment).map_err(CliError::configuration)
}

fn load(input: &WorkspaceInput, stdin: &mut impl Read) -> Result<LoadedWorkspace, CliError> {
    match input {
        WorkspaceInput::Path(path) => load_workspace(path),
        WorkspaceInput::Stdin => {
            let mut source = String::new();
            stdin
                .read_to_string(&mut source)
                .map_err(|error| CliError::stdin(&error))?;
            load_workspace_from_str(&source)
        }
    }
    .map_err(|error| CliError::invalid_workspace(error.to_string()))
}

fn versioned_json(mut value: serde_json::Value) -> serde_json::Value {
    value
        .as_object_mut()
        .expect("command JSON output must be an object")
        .insert("schemaVersion".to_owned(), json!(JSON_SCHEMA_VERSION));
    value
}

fn pretty_json(value: &serde_json::Value) -> String {
    let mut output =
        serde_json::to_string_pretty(value).expect("JSON value serialization cannot fail");
    output.push('\n');
    output
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        INVALID_ARGUMENTS_EXIT_CODE, INVALID_WORKSPACE_EXIT_CODE, help, run, run_with_stdin,
    };

    #[test]
    fn help_option_returns_usage() {
        let output = run(["--help"]);
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, help());
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn unknown_argument_is_rejected() {
        let output = run(["--unknown"]);
        assert_eq!(output.exit_code, INVALID_ARGUMENTS_EXIT_CODE);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.contains("invalid_arguments"));
    }

    #[test]
    fn validate_rejects_yaml_without_opencollection_headers() {
        let mut stdin = Cursor::new(b"{}\n");
        let output = run_with_stdin(["collection", "validate", "-", "--json"], &mut stdin);

        assert_eq!(output.exit_code, INVALID_WORKSPACE_EXIT_CODE);
        assert!(output.stdout.contains("invalid_workspace"));
    }
}
