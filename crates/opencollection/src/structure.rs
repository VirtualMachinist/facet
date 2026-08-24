use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use serde_yaml_ng::{Mapping, Value};

use crate::repository::{
    LoadedWorkspace, SaveError, SaveLock, WorkspaceSource, atomic_write, load_workspace,
    relative_selector,
};

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

/// A stable structural editing failure.
#[derive(Debug)]
pub enum StructureError {
    ItemNotFound {
        kind: ItemKind,
        selector: String,
    },
    DestinationNotFound(String),
    DuplicateDestination(String),
    InvalidDestination(String),
    InvalidName(String),
    InvalidIndex {
        index: usize,
        child_count: usize,
    },
    ReadOnlySource,
    ConcurrentModification(PathBuf),
    RecoveryRequired(String),
    CommittedRefreshFailed {
        result: Box<StructureResult>,
        message: String,
    },
    CommittedCleanupFailed {
        result: Box<StructureResult>,
        path: PathBuf,
        message: String,
    },
    InvalidDocument(String),
    Io {
        path: PathBuf,
        source: io::Error,
    },
}

impl StructureError {
    /// Returns the stable category used by automation adapters.
    #[must_use]
    pub const fn category(&self) -> &'static str {
        match self {
            Self::ItemNotFound {
                kind: ItemKind::Request,
                ..
            } => "request_not_found",
            Self::ItemNotFound {
                kind: ItemKind::Folder,
                ..
            } => "folder_not_found",
            Self::DestinationNotFound(_) => "destination_not_found",
            Self::DuplicateDestination(_) => "duplicate_destination",
            Self::InvalidDestination(_) => "invalid_destination",
            Self::InvalidName(_) => "invalid_name",
            Self::InvalidIndex { .. } => "invalid_index",
            Self::ReadOnlySource => "persistence_read_only",
            Self::ConcurrentModification(_) => "workspace_modified",
            Self::RecoveryRequired(_) => "recovery_required",
            Self::CommittedRefreshFailed { .. } => "committed_refresh_failed",
            Self::CommittedCleanupFailed { .. } => "committed_cleanup_failed",
            Self::InvalidDocument(_) | Self::Io { .. } => "persistence_error",
        }
    }
}

impl fmt::Display for StructureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ItemNotFound { kind, selector } => {
                write!(
                    formatter,
                    "{} selector not found: {selector}",
                    kind.as_str()
                )
            }
            Self::DestinationNotFound(selector) => {
                write!(formatter, "destination folder not found: {selector}")
            }
            Self::DuplicateDestination(selector) => {
                write!(formatter, "destination already exists: {selector}")
            }
            Self::InvalidDestination(message) | Self::InvalidName(message) => {
                formatter.write_str(message)
            }
            Self::InvalidIndex { index, child_count } => write!(
                formatter,
                "insertion index {index} exceeds child count {child_count}"
            ),
            Self::ReadOnlySource => formatter.write_str("an in-memory workspace is read-only"),
            Self::ConcurrentModification(path) => write!(
                formatter,
                "refusing to overwrite externally modified file: {}",
                path.display()
            ),
            Self::RecoveryRequired(message) => {
                write!(
                    formatter,
                    "structural operation requires recovery: {message}"
                )
            }
            Self::CommittedRefreshFailed { result, message } => write!(
                formatter,
                "structural operation committed at {} but workspace refresh failed: {message}",
                result.selector.as_deref().unwrap_or("<deleted>")
            ),
            Self::CommittedCleanupFailed { path, message, .. } => write!(
                formatter,
                "structural operation committed but cleanup is required at {}: {message}",
                path.display()
            ),
            Self::InvalidDocument(message) => formatter.write_str(message),
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
        }
    }
}

impl Error for StructureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<SaveError> for StructureError {
    fn from(error: SaveError) -> Self {
        match error {
            SaveError::ReadOnlySource => Self::ReadOnlySource,
            SaveError::ConcurrentModification(path) => Self::ConcurrentModification(path),
            SaveError::InvalidDocument(message) => Self::InvalidDocument(message),
            SaveError::Serialize(error) => Self::InvalidDocument(error.to_string()),
            SaveError::Io { path, source } => Self::Io { path, source },
            SaveError::RequestNotFound(selector) => Self::ItemNotFound {
                kind: ItemKind::Request,
                selector,
            },
            SaveError::EmptyUpdate => Self::InvalidDocument("empty structural update".to_owned()),
            SaveError::Environment(error) => Self::InvalidDocument(error.to_string()),
        }
    }
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
    let source = match operation {
        StructureOperation::RenameRequest { selector, .. }
        | StructureOperation::DeleteRequest { selector }
        | StructureOperation::DuplicateRequest { selector }
        | StructureOperation::MoveRequest { selector, .. }
        | StructureOperation::ReorderRequest { selector, .. } => {
            Some((ItemKind::Request, selector))
        }
        StructureOperation::RenameFolder { selector, .. }
        | StructureOperation::DeleteFolder { selector }
        | StructureOperation::MoveFolder { selector, .. }
        | StructureOperation::ReorderFolder { selector, .. } => Some((ItemKind::Folder, selector)),
        StructureOperation::CreateRequest { .. } | StructureOperation::CreateFolder { .. } => None,
    };
    if let Some((kind, selector)) = source {
        let exists = match kind {
            ItemKind::Request => workspace.request_key(selector).is_some(),
            ItemKind::Folder => workspace.folder_key(selector).is_some(),
        };
        if !exists {
            return Err(StructureError::ItemNotFound {
                kind,
                selector: selector.clone(),
            });
        }
    }
    let parent = match operation {
        StructureOperation::CreateRequest { parent, .. }
        | StructureOperation::CreateFolder { parent, .. }
        | StructureOperation::MoveRequest { parent, .. }
        | StructureOperation::MoveFolder { parent, .. } => parent.as_deref(),
        _ => None,
    };
    if let Some(parent) = parent
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
    let source = operation_source(operation);
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
    let source_path = operation_source(operation)
        .map(|(selector, _)| parse_selector(selector))
        .transpose()?;
    let removes_source = matches!(
        operation,
        StructureOperation::DeleteRequest { .. }
            | StructureOperation::DeleteFolder { .. }
            | StructureOperation::MoveRequest { .. }
            | StructureOperation::MoveFolder { .. }
            | StructureOperation::ReorderRequest { .. }
            | StructureOperation::ReorderFolder { .. }
    );
    let inserts_item = matches!(
        operation,
        StructureOperation::CreateRequest { .. }
            | StructureOperation::CreateFolder { .. }
            | StructureOperation::DuplicateRequest { .. }
            | StructureOperation::MoveRequest { .. }
            | StructureOperation::MoveFolder { .. }
            | StructureOperation::ReorderRequest { .. }
            | StructureOperation::ReorderFolder { .. }
    );
    let insertion_path = if inserts_item {
        result.selector.as_deref().map(parse_selector).transpose()?
    } else {
        None
    };
    let moved_source = removes_source && inserts_item;
    let mut remaps = BTreeMap::new();
    for (selector, _) in old_items {
        let mut path = parse_selector(selector)?;
        if let Some(source_path) = &source_path
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

fn operation_source(operation: &StructureOperation) -> Option<(&str, ItemKind)> {
    match operation {
        StructureOperation::RenameRequest { selector, .. }
        | StructureOperation::DeleteRequest { selector }
        | StructureOperation::MoveRequest { selector, .. }
        | StructureOperation::ReorderRequest { selector, .. } => {
            Some((selector, ItemKind::Request))
        }
        StructureOperation::RenameFolder { selector, .. }
        | StructureOperation::DeleteFolder { selector }
        | StructureOperation::MoveFolder { selector, .. }
        | StructureOperation::ReorderFolder { selector, .. } => Some((selector, ItemKind::Folder)),
        StructureOperation::CreateRequest { .. }
        | StructureOperation::CreateFolder { .. }
        | StructureOperation::DuplicateRequest { .. } => None,
    }
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

fn mutate_bundled(
    document: &mut Value,
    operation: StructureOperation,
) -> Result<StructureResult, StructureError> {
    match operation {
        StructureOperation::CreateRequest {
            parent,
            index,
            name,
            method,
            url,
        } => {
            validate_name(&name)?;
            let parent_path = destination_path(document, parent.as_deref())?;
            let items = items_mut(document, &parent_path)?;
            let index = checked_index(index, items.len())?;
            items.insert(index, request_value(&name, method, url));
            Ok(result(ItemKind::Request, None, parent, index, &parent_path))
        }
        StructureOperation::CreateFolder {
            parent,
            index,
            name,
        } => {
            validate_name(&name)?;
            let parent_path = destination_path(document, parent.as_deref())?;
            let items = items_mut(document, &parent_path)?;
            let index = checked_index(index, items.len())?;
            items.insert(index, folder_value(&name));
            Ok(result(ItemKind::Folder, None, parent, index, &parent_path))
        }
        StructureOperation::RenameRequest { selector, name } => {
            validate_name(&name)?;
            rename_bundled(document, &selector, ItemKind::Request, &name)?;
            let (parent, index) = selector_parent(&selector)?;
            Ok(StructureResult {
                kind: ItemKind::Request,
                previous_selector: Some(selector.clone()),
                selector: Some(selector),
                parent,
                index: Some(index),
                selector_remaps: BTreeMap::new(),
            })
        }
        StructureOperation::RenameFolder { selector, name } => {
            validate_name(&name)?;
            rename_bundled(document, &selector, ItemKind::Folder, &name)?;
            let (parent, index) = selector_parent(&selector)?;
            Ok(StructureResult {
                kind: ItemKind::Folder,
                previous_selector: Some(selector.clone()),
                selector: Some(selector),
                parent,
                index: Some(index),
                selector_remaps: BTreeMap::new(),
            })
        }
        StructureOperation::DeleteRequest { selector } => {
            delete_bundled(document, &selector, ItemKind::Request)
        }
        StructureOperation::DuplicateRequest { selector } => duplicate_bundled(document, selector),
        StructureOperation::DeleteFolder { selector } => {
            delete_bundled(document, &selector, ItemKind::Folder)
        }
        StructureOperation::MoveRequest {
            selector,
            parent,
            index,
        } => move_bundled(document, selector, ItemKind::Request, parent, index),
        StructureOperation::MoveFolder {
            selector,
            parent,
            index,
        } => move_bundled(document, selector, ItemKind::Folder, parent, index),
        StructureOperation::ReorderRequest { selector, index } => {
            let (parent, _) = selector_parent(&selector)?;
            move_bundled(document, selector, ItemKind::Request, parent, Some(index))
        }
        StructureOperation::ReorderFolder { selector, index } => {
            let (parent, _) = selector_parent(&selector)?;
            move_bundled(document, selector, ItemKind::Folder, parent, Some(index))
        }
    }
}

fn mutate_unbundled(
    root: &Path,
    operation: StructureOperation,
) -> Result<StructureResult, StructureError> {
    match operation {
        StructureOperation::CreateRequest {
            parent,
            index,
            name,
            method,
            url,
        } => {
            validate_name(&name)?;
            let directory = destination_directory(root, parent.as_deref())?;
            let path = directory.join(format!("{}.yml", slug(&name)?));
            ensure_absent(root, &path)?;
            create_atomic(
                &path,
                serde_yaml_ng::to_string(&request_value(&name, method, url))
                    .map_err(|error| StructureError::InvalidDocument(error.to_string()))?
                    .as_bytes(),
            )?;
            let index = match reorder_directories(&[(&directory, Some((&path, index)))]) {
                Ok(indices) => indices[0],
                Err(error) => {
                    if let Err(cleanup) = fs::remove_file(&path) {
                        return Err(StructureError::RecoveryRequired(format!(
                            "could not remove failed request creation {}: {cleanup}",
                            path.display()
                        )));
                    }
                    return Err(error);
                }
            };
            Ok(unbundled_result(
                root,
                ItemKind::Request,
                None,
                &path,
                index,
            ))
        }
        StructureOperation::CreateFolder {
            parent,
            index,
            name,
        } => {
            validate_name(&name)?;
            let directory = destination_directory(root, parent.as_deref())?;
            let path = directory.join(slug(&name)?);
            ensure_absent(root, &path)?;
            fs::create_dir(&path).map_err(|source| io_error(&path, source))?;
            let config = path.join("folder.yml");
            if let Err(error) = create_atomic(
                &config,
                serde_yaml_ng::to_string(&item_value(&name, "folder", None))
                    .map_err(|error| StructureError::InvalidDocument(error.to_string()))?
                    .as_bytes(),
            ) {
                if let Err(cleanup) = fs::remove_dir_all(&path) {
                    return Err(StructureError::RecoveryRequired(format!(
                        "could not remove failed folder creation {}: {cleanup}",
                        path.display()
                    )));
                }
                return Err(error);
            }
            let index = match reorder_directories(&[(&directory, Some((&path, index)))]) {
                Ok(indices) => indices[0],
                Err(error) => {
                    if let Err(cleanup) = fs::remove_dir_all(&path) {
                        return Err(StructureError::RecoveryRequired(format!(
                            "could not remove failed folder creation {}: {cleanup}",
                            path.display()
                        )));
                    }
                    return Err(error);
                }
            };
            Ok(unbundled_result(root, ItemKind::Folder, None, &path, index))
        }
        StructureOperation::RenameRequest { selector, name } => {
            rename_unbundled(root, selector, ItemKind::Request, name)
        }
        StructureOperation::RenameFolder { selector, name } => {
            rename_unbundled(root, selector, ItemKind::Folder, name)
        }
        StructureOperation::DeleteRequest { selector } => {
            delete_unbundled(root, selector, ItemKind::Request)
        }
        StructureOperation::DuplicateRequest { selector } => duplicate_unbundled(root, selector),
        StructureOperation::DeleteFolder { selector } => {
            delete_unbundled(root, selector, ItemKind::Folder)
        }
        StructureOperation::MoveRequest {
            selector,
            parent,
            index,
        } => move_unbundled(root, selector, ItemKind::Request, parent, index),
        StructureOperation::MoveFolder {
            selector,
            parent,
            index,
        } => move_unbundled(root, selector, ItemKind::Folder, parent, index),
        StructureOperation::ReorderRequest { selector, index } => {
            reorder_unbundled(root, selector, ItemKind::Request, index)
        }
        StructureOperation::ReorderFolder { selector, index } => {
            reorder_unbundled(root, selector, ItemKind::Folder, index)
        }
    }
}

fn reorder_unbundled(
    root: &Path,
    selector: String,
    kind: ItemKind,
    index: usize,
) -> Result<StructureResult, StructureError> {
    let path = existing_path(root, &selector, kind)?;
    let parent = path.parent().expect("workspace item must have a parent");
    let indices = reorder_directories(&[(parent, Some((&path, Some(index))))])?;
    Ok(unbundled_result(
        root,
        kind,
        Some(selector),
        &path,
        indices[0],
    ))
}

fn rename_unbundled(
    root: &Path,
    selector: String,
    kind: ItemKind,
    name: String,
) -> Result<StructureResult, StructureError> {
    validate_name(&name)?;
    let old = existing_path(root, &selector, kind)?;
    let parent = old.parent().expect("workspace item must have a parent");
    let index = direct_children(parent)?
        .iter()
        .position(|child| child.path == old)
        .expect("validated item must be an orderable child");
    let extension = (kind == ItemKind::Request).then_some("yml");
    let mut new = parent.join(slug(&name)?);
    if let Some(extension) = extension {
        new.set_extension(extension);
    }
    let old_config = item_config(&old, kind);
    let original = fs::read(&old_config).map_err(|source| io_error(&old_config, source))?;
    let mut value: Value = serde_yaml_ng::from_slice(&original)
        .map_err(|error| StructureError::InvalidDocument(error.to_string()))?;
    set_info_field(&mut value, "name", Value::String(name))?;
    let serialized = serde_yaml_ng::to_string(&value)
        .map_err(|error| StructureError::InvalidDocument(error.to_string()))?;
    if new != old {
        ensure_absent(root, &new)?;
        fs::rename(&old, &new).map_err(|source| io_error(&old, source))?;
    }
    let config = item_config(&new, kind);
    if let Err(error) = atomic_write(&config, serialized.as_bytes(), &original) {
        if new != old
            && let Err(rollback) = fs::rename(&new, &old)
        {
            return Err(StructureError::RecoveryRequired(format!(
                "could not restore {} after rename failed: {rollback}",
                old.display()
            )));
        }
        return Err(error.into());
    }
    Ok(unbundled_result(root, kind, Some(selector), &new, index))
}

fn delete_unbundled(
    root: &Path,
    selector: String,
    kind: ItemKind,
) -> Result<StructureResult, StructureError> {
    let path = existing_path(root, &selector, kind)?;
    let parent = path.parent().expect("workspace item must have a parent");
    let tombstone = deletion_tombstone(root)?;
    fs::rename(&path, &tombstone).map_err(|source| io_error(&path, source))?;
    if let Err(error) = reorder_directories(&[(parent, None)]) {
        return Err(path_rollback_error(
            error,
            &path,
            fs::rename(&tombstone, &path),
        ));
    }
    let result = StructureResult {
        kind,
        previous_selector: Some(selector),
        selector: None,
        parent: relative_parent(root, parent),
        index: None,
        selector_remaps: BTreeMap::new(),
    };
    finish_deletion(&tombstone, result, |path| {
        if kind == ItemKind::Folder {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
    })
}

fn duplicate_unbundled(root: &Path, selector: String) -> Result<StructureResult, StructureError> {
    let source = existing_path(root, &selector, ItemKind::Request)?;
    let parent = source.parent().expect("workspace item must have a parent");
    let source_index = direct_children(parent)?
        .iter()
        .position(|child| child.path == source)
        .expect("validated request must be an orderable child");
    let original = fs::read(&source).map_err(|source_error| io_error(&source, source_error))?;
    let mut value: Value = serde_yaml_ng::from_slice(&original)
        .map_err(|error| StructureError::InvalidDocument(error.to_string()))?;
    ensure_kind(&value, ItemKind::Request, &selector)?;
    let name = copied_request_name(&value)?;
    set_info_field(&mut value, "name", Value::String(name.clone()))?;
    let path = parent.join(format!("{}.yml", slug(&name)?));
    ensure_absent(root, &path)?;
    create_atomic(
        &path,
        serde_yaml_ng::to_string(&value)
            .map_err(|error| StructureError::InvalidDocument(error.to_string()))?
            .as_bytes(),
    )?;
    let index = match reorder_directories(&[(parent, Some((&path, Some(source_index + 1))))]) {
        Ok(indices) => indices[0],
        Err(error) => {
            if let Err(cleanup) = fs::remove_file(&path) {
                return Err(StructureError::RecoveryRequired(format!(
                    "could not remove failed request duplicate {}: {cleanup}",
                    path.display()
                )));
            }
            return Err(error);
        }
    };
    Ok(unbundled_result(
        root,
        ItemKind::Request,
        None,
        &path,
        index,
    ))
}

fn deletion_tombstone(root: &Path) -> Result<PathBuf, StructureError> {
    let parent = root.parent().ok_or_else(|| {
        StructureError::InvalidDestination(
            "workspace root has no parent for recoverable deletion".to_owned(),
        )
    })?;
    let tombstone = parent.join(format!(".probe-delete-{}", unique_suffix()));
    if tombstone.exists() {
        Err(StructureError::DuplicateDestination(
            tombstone.display().to_string(),
        ))
    } else {
        Ok(tombstone)
    }
}

fn finish_deletion(
    tombstone: &Path,
    result: StructureResult,
    cleanup: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<StructureResult, StructureError> {
    match cleanup(tombstone) {
        Ok(()) => Ok(result),
        Err(error) => Err(StructureError::CommittedCleanupFailed {
            result: Box::new(result),
            path: tombstone.to_owned(),
            message: error.to_string(),
        }),
    }
}

fn move_unbundled(
    root: &Path,
    selector: String,
    kind: ItemKind,
    parent_selector: Option<String>,
    index: Option<usize>,
) -> Result<StructureResult, StructureError> {
    let old = existing_path(root, &selector, kind)?;
    let old_parent = old.parent().expect("workspace item must have a parent");
    let destination = destination_directory(root, parent_selector.as_deref())?;
    if kind == ItemKind::Folder && destination.starts_with(&old) {
        return Err(StructureError::InvalidDestination(
            "folder cannot be moved into itself or its descendant".to_owned(),
        ));
    }
    let new = destination.join(
        old.file_name()
            .ok_or_else(|| StructureError::InvalidDestination(selector.clone()))?,
    );
    if new != old {
        ensure_absent(root, &new)?;
        fs::rename(&old, &new).map_err(|source| io_error(&old, source))?;
    }
    let plans = if old_parent == destination {
        vec![(destination.as_path(), Some((new.as_path(), index)))]
    } else {
        vec![
            (old_parent, None),
            (destination.as_path(), Some((new.as_path(), index))),
        ]
    };
    let indices = match reorder_directories(&plans) {
        Ok(indices) => indices,
        Err(error) => {
            let rollback = if new == old {
                Ok(())
            } else {
                fs::rename(&new, &old)
            };
            return Err(path_rollback_error(error, &old, rollback));
        }
    };
    let resulting_index = *indices
        .last()
        .expect("destination plan must return an index");
    Ok(unbundled_result(
        root,
        kind,
        Some(selector),
        &new,
        resulting_index,
    ))
}

fn path_rollback_error(
    operation_error: StructureError,
    original_path: &Path,
    rollback: io::Result<()>,
) -> StructureError {
    match rollback {
        Ok(()) => operation_error,
        Err(rollback_error) => StructureError::RecoveryRequired(format!(
            "operation failed ({operation_error}); could not restore {}: {rollback_error}",
            original_path.display()
        )),
    }
}

type ReorderPlan<'a> = (&'a Path, Option<(&'a Path, Option<usize>)>);

fn reorder_directories(plans: &[ReorderPlan<'_>]) -> Result<Vec<usize>, StructureError> {
    let mut outputs = Vec::with_capacity(plans.len());
    let mut updates = BTreeMap::<PathBuf, (Vec<u8>, Vec<u8>)>::new();
    for (directory, moved) in plans {
        let mut children = direct_children(directory)?;
        let resulting_index = if let Some((path, requested)) = moved {
            let position = children
                .iter()
                .position(|child| child.path == *path)
                .ok_or_else(|| StructureError::InvalidDestination(path.display().to_string()))?;
            let child = children.remove(position);
            let index = checked_index(*requested, children.len())?;
            children.insert(index, child);
            index
        } else {
            0
        };
        for (index, child) in children.iter().enumerate() {
            let original =
                fs::read(&child.config).map_err(|source| io_error(&child.config, source))?;
            let mut value: Value = serde_yaml_ng::from_slice(&original)
                .map_err(|error| StructureError::InvalidDocument(error.to_string()))?;
            set_info_field(&mut value, "seq", Value::Number((index as u64 + 1).into()))?;
            let serialized = serde_yaml_ng::to_string(&value)
                .map_err(|error| StructureError::InvalidDocument(error.to_string()))?
                .into_bytes();
            updates.insert(child.config.clone(), (original, serialized));
        }
        outputs.push(resulting_index);
    }
    write_transaction(updates)?;
    Ok(outputs)
}

fn write_transaction(updates: BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>) -> Result<(), StructureError> {
    write_transaction_with(updates, atomic_write)
}

fn write_transaction_with(
    updates: BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
    mut write: impl FnMut(&Path, &[u8], &[u8]) -> Result<(), SaveError>,
) -> Result<(), StructureError> {
    let snapshots = create_recovery_snapshots(&updates)?;
    let mut written: Vec<PathBuf> = Vec::new();
    for (path, (original, replacement)) in &updates {
        if let Err(error) = write(path, replacement, original) {
            let mut rollback_failure = None;
            for completed in written.into_iter().rev() {
                if let Some((before, after)) = updates.get(&completed)
                    && let Err(rollback_error) = write(&completed, before, after)
                {
                    rollback_failure.get_or_insert_with(|| {
                        format!(
                            "could not restore {}: {rollback_error}",
                            completed.display()
                        )
                    });
                }
            }
            if let Some(message) = rollback_failure {
                return Err(StructureError::RecoveryRequired(format!(
                    "{message}; durable snapshots retained: {}",
                    recovery_snapshot_summary(&snapshots)
                )));
            }
            remove_recovery_snapshots(&snapshots);
            return Err(error.into());
        }
        written.push(path.to_owned());
    }
    remove_recovery_snapshots(&snapshots);
    Ok(())
}

struct RecoverySnapshots {
    directory: Option<PathBuf>,
    files: Vec<(PathBuf, PathBuf)>,
}

fn create_recovery_snapshots(
    updates: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
) -> Result<RecoverySnapshots, StructureError> {
    if updates.is_empty() {
        return Ok(RecoverySnapshots {
            directory: None,
            files: Vec::new(),
        });
    }
    let suffix = unique_suffix();
    let common = common_parent(updates.keys()).ok_or_else(|| {
        StructureError::InvalidDestination(
            "ordering transaction paths do not share a recovery directory".to_owned(),
        )
    })?;
    let directory = common.join(format!(".probe-recovery-{suffix}"));
    fs::create_dir(&directory).map_err(|source| io_error(&directory, source))?;
    let mut snapshots = RecoverySnapshots {
        directory: Some(directory.clone()),
        files: Vec::with_capacity(updates.len()),
    };
    let mut manifest = String::new();
    for (index, (path, (original, _))) in updates.iter().enumerate() {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document");
        let backup = directory.join(format!("{index:04}-{file_name}.bak"));
        if let Err(error) = create_atomic(&backup, original) {
            remove_recovery_snapshots(&snapshots);
            return Err(error);
        }
        manifest.push_str(&format!("{}\t{}\n", backup.display(), path.display()));
        snapshots.files.push((path.clone(), backup));
    }
    let manifest_path = directory.join("manifest.txt");
    if let Err(error) = create_atomic(&manifest_path, manifest.as_bytes()) {
        remove_recovery_snapshots(&snapshots);
        return Err(error);
    }
    Ok(snapshots)
}

fn common_parent<'a>(mut paths: impl Iterator<Item = &'a PathBuf>) -> Option<PathBuf> {
    let mut common = paths.next()?.parent()?.to_owned();
    for path in paths {
        while !path.starts_with(&common) {
            if !common.pop() {
                return None;
            }
        }
    }
    Some(common)
}

fn remove_recovery_snapshots(snapshots: &RecoverySnapshots) {
    if let Some(directory) = &snapshots.directory {
        let _ = fs::remove_dir_all(directory);
    }
}

fn recovery_snapshot_summary(snapshots: &RecoverySnapshots) -> String {
    snapshots
        .files
        .iter()
        .map(|(original, backup)| format!("{} -> {}", original.display(), backup.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug)]
struct DiskChild {
    path: PathBuf,
    config: PathBuf,
    sequence: f64,
}

fn direct_children(directory: &Path) -> Result<Vec<DiskChild>, StructureError> {
    let mut children = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error(directory, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(directory, source))?
    {
        let path = entry.path();
        let config = if path.is_dir() {
            let yml = path.join("folder.yml");
            let yaml = path.join("folder.yaml");
            if yml.is_file() {
                yml
            } else if yaml.is_file() {
                yaml
            } else {
                continue;
            }
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        ) && !matches!(
            path.file_stem().and_then(|value| value.to_str()),
            Some("opencollection" | "folder")
        ) {
            path.clone()
        } else {
            continue;
        };
        let source = fs::read(&config).map_err(|error| io_error(&config, error))?;
        let value: Value = serde_yaml_ng::from_slice(&source)
            .map_err(|error| StructureError::InvalidDocument(error.to_string()))?;
        if !matches!(
            value
                .get("info")
                .and_then(|info| info.get("type"))
                .and_then(Value::as_str),
            Some("http" | "folder")
        ) {
            continue;
        }
        let sequence = value
            .get("info")
            .and_then(|info| info.get("seq"))
            .and_then(Value::as_f64)
            .unwrap_or(f64::INFINITY);
        children.push(DiskChild {
            path,
            config,
            sequence,
        });
    }
    children.sort_by(|left, right| {
        left.sequence
            .total_cmp(&right.sequence)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(children)
}

fn discover_documents(root: &Path) -> Result<BTreeSet<PathBuf>, StructureError> {
    let mut documents = BTreeSet::new();
    let root_config = ["yml", "yaml"]
        .into_iter()
        .map(|extension| root.join(format!("opencollection.{extension}")))
        .find(|path| path.is_file())
        .ok_or_else(|| StructureError::ConcurrentModification(root.join("opencollection.yml")))?;
    documents.insert(root_config);
    discover_item_documents(root, "opencollection", &mut documents)?;
    Ok(documents)
}

fn discover_item_documents(
    directory: &Path,
    reserved_stem: &str,
    documents: &mut BTreeSet<PathBuf>,
) -> Result<(), StructureError> {
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error(directory, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io_error(directory, source))?
    {
        let file_type = entry
            .file_type()
            .map_err(|source| io_error(&entry.path(), source))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            let config = ["yml", "yaml"]
                .into_iter()
                .map(|extension| path.join(format!("folder.{extension}")))
                .find(|candidate| candidate.is_file());
            if let Some(config) = config {
                documents.insert(config);
                discover_item_documents(&path, "folder", documents)?;
            }
        } else if matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        ) && path.file_stem().and_then(|value| value.to_str()) != Some(reserved_stem)
        {
            documents.insert(path);
        }
    }
    Ok(())
}

fn move_bundled(
    document: &mut Value,
    selector: String,
    kind: ItemKind,
    parent: Option<String>,
    requested_index: Option<usize>,
) -> Result<StructureResult, StructureError> {
    if kind == ItemKind::Folder
        && parent.as_deref().is_some_and(|destination| {
            destination == selector || destination.starts_with(&(selector.clone() + "/items/"))
        })
    {
        return Err(StructureError::InvalidDestination(
            "folder cannot be moved into itself or its descendant".to_owned(),
        ));
    }
    let source_path = parse_selector(&selector)?;
    let source_index = *source_path
        .last()
        .ok_or_else(|| StructureError::InvalidDocument("empty selector".to_owned()))?;
    let source_parent = &source_path[..source_path.len() - 1];
    let mut destination_path = destination_path(document, parent.as_deref())?;
    let source_items = items_mut(document, source_parent)?;
    let item = source_items
        .get(source_index)
        .ok_or_else(|| StructureError::ItemNotFound {
            kind,
            selector: selector.clone(),
        })?;
    ensure_kind(item, kind, &selector)?;
    let item = source_items.remove(source_index);
    adjust_path_after_removal(&mut destination_path, source_parent, source_index);
    let destination_items = items_mut(document, &destination_path)?;
    let index = checked_index(requested_index, destination_items.len())?;
    destination_items.insert(index, item);
    let actual_parent = selector_from_path(&destination_path);
    Ok(result(
        kind,
        Some(selector),
        actual_parent,
        index,
        &destination_path,
    ))
}

fn adjust_path_after_removal(
    destination_path: &mut [usize],
    source_parent: &[usize],
    source_index: usize,
) {
    if destination_path.len() > source_parent.len()
        && destination_path[..source_parent.len()] == *source_parent
        && destination_path[source_parent.len()] > source_index
    {
        destination_path[source_parent.len()] -= 1;
    }
}

fn delete_bundled(
    document: &mut Value,
    selector: &str,
    kind: ItemKind,
) -> Result<StructureResult, StructureError> {
    let path = parse_selector(selector)?;
    let index = *path
        .last()
        .ok_or_else(|| StructureError::InvalidDocument("empty selector".to_owned()))?;
    let parent_path = &path[..path.len() - 1];
    let items = items_mut(document, parent_path)?;
    let item = items
        .get(index)
        .ok_or_else(|| StructureError::ItemNotFound {
            kind,
            selector: selector.to_owned(),
        })?;
    ensure_kind(item, kind, selector)?;
    items.remove(index);
    let (parent, _) = selector_parent(selector)?;
    Ok(StructureResult {
        kind,
        previous_selector: Some(selector.to_owned()),
        selector: None,
        parent,
        index: None,
        selector_remaps: BTreeMap::new(),
    })
}

fn duplicate_bundled(
    document: &mut Value,
    selector: String,
) -> Result<StructureResult, StructureError> {
    let path = parse_selector(&selector)?;
    let index = *path
        .last()
        .ok_or_else(|| StructureError::InvalidDocument("empty selector".to_owned()))?;
    let parent_path = &path[..path.len() - 1];
    let mut duplicate = item(document, &path)
        .ok_or_else(|| StructureError::ItemNotFound {
            kind: ItemKind::Request,
            selector: selector.clone(),
        })?
        .clone();
    ensure_kind(&duplicate, ItemKind::Request, &selector)?;
    let name = copied_request_name(&duplicate)?;
    set_info_field(&mut duplicate, "name", Value::String(name))?;
    let items = items_mut(document, parent_path)?;
    let insertion_index = index + 1;
    items.insert(insertion_index, duplicate);
    Ok(result(
        ItemKind::Request,
        None,
        selector_from_path(parent_path),
        insertion_index,
        parent_path,
    ))
}

fn rename_bundled(
    document: &mut Value,
    selector: &str,
    kind: ItemKind,
    name: &str,
) -> Result<(), StructureError> {
    let path = parse_selector(selector)?;
    let item = item_mut(document, &path).ok_or_else(|| StructureError::ItemNotFound {
        kind,
        selector: selector.to_owned(),
    })?;
    ensure_kind(item, kind, selector)?;
    set_info_field(item, "name", Value::String(name.to_owned()))
}

fn copied_request_name(value: &Value) -> Result<String, StructureError> {
    let original = value
        .get("info")
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Untitled request");
    let name = format!("{original} Copied");
    validate_name(&name)?;
    Ok(name)
}

fn destination_path(
    document: &Value,
    selector: Option<&str>,
) -> Result<Vec<usize>, StructureError> {
    let Some(selector) = selector else {
        return Ok(Vec::new());
    };
    let path = parse_selector(selector)?;
    let item = item(document, &path)
        .ok_or_else(|| StructureError::DestinationNotFound(selector.to_owned()))?;
    ensure_kind(item, ItemKind::Folder, selector)
        .map_err(|_| StructureError::DestinationNotFound(selector.to_owned()))?;
    Ok(path)
}

fn result(
    kind: ItemKind,
    previous_selector: Option<String>,
    parent: Option<String>,
    index: usize,
    parent_path: &[usize],
) -> StructureResult {
    StructureResult {
        kind,
        previous_selector,
        selector: Some(selector_for(parent_path, index)),
        parent,
        index: Some(index),
        selector_remaps: BTreeMap::new(),
    }
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
