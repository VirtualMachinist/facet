//! `facet pin`: name a run. The first real write to the machine store's
//! `preferences` table, so an agent can say "replay the known-good auth"
//! next week: `facet replay $(facet pin get auth-ok)`.
//!
//! Keys are `pin.<name>` in the **machine** store, so a pin survives `gc`
//! of the workspace. It may then dangle; `pin get` says so (`dangling`).

use lattice::{is_ulid, now_ms};
use serde_json::{Value, json};

use crate::{
    CommandOutput, FacetError, args,
    presentation::format_utc,
    session::open_machine,
    workspace::{ConfigOverrides, locate_root, open_store},
};

const KEY_PREFIX: &str = "pin.";

pub(crate) fn pin(args: &[String]) -> Result<CommandOutput, FacetError> {
    match args.first().map(String::as_str) {
        Some("get") => get(&args[1..]),
        Some("list") => list(&args[1..]),
        Some("delete") => delete(&args[1..]),
        Some(_) => set(args),
        None => Err(FacetError::invalid_arguments(
            "pin requires <runId> --as <name>, or a subcommand: get, list, delete",
        )),
    }
}

fn set(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--as"], &[])?;
    let (run_id, path) = match parsed.positionals() {
        [run_id] => (run_id.as_str(), None),
        [run_id, path] => (run_id.as_str(), Some(path.as_str())),
        _ => {
            return Err(FacetError::invalid_arguments(
                "pin requires <runId> and an optional <path>",
            ));
        }
    };
    if !is_ulid(run_id) {
        return Err(FacetError::invalid_arguments(format!(
            "pin expects a run ULID (or get, list, delete), got {run_id:?}"
        )));
    }
    let name = parsed
        .value("--as")?
        .ok_or_else(|| FacetError::invalid_arguments("--as <name> is required"))?;
    validate_name(name)?;

    // The run must exist where we can see it; the pin remembers the workspace.
    let (_, root) = locate_root(path);
    let root = root.ok_or_else(|| FacetError::run_not_found(run_id))?;
    let store = open_store(&root, &ConfigOverrides::default())?;
    let row = store
        .run(run_id)
        .map_err(FacetError::lattice)?
        .ok_or_else(|| FacetError::run_not_found(run_id))?;
    let machine = open_machine()?;
    let pinned_at = now_ms();
    let value = json!({
        "runId": row.id,
        "workspaceId": store.workspace_id(),
        "pinnedAt": pinned_at,
    });
    machine
        .set_preference(&key(name), &value.to_string())
        .map_err(FacetError::lattice)?;
    Ok(CommandOutput::new(
        format!("pin {name} → {}\n", row.id),
        json!({ "pin": pin_json(name, &value, Some(false)) }),
    ))
}

fn get(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &[])?;
    let (name, path) = match parsed.positionals() {
        [name] => (name.as_str(), None),
        [name, path] => (name.as_str(), Some(path.as_str())),
        _ => {
            return Err(FacetError::invalid_arguments(
                "pin get requires <name> and an optional <path>",
            ));
        }
    };
    let machine = open_machine()?;
    let value = machine
        .preference(&key(name))
        .map_err(FacetError::lattice)?
        .ok_or_else(|| FacetError::pin_not_found(name))?;
    let value = parse_value(&value)?;
    let dangling = dangling(&value, path)?;
    let run_id = value["runId"].as_str().unwrap_or("").to_owned();
    Ok(CommandOutput::new(
        format!("{run_id}\n"),
        json!({ "pin": pin_json(name, &value, dangling) }),
    ))
}

fn list(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &[])?;
    let path = match parsed.positionals() {
        [] => None,
        [path] => Some(path.as_str()),
        _ => {
            return Err(FacetError::invalid_arguments(
                "pin list accepts at most one <path>",
            ));
        }
    };
    let machine = open_machine()?;
    let rows = machine
        .preferences(KEY_PREFIX)
        .map_err(FacetError::lattice)?;
    let mut lines = vec!["NAME\tRUN\tWORKSPACE\tPINNED\tDANGLING".to_owned()];
    let mut pins = Vec::with_capacity(rows.len());
    for (full_key, raw) in &rows {
        let name = &full_key[KEY_PREFIX.len()..];
        let value = parse_value(raw)?;
        let dangling = dangling(&value, path)?;
        lines.push(format!(
            "{name}\t{}\t{}\t{}\t{}",
            value["runId"].as_str().unwrap_or("-"),
            value["workspaceId"].as_str().unwrap_or("-"),
            value["pinnedAt"]
                .as_i64()
                .map_or_else(|| "-".to_owned(), format_utc),
            dangling.map_or("?", |flag| if flag { "yes" } else { "no" }),
        ));
        pins.push(pin_json(name, &value, dangling));
    }
    Ok(CommandOutput::new(
        format!("{}\n", lines.join("\n")),
        json!({ "pins": pins }),
    ))
}

fn delete(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &[])?;
    let [name] = parsed.positionals() else {
        return Err(FacetError::invalid_arguments(
            "pin delete requires exactly one <name>",
        ));
    };
    let machine = open_machine()?;
    let deleted = machine
        .delete_preference(&key(name))
        .map_err(FacetError::lattice)?;
    Ok(CommandOutput::new(
        if deleted {
            format!("pin {name} deleted\n")
        } else {
            format!("pin {name} was not set\n")
        },
        json!({ "name": name, "deleted": deleted }),
    ))
}

fn key(name: &str) -> String {
    format!("{KEY_PREFIX}{name}")
}

fn validate_name(name: &str) -> Result<(), FacetError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(FacetError::invalid_arguments(format!(
            "pin names are 1-64 characters of letters, digits, '-', '_' or '.', got {name:?}"
        )))
    }
}

fn parse_value(raw: &str) -> Result<Value, FacetError> {
    serde_json::from_str(raw)
        .map_err(|error| FacetError::lattice_corrupt(format!("malformed pin value: {error}")))
}

/// `Some(true)` when the pinned run is gone from the workspace store
/// discovered from `path`, `Some(false)` when it is there, `None` when that
/// workspace is not the pin's (cannot tell from here).
fn dangling(value: &Value, path: Option<&str>) -> Result<Option<bool>, FacetError> {
    let (Some(run_id), Some(workspace_id)) =
        (value["runId"].as_str(), value["workspaceId"].as_str())
    else {
        return Ok(Some(true));
    };
    let (_, root) = locate_root(path);
    let Some(root) = root else {
        return Ok(None);
    };
    let store = open_store(&root, &ConfigOverrides::default())?;
    if store.workspace_id() != workspace_id {
        return Ok(None);
    }
    Ok(Some(
        store.run(run_id).map_err(FacetError::lattice)?.is_none(),
    ))
}

fn pin_json(name: &str, value: &Value, dangling: Option<bool>) -> Value {
    json!({
        "name": name,
        "runId": value["runId"],
        "workspaceId": value["workspaceId"],
        "pinnedAt": value["pinnedAt"],
        "dangling": dangling,
    })
}
