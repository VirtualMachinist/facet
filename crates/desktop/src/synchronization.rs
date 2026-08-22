use std::collections::{BTreeMap, BTreeSet};

use probe_core::HttpRequest;
use probe_opencollection::LoadedWorkspace;

#[derive(Clone, Debug)]
pub(crate) struct LocalRequestState {
    pub(crate) selector: String,
    pub(crate) baseline: HttpRequest,
    pub(crate) local: HttpRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SynchronizationConflict {
    Modified {
        selector: String,
        fields: Vec<&'static str>,
    },
    Deleted {
        selector: String,
    },
    AmbiguousRename {
        selector: String,
    },
}

impl SynchronizationConflict {
    pub(crate) fn description(&self) -> String {
        match self {
            Self::Modified { selector, fields } => {
                format!(
                    "{selector} changed locally and on disk ({})",
                    fields.join(", ")
                )
            }
            Self::Deleted { selector } => {
                format!("{selector} was deleted on disk while it has local changes")
            }
            Self::AmbiguousRename { selector } => {
                format!("the rename of {selector} could not be identified safely")
            }
        }
    }
}

pub(crate) struct ReconciledWorkspace {
    pub(crate) workspace: LoadedWorkspace,
    pub(crate) disk_baselines: BTreeMap<String, HttpRequest>,
    pub(crate) selector_remaps: BTreeMap<String, String>,
}

pub(crate) enum ReconcileResult {
    Applied(Box<ReconciledWorkspace>),
    Conflicted(Vec<SynchronizationConflict>),
}

/// Reconciles a freshly repository-loaded workspace with local editor drafts.
///
/// Filesystem data remains the persistence baseline. Local changes are merged one
/// request field at a time when the disk changed a different field. Any overlap is
/// returned to the desktop for an explicit user decision.
pub(crate) fn reconcile(
    local: Vec<LocalRequestState>,
    mut fresh: LoadedWorkspace,
    rename_hints: &BTreeMap<String, String>,
) -> ReconcileResult {
    let disk_baselines: BTreeMap<_, _> = fresh
        .requests()
        .iter()
        .filter_map(|located| {
            fresh
                .workspace()
                .request(located.key())
                .cloned()
                .map(|request| (located.selector().to_owned(), request))
        })
        .collect();
    let mut claimed = BTreeSet::new();
    let mut selector_remaps = rename_hints.clone();
    let mut conflicts = Vec::new();

    for state in &local {
        let target = find_target_selector(
            state,
            &local,
            &disk_baselines,
            rename_hints,
            &claimed,
            &mut conflicts,
        );
        let Some(target) = target else {
            if state.local != state.baseline {
                conflicts.push(SynchronizationConflict::Deleted {
                    selector: state.selector.clone(),
                });
            }
            continue;
        };
        claimed.insert(target.clone());
        selector_remaps.insert(state.selector.clone(), target.clone());

        let disk = &disk_baselines[&target];
        let (merged, fields) = merge_request(&state.baseline, &state.local, disk);
        if fields.is_empty() {
            let key = fresh
                .request_key(&target)
                .expect("fresh selector must resolve to a request key");
            *fresh
                .request_mut(key)
                .expect("fresh request key must remain valid") = merged;
        } else {
            conflicts.push(SynchronizationConflict::Modified {
                selector: target,
                fields,
            });
        }
    }

    if conflicts.is_empty() {
        ReconcileResult::Applied(Box::new(ReconciledWorkspace {
            workspace: fresh,
            disk_baselines,
            selector_remaps,
        }))
    } else {
        ReconcileResult::Conflicted(conflicts)
    }
}

fn find_target_selector(
    state: &LocalRequestState,
    local: &[LocalRequestState],
    disk: &BTreeMap<String, HttpRequest>,
    rename_hints: &BTreeMap<String, String>,
    claimed: &BTreeSet<String>,
    conflicts: &mut Vec<SynchronizationConflict>,
) -> Option<String> {
    if let Some(target) = hinted_selector(&state.selector, rename_hints)
        && disk.contains_key(&target)
        && !claimed.contains(&target)
    {
        return Some(target);
    }
    if let Some(exact) = disk.get(&state.selector) {
        let belongs_to_another_request = local
            .iter()
            .any(|other| other.selector != state.selector && other.baseline == *exact);
        if !belongs_to_another_request {
            return Some(state.selector.clone());
        }
    }

    let candidates: Vec<_> = disk
        .iter()
        .filter(|(selector, request)| !claimed.contains(*selector) && *request == &state.baseline)
        .map(|(selector, _)| selector.clone())
        .collect();
    match candidates.as_slice() {
        [selector] => Some(selector.clone()),
        [] => None,
        _ if state.local != state.baseline => {
            conflicts.push(SynchronizationConflict::AmbiguousRename {
                selector: state.selector.clone(),
            });
            None
        }
        _ => None,
    }
}

fn hinted_selector(selector: &str, rename_hints: &BTreeMap<String, String>) -> Option<String> {
    rename_hints
        .iter()
        .filter(|(from, _)| {
            selector == from.as_str()
                || selector
                    .strip_prefix(from.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .max_by_key(|(from, _)| from.len())
        .map(|(from, to)| format!("{to}{}", &selector[from.len()..]))
}

fn merge_request(
    baseline: &HttpRequest,
    local: &HttpRequest,
    disk: &HttpRequest,
) -> (HttpRequest, Vec<&'static str>) {
    let mut merged = baseline.clone();
    let mut conflicts = Vec::new();

    merge_field(
        &baseline.metadata.name,
        &local.metadata.name,
        &disk.metadata.name,
        &mut merged.metadata.name,
        "name",
        &mut conflicts,
    );
    merge_field(
        &baseline.metadata.sequence,
        &local.metadata.sequence,
        &disk.metadata.sequence,
        &mut merged.metadata.sequence,
        "sequence",
        &mut conflicts,
    );
    merge_field(
        &baseline.method,
        &local.method,
        &disk.method,
        &mut merged.method,
        "method",
        &mut conflicts,
    );
    merge_field(
        &baseline.url,
        &local.url,
        &disk.url,
        &mut merged.url,
        "URL",
        &mut conflicts,
    );
    merge_field(
        &baseline.headers,
        &local.headers,
        &disk.headers,
        &mut merged.headers,
        "headers",
        &mut conflicts,
    );
    merge_field(
        &baseline.query_parameters,
        &local.query_parameters,
        &disk.query_parameters,
        &mut merged.query_parameters,
        "query parameters",
        &mut conflicts,
    );
    merge_field(
        &baseline.path_parameters,
        &local.path_parameters,
        &disk.path_parameters,
        &mut merged.path_parameters,
        "path parameters",
        &mut conflicts,
    );
    merge_field(
        &baseline.body,
        &local.body,
        &disk.body,
        &mut merged.body,
        "body",
        &mut conflicts,
    );
    merge_field(
        &baseline.authentication,
        &local.authentication,
        &disk.authentication,
        &mut merged.authentication,
        "authentication",
        &mut conflicts,
    );
    merge_field(
        &baseline.settings,
        &local.settings,
        &disk.settings,
        &mut merged.settings,
        "settings",
        &mut conflicts,
    );
    (merged, conflicts)
}

fn merge_field<T: Clone + PartialEq>(
    baseline: &T,
    local: &T,
    disk: &T,
    output: &mut T,
    name: &'static str,
    conflicts: &mut Vec<&'static str>,
) {
    if local == baseline {
        output.clone_from(disk);
    } else if disk == baseline || local == disk {
        output.clone_from(local);
    } else {
        conflicts.push(name);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        LocalRequestState, ReconcileResult, SynchronizationConflict, hinted_selector, reconcile,
    };

    fn fixture_copy() -> PathBuf {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/opencollection/phase1-bundled.yml");
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "probe-sync-{}-{timestamp}-{}.yml",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::copy(source, &path).unwrap();
        path
    }

    fn request_state(
        workspace: &probe_opencollection::LoadedWorkspace,
        index: usize,
    ) -> LocalRequestState {
        let located = &workspace.requests()[index];
        let request = workspace
            .workspace()
            .request(located.key())
            .unwrap()
            .clone();
        LocalRequestState {
            selector: located.selector().to_owned(),
            baseline: request.clone(),
            local: request,
        }
    }

    #[test]
    fn merges_non_overlapping_local_and_disk_changes() {
        let path = fixture_copy();
        let original = probe_opencollection::load_workspace(&path).unwrap();
        let mut state = request_state(&original, 0);
        state.local.url = Some("https://local.example".to_owned());

        let mut source = fs::read_to_string(&path).unwrap();
        source = source.replacen("method: GET", "method: PATCH", 1);
        fs::write(&path, source).unwrap();
        let fresh = probe_opencollection::load_workspace(&path).unwrap();
        let ReconcileResult::Applied(result) = reconcile(vec![state], fresh, &BTreeMap::new())
        else {
            panic!("non-overlapping changes should merge")
        };
        let request = result
            .workspace
            .workspace()
            .request(result.workspace.requests()[0].key())
            .unwrap();
        assert_eq!(request.url.as_deref(), Some("https://local.example"));
        assert_eq!(request.method.as_deref(), Some("PATCH"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn merges_non_overlapping_path_parameter_changes() {
        let path = fixture_copy();
        let original = probe_opencollection::load_workspace(&path).unwrap();
        let mut state = request_state(&original, 0);
        state.local.path_parameters = vec![probe_core::QueryParameter {
            name: "ownerId".to_owned(),
            value: "99".to_owned(),
            disabled: false,
        }];

        let mut source = fs::read_to_string(&path).unwrap();
        source = source.replacen("value: \"25\"", "value: \"50\"", 1);
        fs::write(&path, source).unwrap();
        let fresh = probe_opencollection::load_workspace(&path).unwrap();
        let ReconcileResult::Applied(result) = reconcile(vec![state], fresh, &BTreeMap::new())
        else {
            panic!("non-overlapping path parameter changes should merge")
        };
        let request = result
            .workspace
            .workspace()
            .request(result.workspace.requests()[0].key())
            .unwrap();
        assert_eq!(request.path_parameters[0].value, "99");
        assert_eq!(request.query_parameters[0].value, "50");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_overlapping_field_changes() {
        let path = fixture_copy();
        let original = probe_opencollection::load_workspace(&path).unwrap();
        let mut state = request_state(&original, 0);
        state.local.method = Some("POST".to_owned());
        let mut source = fs::read_to_string(&path).unwrap();
        source = source.replacen("method: GET", "method: PATCH", 1);
        fs::write(&path, source).unwrap();
        let fresh = probe_opencollection::load_workspace(&path).unwrap();

        let ReconcileResult::Conflicted(conflicts) =
            reconcile(vec![state], fresh, &BTreeMap::new())
        else {
            panic!("overlapping changes should conflict")
        };
        assert!(matches!(
            conflicts.as_slice(),
            [SynchronizationConflict::Modified { fields, .. }] if fields == &["method"]
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn remaps_a_dirty_request_through_a_confident_rename_hint() {
        let path = fixture_copy();
        let original = probe_opencollection::load_workspace(&path).unwrap();
        let mut state = request_state(&original, 1);
        state.local.url = Some("https://local.example".to_owned());
        let old = "renamed-request.yml".to_owned();
        state.selector.clone_from(&old);

        // Bundled selectors are structural, so emulate a watcher-provided selector rename
        // against another otherwise-identical request.
        let target = original.requests()[1].selector().to_owned();
        let mut hints = BTreeMap::new();
        hints.insert(old.clone(), target.clone());
        let fresh = probe_opencollection::load_workspace(&path).unwrap();
        let ReconcileResult::Applied(result) = reconcile(vec![state], fresh, &hints) else {
            panic!("a watcher rename pair should be authoritative")
        };
        assert_eq!(result.selector_remaps.get(&old), Some(&target));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn preserves_a_dirty_request_when_it_was_deleted() {
        let path = fixture_copy();
        let original = probe_opencollection::load_workspace(&path).unwrap();
        let mut state = request_state(&original, 0);
        state.baseline.url = Some("https://deleted.example".to_owned());
        state.local = state.baseline.clone();
        state.local.method = Some("POST".to_owned());
        let fresh = probe_opencollection::load_workspace(&path).unwrap();
        state.selector = "missing.yml".to_owned();

        let ReconcileResult::Conflicted(conflicts) =
            reconcile(vec![state], fresh, &BTreeMap::new())
        else {
            panic!("a dirty deletion should conflict")
        };
        assert!(matches!(
            conflicts.as_slice(),
            [SynchronizationConflict::Deleted { .. }]
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn folder_rename_hints_remap_descendant_selectors() {
        let hints = BTreeMap::from([("old-folder".to_owned(), "new-folder".to_owned())]);
        assert_eq!(
            hinted_selector("old-folder/nested/request.yml", &hints).as_deref(),
            Some("new-folder/nested/request.yml")
        );
        assert_eq!(hinted_selector("old-folderish/request.yml", &hints), None);
    }

    #[test]
    fn bundled_deletion_remaps_the_surviving_structural_selector() {
        let original = probe_opencollection::load_workspace_from_str(
            "opencollection: 1.0.0\ninfo: { name: Test }\nbundled: true\nitems:\n  - info: { name: First, type: http }\n    http: { method: GET, url: https://first.example }\n  - info: { name: Second, type: http }\n    http: { method: GET, url: https://second.example }\n",
        )
        .unwrap();
        let local = vec![request_state(&original, 0), request_state(&original, 1)];
        let fresh = probe_opencollection::load_workspace_from_str(
            "opencollection: 1.0.0\ninfo: { name: Test }\nbundled: true\nitems:\n  - info: { name: Second, type: http }\n    http: { method: GET, url: https://second.example }\n",
        )
        .unwrap();

        let ReconcileResult::Applied(result) = reconcile(local, fresh, &BTreeMap::new()) else {
            panic!("deleting a clean request should apply")
        };
        assert_eq!(
            result.selector_remaps.get("items/1").map(String::as_str),
            Some("items/0")
        );
        assert!(!result.selector_remaps.contains_key("items/0"));
        assert_eq!(result.workspace.workspace().request_count(), 1);
    }
}
