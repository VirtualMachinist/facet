use std::{borrow::Cow, io::Read, path::PathBuf};

use probe_core::{
    EnvSecretProvider, ExpectationOutcome, HttpRequest, RequestUpdate, RequestVariableInfo,
    ResolvedEnvironment, SecretProvider, StatusExpectation, VariableUsage,
    discover_request_variables, evaluate_expectations, resolve_environment_with_secret_provider,
    resolve_request, resolve_request_redacted, resolve_request_redacted_strict,
    resolve_request_strict,
};
use probe_http::{ExecutionOptions, HttpEngine, HttpResponse};
use serde_json::json;

use crate::{
    CliError, CommandOutput, WorkspaceInput,
    error::expectation_json,
    load,
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

pub(crate) struct RunOptions<'a> {
    pub(crate) environment: Option<&'a str>,
    pub(crate) variables: &'a [(String, String)],
    pub(crate) output: Option<&'a PathBuf>,
    pub(crate) strict_variables: bool,
    pub(crate) dry_run: bool,
    pub(crate) expectations: &'a [StatusExpectation],
    pub(crate) secret_provider: Option<EnvSecretProvider>,
}

pub(crate) fn run(
    input: &WorkspaceInput,
    selector: &str,
    options: &RunOptions<'_>,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let secret_provider = options
        .secret_provider
        .as_ref()
        .map(|provider| provider as &dyn SecretProvider);
    let resolved_environment = resolve_selected_environment(
        &loaded,
        options.environment,
        options.variables,
        options.strict_variables,
        secret_provider,
    )?;
    let request = interpolate_selected(
        &loaded,
        selector,
        resolved_environment.as_ref(),
        options.strict_variables,
        false,
    )?;
    let display = interpolate_selected(
        &loaded,
        selector,
        resolved_environment.as_ref(),
        options.strict_variables,
        true,
    )?;
    let prepared = request.prepare_http().map_err(CliError::graphql)?;
    if options.dry_run {
        return Ok(CommandOutput {
            human: dry_run_human(&display),
            json: dry_run_json(&display).map_err(CliError::graphql)?,
        });
    }
    let execution = ExecutionOptions {
        base_directory: input.base_directory(),
        ..ExecutionOptions::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| CliError::runtime(&error))?;
    let response = runtime.block_on(async {
        let engine = HttpEngine::new().map_err(CliError::http)?;
        if let Some(output) = options.output {
            engine
                .execute_cancellable_to_file(&prepared, &execution, output, tokio::signal::ctrl_c())
                .await
                .map_err(CliError::http)
        } else {
            engine
                .execute_cancellable(&prepared, &execution, tokio::signal::ctrl_c())
                .await
                .map_err(CliError::http)
        }
    })?;
    let outcomes = evaluate_expectations(options.expectations, response.status);
    if outcomes.iter().any(|outcome| !outcome.ok) {
        return Err(CliError::expectation_failed(&outcomes));
    }
    response_output(&display, &response, options.output, &outcomes)
}

fn selected_request<'a>(
    loaded: &'a probe_opencollection::LoadedWorkspace,
    selector: &str,
    environment: Option<&str>,
    variables: &[(String, String)],
    strict_variables: bool,
) -> Result<Cow<'a, HttpRequest>, CliError> {
    let resolved =
        resolve_selected_environment(loaded, environment, variables, strict_variables, None)?;
    interpolate_selected(loaded, selector, resolved.as_ref(), strict_variables, false)
}

fn resolve_selected_environment(
    loaded: &probe_opencollection::LoadedWorkspace,
    environment: Option<&str>,
    variables: &[(String, String)],
    strict_variables: bool,
    secret_provider: Option<&dyn SecretProvider>,
) -> Result<Option<ResolvedEnvironment>, CliError> {
    if environment.is_none()
        && variables.is_empty()
        && !strict_variables
        && secret_provider.is_none()
    {
        return Ok(None);
    }
    resolve_environment_with_secret_provider(
        loaded.workspace().environments(),
        environment,
        variables,
        secret_provider,
    )
    .map(Some)
    .map_err(CliError::configuration)
}

fn interpolate_selected<'a>(
    loaded: &'a probe_opencollection::LoadedWorkspace,
    selector: &str,
    environment: Option<&ResolvedEnvironment>,
    strict_variables: bool,
    redact_secrets: bool,
) -> Result<Cow<'a, HttpRequest>, CliError> {
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| CliError::request_not_found(selector))?;
    let request = loaded
        .workspace()
        .request(key)
        .expect("repository request key must resolve");
    let Some(environment) = environment else {
        return Ok(Cow::Borrowed(request));
    };
    let resolved = match (strict_variables, redact_secrets) {
        (false, false) => resolve_request(request, environment),
        (true, false) => resolve_request_strict(request, environment),
        (false, true) => resolve_request_redacted(request, environment),
        (true, true) => resolve_request_redacted_strict(request, environment),
    };
    resolved.map(Cow::Owned).map_err(CliError::configuration)
}

fn response_output(
    request: &HttpRequest,
    response: &HttpResponse,
    output: Option<&PathBuf>,
    outcomes: &[ExpectationOutcome],
) -> Result<CommandOutput, CliError> {
    let output = output.map(PathBuf::as_path);
    let mut json = response_json(request, response, output).map_err(CliError::graphql)?;
    if !outcomes.is_empty() {
        json.as_object_mut()
            .expect("request run JSON output must be an object")
            .insert(
                "expectations".to_owned(),
                json!(outcomes.iter().map(expectation_json).collect::<Vec<_>>()),
            );
    }
    Ok(CommandOutput {
        human: response_human(request, response, output),
        json,
    })
}
