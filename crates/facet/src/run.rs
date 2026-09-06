//! `facet request run`: execute through the shared core and HTTP engine
//! exactly as `probe request run` does, then record the run in Lattice.

use std::{borrow::Cow, io::Read, path::PathBuf, time::Instant};

use facet_record::{
    RecordRequest, Recording, actor_from_env, record, recording_disabled, session_from_env,
};
use lattice::now_ms;
use probe_core::{
    HttpRequest, resolve_environment_with_overrides, resolve_request, resolve_request_strict,
};
use probe_http::{ExecutionOptions, HttpEngine};
use probe_opencollection::LoadedWorkspace;
use serde_json::json;

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
    let variables = parsed
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
        .collect::<Result<Vec<_>, _>>()?;
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
    let options = ExecutionOptions {
        base_directory: input.base_directory(),
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
        if let Some(output) = &output {
            engine
                .execute_cancellable_to_file(&request, &options, output, tokio::signal::ctrl_c())
                .await
        } else {
            engine
                .execute_cancellable(&request, &options, tokio::signal::ctrl_c())
                .await
        }
    });
    let elapsed_ms = i64::try_from(clock.elapsed().as_millis()).unwrap_or(i64::MAX);

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
            started_at,
            elapsed_ms,
            result: &result,
            output: output.as_deref(),
            tags: &tags,
            actor: &actor,
            session: session.as_deref(),
        })
    } else {
        Recording::Skipped("disabled")
    };

    match result {
        Ok(response) => {
            let output = output.as_deref();
            let human = format!(
                "{}\nLattice: {}\n",
                response_human(&request, &response, output),
                recording.human()
            );
            let mut json = response_json(&request, &response, output);
            json["lattice"] = recording.json();
            let mut command = CommandOutput::new(human, json);
            if let Some(warning) = recording.warning() {
                command = command.warn(warning);
            }
            Ok(command)
        }
        Err(error) => {
            Err(FacetError::http(&error).with_details(json!({ "lattice": recording.json() })))
        }
    }
}

/// Mirrors `probe-cli`'s selection and interpolation rules.
fn selected_request<'a>(
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
