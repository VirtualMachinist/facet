//! `facet session start|end|list|show`: agent sessions in the machine store.
//!
//! A session is the unit a harness turns on and off around a run of work.
//! `session start` prints a bare ULID in human mode so
//! `export FACET_SESSION=$(facet session start)` works without `jq`;
//! `request run` stamps that id on every recorded run (and mints the session
//! row if the harness invented the id itself, see `facet-record`). Sessions
//! are cross-workspace, so they live in the machine store only; the `runs`
//! count is taken from the workspace store discovered from the current
//! directory and omitted when there is none.

use std::env;

use facet_record::{actor_from_env, session_from_env};
use lattice::{
    CONFIG_FILE, HistoryQuery, LatticeConfig, LatticeError, MachineStore, SessionQuery, SessionRow,
    WorkspaceStore, machine_config_dir, now_ms,
};
use serde_json::{Map, Value, json};

use crate::{
    CommandOutput, FacetError, args,
    presentation::format_utc,
    workspace::{ConfigOverrides, locate_root, open_store},
};

const DEFAULT_LIMIT: usize = 50;

/// The `current` alias resolves `FACET_SESSION` wherever an id is accepted.
pub(crate) const CURRENT: &str = "current";

pub(crate) fn session(args: &[String]) -> Result<CommandOutput, FacetError> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(FacetError::invalid_arguments(
            "session requires a subcommand: start, end, list, or show",
        ));
    };
    match verb.as_str() {
        "start" => start(rest),
        "end" => end(rest),
        "list" => list(rest),
        "show" => show(rest),
        other => Err(FacetError::invalid_arguments(format!(
            "unknown session subcommand: {other} (expected start, end, list, or show)"
        ))),
    }
}

/// Resolves `current` to `FACET_SESSION`; any other value is returned as is.
pub(crate) fn resolve_session_id(value: &str) -> Result<String, FacetError> {
    if value == CURRENT {
        session_from_env().ok_or_else(FacetError::session_not_set)
    } else {
        Ok(value.to_owned())
    }
}

fn start(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--actor", "--meta"], &[])?;
    if !parsed.positionals().is_empty() {
        return Err(FacetError::invalid_arguments(
            "session start takes no positional arguments",
        ));
    }
    let actor = parsed
        .value("--actor")?
        .map_or_else(actor_from_env, str::to_owned);
    let meta = build_meta(parsed.value("--meta")?)?;
    let machine = open_machine()?;
    let row = machine
        .start_session(&actor, Some(&meta), now_ms())
        .map_err(FacetError::lattice)?;
    let runs = count_runs(workspace_from_cwd().as_ref(), &row.id)?;
    Ok(CommandOutput::new(
        format!("{}\n", row.id),
        json!({ "session": session_json(&row, runs) }),
    ))
}

fn end(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &[])?;
    let id = match parsed.positionals() {
        [] => session_from_env().ok_or_else(FacetError::session_not_set)?,
        [id] => resolve_session_id(id)?,
        _ => {
            return Err(FacetError::invalid_arguments(
                "session end accepts at most one <id>",
            ));
        }
    };
    let machine = open_machine()?;
    let before = machine
        .session(&id)
        .map_err(FacetError::lattice)?
        .ok_or_else(|| FacetError::session_not_found(&id))?;
    let already_ended = before.ended_at.is_some();
    let row = if already_ended {
        before
    } else {
        machine
            .end_session(&id, now_ms())
            .map_err(FacetError::lattice)?
            .ok_or_else(|| FacetError::session_not_found(&id))?
    };
    let ended_at = row.ended_at.map_or_else(|| "-".to_owned(), format_utc);
    let human = if already_ended {
        format!("session {} already ended at {ended_at}\n", row.id)
    } else {
        format!("session {} ended at {ended_at}\n", row.id)
    };
    let runs = count_runs(workspace_from_cwd().as_ref(), &row.id)?;
    Ok(CommandOutput::new(
        human,
        json!({ "session": session_json(&row, runs), "alreadyEnded": already_ended }),
    ))
}

fn show(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &[])?;
    let [id] = parsed.positionals() else {
        return Err(FacetError::invalid_arguments(
            "session show requires exactly one <id> (or `current`)",
        ));
    };
    let id = resolve_session_id(id)?;
    let machine = open_machine()?;
    let row = machine
        .session(&id)
        .map_err(FacetError::lattice)?
        .ok_or_else(|| FacetError::session_not_found(&id))?;
    let runs = count_runs(workspace_from_cwd().as_ref(), &row.id)?;
    let mut human = format!(
        "ID\t{}\nACTOR\t{}\nSTARTED\t{}\nENDED\t{}\n",
        row.id,
        row.actor,
        format_utc(row.started_at),
        row.ended_at.map_or_else(|| "-".to_owned(), format_utc),
    );
    if let Some(runs) = runs {
        human.push_str(&format!("RUNS\t{runs}\n"));
    }
    if let Some(meta) = &row.meta {
        human.push_str(&format!("META\t{meta}\n"));
    }
    Ok(CommandOutput::new(
        human,
        json!({ "session": session_json(&row, runs) }),
    ))
}

fn list(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--limit", "--actor"], &["--open"])?;
    if !parsed.positionals().is_empty() {
        return Err(FacetError::invalid_arguments(
            "session list takes no positional arguments",
        ));
    }
    let query = SessionQuery {
        limit: parsed
            .parsed_value::<usize>("--limit", "a positive number")?
            .unwrap_or(DEFAULT_LIMIT),
        actor: parsed.value("--actor")?.map(str::to_owned),
        open_only: parsed.switch("--open"),
    };
    if query.limit == 0 {
        return Err(FacetError::invalid_arguments("--limit must be at least 1"));
    }
    let machine = open_machine()?;
    let rows = machine.sessions(&query).map_err(FacetError::lattice)?;
    let workspace = workspace_from_cwd();

    let mut lines = vec!["ID\tACTOR\tSTARTED\tENDED\tRUNS".to_owned()];
    let mut sessions = Vec::with_capacity(rows.len());
    for row in &rows {
        let runs = count_runs(workspace.as_ref(), &row.id)?;
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}",
            row.id,
            row.actor,
            format_utc(row.started_at),
            row.ended_at.map_or_else(|| "-".to_owned(), format_utc),
            runs.map_or_else(|| "-".to_owned(), |runs| runs.to_string()),
        ));
        sessions.push(session_json(row, runs));
    }
    Ok(CommandOutput::new(
        format!("{}\n", lines.join("\n")),
        json!({ "sessions": sessions }),
    ))
}

/// Opens the machine store with the machine-level configuration only.
/// Sessions are cross-workspace, so no workspace file applies.
pub(crate) fn open_machine() -> Result<MachineStore, FacetError> {
    let mut config = LatticeConfig::default();
    if let Some(dir) = machine_config_dir() {
        config
            .apply_file(&dir.join(CONFIG_FILE))
            .map_err(|error| FacetError::lattice(LatticeError::Config(error)))?;
    }
    MachineStore::open(&config).map_err(FacetError::lattice)
}

/// The workspace store discovered from the current directory, if any. Used
/// only to count a session's runs; a missing store just omits the count.
fn workspace_from_cwd() -> Option<WorkspaceStore> {
    let (_, root) = locate_root(None);
    open_store(&root?, &ConfigOverrides::default()).ok()
}

fn count_runs(store: Option<&WorkspaceStore>, session_id: &str) -> Result<Option<i64>, FacetError> {
    let Some(store) = store else {
        return Ok(None);
    };
    let query = HistoryQuery {
        limit: usize::MAX,
        session_id: Some(session_id.to_owned()),
        ..HistoryQuery::default()
    };
    let rows = store.history(&query).map_err(FacetError::lattice)?;
    Ok(Some(i64::try_from(rows.len()).unwrap_or(i64::MAX)))
}

/// Session metadata: Herdr ids from the environment when present, the
/// current directory, then the caller's `--meta` object on top (it wins on
/// key collisions). Pointers only; never transcripts or secrets.
fn build_meta(explicit: Option<&str>) -> Result<String, FacetError> {
    let mut meta = Map::new();
    let mut herdr = Map::new();
    for (variable, key) in [
        ("HERDR_WORKSPACE_ID", "workspace"),
        ("HERDR_TAB_ID", "tab"),
        ("HERDR_PANE_ID", "pane"),
    ] {
        if let Ok(value) = env::var(variable)
            && !value.is_empty()
        {
            herdr.insert(key.to_owned(), json!(value));
        }
    }
    if !herdr.is_empty() {
        meta.insert("herdr".to_owned(), Value::Object(herdr));
    }
    if let Ok(cwd) = env::current_dir() {
        meta.insert("cwd".to_owned(), json!(cwd.to_string_lossy()));
    }
    if let Some(text) = explicit {
        let value: Value = serde_json::from_str(text).map_err(|error| {
            FacetError::invalid_arguments(format!("--meta expects a JSON object: {error}"))
        })?;
        let Value::Object(user) = value else {
            return Err(FacetError::invalid_arguments(
                "--meta expects a JSON object",
            ));
        };
        meta.extend(user);
    }
    Ok(Value::Object(meta).to_string())
}

fn session_json(row: &SessionRow, runs: Option<i64>) -> Value {
    let mut value = json!({
        "id": row.id,
        "actor": row.actor,
        "startedAt": row.started_at,
        "endedAt": row.ended_at,
        "meta": row
            .meta
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or(Value::Null),
    });
    if let Some(runs) = runs {
        value["runs"] = json!(runs);
    }
    value
}
