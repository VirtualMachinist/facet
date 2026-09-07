//! `facet request run`: execute through the shared core and HTTP engine
//! exactly as `probe request run` does, then record the run in Lattice.
//! `facet replay` reuses [`execute`] and [`render`] with its own resolution.

use std::{
    borrow::Cow,
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};

use facet_record::{
    RecordRequest, Recording, actor_from_env, record, recording_disabled, session_from_env,
};
use lattice::now_ms;
use probe_core::{
    HttpRequest, resolve_environment_with_overrides, resolve_request, resolve_request_strict,
};
use probe_http::{ExecutionOptions, HttpEngine, HttpError, HttpResponse};
use probe_opencollection::LoadedWorkspace;
use serde_json::{Value, json};

use crate::{
    CommandOutput, FacetError, args,
    presentation::{response_human, response_json},
    workspace::{WorkspaceInput, load, overrides_from_parsed},
};

const VALUE_FLAGS: &[&str] = &[
    "--environment",
    "--output",
    "--var",
    "--tag",
    "--inline-body-max",
    "--history-retention",
];
const SWITCH_FLAGS: &[&str] = &["--strict-variables", "--no-record"];

/// One executed request: when it started, how long it took, and the outcome.
pub(crate) struct Execution {
    pub(crate) started_at: i64,
    pub(crate) elapsed_ms: i64,
    pub(crate) result: Result<HttpResponse, HttpError>,
}

pub(crate) fn run(args: &[String], stdin: &mut impl Read) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, VALUE_FLAGS, SWITCH_FLAGS)?;
    let [path, selector] = parsed.positionals() else {
        return Err(FacetError::invalid_arguments(
            "request run requires <path> and <selector>",
        ));
    };
    let input = WorkspaceInput::from_argument(path);
    let environment = parsed.value("--environment")?.map(str::to_owned);
    let output = parsed.value("--output")?.map(PathBuf::from);
    let strict_variables = parsed.switch("--strict-variables");
    let variables = parse_vars(&parsed)?;
    let tags: Vec<String> = parsed
        .values("--tag")
        .into_iter()
        .map(str::to_owned)
        .collect();
    let should_record = !parsed.switch("--no-record") && !recording_disabled();
    let overrides = overrides_from_parsed(&parsed)?;

    let loaded = load(&input, stdin)?;
    let request = selected_request(
        &loaded,
        selector,
        environment.as_deref(),
        &variables,
        strict_variables,
    )?;
    let execution = execute(&request, input.base_directory(), output.as_deref())?;

    let recording = if should_record {
        let root = input.base_directory();
        let actor = actor_from_env();
        let session = session_from_env();
        record(&RecordRequest {
            root: root.as_deref(),
            overrides: &overrides,
            selector,
            environment: environment.as_deref(),
            request: &request,
            started_at: execution.started_at,
            elapsed_ms: execution.elapsed_ms,
            result: &execution.result,
            output: output.as_deref(),
            tags: &tags,
            actor: &actor,
            session: session.as_deref(),
            replayed_from: None,
            var_names: &var_names(&variables),
        })
    } else {
        Recording::Skipped("disabled")
    };

    render(
        &request,
        execution,
        output.as_deref(),
        recording.json(),
        recording.human(),
        recording.warning().into_iter().collect(),
    )
}

/// Parses repeated `--var NAME=VALUE` flags in order.
pub(crate) fn parse_vars(parsed: &args::Parsed) -> Result<Vec<(String, String)>, FacetError> {
    parsed
        .values("--var")
        .into_iter()
        .map(|entry| {
            entry
                .split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .ok_or_else(|| {
                    FacetError::invalid_arguments(format!(
                        "--var expects NAME=VALUE, got {entry:?}"
                    ))
                })
        })
        .collect()
}

/// The names of the `--var` overrides, in order; values never leave here.
pub(crate) fn var_names(variables: &[(String, String)]) -> Vec<String> {
    variables.iter().map(|(name, _)| name.clone()).collect()
}

/// Executes one resolved request on a fresh single-thread runtime, with
/// Ctrl-C cancellation and optional streaming to `output`.
pub(crate) fn execute(
    request: &HttpRequest,
    base_directory: Option<PathBuf>,
    output: Option<&Path>,
) -> Result<Execution, FacetError> {
    let options = ExecutionOptions {
        base_directory,
        ..ExecutionOptions::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| FacetError::runtime(&error))?;
    let engine = HttpEngine::new().map_err(|error| FacetError::http(&error))?;

    let started_at = now_ms();
    let clock = Instant::now();
    let result = runtime.block_on(async {
        if let Some(output) = output {
            engine
                .execute_cancellable_to_file(request, &options, output, tokio::signal::ctrl_c())
                .await
        } else {
            engine
                .execute_cancellable(request, &options, tokio::signal::ctrl_c())
                .await
        }
    });
    let elapsed_ms = i64::try_from(clock.elapsed().as_millis()).unwrap_or(i64::MAX);
    Ok(Execution {
        started_at,
        elapsed_ms,
        result,
    })
}

/// Renders the upstream response document plus Facet's `lattice` field
/// (`lattice_json` / `lattice_human`), or the upstream error envelope with
/// `error.details.lattice` on a transport failure.
pub(crate) fn render(
    request: &HttpRequest,
    execution: Execution,
    output: Option<&Path>,
    lattice_json: Value,
    lattice_human: String,
    warnings: Vec<String>,
) -> Result<CommandOutput, FacetError> {
    match execution.result {
        Ok(response) => {
            let human = format!(
                "{}\nLattice: {lattice_human}\n",
                response_human(request, &response, output),
            );
            let mut json = response_json(request, &response, output);
            json["lattice"] = lattice_json;
            let mut command = CommandOutput::new(human, json);
            for warning in warnings {
                command = command.warn(warning);
            }
            Ok(command)
        }
        Err(error) => {
            Err(FacetError::http(&error).with_details(json!({ "lattice": lattice_json })))
        }
    }
}

/// Mirrors `probe-cli`'s selection and interpolation rules.
pub(crate) fn selected_request<'a>(
    loaded: &'a LoadedWorkspace,
    selector: &str,
    environment: Option<&str>,
    variables: &[(String, String)],
    strict_variables: bool,
) -> Result<Cow<'a, HttpRequest>, FacetError> {
    let key = loaded
        .request_key(selector)
        .ok_or_else(|| FacetError::request_not_found(selector))?;
    let workspace = loaded.workspace();
    let request = workspace
        .request(key)
        .expect("repository request key must resolve");
    if environment.is_some() || !variables.is_empty() || strict_variables {
        let environment =
            resolve_environment_with_overrides(workspace.environments(), environment, variables)
                .map_err(FacetError::configuration)?;
        if strict_variables {
            resolve_request_strict(request, &environment)
        } else {
            resolve_request(request, &environment)
        }
        .map(Cow::Owned)
        .map_err(FacetError::configuration)
    } else {
        Ok(Cow::Borrowed(request))
    }
}
