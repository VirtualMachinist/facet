//! `facet env set|list|delete`: Facet's machine-store environment values,
//! keyed by workspace. This is **not** Probe's `environment` command (which
//! edits YAML and rejects secrets); the two stores are different and the
//! names do not collide. Values are written here and read by the hydration
//! overlay at resolve time. No command ever prints a value.

use lattice::{LatticeConfig, LatticeError, MachineStore, SecretConfig};
use serde_json::{Value, json};

use crate::{
    CommandOutput, FacetError, args,
    presentation::format_utc,
    workspace::{ConfigOverrides, WorkspaceInput, locate_root, open_store},
};

pub(crate) fn env(args: &[String]) -> Result<CommandOutput, FacetError> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(FacetError::invalid_arguments(
            "env requires a subcommand: set, list, or delete",
        ));
    };
    match verb.as_str() {
        "set" => set(rest),
        "list" => list(rest),
        "delete" => delete(rest),
        other => Err(FacetError::invalid_arguments(format!(
            "unknown env subcommand: {other} (expected set, list, or delete)"
        ))),
    }
}

fn set(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--environment", "--name", "--value"], &["--secret"])?;
    let path = single_optional_path(parsed.positionals(), "env set")?;
    let environment = required(&parsed, "--environment")?;
    let name = required(&parsed, "--name")?;
    let value = required(&parsed, "--value")?;
    let secret = parsed.switch("--secret");

    // `set` may be the first Lattice write for a collection: create the
    // workspace store beside it when none exists yet.
    let (start, root) = locate_root(path);
    let root = match root {
        Some(root) => root,
        None => WorkspaceInput::from_argument(&start.to_string_lossy())
            .base_directory()
            .ok_or_else(|| FacetError::invalid_arguments("env set needs a collection on disk"))?,
    };
    let store = open_store(&root, &ConfigOverrides::default())?;
    let machine = open_machine(&store.config().clone())?;
    if secret {
        // Fail before writing when the backend cannot hold a secret.
        SecretConfig::from_env()
            .map_err(|error| FacetError::lattice(LatticeError::Secret(error)))?;
    }
    machine
        .set_environment(store.workspace_id(), environment, name, value, secret)
        .map_err(FacetError::lattice)?;
    let row = machine
        .environments(store.workspace_id())
        .map_err(FacetError::lattice)?
        .into_iter()
        .find(|row| row.name == environment && row.key == name)
        .unwrap_or_else(|| lattice::EnvironmentRow {
            name: environment.to_owned(),
            key: name.to_owned(),
            secret,
            updated_at: lattice::now_ms(),
        });
    Ok(CommandOutput::new(
        format!(
            "{}/{} set ({}) for workspace {}\n",
            row.name,
            row.key,
            if row.secret { "secret" } else { "plain" },
            store.workspace_id()
        ),
        json!({
            "workspace": workspace_json(&store),
            "entry": entry_json(&row),
        }),
    ))
}

fn list(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--environment"], &[])?;
    let path = single_optional_path(parsed.positionals(), "env list")?;
    let filter = parsed.value("--environment")?;
    let (_, root) = locate_root(path);
    let Some(root) = root else {
        return Ok(CommandOutput::new(
            "No Lattice store found; nothing set.\n",
            json!({ "workspace": Value::Null, "entries": [] }),
        ));
    };
    let store = open_store(&root, &ConfigOverrides::default())?;
    let machine = open_machine(&store.config().clone())?;
    let rows: Vec<_> = machine
        .environments(store.workspace_id())
        .map_err(FacetError::lattice)?
        .into_iter()
        .filter(|row| filter.is_none_or(|name| row.name == name))
        .collect();
    let mut lines = vec!["ENVIRONMENT\tNAME\tSECRET\tUPDATED".to_owned()];
    for row in &rows {
        lines.push(format!(
            "{}\t{}\t{}\t{}",
            row.name,
            row.key,
            if row.secret { "yes" } else { "no" },
            format_utc(row.updated_at)
        ));
    }
    Ok(CommandOutput::new(
        format!("{}\n", lines.join("\n")),
        json!({
            "workspace": workspace_json(&store),
            "entries": rows.iter().map(entry_json).collect::<Vec<_>>(),
        }),
    ))
}

fn delete(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--environment", "--name"], &[])?;
    let path = single_optional_path(parsed.positionals(), "env delete")?;
    let environment = required(&parsed, "--environment")?;
    let name = required(&parsed, "--name")?;
    let (start, root) = locate_root(path);
    let root = root.ok_or_else(|| FacetError::lattice_not_found(&start))?;
    let store = open_store(&root, &ConfigOverrides::default())?;
    let machine = open_machine(&store.config().clone())?;
    let deleted = machine
        .delete_environment(store.workspace_id(), environment, name)
        .map_err(FacetError::lattice)?;
    Ok(CommandOutput::new(
        if deleted {
            format!("{environment}/{name} deleted\n")
        } else {
            format!("{environment}/{name} was not set\n")
        },
        json!({
            "workspace": workspace_json(&store),
            "environment": environment,
            "name": name,
            "deleted": deleted,
        }),
    ))
}

fn open_machine(config: &LatticeConfig) -> Result<MachineStore, FacetError> {
    MachineStore::open(config).map_err(FacetError::lattice)
}

fn required<'a>(parsed: &'a args::Parsed, flag: &str) -> Result<&'a str, FacetError> {
    parsed
        .value(flag)?
        .ok_or_else(|| FacetError::invalid_arguments(format!("{flag} is required")))
}

fn single_optional_path<'a>(
    positionals: &'a [String],
    command: &str,
) -> Result<Option<&'a str>, FacetError> {
    match positionals {
        [] => Ok(None),
        [path] => Ok(Some(path.as_str())),
        _ => Err(FacetError::invalid_arguments(format!(
            "{command} accepts at most one <path>"
        ))),
    }
}

fn workspace_json(store: &lattice::WorkspaceStore) -> Value {
    json!({
        "id": store.workspace_id(),
        "path": store.root().to_string_lossy(),
    })
}

/// Metadata only. There is no field for the value and there never will be.
fn entry_json(row: &lattice::EnvironmentRow) -> Value {
    json!({
        "environment": row.name,
        "name": row.key,
        "secret": row.secret,
        "updatedAt": row.updated_at,
    })
}
