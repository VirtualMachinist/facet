//! `facet history`, `facet blob`, and `facet gc`: the agent-facing reads and
//! the one maintenance command over the workspace Lattice store.
//! Metadata is cheap and returned by default; payloads are explicit pulls.

use std::{fs, path::PathBuf};

use lattice::{HistoryQuery, RunRow, SqlValue, WorkspaceStore};
use probe_http::MAX_IN_MEMORY_RESPONSE_BYTES;
use serde_json::{Value, json};

use crate::{
    CommandOutput, FacetError, args,
    presentation::{format_utc, human_size, stored_body_json},
    workspace::{ConfigOverrides, locate_root, open_store, overrides_from_parsed},
};

const DEFAULT_LIMIT: usize = 50;

const HISTORY_VALUE_FLAGS: &[&str] = &[
    "--limit",
    "--request",
    "--status",
    "--actor",
    "--since",
    "--sql",
];
const HISTORY_SWITCH_FLAGS: &[&str] = &["--bodies"];

pub(crate) fn history(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, HISTORY_VALUE_FLAGS, HISTORY_SWITCH_FLAGS)?;
    let path = single_optional_path(parsed.positionals(), "history")?;
    let (start, root) = locate_root(path);

    if let Some(sql) = parsed.value("--sql")? {
        let filters = ["--limit", "--request", "--status", "--actor", "--since"];
        if filters
            .iter()
            .any(|flag| parsed.value(flag).ok().flatten().is_some())
            || parsed.switch("--bodies")
        {
            return Err(FacetError::invalid_arguments(
                "--sql cannot be combined with history filters or --bodies",
            ));
        }
        let root = root.ok_or_else(|| FacetError::lattice_not_found(&start))?;
        let store = open_store(&root, &ConfigOverrides::default())?;
        let result = store.query(sql).map_err(FacetError::lattice)?;
        return Ok(sql_output(&store, result));
    }

    let query = HistoryQuery {
        limit: parsed
            .parsed_value::<usize>("--limit", "a positive number")?
            .unwrap_or(DEFAULT_LIMIT),
        request_path: parsed.value("--request")?.map(str::to_owned),
        status: parsed.parsed_value::<i64>("--status", "an HTTP status code")?,
        actor: parsed.value("--actor")?.map(str::to_owned),
        since: parsed.parsed_value::<i64>("--since", "Unix milliseconds")?,
    };
    if query.limit == 0 {
        return Err(FacetError::invalid_arguments("--limit must be at least 1"));
    }
    let include_bodies = parsed.switch("--bodies");

    let Some(root) = root else {
        return Ok(CommandOutput::new(
            "No Lattice store found; run a request with facet first.\n",
            json!({ "workspace": Value::Null, "runs": [] }),
        ));
    };
    let store = open_store(&root, &ConfigOverrides::default())?;
    let rows = store.history(&query).map_err(FacetError::lattice)?;

    let mut lines = vec!["ID\tSTARTED\tSTATUS\tMS\tMETHOD\tREQUEST\tSIZE\tACTOR".to_owned()];
    let mut runs = Vec::with_capacity(rows.len());
    for row in &rows {
        lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.id,
            format_utc(row.started_at),
            row.status
                .map_or_else(|| "ERR".to_owned(), |status| status.to_string()),
            row.duration_ms
                .map_or_else(|| "-".to_owned(), |ms| ms.to_string()),
            row.method,
            row.request_path,
            row.res_body
                .len
                .map_or_else(|| "-".to_owned(), |len| human_size(len as usize)),
            row.actor,
        ));
        runs.push(run_json(&store, row, include_bodies)?);
    }
    Ok(CommandOutput::new(
        format!("{}\n", lines.join("\n")),
        json!({ "workspace": workspace_json(&store), "runs": runs }),
    ))
}

pub(crate) fn blob(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--output"], &[])?;
    let (hash, path) = match parsed.positionals() {
        [hash] => (hash.as_str(), None),
        [hash, path] => (hash.as_str(), Some(path.as_str())),
        _ => {
            return Err(FacetError::invalid_arguments(
                "blob requires <hash> and an optional <path>",
            ));
        }
    };
    let output = parsed.value("--output")?.map(PathBuf::from);
    let (_, root) = locate_root(path);
    let root = root.ok_or_else(|| FacetError::blob_not_found(hash))?;
    let store = open_store(&root, &ConfigOverrides::default())?;
    let blob = store
        .blob(hash)
        .map_err(FacetError::lattice)?
        .ok_or_else(|| FacetError::blob_not_found(hash))?;

    let (human, body) = if let Some(output) = &output {
        fs::write(output, &blob.bytes).map_err(|error| FacetError::output(output, &error))?;
        let mut body = stored_body_json(None, None);
        body["omitted"] = json!(false);
        body["omissionReason"] = Value::Null;
        body["outputPath"] = json!(output.to_string_lossy());
        (
            format!("Blob {} written to {}\n", blob.hash, output.display()).into_bytes(),
            body,
        )
    } else {
        let mut body = if blob.bytes.len() > MAX_IN_MEMORY_RESPONSE_BYTES {
            stored_body_json(None, Some("too_large"))
        } else {
            stored_body_json(Some(&blob.bytes), None)
        };
        body["outputPath"] = Value::Null;
        (blob.bytes.clone(), body)
    };
    Ok(CommandOutput::new(
        human,
        json!({
            "blob": {
                "hash": blob.hash,
                "sizeBytes": blob.len,
                "contentType": blob.content_type,
                "body": body,
            }
        }),
    ))
}

pub(crate) fn gc(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--history-retention"], &["--yes"])?;
    let path = single_optional_path(parsed.positionals(), "gc")?;
    let overrides = overrides_from_parsed(&parsed)?;
    let apply = parsed.switch("--yes");
    let (_, root) = locate_root(path);

    let Some(root) = root else {
        return Ok(CommandOutput::new(
            "No Lattice store found; nothing to collect.\n",
            json!({
                "workspace": Value::Null,
                "applied": false,
                "retention": "unlimited",
                "runsExpired": 0,
                "orphans": [],
                "registryOrphans": 0,
                "bytesReclaimable": 0,
            }),
        ));
    };
    let store = open_store(&root, &overrides)?;
    let report = store.gc(apply).map_err(FacetError::lattice)?;

    let mut human = format!(
        "Retention: {}\nExpired runs: {}\nOrphaned blobs: {} ({})\n",
        report.retention,
        report.runs_expired,
        report.orphans.len(),
        human_size(report.bytes_reclaimable as usize),
    );
    for (hash, len) in &report.orphans {
        human.push_str(&format!("  {hash}\t{}\n", human_size(*len as usize)));
    }
    if report.applied {
        human.push_str("Deleted.\n");
    } else {
        human.push_str("Dry run; nothing deleted. Re-run with --yes to delete.\n");
    }
    Ok(CommandOutput::new(
        human,
        json!({
            "workspace": workspace_json(&store),
            "applied": report.applied,
            "retention": report.retention.to_string(),
            "runsExpired": report.runs_expired,
            "orphans": report.orphans.iter().map(|(hash, len)| json!({
                "hash": hash,
                "sizeBytes": len,
            })).collect::<Vec<_>>(),
            "registryOrphans": report.registry_orphans,
            "bytesReclaimable": report.bytes_reclaimable,
        }),
    ))
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

fn workspace_json(store: &WorkspaceStore) -> Value {
    json!({
        "id": store.workspace_id(),
        "path": store.root().to_string_lossy(),
    })
}

fn run_json(
    store: &WorkspaceStore,
    row: &RunRow,
    include_bodies: bool,
) -> Result<Value, FacetError> {
    let request_body = body_json(row, &row.req_body, include_bodies, store, true)?;
    let response_body = body_json(row, &row.res_body, include_bodies, store, false)?;
    Ok(json!({
        "id": row.id,
        "startedAt": row.started_at,
        "durationMs": row.duration_ms,
        "requestPath": row.request_path,
        "requestHash": row.request_hash,
        "environment": row.environment,
        "method": row.method,
        "url": row.url,
        "status": row.status,
        "error": row.error,
        "actor": row.actor,
        "sessionId": row.session_id,
        "tags": parse_json_or(row.tags.as_deref(), json!([])),
        "request": {
            "headers": parse_json_or(row.req_headers.as_deref(), Value::Null),
            "body": request_body,
        },
        "response": {
            "headers": parse_json_or(row.res_headers.as_deref(), Value::Null),
            "contentType": row.res_content_type,
            "body": response_body,
        },
    }))
}

fn body_json(
    row: &RunRow,
    body: &lattice::BodyRef,
    include_bodies: bool,
    store: &WorkspaceStore,
    is_request: bool,
) -> Result<Value, FacetError> {
    let mut value = json!({
        "sizeBytes": body.len,
        "hash": body.hash,
        "retention": body.retention(),
    });
    if include_bodies {
        let content = match body.retention() {
            "blob" => stored_body_json(None, Some("blob")),
            "inline" => {
                let bytes = if is_request {
                    store.request_body(row)
                } else {
                    store.response_body(row)
                }
                .map_err(FacetError::lattice)?;
                stored_body_json(bytes.as_deref(), None)
            }
            _ => stored_body_json(None, None),
        };
        for (key, item) in content.as_object().expect("body JSON is an object") {
            value[key] = item.clone();
        }
    }
    Ok(value)
}

fn parse_json_or(source: Option<&str>, fallback: Value) -> Value {
    source
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(fallback)
}

fn sql_output(store: &WorkspaceStore, result: lattice::SqlResult) -> CommandOutput {
    let mut lines = vec![result.columns.join("\t")];
    let mut rows = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        lines.push(
            row.iter()
                .map(sql_value_human)
                .collect::<Vec<_>>()
                .join("\t"),
        );
        rows.push(Value::Array(row.iter().map(sql_value_json).collect()));
    }
    CommandOutput::new(
        format!("{}\n", lines.join("\n")),
        json!({
            "workspace": workspace_json(store),
            "columns": result.columns,
            "rows": rows,
        }),
    )
}

fn sql_value_json(value: &SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(number) => json!(number),
        SqlValue::Real(number) => json!(number),
        SqlValue::Text(text) => json!(text),
        SqlValue::Blob(bytes) => json!({ "type": "blob", "sizeBytes": bytes.len() }),
    }
}

fn sql_value_human(value: &SqlValue) -> String {
    match value {
        SqlValue::Null => "NULL".to_owned(),
        SqlValue::Integer(number) => number.to_string(),
        SqlValue::Real(number) => number.to_string(),
        SqlValue::Text(text) => text.clone(),
        SqlValue::Blob(bytes) => format!("<blob {} bytes>", bytes.len()),
    }
}
