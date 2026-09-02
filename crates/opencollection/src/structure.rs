use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use serde_yaml_ng::{Mapping, Value};

use crate::repository::{
    LoadedWorkspace, SaveError, SaveLock, WorkspaceSource, atomic_write, load_workspace,
    relative_selector,
};

mod bundled;
mod errors;
mod filesystem;
mod unbundled;

use bundled::*;
pub use errors::StructureError;
use filesystem::*;
use unbundled::*;

/// The kind of collection item affected by a structural operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemKind {
    /// An HTTP request.
    Request,
    /// A folder.
    Folder,
}

impl ItemKind {
    /// Returns the stable lowercase representation used by CLI JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Folder => "folder",
        }
    }
}

/// A repository-owned structural workspace operation.
#[derive(Clone, Debug, PartialEq)]
pub enum StructureOperation {
    /// Creates an HTTP request.
    CreateRequest {
        /// Destination folder selector, or `None` for the root.
        parent: Option<String>,
        /// Insertion index, or `None` to append.
        index: Option<usize>,
        /// Request name.
        name: String,
        /// Initial method.
        method: Option<String>,
        /// Initial URL.
        url: Option<String>,
    },
    /// Creates an empty folder.
    CreateFolder {
        /// Destination folder selector, or `None` for the root.
        parent: Option<String>,
        /// Insertion index, or `None` to append.
        index: Option<usize>,
        /// Folder name.
        name: String,
    },
    /// Renames a request, including its path in an unbundled workspace.
    RenameRequest { selector: String, name: String },
    /// Renames a folder, including its path in an unbundled workspace.
    RenameFolder { selector: String, name: String },
    /// Deletes a request.
    DeleteRequest { selector: String },
    /// Duplicates a request after the original sibling.
    DuplicateRequest { selector: String },
    /// Deletes a folder and its descendants.
    DeleteFolder { selector: String },
    /// Moves or reorders a request.
    MoveRequest {
        selector: String,
        parent: Option<String>,
        index: Option<usize>,
    },
    /// Moves or reorders a folder.
    MoveFolder {
        selector: String,
        parent: Option<String>,
        index: Option<usize>,
    },
    /// Reorders a request within its current parent.
    ReorderRequest { selector: String, index: usize },
    /// Reorders a folder within its current parent.
    ReorderFolder { selector: String, index: usize },
}

impl StructureOperation {
    fn selected_item(&self) -> Option<(&str, ItemKind)> {
        match self {
            Self::DuplicateRequest { selector } => Some((selector, ItemKind::Request)),
            _ => self.source(),
        }
    }

    fn source(&self) -> Option<(&str, ItemKind)> {
        match self {
            Self::RenameRequest { selector, .. }
            | Self::DeleteRequest { selector }
            | Self::MoveRequest { selector, .. }
            | Self::ReorderRequest { selector, .. } => Some((selector, ItemKind::Request)),
            Self::RenameFolder { selector, .. }
            | Self::DeleteFolder { selector }
            | Self::MoveFolder { selector, .. }
            | Self::ReorderFolder { selector, .. } => Some((selector, ItemKind::Folder)),
            Self::CreateRequest { .. }
            | Self::CreateFolder { .. }
            | Self::DuplicateRequest { .. } => None,
        }
    }

    fn destination_parent(&self) -> Option<&str> {
        match self {
            Self::CreateRequest { parent, .. }
            | Self::CreateFolder { parent, .. }
            | Self::MoveRequest { parent, .. }
            | Self::MoveFolder { parent, .. } => parent.as_deref(),
            _ => None,
        }
    }

    fn removes_source(&self) -> bool {
        matches!(
            self,
            Self::DeleteRequest { .. }
                | Self::DeleteFolder { .. }
                | Self::MoveRequest { .. }
                | Self::MoveFolder { .. }
                | Self::ReorderRequest { .. }
                | Self::ReorderFolder { .. }
        )
    }

    fn inserts_item(&self) -> bool {
        matches!(
            self,
            Self::CreateRequest { .. }
                | Self::CreateFolder { .. }
                | Self::DuplicateRequest { .. }
                | Self::MoveRequest { .. }
                | Self::MoveFolder { .. }
                | Self::ReorderRequest { .. }
                | Self::ReorderFolder { .. }
        )
    }
}

/// The persisted location resulting from a structural operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureResult {
    /// Affected item kind.
    pub kind: ItemKind,
    /// Selector before the operation, when an existing item was changed.
    pub previous_selector: Option<String>,
    /// Selector after the operation, or `None` after deletion.
    pub selector: Option<String>,
    /// Parent folder selector after the operation.
    pub parent: Option<String>,
    /// Zero-based persisted position after the operation.
    pub index: Option<usize>,
    /// Complete old-to-new selector mapping for every surviving known item.
    pub selector_remaps: BTreeMap<String, String>,
}

impl LoadedWorkspace {
    /// Applies a structural edit and refreshes all runtime keys and repository selectors.
    pub fn apply_structure(
        &mut self,
        operation: StructureOperation,
    ) -> Result<StructureResult, StructureError> {
        validate_operation_selectors(self, &operation)?;
        let source = self.source.clone();
        let request_snapshots = self
            .requests()
            .iter()
            .map(|located| {
                (
                    located.selector().to_owned(),
                    self.workspace()
                        .request(located.key())
                        .expect("repository request key must resolve")
                        .clone(),
                )
            })
            .collect::<Vec<_>>();
        let old_items = self
            .requests()
            .iter()
            .map(|located| (located.selector().to_owned(), ItemKind::Request))
            .chain(
                self.folders()
                    .iter()
                    .map(|located| (located.selector().to_owned(), ItemKind::Folder)),
            )
            .collect::<Vec<_>>();
        let mut result = match &source {
            WorkspaceSource::Bundled(path) => self.apply_bundled(path, operation.clone())?,
            WorkspaceSource::Unbundled(root) => self.apply_unbundled(root, operation.clone())?,
            WorkspaceSource::Memory => return Err(StructureError::ReadOnlySource),
        };
        result.selector_remaps = build_selector_remaps(&source, &operation, &result, &old_items)?;
        let path = match source {
            WorkspaceSource::Bundled(path) | WorkspaceSource::Unbundled(path) => path,
            WorkspaceSource::Memory => unreachable!(),
        };
        let mut fresh = reload_committed_workspace(path, &result, |path| {
            load_workspace(path).map_err(|error| error.to_string())
        })?;
        for (old_selector, mut request) in request_snapshots {
            let Some(new_selector) = result.selector_remaps.get(&old_selector) else {
                continue;
            };
            if let StructureOperation::RenameRequest { selector, name } = &operation
                && selector == &old_selector
            {
                request.metadata.name = Some(name.clone());
            }
            if let Some(key) = fresh.request_key(new_selector) {
                let fresh_request = fresh
                    .request_mut(key)
                    .expect("fresh repository request key must resolve");
                request.metadata.sequence = fresh_request.metadata.sequence;
                *fresh_request = request;
            }
        }
        *self = fresh;
        Ok(result)
    }

    fn apply_bundled(
        &self,
        path: &Path,
        operation: StructureOperation,
    ) -> Result<StructureResult, StructureError> {
        let baseline = self
            .documents
            .get(path)
            .ok_or_else(|| StructureError::InvalidDocument("missing bundled baseline".to_owned()))?
            .original_source
            .clone();
        let _lock = SaveLock::acquire(path)?;
        verify_source(path, &baseline)?;
        let mut document: Value = serde_yaml_ng::from_slice(&baseline)
            .map_err(|error| StructureError::InvalidDocument(error.to_string()))?;
        let result = mutate_bundled(&mut document, operation)?;
        let serialized = serde_yaml_ng::to_string(&document)
            .map_err(|error| StructureError::InvalidDocument(error.to_string()))?;
        atomic_write(path, serialized.as_bytes(), &baseline)?;
        Ok(result)
    }

    fn apply_unbundled(
        &self,
        root: &Path,
        operation: StructureOperation,
    ) -> Result<StructureResult, StructureError> {
        let root_config = root.join("opencollection.yml");
        let lock_path = if root_config.exists() {
            root_config
        } else {
            root.join("opencollection.yaml")
        };
        let _lock = SaveLock::acquire(&lock_path)?;
        let expected_paths: BTreeSet<_> = self.documents.keys().cloned().collect();
        let current_paths = discover_documents(root)?;
        if expected_paths != current_paths {
            let changed = expected_paths
                .symmetric_difference(&current_paths)
                .next()
                .expect("different sets have a symmetric difference")
                .clone();
            return Err(StructureError::ConcurrentModification(changed));
        }
        for (path, document) in &self.documents {
            verify_source(path, &document.original_source)?;
        }
        mutate_unbundled(root, operation)
    }
}

fn reload_committed_workspace(
    path: PathBuf,
    result: &StructureResult,
    loader: impl FnOnce(PathBuf) -> Result<LoadedWorkspace, String>,
) -> Result<LoadedWorkspace, StructureError> {
    loader(path).map_err(|message| StructureError::CommittedRefreshFailed {
        result: Box::new(result.clone()),
        message,
    })
}

fn validate_operation_selectors(
    workspace: &LoadedWorkspace,
    operation: &StructureOperation,
) -> Result<(), StructureError> {
    if let Some((selector, kind)) = operation.selected_item() {
        let exists = match kind {
            ItemKind::Request => workspace.request_key(selector).is_some(),
            ItemKind::Folder => workspace.folder_key(selector).is_some(),
        };
        if !exists {
            return Err(StructureError::ItemNotFound {
                kind,
                selector: selector.to_owned(),
            });
        }
    }
    if let Some(parent) = operation.destination_parent()
        && workspace.folder_key(parent).is_none()
    {
        return Err(StructureError::DestinationNotFound(parent.to_owned()));
    }
    Ok(())
}

fn build_selector_remaps(
    source: &WorkspaceSource,
    operation: &StructureOperation,
    result: &StructureResult,
    old_items: &[(String, ItemKind)],
) -> Result<BTreeMap<String, String>, StructureError> {
    match source {
        WorkspaceSource::Bundled(_) => bundled_selector_remaps(operation, result, old_items),
        WorkspaceSource::Unbundled(_) => {
            Ok(unbundled_selector_remaps(operation, result, old_items))
        }
        WorkspaceSource::Memory => Ok(BTreeMap::new()),
    }
}

fn unbundled_selector_remaps(
    operation: &StructureOperation,
    result: &StructureResult,
    old_items: &[(String, ItemKind)],
) -> BTreeMap<String, String> {
    let source = operation.source();
    let deleted = matches!(
        operation,
        StructureOperation::DeleteRequest { .. } | StructureOperation::DeleteFolder { .. }
    );
    old_items
        .iter()
        .filter_map(|(selector, _)| {
            let Some((source_selector, source_kind)) = source else {
                return Some((selector.clone(), selector.clone()));
            };
            let affected = selector == source_selector
                || (source_kind == ItemKind::Folder
                    && selector.starts_with(&(source_selector.to_owned() + "/")));
            if !affected {
                return Some((selector.clone(), selector.clone()));
            }
            if deleted {
                return None;
            }
            let target = result.selector.as_deref()?;
            Some((
                selector.clone(),
                format!("{target}{}", &selector[source_selector.len()..]),
            ))
        })
        .collect()
}

fn bundled_selector_remaps(
    operation: &StructureOperation,
    result: &StructureResult,
    old_items: &[(String, ItemKind)],
) -> Result<BTreeMap<String, String>, StructureError> {
    let source_path = operation
        .source()
        .map(|(selector, _)| parse_selector(selector))
        .transpose()?;
    let removes_source = operation.removes_source();
    let inserts_item = operation.inserts_item();
    let insertion_path = if inserts_item {
        result.selector.as_deref().map(parse_selector).transpose()?
    } else {
        None
    };
    let moved_source = removes_source && inserts_item;
    let mut remaps = BTreeMap::new();
    for (selector, _) in old_items {
        let mut path = parse_selector(selector)?;
        if removes_source
            && let Some(source_path) = &source_path
            && path.starts_with(source_path)
        {
            if !moved_source {
                continue;
            }
            let target = insertion_path
                .as_ref()
                .expect("move result must contain a selector");
            let mut moved = target.clone();
            moved.extend_from_slice(&path[source_path.len()..]);
            remaps.insert(selector.clone(), selector_from_full_path(&moved));
            continue;
        }
        if removes_source {
            shift_after_removal(
                &mut path,
                source_path
                    .as_ref()
                    .expect("removing operation must have a source"),
            );
        }
        if let Some(insertion_path) = &insertion_path {
            shift_after_insertion(&mut path, insertion_path);
        }
        remaps.insert(selector.clone(), selector_from_full_path(&path));
    }
    Ok(remaps)
}

fn shift_after_removal(path: &mut [usize], removed: &[usize]) {
    let parent_len = removed.len() - 1;
    if path.len() > parent_len
        && path[..parent_len] == removed[..parent_len]
        && path[parent_len] > removed[parent_len]
    {
        path[parent_len] -= 1;
    }
}

fn shift_after_insertion(path: &mut [usize], inserted: &[usize]) {
    let parent_len = inserted.len() - 1;
    if path.len() > parent_len
        && path[..parent_len] == inserted[..parent_len]
        && path[parent_len] >= inserted[parent_len]
    {
        path[parent_len] += 1;
    }
}

fn selector_from_full_path(path: &[usize]) -> String {
    selector_for(&path[..path.len() - 1], path[path.len() - 1])
}

fn request_value(name: &str, method: Option<String>, url: Option<String>) -> Value {
    let mut http = Mapping::new();
    if let Some(method) = method {
        http.insert(Value::String("method".to_owned()), Value::String(method));
    }
    if let Some(url) = url {
        http.insert(Value::String("url".to_owned()), Value::String(url));
    }
    item_value(name, "http", Some(Value::Mapping(http)))
}

fn folder_value(name: &str) -> Value {
    let mut value = item_value(name, "folder", None);
    value
        .as_mapping_mut()
        .expect("new folder is a mapping")
        .insert(
            Value::String("items".to_owned()),
            Value::Sequence(Vec::new()),
        );
    value
}

fn item_value(name: &str, kind: &str, details: Option<Value>) -> Value {
    let mut info = Mapping::new();
    info.insert(
        Value::String("name".to_owned()),
        Value::String(name.to_owned()),
    );
    info.insert(
        Value::String("type".to_owned()),
        Value::String(kind.to_owned()),
    );
    let mut item = Mapping::new();
    item.insert(Value::String("info".to_owned()), Value::Mapping(info));
    if let Some(details) = details {
        item.insert(Value::String(kind.to_owned()), details);
    }
    Value::Mapping(item)
}

fn ensure_kind(value: &Value, expected: ItemKind, selector: &str) -> Result<(), StructureError> {
    let actual = value
        .get("info")
        .and_then(|info| info.get("type"))
        .and_then(Value::as_str);
    let matches = matches!(
        (expected, actual),
        (ItemKind::Request, Some("http")) | (ItemKind::Folder, Some("folder"))
    );
    if matches {
        Ok(())
    } else {
        Err(StructureError::ItemNotFound {
            kind: expected,
            selector: selector.to_owned(),
        })
    }
}

fn item<'a>(document: &'a Value, path: &[usize]) -> Option<&'a Value> {
    let mut current = document;
    for index in path {
        current = current.get("items")?.as_sequence()?.get(*index)?;
    }
    Some(current)
}

fn item_mut<'a>(document: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    let mut current = document;
    for index in path {
        current = current
            .as_mapping_mut()?
            .get_mut(Value::String("items".to_owned()))?
            .as_sequence_mut()?
            .get_mut(*index)?;
    }
    Some(current)
}

fn items_mut<'a>(
    document: &'a mut Value,
    parent_path: &[usize],
) -> Result<&'a mut Vec<Value>, StructureError> {
    let parent = item_mut(document, parent_path)
        .ok_or_else(|| StructureError::InvalidDocument("item parent does not exist".to_owned()))?;
    let mapping = parent.as_mapping_mut().ok_or_else(|| {
        StructureError::InvalidDocument("item parent is not a mapping".to_owned())
    })?;
    let key = Value::String("items".to_owned());
    if !mapping.contains_key(&key) {
        mapping.insert(key.clone(), Value::Sequence(Vec::new()));
    }
    mapping
        .get_mut(&key)
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| StructureError::InvalidDocument("items is not a sequence".to_owned()))
}

fn parse_selector(selector: &str) -> Result<Vec<usize>, StructureError> {
    let parts: Vec<_> = selector.split('/').collect();
    if parts.len() < 2 || parts.len() % 2 != 0 {
        return Err(StructureError::InvalidDestination(format!(
            "invalid bundled selector: {selector}"
        )));
    }
    let mut path = Vec::with_capacity(parts.len() / 2);
    for pair in parts.as_chunks::<2>().0 {
        if pair[0] != "items" {
            return Err(StructureError::InvalidDestination(format!(
                "invalid bundled selector: {selector}"
            )));
        }
        path.push(pair[1].parse::<usize>().map_err(|_| {
            StructureError::InvalidDestination(format!("invalid bundled selector: {selector}"))
        })?);
    }
    Ok(path)
}

fn selector_for(parent_path: &[usize], index: usize) -> String {
    let mut selector = String::new();
    for ancestor in parent_path {
        selector.push_str(&format!("items/{ancestor}/"));
    }
    selector.push_str(&format!("items/{index}"));
    selector
}

fn selector_from_path(path: &[usize]) -> Option<String> {
    path.last()
        .map(|index| selector_for(&path[..path.len() - 1], *index))
}

fn selector_parent(selector: &str) -> Result<(Option<String>, usize), StructureError> {
    let mut path = parse_selector(selector)?;
    let index = path.pop().expect("parsed selector is nonempty");
    let parent =
        (!path.is_empty()).then(|| selector_for(&path[..path.len() - 1], path[path.len() - 1]));
    Ok((parent, index))
}

fn set_info_field(value: &mut Value, name: &str, field: Value) -> Result<(), StructureError> {
    let mapping = value.as_mapping_mut().ok_or_else(|| {
        StructureError::InvalidDocument("collection item is not a mapping".to_owned())
    })?;
    let info = mapping
        .get_mut(Value::String("info".to_owned()))
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| StructureError::InvalidDocument("item info is not a mapping".to_owned()))?;
    info.insert(Value::String(name.to_owned()), field);
    Ok(())
}

fn checked_index(requested: Option<usize>, child_count: usize) -> Result<usize, StructureError> {
    let index = requested.unwrap_or(child_count);
    if index > child_count {
        Err(StructureError::InvalidIndex { index, child_count })
    } else {
        Ok(index)
    }
}

fn validate_name(name: &str) -> Result<(), StructureError> {
    if name.trim().is_empty() {
        Err(StructureError::InvalidName(
            "item name must not be empty".to_owned(),
        ))
    } else {
        Ok(())
    }
}

fn slug(name: &str) -> Result<String, StructureError> {
    let mut output = String::new();
    let mut separator = false;
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('-');
            }
            output.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    if output.is_empty() {
        Err(StructureError::InvalidName(
            "item name must contain an ASCII letter or number for an unbundled path".to_owned(),
        ))
    } else {
        Ok(output)
    }
}

fn destination_directory(root: &Path, parent: Option<&str>) -> Result<PathBuf, StructureError> {
    let Some(parent) = parent else {
        return Ok(root.to_owned());
    };
    let path = safe_join(root, parent)?;
    if path.is_dir() && (path.join("folder.yml").is_file() || path.join("folder.yaml").is_file()) {
        Ok(path)
    } else {
        Err(StructureError::DestinationNotFound(parent.to_owned()))
    }
}

fn existing_path(root: &Path, selector: &str, kind: ItemKind) -> Result<PathBuf, StructureError> {
    let path = safe_join(root, selector)?;
    let valid = match kind {
        ItemKind::Request => path.is_file(),
        ItemKind::Folder => {
            path.is_dir()
                && (path.join("folder.yml").is_file() || path.join("folder.yaml").is_file())
        }
    };
    if valid {
        Ok(path)
    } else {
        Err(StructureError::ItemNotFound {
            kind,
            selector: selector.to_owned(),
        })
    }
}

fn safe_join(root: &Path, selector: &str) -> Result<PathBuf, StructureError> {
    let relative = Path::new(selector);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(StructureError::InvalidDestination(format!(
            "selector must be a normalized workspace-relative path: {selector}"
        )));
    }
    Ok(root.join(relative))
}

fn ensure_absent(root: &Path, path: &Path) -> Result<(), StructureError> {
    if path.exists() {
        Err(StructureError::DuplicateDestination(relative_selector(
            root, path,
        )))
    } else {
        Ok(())
    }
}

fn item_config(path: &Path, kind: ItemKind) -> PathBuf {
    if kind == ItemKind::Request {
        path.to_owned()
    } else {
        let yml = path.join("folder.yml");
        if yml.is_file() {
            yml
        } else {
            path.join("folder.yaml")
        }
    }
}

fn unbundled_result(
    root: &Path,
    kind: ItemKind,
    previous_selector: Option<String>,
    path: &Path,
    index: usize,
) -> StructureResult {
    StructureResult {
        kind,
        previous_selector,
        selector: Some(relative_selector(root, path)),
        parent: relative_parent(root, path.parent().expect("item must have parent")),
        index: Some(index),
        selector_remaps: BTreeMap::new(),
    }
}

fn relative_parent(root: &Path, parent: &Path) -> Option<String> {
    (parent != root).then(|| relative_selector(root, parent))
}

fn verify_source(path: &Path, expected: &[u8]) -> Result<(), StructureError> {
    let current = fs::read(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            StructureError::ConcurrentModification(path.to_owned())
        } else {
            io_error(path, source)
        }
    })?;
    if current == expected {
        Ok(())
    } else {
        Err(StructureError::ConcurrentModification(path.to_owned()))
    }
}

fn create_atomic(path: &Path, contents: &[u8]) -> Result<(), StructureError> {
    let mut file = AtomicWriteFile::open(path).map_err(|source| io_error(path, source))?;
    file.write_all(contents)
        .map_err(|source| io_error(path, source))?;
    file.sync_all().map_err(|source| io_error(path, source))?;
    file.commit().map_err(|source| io_error(path, source))
}

fn io_error(path: &Path, source: io::Error) -> StructureError {
    StructureError::Io {
        path: path.to_owned(),
        source,
    }
}

fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_SUFFIX: AtomicU64 = AtomicU64::new(0);

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
        + "-"
        + &std::process::id().to_string()
        + "-"
        + &NEXT_SUFFIX.fetch_add(1, Ordering::Relaxed).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_reports_a_failed_rollback() {
        let root = std::env::temp_dir().join(format!(
            "probe-transaction-test-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir(&root).unwrap();
        let mut updates = BTreeMap::new();
        updates.insert(root.join("a.yml"), (b"first".to_vec(), b"changed".to_vec()));
        updates.insert(
            root.join("b.yml"),
            (b"second".to_vec(), b"changed".to_vec()),
        );
        let mut calls = 0;

        let error = write_transaction_with(updates, |path, _, _| {
            calls += 1;
            match calls {
                1 => Ok(()),
                2 => Err(SaveError::ConcurrentModification(path.to_owned())),
                3 => Err(SaveError::Io {
                    path: path.to_owned(),
                    source: io::Error::other("rollback failed"),
                }),
                _ => unreachable!(),
            }
        })
        .unwrap_err();

        assert!(matches!(error, StructureError::RecoveryRequired(_)));
        assert_eq!(calls, 3);
        let snapshots = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0].is_dir());
        let recovery_files = fs::read_dir(&snapshots[0])
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert!(
            recovery_files
                .iter()
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("bak"))
                .all(|path| matches!(fs::read(path).unwrap().as_slice(), b"first" | b"second"))
        );
        assert_eq!(
            recovery_files
                .iter()
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("bak"))
                .count(),
            2
        );
        assert!(snapshots[0].join("manifest.txt").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rename_preflights_every_fallible_document_read_before_moving_the_path() {
        let root = std::env::temp_dir().join(format!(
            "probe-rename-test-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("alpha.yml"),
            "info: { name: Alpha, type: http }\nhttp: { method: GET }\n",
        )
        .unwrap();
        fs::write(root.join("broken.yml"), "[\n").unwrap();

        let error = rename_unbundled(
            &root,
            "alpha.yml".to_owned(),
            ItemKind::Request,
            "Renamed".to_owned(),
        )
        .unwrap_err();

        assert!(matches!(error, StructureError::InvalidDocument(_)));
        assert!(root.join("alpha.yml").exists());
        assert!(!root.join("renamed.yml").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_refresh_failure_is_distinct_from_a_failed_write() {
        let result = StructureResult {
            kind: ItemKind::Request,
            previous_selector: None,
            selector: Some("created.yml".to_owned()),
            parent: None,
            index: Some(0),
            selector_remaps: BTreeMap::new(),
        };

        let error = reload_committed_workspace(PathBuf::from("workspace"), &result, |_| {
            Err("transient read failure".to_owned())
        })
        .unwrap_err();

        assert_eq!(error.category(), "committed_refresh_failed");
        assert!(matches!(
            error,
            StructureError::CommittedRefreshFailed {
                result: committed,
                ..
            } if *committed == result
        ));
    }

    #[test]
    fn committed_cleanup_failure_retains_the_result_and_external_tombstone() {
        let result = StructureResult {
            kind: ItemKind::Folder,
            previous_selector: Some("group".to_owned()),
            selector: None,
            parent: None,
            index: None,
            selector_remaps: BTreeMap::new(),
        };
        let tombstone = PathBuf::from("outside").join(".probe-delete-test");

        let error = finish_deletion(&tombstone, result.clone(), |_| {
            Err(io::Error::other("cleanup failed"))
        })
        .unwrap_err();

        assert_eq!(error.category(), "committed_cleanup_failed");
        assert!(matches!(
            error,
            StructureError::CommittedCleanupFailed {
                result: committed,
                path,
                ..
            } if *committed == result && path == tombstone
        ));
    }

    #[test]
    fn path_rollback_failure_preserves_the_original_recovery_details() {
        let error = path_rollback_error(
            StructureError::RecoveryRequired(
                "durable snapshots retained in recovery manifest".to_owned(),
            ),
            Path::new("workspace-item"),
            Err(io::Error::other("rename failed")),
        );
        let message = error.to_string();

        assert!(message.contains("recovery manifest"));
        assert!(message.contains("workspace-item"));
        assert!(message.contains("rename failed"));
    }

    #[test]
    fn unique_suffixes_do_not_collide_within_a_process() {
        let suffixes = (0..1_000).map(|_| unique_suffix()).collect::<BTreeSet<_>>();

        assert_eq!(suffixes.len(), 1_000);
    }
}
