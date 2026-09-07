//! Hash-first comparison of two recorded runs (`facet diff`, TUI `:diff`).
//!
//! [`compare`] is pure: it looks only at the two rows. Body equality needs
//! bytes when a body is inline (no hash on the row), so [`diff_request_bodies`]
//! and [`diff_response_bodies`] take the store and settle it. Headers compare
//! as sorted `name: value` lists; they are already redacted, so a token change
//! shows as no change, and `requestHash` will not move either since auth
//! enters it by scheme only.

use lattice::{BodyRef, LatticeError, RunRow, WorkspaceStore, sha256_hex};
use probe_http::MAX_IN_MEMORY_RESPONSE_BYTES;
use serde_json::{Value, json};

/// Lines per side above which a unified diff is not attempted.
const MAX_DIFF_LINES: usize = 2000;

/// One differing scalar or header field. `field` uses the `history` JSON
/// path names so an agent can map back without a legend.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    /// `history` JSON path, e.g. `status` or `response.headers`.
    pub field: &'static str,
    /// Value on run A.
    pub a: Value,
    /// Value on run B.
    pub b: Value,
}

/// Fields that are always compared and reported but never flip `equal`:
/// they describe provenance, not bytes or outcome.
pub const PROVENANCE_FIELDS: &[&str] = &["durationMs", "actor", "tags"];

/// Metadata comparison of two runs.
#[derive(Clone, Debug, PartialEq)]
pub struct RunDiff {
    /// Every differing field, provenance included.
    pub changes: Vec<FieldChange>,
    /// True when nothing but [`PROVENANCE_FIELDS`] differs. Body equality
    /// for inline bodies is settled separately with bytes.
    pub metadata_equal: bool,
    /// `requestHash` equal.
    pub request_hash_equal: bool,
}

/// Compares the recorded metadata of two runs. Pure; no I/O.
#[must_use]
pub fn compare(a: &RunRow, b: &RunRow) -> RunDiff {
    let mut changes = Vec::new();
    let mut push = |field: &'static str, left: Value, right: Value| {
        if left != right {
            changes.push(FieldChange {
                field,
                a: left,
                b: right,
            });
        }
    };
    push("method", json!(a.method), json!(b.method));
    push("url", json!(a.url), json!(b.url));
    push("status", json!(a.status), json!(b.status));
    push("error", json!(a.error), json!(b.error));
    push("durationMs", json!(a.duration_ms), json!(b.duration_ms));
    push("environment", json!(a.environment), json!(b.environment));
    push("actor", json!(a.actor), json!(b.actor));
    push("requestHash", json!(a.request_hash), json!(b.request_hash));
    push(
        "request.headers",
        header_lines(a.req_headers.as_deref()),
        header_lines(b.req_headers.as_deref()),
    );
    push(
        "response.headers",
        header_lines(a.res_headers.as_deref()),
        header_lines(b.res_headers.as_deref()),
    );
    push(
        "request.body.hash",
        json!(a.req_body.hash),
        json!(b.req_body.hash),
    );
    push(
        "response.body.hash",
        json!(a.res_body.hash),
        json!(b.res_body.hash),
    );
    push(
        "response.contentType",
        json!(a.res_content_type),
        json!(b.res_content_type),
    );
    push(
        "tags",
        tags_value(a.tags.as_deref()),
        tags_value(b.tags.as_deref()),
    );
    let metadata_equal = changes
        .iter()
        .all(|change| PROVENANCE_FIELDS.contains(&change.field));
    RunDiff {
        changes,
        metadata_equal,
        request_hash_equal: a.request_hash == b.request_hash,
    }
}

/// One side of a body comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodySide {
    /// SHA-256 of the body: the blob hash, or computed from inline bytes.
    pub hash: Option<String>,
    /// Body length in bytes.
    pub size_bytes: Option<u64>,
    /// `blob`, `inline`, or `none`.
    pub retention: &'static str,
}

/// Outcome of comparing one body across two runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyDiff {
    /// Run A's body.
    pub a: BodySide,
    /// Run B's body.
    pub b: BodySide,
    /// Whether the bytes are equal (hashes first; inline bodies by bytes).
    pub equal: bool,
    /// Unified diff when requested, the bodies differ, and both are inline
    /// UTF-8 text.
    pub text: Option<String>,
    /// Why `text` is absent: `blob`, `binary`, `too_large`, or `none`.
    pub omission_reason: Option<&'static str>,
}

impl BodyDiff {
    /// JSON shape used by `facet diff`.
    #[must_use]
    pub fn json(&self) -> Value {
        json!({
            "a": side_json(&self.a),
            "b": side_json(&self.b),
            "equal": self.equal,
            "text": self.text,
            "omissionReason": self.omission_reason,
        })
    }
}

fn side_json(side: &BodySide) -> Value {
    json!({
        "hash": side.hash,
        "sizeBytes": side.size_bytes,
        "retention": side.retention,
    })
}

/// Request bodies are hash-only (v2), so this never reads bytes and never
/// produces text; pull a body with `facet blob` when one is needed.
#[must_use]
pub fn diff_request_bodies(a: &RunRow, b: &RunRow) -> BodyDiff {
    let left = side_from_ref(&a.req_body, None);
    let right = side_from_ref(&b.req_body, None);
    let equal = left.hash == right.hash;
    let omission_reason = if equal {
        None
    } else if left.retention == "none" && right.retention == "none" {
        Some("none")
    } else {
        Some("blob")
    };
    BodyDiff {
        a: left,
        b: right,
        equal,
        text: None,
        omission_reason,
    }
}

/// Compares response bodies. Blob bodies compare by hash without reading.
/// Inline bodies are read (they are at most `inline_body_max`) and hashed;
/// with `with_text`, differing inline UTF-8 bodies yield a unified diff.
pub fn diff_response_bodies(
    store: &WorkspaceStore,
    a: &RunRow,
    b: &RunRow,
    with_text: bool,
) -> Result<BodyDiff, LatticeError> {
    let bytes_a = inline_bytes(store, a)?;
    let bytes_b = inline_bytes(store, b)?;
    let left = side_from_ref(&a.res_body, bytes_a.as_deref());
    let right = side_from_ref(&b.res_body, bytes_b.as_deref());
    let equal = left.hash == right.hash;
    let mut diff = BodyDiff {
        a: left,
        b: right,
        equal,
        text: None,
        omission_reason: None,
    };
    if equal || !with_text {
        return Ok(diff);
    }
    diff.omission_reason = match (&bytes_a, &bytes_b) {
        (Some(bytes_a), Some(bytes_b)) => {
            if bytes_a.len() > MAX_IN_MEMORY_RESPONSE_BYTES
                || bytes_b.len() > MAX_IN_MEMORY_RESPONSE_BYTES
            {
                Some("too_large")
            } else {
                match (std::str::from_utf8(bytes_a), std::str::from_utf8(bytes_b)) {
                    (Ok(text_a), Ok(text_b)) => match unified_diff(text_a, text_b, "a", "b") {
                        Some(text) => {
                            diff.text = Some(text);
                            None
                        }
                        None => Some("too_large"),
                    },
                    _ => Some("binary"),
                }
            }
        }
        _ if a.res_body.hash.is_some() || b.res_body.hash.is_some() => Some("blob"),
        _ => Some("none"),
    };
    Ok(diff)
}

fn inline_bytes(store: &WorkspaceStore, run: &RunRow) -> Result<Option<Vec<u8>>, LatticeError> {
    if run.res_body.hash.is_some() || !run.res_body.inline_present {
        return Ok(None);
    }
    store.response_body(run)
}

fn side_from_ref(body: &BodyRef, inline: Option<&[u8]>) -> BodySide {
    let hash = body.hash.clone().or_else(|| inline.map(sha256_hex));
    BodySide {
        hash,
        size_bytes: body.len.or_else(|| inline.map(|bytes| bytes.len() as u64)),
        retention: body.retention(),
    }
}

fn header_lines(source: Option<&str>) -> Value {
    let Some(Value::Array(items)) = source.and_then(|text| serde_json::from_str(text).ok()) else {
        return Value::Null;
    };
    let mut lines: Vec<String> = items
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let value = item.get("value")?.as_str()?;
            Some(format!("{}: {value}", name.to_ascii_lowercase()))
        })
        .collect();
    lines.sort();
    json!(lines)
}

fn tags_value(source: Option<&str>) -> Value {
    let mut tags: Vec<String> = source
        .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
        .unwrap_or_default();
    tags.sort();
    json!(tags)
}

/// Line-based unified diff with three lines of context, computed with an
/// in-crate LCS (no dependency). Returns `None` when either side exceeds
/// [`MAX_DIFF_LINES`].
#[must_use]
pub fn unified_diff(a: &str, b: &str, label_a: &str, label_b: &str) -> Option<String> {
    let lines_a: Vec<&str> = a.lines().collect();
    let lines_b: Vec<&str> = b.lines().collect();
    if lines_a.len() > MAX_DIFF_LINES || lines_b.len() > MAX_DIFF_LINES {
        return None;
    }
    let ops = edit_script(&lines_a, &lines_b);
    let mut out = format!("--- {label_a}\n+++ {label_b}\n");
    let mut index = 0;
    while index < ops.len() {
        if ops[index].0 == Op::Equal {
            index += 1;
            continue;
        }
        // Hunk: from `start` (3 lines of context before) to the last change
        // whose following equal run is shorter than six lines.
        let start = index.saturating_sub(3);
        let mut end = index;
        loop {
            while end < ops.len() && ops[end].0 != Op::Equal {
                end += 1;
            }
            let mut run = end;
            while run < ops.len() && ops[run].0 == Op::Equal {
                run += 1;
            }
            if run < ops.len() && run - end <= 6 {
                end = run;
                continue;
            }
            end = (end + 3).min(ops.len());
            break;
        }
        let (mut a_count, mut b_count) = (0, 0);
        let mut a_start = None;
        let mut b_start = None;
        let mut body = String::new();
        for (op, a_line, b_line) in &ops[start..end] {
            match op {
                Op::Equal => {
                    a_start.get_or_insert(*a_line);
                    b_start.get_or_insert(*b_line);
                    a_count += 1;
                    b_count += 1;
                    body.push(' ');
                    body.push_str(lines_a[*a_line]);
                }
                Op::Delete => {
                    a_start.get_or_insert(*a_line);
                    b_start.get_or_insert(*b_line);
                    a_count += 1;
                    body.push('-');
                    body.push_str(lines_a[*a_line]);
                }
                Op::Insert => {
                    a_start.get_or_insert(*a_line);
                    b_start.get_or_insert(*b_line);
                    b_count += 1;
                    body.push('+');
                    body.push_str(lines_b[*b_line]);
                }
            }
            body.push('\n');
        }
        out.push_str(&format!(
            "@@ -{},{a_count} +{},{b_count} @@\n",
            a_start.unwrap_or(0) + 1,
            b_start.unwrap_or(0) + 1
        ));
        out.push_str(&body);
        index = end;
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Equal,
    Delete,
    Insert,
}

/// Edit script as `(op, index into a, index into b)`; for `Equal` and
/// `Delete` the a-index is the line, for `Insert` the b-index is the line.
fn edit_script(a: &[&str], b: &[&str]) -> Vec<(Op, usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let width = m + 1;
    let mut table = vec![0_u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if a[i] == b[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push((Op::Equal, i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            ops.push((Op::Delete, i, j));
            i += 1;
        } else {
            ops.push((Op::Insert, i, j));
            j += 1;
        }
    }
    while i < n {
        ops.push((Op::Delete, i, j));
        i += 1;
    }
    while j < m {
        ops.push((Op::Insert, i, j));
        j += 1;
    }
    ops
}

#[cfg(test)]
mod tests {
    use lattice::{BodyRef, RunRow};

    use super::{compare, diff_request_bodies, unified_diff};

    fn row(status: Option<i64>, res_headers: &str) -> RunRow {
        RunRow {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_owned(),
            started_at: 1,
            duration_ms: Some(10),
            request_path: "items/0".to_owned(),
            request_hash: "9f".to_owned(),
            environment: Some("local".to_owned()),
            method: "GET".to_owned(),
            url: "http://x/".to_owned(),
            status,
            error: None,
            req_headers: None,
            res_headers: Some(res_headers.to_owned()),
            req_body: BodyRef::default(),
            res_body: BodyRef::default(),
            res_content_type: None,
            session_id: None,
            actor: "human".to_owned(),
            tags: Some(r#"["b","a"]"#.to_owned()),
            replayed_from: None,
            var_names: None,
        }
    }

    #[test]
    fn provenance_never_flips_equal_and_headers_sort() {
        let a = row(
            Some(200),
            r#"[{"name":"B","value":"2"},{"name":"a","value":"1"}]"#,
        );
        let mut b = row(
            Some(200),
            r#"[{"name":"a","value":"1"},{"name":"b","value":"2"}]"#,
        );
        b.duration_ms = Some(99);
        b.tags = Some(r#"["a","b"]"#.to_owned());
        let diff = compare(&a, &b);
        assert!(diff.metadata_equal);
        assert_eq!(diff.changes.len(), 1);
        assert_eq!(diff.changes[0].field, "durationMs");

        b.tags = Some(r#"["a","b","retry"]"#.to_owned());
        b.actor = "halo-qa".to_owned();
        let diff = compare(&a, &b);
        assert!(diff.metadata_equal, "tags and actor are provenance");
        assert_eq!(diff.changes.len(), 3);

        b.status = Some(500);
        let diff = compare(&a, &b);
        assert!(!diff.metadata_equal);
        assert!(diff.changes.iter().any(|change| change.field == "status"));
    }

    #[test]
    fn request_bodies_compare_by_hash_only() {
        let mut a = row(Some(200), "[]");
        let mut b = row(Some(200), "[]");
        assert!(diff_request_bodies(&a, &b).equal);
        a.req_body.hash = Some("aa".to_owned());
        b.req_body.hash = Some("bb".to_owned());
        let diff = diff_request_bodies(&a, &b);
        assert!(!diff.equal);
        assert_eq!(diff.omission_reason, Some("blob"));
        assert!(diff.text.is_none());
    }

    #[test]
    fn unified_diff_marks_changed_lines_with_context() {
        let a = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
        let b = "one\ntwo\nthree\nFOUR\nfive\nsix\nseven\n";
        let text = unified_diff(a, b, "a", "b").unwrap();
        assert!(
            text.starts_with("--- a\n+++ b\n@@ -1,7 +1,7 @@\n"),
            "{text}"
        );
        assert!(text.contains(" three\n-four\n+FOUR\n five\n"), "{text}");
        assert!(
            unified_diff("x", "x", "a", "b")
                .unwrap()
                .ends_with("+++ b\n")
        );
    }
}
