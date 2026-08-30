//! Command-line presentation adapter for Probe.

#![forbid(unsafe_code)]

use std::io::{self, Read};

use serde_json::json;

mod collection;
mod command;
mod environment;
mod error;
mod presentation;
mod request;
mod structure;
mod workspace;

use command::{Command, parse as parse_command};
use error::CliError;
use workspace::{WorkspaceInput, load};

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
        "  collection import postman <source> <destination>  Import a Postman collection\n",
        "  collection import yaak <source> <destination>     Import a Yaak workspace\n",
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
        "  environment delete <path>           Delete an environment\n",
        "  environment rename <path>           Rename an environment\n",
        "\n",
        "Options:\n",
        "      --environment <name>  Environment used to resolve or mutate variables\n",
        "      --extends <name>       Parent environment for environment create\n",
        "      --output <file>        Write the response body to a file\n",
        "      --var <NAME=VALUE>     Override a variable for this request execution; may be repeated\n",
        "      --name <name>          Set a request, folder, collection, environment, or variable name\n",
        "      --method <method>      Set an HTTP method\n",
        "      --url <url>            Set a request URL\n",
        "      --value <value>        Set an environment variable value\n",
        "      --parent <selector>    Destination folder (omit for collection root)\n",
        "      --index <index>        Zero-based insertion position (omit to append)\n",
        "      --workspace <id>       Select a workspace from a multi-workspace import\n",
        "      --allow-partial        Explicitly allow lossy import conversion\n",
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
    "  import postman <source.json> <destination> [--allow-partial]\n",
    "  import yaak <source> <destination> [--workspace <id>] [--allow-partial]\n",
    "  validate <path|->              Validate a bundled file, stdin (-), or unbundled directory\n",
    "\n",
    "create writes a new bundled collection and refuses to overwrite an existing path.\n",
    "When --name is omitted, the collection title is the file stem. A missing .yml\n",
    "extension is added. validate accepts a bundled YAML file, stdin (-), or an unbundled directory.\n",
    "Postman import accepts an official Collection v2.0 or v2.1 JSON export. Yaak import\n",
    "accepts an official export JSON file or Directory Sync folder. Imports never\n",
    "overwrites an existing destination. Multi-workspace exports require --workspace.\n",
);

const REQUEST_HELP: &str = concat!(
    "Usage: probe request <COMMAND>\n",
    "\n",
    "Commands:\n",
    "  list <path|->                 List requests and repository selectors\n",
    "  get <path|-> <selector> [--environment <name>]  Inspect one request\n",
    "  run <path|-> <selector> [--environment <name>] [--var <NAME=VALUE>]... [--output <file>]\n",
    "  set <path> <selector> [--name <name>] [--method <method>] [--url <url>]\n",
    "  create <path> --name <name> [--parent <folder>] [--index <index>] [--method <method>] [--url <url>]\n",
    "  rename <path> <selector> --name <name>\n",
    "  delete <path> <selector>\n",
    "  move <path> <selector> [--parent <folder>] [--index <index>]\n",
    "  reorder <path> <selector> --index <index>\n",
    "\n",
    "Options:\n",
    "      --var <NAME=VALUE>  Override a variable for this request execution. May be specified multiple times.\n",
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
    "  delete <path> --environment <name>\n",
    "  rename <path> --environment <name> --name <new>\n",
    "\n",
    "Create adds a new environment with an optional parent. Delete removes a leaf environment.\n",
    "Rename changes a leaf environment's name. Set writes a plain variable on the named\n",
    "environment. A parent-only variable is overridden on the selected environment.\n",
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
    let json_count = remove_flags(&mut args, &["--json"]);
    let json_output = json_count == 1;
    let quiet_count = remove_flags(&mut args, &["-q", "--quiet"]);
    let quiet = quiet_count == 1;

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

    match parse_command(args).and_then(|command| execute(command, stdin)) {
        Ok(output) => RunOutput::success(output.render(json_output, quiet)),
        Err(error) => RunOutput::failure(error, json_output),
    }
}

fn remove_flags(args: &mut Vec<String>, flags: &[&str]) -> usize {
    let count = args
        .iter()
        .filter(|argument| flags.contains(&argument.as_str()))
        .count();
    args.retain(|argument| !flags.contains(&argument.as_str()));
    count
}

fn execute(command: Command, stdin: &mut impl Read) -> Result<CommandOutput, CliError> {
    match command {
        Command::CreateCollection { path, name } => collection::create(path, name),
        Command::ImportYaak {
            source,
            destination,
            workspace,
            allow_partial,
        } => collection::import_yaak(source, destination, workspace.as_deref(), allow_partial),
        Command::ImportPostman {
            source,
            destination,
            allow_partial,
        } => collection::import_postman(source, destination, allow_partial),
        Command::Validate { input } => collection::validate(&input, stdin),
        Command::ListRequests { input } => request::list(&input, stdin),
        Command::ListFolders { input } => structure::list_folders(&input, stdin),
        Command::ListEnvironments { input } => environment::list(&input, stdin),
        Command::Get {
            input,
            selector,
            environment,
        } => request::get(&input, &selector, environment.as_deref(), stdin),
        Command::Run {
            input,
            selector,
            environment,
            variables,
            output,
        } => request::run(
            &input,
            &selector,
            environment.as_deref(),
            &variables,
            output.as_ref(),
            stdin,
        ),
        Command::Set {
            input,
            selector,
            update,
        } => request::update(&input, &selector, &update, stdin),
        Command::Structure {
            input,
            operation_name,
            operation,
        } => structure::edit(&input, operation_name, operation, stdin),
        Command::EnvironmentSet {
            input,
            environment,
            name,
            value,
        } => environment::set_variable(&input, &environment, &name, value, stdin),
        Command::EnvironmentUnset {
            input,
            environment,
            name,
        } => environment::unset_variable(&input, &environment, &name, stdin),
        Command::EnvironmentCreate {
            input,
            name,
            extends,
        } => environment::create(&input, &name, extends, stdin),
        Command::EnvironmentDelete { input, environment } => {
            environment::delete(&input, &environment, stdin)
        }
        Command::EnvironmentRename {
            input,
            environment,
            name,
        } => environment::rename(&input, &environment, &name, stdin),
    }
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

    use super::{INVALID_WORKSPACE_EXIT_CODE, run_with_stdin};

    #[test]
    fn validate_rejects_yaml_without_opencollection_headers() {
        let mut stdin = Cursor::new(b"{}\n");
        let output = run_with_stdin(["collection", "validate", "-", "--json"], &mut stdin);

        assert_eq!(output.exit_code, INVALID_WORKSPACE_EXIT_CODE);
        assert!(output.stdout.contains("invalid_workspace"));
    }
}
