use std::path::PathBuf;

use probe_core::{GraphqlUpdate, RequestUpdate};
use probe_opencollection::{CreatedRequestProtocol, StructureOperation};
use serde_json::{Map, Value};

use crate::{CliError, WorkspaceInput};

const ENVIRONMENT: u32 = 1 << 0;
const OUTPUT: u32 = 1 << 1;
const NAME: u32 = 1 << 2;
const METHOD: u32 = 1 << 3;
const URL: u32 = 1 << 4;
const PARENT: u32 = 1 << 5;
const INDEX: u32 = 1 << 6;
const VALUE: u32 = 1 << 7;
const EXTENDS: u32 = 1 << 8;
const WORKSPACE: u32 = 1 << 9;
const ALLOW_PARTIAL: u32 = 1 << 10;
const VAR: u32 = 1 << 11;
const STRICT_VARIABLES: u32 = 1 << 12;
const GRAPHQL_QUERY: u32 = 1 << 13;
const GRAPHQL_VARIABLES: u32 = 1 << 14;
const GRAPHQL_OPERATION_NAME: u32 = 1 << 15;
const GRAPHQL_EXTENSIONS: u32 = 1 << 16;
const TYPE: u32 = 1 << 17;
const DRY_RUN: u32 = 1 << 18;

#[derive(Debug)]
pub(crate) enum Command {
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
    ImportPostman {
        source: PathBuf,
        destination: PathBuf,
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
        strict_variables: bool,
    },
    Variables {
        input: WorkspaceInput,
        selector: String,
        environment: Option<String>,
    },
    Run {
        input: WorkspaceInput,
        selector: String,
        environment: Option<String>,
        variables: Vec<(String, String)>,
        output: Option<PathBuf>,
        strict_variables: bool,
        dry_run: bool,
    },
    Set {
        input: WorkspaceInput,
        selector: String,
        update: Box<RequestUpdate>,
    },
    Structure {
        input: WorkspaceInput,
        operation_name: &'static str,
        operation: Box<StructureOperation>,
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
    EnvironmentDelete {
        input: WorkspaceInput,
        environment: String,
    },
    EnvironmentRename {
        input: WorkspaceInput,
        environment: String,
        name: String,
    },
}

struct Options {
    environment: Option<String>,
    output: Option<PathBuf>,
    update: RequestUpdate,
    parent: Option<String>,
    index: Option<usize>,
    value: Option<String>,
    extends: Option<String>,
    workspace: Option<String>,
    allow_partial: bool,
    variables: Vec<(String, String)>,
    strict_variables: bool,
    graphql_query: Option<String>,
    graphql_variables: Option<String>,
    graphql_operation_name: Option<String>,
    graphql_extensions: Option<String>,
    request_type: Option<String>,
    dry_run: bool,
}

impl Options {
    fn present(&self) -> u32 {
        option_bit(self.environment.is_some(), ENVIRONMENT)
            | option_bit(self.output.is_some(), OUTPUT)
            | option_bit(self.update.name.is_some(), NAME)
            | option_bit(self.update.method.is_some(), METHOD)
            | option_bit(self.update.url.is_some(), URL)
            | option_bit(self.parent.is_some(), PARENT)
            | option_bit(self.index.is_some(), INDEX)
            | option_bit(self.value.is_some(), VALUE)
            | option_bit(self.extends.is_some(), EXTENDS)
            | option_bit(self.workspace.is_some(), WORKSPACE)
            | option_bit(self.allow_partial, ALLOW_PARTIAL)
            | option_bit(!self.variables.is_empty(), VAR)
            | option_bit(self.strict_variables, STRICT_VARIABLES)
            | option_bit(self.graphql_query.is_some(), GRAPHQL_QUERY)
            | option_bit(self.graphql_variables.is_some(), GRAPHQL_VARIABLES)
            | option_bit(
                self.graphql_operation_name.is_some(),
                GRAPHQL_OPERATION_NAME,
            )
            | option_bit(self.graphql_extensions.is_some(), GRAPHQL_EXTENSIONS)
            | option_bit(self.request_type.is_some(), TYPE)
            | option_bit(self.dry_run, DRY_RUN)
    }

    fn allow(&self, allowed: u32) -> Result<(), CliError> {
        if self.present() & !allowed == 0 {
            Ok(())
        } else {
            Err(invalid_command())
        }
    }
}

const fn option_bit(present: bool, bit: u32) -> u32 {
    if present { bit } else { 0 }
}

pub(crate) fn parse(mut args: Vec<String>) -> Result<Command, CliError> {
    let options = extract_options(&mut args)?;
    validate_scoped_options(&args, &options)?;

    match args.as_slice() {
        [group, action, path] if group == "collection" && action == "create" && path != "-" => {
            options.allow(NAME)?;
            Ok(Command::CreateCollection {
                path: PathBuf::from(path),
                name: options.update.name,
            })
        }
        [group, action, format, source, destination]
            if group == "collection"
                && action == "import"
                && format == "yaak"
                && source != "-"
                && destination != "-" =>
        {
            options.allow(WORKSPACE | ALLOW_PARTIAL)?;
            Ok(Command::ImportYaak {
                source: PathBuf::from(source),
                destination: PathBuf::from(destination),
                workspace: options.workspace,
                allow_partial: options.allow_partial,
            })
        }
        [group, action, format, source, destination]
            if group == "collection"
                && action == "import"
                && format == "postman"
                && source != "-"
                && destination != "-" =>
        {
            options.allow(ALLOW_PARTIAL)?;
            Ok(Command::ImportPostman {
                source: PathBuf::from(source),
                destination: PathBuf::from(destination),
                allow_partial: options.allow_partial,
            })
        }
        [group, action, path] if group == "collection" && action == "validate" => {
            options.allow(0)?;
            Ok(Command::Validate { input: input(path) })
        }
        [group, action, path] if group == "request" && action == "list" => {
            options.allow(0)?;
            Ok(Command::ListRequests { input: input(path) })
        }
        [group, action, path] if group == "folder" && action == "list" => {
            options.allow(0)?;
            Ok(Command::ListFolders { input: input(path) })
        }
        [group, action, path, selector] if group == "request" && action == "get" => {
            options.allow(ENVIRONMENT | STRICT_VARIABLES)?;
            Ok(Command::Get {
                input: input(path),
                selector: selector.clone(),
                environment: options.environment,
                strict_variables: options.strict_variables,
            })
        }
        [group, action, path, selector] if group == "request" && action == "variables" => {
            options.allow(ENVIRONMENT)?;
            Ok(Command::Variables {
                input: input(path),
                selector: selector.clone(),
                environment: options.environment,
            })
        }
        [group, action, path, selector] if group == "request" && action == "run" => {
            options.allow(ENVIRONMENT | OUTPUT | VAR | STRICT_VARIABLES | DRY_RUN)?;
            if options.dry_run && options.output.is_some() {
                return Err(CliError::invalid_arguments(
                    "--dry-run cannot be combined with --output",
                ));
            }
            Ok(Command::Run {
                input: input(path),
                selector: selector.clone(),
                environment: options.environment,
                variables: options.variables,
                output: options.output,
                strict_variables: options.strict_variables,
                dry_run: options.dry_run,
            })
        }
        [group, action, path, selector] if group == "request" && action == "set" => {
            options.allow(
                NAME | METHOD
                    | URL
                    | GRAPHQL_QUERY
                    | GRAPHQL_VARIABLES
                    | GRAPHQL_OPERATION_NAME
                    | GRAPHQL_EXTENSIONS,
            )?;
            let update = graphql_update(options)?;
            if update.is_empty() {
                return Err(invalid_command());
            }
            Ok(Command::Set {
                input: input(path),
                selector: selector.clone(),
                update: Box::new(update),
            })
        }
        [group, action, path] if group == "request" && action == "create" => {
            options.allow(
                NAME | METHOD
                    | URL
                    | PARENT
                    | INDEX
                    | TYPE
                    | GRAPHQL_QUERY
                    | GRAPHQL_VARIABLES
                    | GRAPHQL_OPERATION_NAME
                    | GRAPHQL_EXTENSIONS,
            )?;
            let protocol = created_request_protocol(&options)?;
            let parent = options.parent.clone();
            let index = options.index;
            let update = graphql_update(options)?;
            let name = update.name.ok_or_else(invalid_command)?;
            Ok(Command::Structure {
                input: input(path),
                operation_name: "create",
                operation: Box::new(StructureOperation::CreateRequest {
                    parent,
                    index,
                    name,
                    method: update.method,
                    url: update.url,
                    protocol,
                    graphql: update.graphql,
                }),
            })
        }
        [group, action, path] if group == "folder" && action == "create" => {
            options.allow(NAME | PARENT | INDEX)?;
            Ok(Command::Structure {
                input: input(path),
                operation_name: "create",
                operation: Box::new(StructureOperation::CreateFolder {
                    parent: options.parent,
                    index: options.index,
                    name: options.update.name.ok_or_else(invalid_command)?,
                }),
            })
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder") && action == "rename" =>
        {
            options.allow(NAME)?;
            let name = options.update.name.ok_or_else(invalid_command)?;
            let operation = if group == "request" {
                StructureOperation::RenameRequest {
                    selector: selector.clone(),
                    name,
                }
            } else {
                StructureOperation::RenameFolder {
                    selector: selector.clone(),
                    name,
                }
            };
            Ok(structure(input(path), "rename", operation))
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder") && action == "delete" =>
        {
            options.allow(0)?;
            let operation = if group == "request" {
                StructureOperation::DeleteRequest {
                    selector: selector.clone(),
                }
            } else {
                StructureOperation::DeleteFolder {
                    selector: selector.clone(),
                }
            };
            Ok(structure(input(path), "delete", operation))
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder") && action == "move" =>
        {
            options.allow(PARENT | INDEX)?;
            let operation = if group == "request" {
                StructureOperation::MoveRequest {
                    selector: selector.clone(),
                    parent: options.parent,
                    index: options.index,
                }
            } else {
                StructureOperation::MoveFolder {
                    selector: selector.clone(),
                    parent: options.parent,
                    index: options.index,
                }
            };
            Ok(structure(input(path), "move", operation))
        }
        [group, action, path, selector]
            if matches!(group.as_str(), "request" | "folder") && action == "reorder" =>
        {
            options.allow(INDEX)?;
            let index = options.index.ok_or_else(invalid_command)?;
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
            Ok(structure(input(path), "reorder", operation))
        }
        [group, action, path] if group == "environment" && action == "list" => {
            options.allow(0)?;
            Ok(Command::ListEnvironments { input: input(path) })
        }
        [group, action, path] if group == "environment" && action == "create" => {
            options.allow(NAME | EXTENDS)?;
            Ok(Command::EnvironmentCreate {
                input: input(path),
                name: options.update.name.ok_or_else(invalid_command)?,
                extends: options.extends,
            })
        }
        [group, action, path] if group == "environment" && action == "set" => {
            options.allow(ENVIRONMENT | NAME | VALUE)?;
            Ok(Command::EnvironmentSet {
                input: input(path),
                environment: options.environment.ok_or_else(invalid_command)?,
                name: options.update.name.ok_or_else(invalid_command)?,
                value: options.value.ok_or_else(invalid_command)?,
            })
        }
        [group, action, path] if group == "environment" && action == "unset" => {
            options.allow(ENVIRONMENT | NAME)?;
            Ok(Command::EnvironmentUnset {
                input: input(path),
                environment: options.environment.ok_or_else(invalid_command)?,
                name: options.update.name.ok_or_else(invalid_command)?,
            })
        }
        [group, action, path] if group == "environment" && action == "delete" => {
            options.allow(ENVIRONMENT)?;
            Ok(Command::EnvironmentDelete {
                input: input(path),
                environment: options.environment.ok_or_else(invalid_command)?,
            })
        }
        [group, action, path] if group == "environment" && action == "rename" => {
            options.allow(ENVIRONMENT | NAME)?;
            Ok(Command::EnvironmentRename {
                input: input(path),
                environment: options.environment.ok_or_else(invalid_command)?,
                name: options.update.name.ok_or_else(invalid_command)?,
            })
        }
        _ => Err(invalid_command()),
    }
}

fn validate_scoped_options(args: &[String], options: &Options) -> Result<(), CliError> {
    let is_environment_create =
        matches!(args, [group, action, _] if group == "environment" && action == "create");
    if options.extends.is_some() && !is_environment_create {
        return Err(CliError::invalid_arguments(
            "--extends is only valid for environment create",
        ));
    }
    let is_yaak_import = matches!(args, [group, action, format, _, _] if group == "collection" && action == "import" && format == "yaak");
    let is_postman_import = matches!(args, [group, action, format, _, _] if group == "collection" && action == "import" && format == "postman");
    if options.workspace.is_some() && !is_yaak_import {
        return Err(CliError::invalid_arguments(
            "--workspace is only valid for Yaak import",
        ));
    }
    if options.allow_partial && !(is_yaak_import || is_postman_import) {
        return Err(CliError::invalid_arguments(
            "--allow-partial is only valid for collection import",
        ));
    }
    let is_request_create =
        matches!(args, [group, action, _] if group == "request" && action == "create");
    if options.request_type.is_some() && !is_request_create {
        return Err(CliError::invalid_arguments(
            "--type is only valid for request create",
        ));
    }
    Ok(())
}

fn extract_options(args: &mut Vec<String>) -> Result<Options, CliError> {
    Ok(Options {
        environment: extract_string_option(args, "--environment")?,
        output: extract_string_option(args, "--output")?.map(PathBuf::from),
        update: RequestUpdate {
            name: extract_string_option(args, "--name")?,
            method: extract_string_option(args, "--method")?,
            url: extract_string_option(args, "--url")?,
            ..RequestUpdate::default()
        },
        parent: extract_string_option(args, "--parent")?,
        index: extract_index(args)?,
        value: extract_string_option(args, "--value")?,
        extends: extract_string_option(args, "--extends")?,
        workspace: extract_string_option(args, "--workspace")?,
        allow_partial: extract_flag(args, "--allow-partial")?,
        variables: extract_variables(args)?,
        strict_variables: extract_flag(args, "--strict-variables")?,
        graphql_query: extract_string_option(args, "--graphql-query")?,
        graphql_variables: extract_string_option(args, "--graphql-variables")?,
        graphql_operation_name: extract_string_option(args, "--graphql-operation-name")?,
        graphql_extensions: extract_string_option(args, "--graphql-extensions")?,
        request_type: extract_string_option(args, "--type")?,
        dry_run: extract_flag(args, "--dry-run")?,
    })
}

fn graphql_requested(options: &Options) -> bool {
    options.graphql_query.is_some()
        || options.graphql_variables.is_some()
        || options.graphql_operation_name.is_some()
        || options.graphql_extensions.is_some()
}

fn created_request_protocol(options: &Options) -> Result<CreatedRequestProtocol, CliError> {
    match options.request_type.as_deref() {
        None if graphql_requested(options) => Ok(CreatedRequestProtocol::Graphql),
        None | Some("http") => {
            if graphql_requested(options) {
                Err(CliError::invalid_arguments(
                    "GraphQL fields cannot be applied to an HTTP request",
                ))
            } else {
                Ok(CreatedRequestProtocol::Http)
            }
        }
        Some("graphql") => Ok(CreatedRequestProtocol::Graphql),
        Some(_) => Err(CliError::invalid_arguments(
            "--type must be http or graphql",
        )),
    }
}

fn graphql_update(mut options: Options) -> Result<RequestUpdate, CliError> {
    if !graphql_requested(&options) {
        return Ok(options.update);
    }
    let graphql = GraphqlUpdate {
        query: options.graphql_query.take(),
        variables: options
            .graphql_variables
            .as_deref()
            .map(|source| parse_graphql_object(source, "variables"))
            .transpose()?,
        operation_name: options
            .graphql_operation_name
            .as_deref()
            .map(|source| parse_graphql_string(source, "operation name"))
            .transpose()?,
        extensions: options
            .graphql_extensions
            .as_deref()
            .map(|source| parse_graphql_object(source, "extensions"))
            .transpose()?,
    };
    options.update.graphql = Some(graphql);
    Ok(options.update)
}

fn parse_graphql_object(source: &str, field: &str) -> Result<Option<Map<String, Value>>, CliError> {
    let value: Value = serde_json::from_str(source).map_err(|_| {
        CliError::invalid_arguments(format!("GraphQL {field} must be a JSON object or null"))
    })?;
    match value {
        Value::Null => Ok(None),
        Value::Object(value) => Ok(Some(value)),
        _ => Err(CliError::invalid_arguments(format!(
            "GraphQL {field} must be a JSON object or null"
        ))),
    }
}

fn parse_graphql_string(source: &str, field: &str) -> Result<Option<String>, CliError> {
    let value: Value = serde_json::from_str(source).map_err(|_| {
        CliError::invalid_arguments(format!("GraphQL {field} must be a JSON string or null"))
    })?;
    match value {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value)),
        _ => Err(CliError::invalid_arguments(format!(
            "GraphQL {field} must be a JSON string or null"
        ))),
    }
}

fn extract_variables(args: &mut Vec<String>) -> Result<Vec<(String, String)>, CliError> {
    let mut variables = Vec::new();
    while let Some(position) = args.iter().position(|argument| argument == "--var") {
        if position + 1 >= args.len() {
            return Err(invalid_variable());
        }
        let argument = args.remove(position + 1);
        args.remove(position);
        let Some((name, value)) = argument.split_once('=') else {
            return Err(invalid_variable());
        };
        if name.is_empty() {
            return Err(invalid_variable());
        }
        variables.push((name.to_owned(), value.to_owned()));
    }
    Ok(variables)
}

fn invalid_variable() -> CliError {
    CliError::invalid_arguments("--var requires NAME=VALUE with a non-empty name")
}

fn extract_flag(args: &mut Vec<String>, option: &'static str) -> Result<bool, CliError> {
    let count = args.iter().filter(|argument| argument == &option).count();
    if count > 1 {
        return Err(duplicate_option(option));
    }
    args.retain(|argument| argument != option);
    Ok(count == 1)
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
        [_, _, ..] => Err(duplicate_option(option)),
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
    extract_string_option(args, "--index")?
        .map(|value| {
            value
                .parse()
                .map_err(|_| CliError::invalid_arguments("--index requires a non-negative integer"))
        })
        .transpose()
}

fn input(path: &str) -> WorkspaceInput {
    WorkspaceInput::from_argument(path)
}

fn structure(
    input: WorkspaceInput,
    operation_name: &'static str,
    operation: StructureOperation,
) -> Command {
    Command::Structure {
        input,
        operation_name,
        operation: Box::new(operation),
    }
}

fn duplicate_option(option: &str) -> CliError {
    CliError::invalid_arguments(format!("{option} may only be specified once"))
}

fn invalid_command() -> CliError {
    CliError::invalid_arguments("invalid command; run 'probe --help' for usage")
}
