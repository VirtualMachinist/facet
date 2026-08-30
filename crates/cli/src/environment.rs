use std::io::Read;

use probe_core::EnvironmentResolutionError;
use serde_json::json;

use crate::{CliError, CommandOutput, WorkspaceInput, load};

pub(crate) fn create(
    input: &WorkspaceInput,
    name: &str,
    extends: Option<String>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    let output = create_output(name, extends.as_deref());
    loaded
        .create_environment(name.to_owned(), extends)
        .map_err(CliError::persistence)?;
    Ok(output)
}

pub(crate) fn delete(
    input: &WorkspaceInput,
    environment: &str,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    loaded
        .delete_environment(environment)
        .map_err(CliError::persistence)?;
    Ok(CommandOutput {
        human: format!("Deleted environment {environment}\n"),
        json: json!({
            "environment": environment,
            "operation": "delete",
        }),
    })
}

pub(crate) fn rename(
    input: &WorkspaceInput,
    environment: &str,
    name: &str,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    let mut replacement = loaded
        .workspace()
        .environments()
        .iter()
        .find(|candidate| candidate.name == environment)
        .cloned()
        .ok_or_else(|| {
            CliError::configuration(EnvironmentResolutionError::EnvironmentNotFound(
                environment.to_owned(),
            ))
        })?;
    let name = name.trim();
    replacement.name = name.to_owned();
    loaded
        .replace_environment(environment, replacement)
        .map_err(CliError::persistence)?;
    Ok(CommandOutput {
        human: format!("Renamed environment {environment} to {name}\n"),
        json: json!({
            "environment": name,
            "operation": "rename",
            "previousEnvironment": environment,
        }),
    })
}

pub(crate) fn list(
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

pub(crate) fn set_variable(
    input: &WorkspaceInput,
    environment: &str,
    name: &str,
    value: String,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    let output = variable_output("set", "Set", environment, name, Some(&value));
    loaded
        .update_environment_variable(environment, name, value)
        .map_err(CliError::persistence)?;
    Ok(output)
}

pub(crate) fn unset_variable(
    input: &WorkspaceInput,
    environment: &str,
    name: &str,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let mut loaded = load(input, stdin)?;
    loaded
        .unset_environment_variable(environment, name)
        .map_err(CliError::persistence)?;
    Ok(variable_output("unset", "Unset", environment, name, None))
}

fn create_output(name: &str, extends: Option<&str>) -> CommandOutput {
    let mut json = json!({
        "environment": name,
        "operation": "create",
    });
    if let Some(extends) = extends {
        json["extends"] = json!(extends);
    }
    CommandOutput {
        human: format!("Created environment {name}\n"),
        json,
    }
}

fn variable_output(
    operation: &str,
    verb: &str,
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
        json["value"] = json!(value);
    }
    CommandOutput {
        human: format!("{verb} environment variable {environment}.{name}\n"),
        json,
    }
}
