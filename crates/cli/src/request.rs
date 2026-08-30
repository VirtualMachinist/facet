use std::{borrow::Cow, io::Read, path::PathBuf};

use probe_core::{HttpRequest, RequestUpdate, resolve_environment_with_overrides, resolve_request};
use probe_http::{ExecutionOptions, HttpEngine, HttpResponse};
use serde_json::json;

use crate::{
    CliError, CommandOutput, WorkspaceInput, load,
    presentation::{request_human, request_json, response_human, response_json},
};

pub(crate) fn list(
    input: &WorkspaceInput,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
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

pub(crate) fn get(
    input: &WorkspaceInput,
    selector: &str,
    environment: Option<&str>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let request = selected_request(&loaded, selector, environment, &[])?;
    Ok(CommandOutput {
        human: request_human(selector, environment, &request),
        json: request_json(selector, environment, &request),
    })
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
            request_human(selector, None, request)
        ),
        json: request_json(selector, None, request),
    })
}

pub(crate) fn run(
    input: &WorkspaceInput,
    selector: &str,
    environment: Option<&str>,
    variables: &[(String, String)],
    output: Option<&PathBuf>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let request = selected_request(&loaded, selector, environment, variables)?;
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

fn selected_request<'a>(
    loaded: &'a probe_opencollection::LoadedWorkspace,
    selector: &str,
    environment: Option<&str>,
    variables: &[(String, String)],
) -> Result<Cow<'a, HttpRequest>, CliError> {
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| CliError::request_not_found(selector))?;
    if environment.is_some() || !variables.is_empty() {
        let workspace = loaded.workspace();
        let environment =
            resolve_environment_with_overrides(workspace.environments(), environment, variables)
                .map_err(CliError::configuration)?;
        let request = workspace
            .request(key)
            .expect("repository request key must resolve");
        resolve_request(request, &environment)
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
) -> CommandOutput {
    let output = output.map(PathBuf::as_path);
    CommandOutput {
        human: response_human(request, response, output),
        json: response_json(request, response, output),
    }
}
