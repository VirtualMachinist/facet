use std::path::PathBuf;

use probe_core::RequestUpdate;
use probe_opencollection::StructureOperation;

use crate::{CliError, WorkspaceInput};

const ENVIRONMENT: u16 = 1 << 0;
const OUTPUT: u16 = 1 << 1;
const NAME: u16 = 1 << 2;
const METHOD: u16 = 1 << 3;
const URL: u16 = 1 << 4;
const PARENT: u16 = 1 << 5;
const INDEX: u16 = 1 << 6;
const VALUE: u16 = 1 << 7;
const EXTENDS: u16 = 1 << 8;
const WORKSPACE: u16 = 1 << 9;
const ALLOW_PARTIAL: u16 = 1 << 10;
const VAR: u16 = 1 << 11;

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
}

impl Options {
    fn present(&self) -> u16 {
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
    }

    fn allow(&self, allowed: u16) -> Result<(), CliError> {
        if self.present() & !allowed == 0 {
            Ok(())
        } else {
            Err(invalid_command())
        }
    }
}

const fn option_bit(present: bool, bit: u16) -> u16 {
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
            options.allow(ENVIRONMENT)?;
            Ok(Command::Get {
                input: input(path),
                selector: selector.clone(),
                environment: options.environment,
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
            options.allow(ENVIRONMENT | OUTPUT | VAR)?;
            Ok(Command::Run {
                input: input(path),
                selector: selector.clone(),
                environment: options.environment,
                variables: options.variables,
                output: options.output,
            })
        }
        [group, action, path, selector] if group == "request" && action == "set" => {
            options.allow(NAME | METHOD | URL)?;
            if options.update.is_empty() {
                return Err(invalid_command());
            }
            Ok(Command::Set {
                input: input(path),
                selector: selector.clone(),
                update: options.update,
            })
        }
        [group, action, path] if group == "request" && action == "create" => {
            options.allow(NAME | METHOD | URL | PARENT | INDEX)?;
            let name = options.update.name.ok_or_else(invalid_command)?;
            Ok(Command::Structure {
                input: input(path),
                operation_name: "create",
                operation: StructureOperation::CreateRequest {
                    parent: options.parent,
                    index: options.index,
                    name,
                    method: options.update.method,
                    url: options.update.url,
                },
            })
        }
        [group, action, path] if group == "folder" && action == "create" => {
            options.allow(NAME | PARENT | INDEX)?;
            Ok(Command::Structure {
                input: input(path),
                operation_name: "create",
                operation: StructureOperation::CreateFolder {
                    parent: options.parent,
                    index: options.index,
                    name: options.update.name.ok_or_else(invalid_command)?,
                },
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
    })
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
        operation,
    }
}

fn duplicate_option(option: &str) -> CliError {
    CliError::invalid_arguments(format!("{option} may only be specified once"))
}

fn invalid_command() -> CliError {
    CliError::invalid_arguments("invalid command; run 'probe --help' for usage")
}
