use std::{borrow::Cow, io::Read, path::PathBuf};

use probe_core::{
    HttpRequest, RequestUpdate, RequestVariableInfo, VariableUsage, discover_request_variables,
    resolve_environment_with_overrides, resolve_request, resolve_request_strict,
};
use probe_http::{ExecutionOptions, HttpEngine, HttpResponse};
use serde_json::json;

use crate::{
    CliError, CommandOutput, WorkspaceInput, load,
    presentation::{
        dry_run_human, dry_run_json, request_human, request_json, response_human, response_json,
    },
};

pub(crate) fn list(
    input: &WorkspaceInput,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let mut lines = vec!["SELECTOR\tTYPE\tMETHOD\tNAME\tURL".to_owned()];
    let mut requests = Vec::with_capacity(loaded.requests().len());
    for located in loaded.requests() {
        let request = loaded
            .workspace()
            .request(located.key())
            .expect("repository request key must resolve");
        let name = request.metadata.name.as_deref().unwrap_or("");
        let request_type = request.protocol.as_str();
        let method = request.method.as_deref().unwrap_or("");
        let url = request.url.as_deref().unwrap_or("");
        lines.push(format!(
            "{}\t{request_type}\t{method}\t{name}\t{url}",
            located.selector()
        ));
        requests.push(json!({
            "method": request.method,
            "name": request.metadata.name,
            "selector": located.selector(),
            "type": request_type,
            "url": request.url,
        }));
    }
    Ok(CommandOutput {
        human: format!("{}\n", lines.join("\n")),
        json: json!({ "requests": requests }),
    })
}

pub(crate) fn get(
    input: &WorkspaceInput,
    selector: &str,
    environment: Option<&str>,
    strict_variables: bool,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let request = selected_request(&loaded, selector, environment, &[], strict_variables)?;
    Ok(CommandOutput {
        human: request_human(selector, environment, &request).map_err(CliError::graphql)?,
        json: request_json(selector, environment, &request).map_err(CliError::graphql)?,
    })
}

pub(crate) fn variables(
    input: &WorkspaceInput,
    selector: &str,
    environment: Option<&str>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| CliError::request_not_found(selector))?;
    let workspace = loaded.workspace();
    let request = workspace
        .request(key)
        .expect("repository request key must resolve");
    let variables = discover_request_variables(request, workspace.environments(), environment)
        .map_err(CliError::configuration)?;
    Ok(variable_output(&variables))
}

fn variable_output(variables: &[RequestVariableInfo]) -> CommandOutput {
    let mut lines = vec!["NAME\tDEFINED\tSECRET\tUSED IN".to_owned()];
    let json_variables = variables
        .iter()
        .map(|variable| {
            let usages = variable
                .usages
                .iter()
                .map(variable_usage_json)
                .collect::<Vec<_>>();
            lines.push(format!(
                "{}\t{}\t{}\t{}",
                variable.name,
                variable.defined,
                variable.secret,
                variable
                    .usages
                    .iter()
                    .map(variable_usage_human)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            json!({
                "name": variable.name,
                "defined": variable.defined,
                "secret": variable.secret,
                "usages": usages,
            })
        })
        .collect::<Vec<_>>();
    CommandOutput {
        human: format!("{}\n", lines.join("\n")),
        json: json!({ "variables": json_variables }),
    }
}

fn variable_usage_human(usage: &VariableUsage) -> String {
    match usage {
        VariableUsage::Method => "method".to_owned(),
        VariableUsage::Url => "url".to_owned(),
        VariableUsage::Header { name } => format!("header: {name}"),
        VariableUsage::QueryParameter { name } => format!("query parameter: {name}"),
        VariableUsage::PathParameter { name } => format!("path parameter: {name}"),
        VariableUsage::Body => "body".to_owned(),
        VariableUsage::GraphqlQuery => "GraphQL query".to_owned(),
        VariableUsage::GraphqlVariables => "GraphQL variables".to_owned(),
        VariableUsage::GraphqlOperationName => "GraphQL operation name".to_owned(),
        VariableUsage::GraphqlExtensions => "GraphQL extensions".to_owned(),
        VariableUsage::FormUrlEncoded { name } => format!("form field: {name}"),
        VariableUsage::Multipart { name } => format!("multipart: {name}"),
        VariableUsage::File => "file".to_owned(),
        VariableUsage::Authentication { name } => format!("authentication: {name}"),
    }
}

fn variable_usage_json(usage: &VariableUsage) -> serde_json::Value {
    match usage {
        VariableUsage::Method => json!({ "location": "method" }),
        VariableUsage::Url => json!({ "location": "url" }),
        VariableUsage::Header { name } => json!({ "location": "header", "name": name }),
        VariableUsage::QueryParameter { name } => {
            json!({ "location": "query_parameter", "name": name })
        }
        VariableUsage::PathParameter { name } => {
            json!({ "location": "path_parameter", "name": name })
        }
        VariableUsage::Body => json!({ "location": "body" }),
        VariableUsage::GraphqlQuery => json!({ "location": "graphql_query" }),
        VariableUsage::GraphqlVariables => json!({ "location": "graphql_variables" }),
        VariableUsage::GraphqlOperationName => json!({ "location": "graphql_operation_name" }),
        VariableUsage::GraphqlExtensions => json!({ "location": "graphql_extensions" }),
        VariableUsage::FormUrlEncoded { name } => {
            json!({ "location": "form_urlencoded", "name": name })
        }
        VariableUsage::Multipart { name } => {
            json!({ "location": "multipart", "name": name })
        }
        VariableUsage::File => json!({ "location": "file" }),
        VariableUsage::Authentication { name } => {
            json!({ "location": "authentication", "name": name })
        }
    }
}

pub(crate) fn update(
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
            request_human(selector, None, request).map_err(CliError::graphql)?
        ),
        json: request_json(selector, None, request).map_err(CliError::graphql)?,
    })
}

pub(crate) fn run(
    input: &WorkspaceInput,
    selector: &str,
    environment: Option<&str>,
    variables: &[(String, String)],
    output: Option<&PathBuf>,
    strict_variables: bool,
    dry_run: bool,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let request = selected_request(&loaded, selector, environment, variables, strict_variables)?;
    let prepared = request.prepare_http().map_err(CliError::graphql)?;
    if dry_run {
        return Ok(CommandOutput {
            human: dry_run_human(&request),
            json: dry_run_json(&request).map_err(CliError::graphql)?,
        });
    }
    let options = ExecutionOptions {
        base_directory: input.base_directory(),
        ..ExecutionOptions::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CliError::runtime(&error))?;
    let response = runtime.block_on(async {
        let engine = HttpEngine::new().map_err(CliError::http)?;
        if let Some(output) = output {
            engine
                .execute_cancellable_to_file(&prepared, &options, output, tokio::signal::ctrl_c())
                .await
                .map_err(CliError::http)
        } else {
            engine
                .execute_cancellable(&prepared, &options, tokio::signal::ctrl_c())
                .await
                .map_err(CliError::http)
        }
    })?;
    response_output(&request, &response, output)
}

fn selected_request<'a>(
    loaded: &'a probe_opencollection::LoadedWorkspace,
    selector: &str,
    environment: Option<&str>,
    variables: &[(String, String)],
    strict_variables: bool,
) -> Result<Cow<'a, HttpRequest>, CliError> {
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| CliError::request_not_found(selector))?;
    if environment.is_some() || !variables.is_empty() || strict_variables {
        let workspace = loaded.workspace();
        let environment =
            resolve_environment_with_overrides(workspace.environments(), environment, variables)
                .map_err(CliError::configuration)?;
        let request = workspace
            .request(key)
            .expect("repository request key must resolve");
        if strict_variables {
            resolve_request_strict(request, &environment)
        } else {
            resolve_request(request, &environment)
        }
        .map(Cow::Owned)
        .map_err(CliError::configuration)
    } else {
        Ok(Cow::Borrowed(
            loaded
                .workspace()
                .request(key)
                .expect("repository request key must resolve"),
        ))
    }
}

fn response_output(
    request: &HttpRequest,
    response: &HttpResponse,
    output: Option<&PathBuf>,
) -> Result<CommandOutput, CliError> {
    let output = output.map(PathBuf::as_path);
    Ok(CommandOutput {
        human: response_human(request, response, output),
        json: response_json(request, response, output).map_err(CliError::graphql)?,
    })
}
