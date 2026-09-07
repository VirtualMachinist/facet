//! `facet diff <a> <b> [<path>]`: hash-first comparison of two recorded runs.
//! Exit 0 when equal, 1 when different (assertion family), 4 when a run is
//! missing, 9 when there is no store.

use facet_record::{compare, diff_request_bodies, diff_response_bodies};
use lattice::RunRow;
use serde_json::{Value, json};

use crate::{
    ASSERTION_EXIT_CODE, CommandOutput, FacetError, args,
    workspace::{ConfigOverrides, locate_root, open_store},
};

pub(crate) fn diff(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &["--bodies"])?;
    let (id_a, id_b, path) = match parsed.positionals() {
        [a, b] => (a.as_str(), b.as_str(), None),
        [a, b, path] => (a.as_str(), b.as_str(), Some(path.as_str())),
        _ => {
            return Err(FacetError::invalid_arguments(
                "diff requires <idA> <idB> and an optional <path>",
            ));
        }
    };
    let with_bodies = parsed.switch("--bodies");
    let (start, root) = locate_root(path);
    let root = root.ok_or_else(|| FacetError::lattice_not_found(&start))?;
    let store = open_store(&root, &ConfigOverrides::default())?;

    let row_a = store.run(id_a).map_err(FacetError::lattice)?;
    let row_b = store.run(id_b).map_err(FacetError::lattice)?;
    let (Some(a), Some(b)) = (&row_a, &row_b) else {
        let missing: Vec<&str> = [(id_a, row_a.is_none()), (id_b, row_b.is_none())]
            .into_iter()
            .filter(|(_, missing)| *missing)
            .map(|(id, _)| id)
            .collect();
        return Err(FacetError::runs_not_found(&missing));
    };

    let meta = compare(a, b);
    let request_body = diff_request_bodies(a, b);
    let response_body =
        diff_response_bodies(&store, a, b, with_bodies).map_err(FacetError::lattice)?;
    let equal = meta.metadata_equal && request_body.equal && response_body.equal;

    let mut human = String::new();
    if meta.changes.is_empty() {
        human.push_str("no metadata changes\n");
    } else {
        human.push_str("FIELD\tA\tB\n");
        for change in &meta.changes {
            human.push_str(&format!(
                "{}\t{}\t{}\n",
                change.field,
                compact(&change.a),
                compact(&change.b)
            ));
        }
    }
    human.push_str(&format!(
        "request hash: {}\n",
        if meta.request_hash_equal {
            format!("equal ({})", short(&a.request_hash))
        } else {
            format!(
                "differs ({} → {})",
                short(&a.request_hash),
                short(&b.request_hash)
            )
        }
    ));
    human.push_str(&format!(
        "request body: {}\n",
        body_line(
            request_body.equal,
            request_body.a.hash.as_deref(),
            request_body.b.hash.as_deref()
        )
    ));
    human.push_str(&format!(
        "response body: {}\n",
        body_line(
            response_body.equal,
            response_body.a.hash.as_deref(),
            response_body.b.hash.as_deref()
        )
    ));
    if let Some(text) = &response_body.text {
        human.push_str(text);
    }

    let json = json!({
        "workspace": { "id": store.workspace_id(), "path": store.root().to_string_lossy() },
        "a": stamp(a),
        "b": stamp(b),
        "equal": equal,
        "changes": meta.changes.iter().map(|change| json!({
            "field": change.field,
            "a": change.a,
            "b": change.b,
        })).collect::<Vec<_>>(),
        "request": {
            "hash": { "a": a.request_hash, "b": b.request_hash, "equal": meta.request_hash_equal },
            "body": request_body.json(),
        },
        "response": {
            "body": response_body.json(),
        },
    });
    let output = CommandOutput::new(human, json);
    Ok(if equal {
        output
    } else {
        output.with_exit_code(ASSERTION_EXIT_CODE)
    })
}

fn stamp(row: &RunRow) -> Value {
    json!({ "id": row.id, "startedAt": row.started_at, "status": row.status })
}

fn short(hash: &str) -> String {
    format!("{}…", &hash[..12.min(hash.len())])
}

fn body_line(equal: bool, a: Option<&str>, b: Option<&str>) -> String {
    let show = |hash: Option<&str>| hash.map_or_else(|| "none".to_owned(), short);
    if equal {
        format!("equal ({})", show(a))
    } else {
        format!("differs ({} → {})", show(a), show(b))
    }
}

fn compact(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "-".to_owned(),
        other => other.to_string(),
    }
}
