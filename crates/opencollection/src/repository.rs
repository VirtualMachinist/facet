use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use atomic_write_file::AtomicWriteFile;
#[cfg(any(unix, windows))]
use fs4::FileExt;
use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, CollectionItem, Environment,
    EnvironmentResolutionError, EnvironmentVariable, FileReference, FolderKey, FormField, Header,
    MultipartPart, MultipartPartKind, MultipartValue, QueryParameter, RawBodyKind, RequestBody,
    RequestKey, RequestUpdate, Variable, VariableValue, VariableValueSet, VariableValueVariant,
    Workspace, WorkspaceItemRef, validate_environments,
};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use super::{EnvironmentDocument, ParseError, parse, project_item};

/// A request and its repository-backed selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocatedRequest {
    selector: String,
    key: RequestKey,
    persistence: Option<RequestPersistence>,
}

impl LocatedRequest {
    /// Returns the selector accepted by CLI and repository operations.
    #[must_use]
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// Returns the request's session-only workspace key.
    #[must_use]
    pub const fn key(&self) -> RequestKey {
        self.key
    }
}

/// A folder and its repository-backed selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocatedFolder {
    selector: String,
    key: FolderKey,
}

impl LocatedFolder {
    /// Returns the stable selector used to restore presentation state.
    #[must_use]
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// Returns the folder's session-only workspace key.
    #[must_use]
    pub const fn key(&self) -> FolderKey {
        self.key
    }
}

/// A loaded OpenCollection workspace and its persistence-locator index.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedWorkspace {
    workspace: Workspace,
    requests: Vec<LocatedRequest>,
    folders: Vec<LocatedFolder>,
    request_keys_by_selector: BTreeMap<String, RequestKey>,
    folder_keys_by_selector: BTreeMap<String, FolderKey>,
    request_selectors_by_key: BTreeMap<RequestKey, String>,
    folder_selectors_by_key: BTreeMap<FolderKey, String>,
    environment_persistence: BTreeMap<String, EnvironmentPersistence>,
    pub(crate) documents: BTreeMap<PathBuf, SourceDocument>,
    pub(crate) source: WorkspaceSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceSource {
    Bundled(PathBuf),
    Unbundled(PathBuf),
    Memory,
}

impl LoadedWorkspace {
    /// Returns the in-memory domain workspace.
    #[must_use]
    pub const fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Returns whether repository locators are workspace-relative filesystem paths.
    ///
    /// Unbundled collections use paths. Bundled and in-memory documents use structural
    /// item selectors, which do not collide by file name.
    #[must_use]
    pub const fn uses_path_locators(&self) -> bool {
        matches!(self.source, WorkspaceSource::Unbundled(_))
    }

    /// Returns the filesystem path this workspace was loaded from, if any.
    #[must_use]
    pub fn source_path(&self) -> Option<&Path> {
        match &self.source {
            WorkspaceSource::Bundled(path) | WorkspaceSource::Unbundled(path) => Some(path),
            WorkspaceSource::Memory => None,
        }
    }

    /// Mutably looks up a request in the loaded in-memory workspace.
    ///
    /// Desktop editors use this fast path to apply draft changes immediately. Saving
    /// remains an explicit, separate repository operation.
    pub fn request_mut(&mut self, key: RequestKey) -> Option<&mut probe_core::HttpRequest> {
        self.workspace.request_mut(key)
    }

    /// Updates a plain environment variable in the in-memory workspace.
    pub fn set_environment_variable(
        &mut self,
        environment_name: &str,
        variable_name: &str,
        value: String,
    ) -> Result<(), EnvironmentResolutionError> {
        self.workspace
            .set_environment_variable(environment_name, variable_name, value)
    }

    /// Applies a variable set in memory and atomically persists its OpenCollection document.
    ///
    /// The save is rejected if the source file no longer exactly matches the bytes
    /// loaded by this repository instance. On persistence failure, the in-memory
    /// environment remains updated so callers can report or retry the dirty state.
    pub fn update_environment_variable(
        &mut self,
        environment_name: &str,
        variable_name: &str,
        value: String,
    ) -> Result<(), SaveError> {
        self.workspace
            .set_environment_variable(environment_name, variable_name, value)
            .map_err(SaveError::Environment)?;
        let variable = self
            .plain_environment_variable(environment_name, variable_name)
            .cloned()
            .ok_or_else(|| {
                SaveError::InvalidDocument(format!(
                    "environment '{environment_name}' is missing variable '{variable_name}' after update"
                ))
            })?;
        self.persist_environment_mutation(
            environment_name,
            EnvironmentYamlMutation::Set { variable },
        )
    }

    /// Removes a variable from the named environment and atomically persists the document.
    pub fn unset_environment_variable(
        &mut self,
        environment_name: &str,
        variable_name: &str,
    ) -> Result<(), SaveError> {
        self.workspace
            .unset_environment_variable(environment_name, variable_name)
            .map_err(SaveError::Environment)?;
        self.persist_environment_mutation(
            environment_name,
            EnvironmentYamlMutation::Unset {
                name: variable_name.to_owned(),
            },
        )
    }

    /// Creates a new environment in memory and atomically persists its OpenCollection document.
    pub fn create_environment(
        &mut self,
        name: String,
        extends: Option<String>,
    ) -> Result<(), SaveError> {
        self.workspace
            .create_environment(name.clone(), extends)
            .map_err(SaveError::Environment)?;
        let environment = self
            .workspace
            .environments()
            .iter()
            .find(|environment| environment.name == name)
            .expect("created environment must be present");
        let (document_path, serialized) = match &self.source {
            WorkspaceSource::Memory => return Err(SaveError::ReadOnlySource),
            WorkspaceSource::Bundled(document_path) => {
                let document_path = document_path.clone();
                let original_source = self
                    .documents
                    .get(&document_path)
                    .expect("bundled workspace must retain its source document")
                    .original_source
                    .clone();
                let serialized = persist_bundled_environment_create(
                    &document_path,
                    &original_source,
                    environment,
                )?;
                (document_path, serialized)
            }
            WorkspaceSource::Unbundled(root) => {
                persist_unbundled_environment_create(root, environment)?
            }
        };
        let bundled_index = match &self.source {
            WorkspaceSource::Bundled(_) => Some(self.workspace.environments().len() - 1),
            WorkspaceSource::Unbundled(_) | WorkspaceSource::Memory => None,
        };
        self.environment_persistence.insert(
            name,
            EnvironmentPersistence {
                document_path: document_path.clone(),
                bundled_index,
            },
        );
        self.documents.insert(
            document_path,
            SourceDocument {
                original_source: serialized,
            },
        );
        Ok(())
    }

    /// Captures an environment-variable save that can be executed away from the UI thread.
    ///
    /// The in-memory workspace must already contain the updated variable. Preparing is
    /// in-memory only. [`PreparedEnvironmentSave::execute`] performs the conflict check
    /// and atomic filesystem write.
    pub fn prepare_environment_variable_save(
        &self,
        environment_name: &str,
        variable_name: &str,
    ) -> Result<PreparedEnvironmentSave, SaveError> {
        let variable = self
            .plain_environment_variable(environment_name, variable_name)
            .cloned()
            .ok_or_else(|| {
                SaveError::Environment(EnvironmentResolutionError::VariableNotFound {
                    environment: environment_name.to_owned(),
                    variable: variable_name.to_owned(),
                })
            })?;
        self.prepare_environment_mutation(
            environment_name,
            EnvironmentYamlMutation::Set { variable },
        )
    }

    /// Refreshes the retained conflict baseline after a prepared environment save succeeds.
    pub fn complete_environment_save(&mut self, saved: CompletedEnvironmentSave) {
        self.documents.insert(
            saved.document_path,
            SourceDocument {
                original_source: saved.serialized_source,
            },
        );
    }

    fn persist_environment_mutation(
        &mut self,
        environment_name: &str,
        mutation: EnvironmentYamlMutation,
    ) -> Result<(), SaveError> {
        let prepared = self.prepare_environment_mutation(environment_name, mutation)?;
        let saved = prepared.execute()?;
        self.complete_environment_save(saved);
        Ok(())
    }

    fn prepare_environment_mutation(
        &self,
        environment_name: &str,
        mutation: EnvironmentYamlMutation,
    ) -> Result<PreparedEnvironmentSave, SaveError> {
        let persistence = self
            .environment_persistence
            .get(environment_name)
            .cloned()
            .ok_or(SaveError::ReadOnlySource)?;
        let original_source = self
            .documents
            .get(&persistence.document_path)
            .expect("filesystem environment must retain its source document")
            .original_source
            .clone();
        Ok(PreparedEnvironmentSave {
            persistence,
            original_source,
            mutation,
        })
    }

    fn plain_environment_variable(
        &self,
        environment_name: &str,
        variable_name: &str,
    ) -> Option<&Variable> {
        self.workspace
            .environments()
            .iter()
            .find(|environment| environment.name == environment_name)?
            .variables
            .iter()
            .find_map(|variable| match variable {
                EnvironmentVariable::Plain(variable)
                    if variable.name.as_deref() == Some(variable_name) =>
                {
                    Some(variable)
                }
                _ => None,
            })
    }

    /// Returns requests in collection traversal order.
    #[must_use]
    pub fn requests(&self) -> &[LocatedRequest] {
        &self.requests
    }

    /// Returns folders in collection traversal order.
    #[must_use]
    pub fn folders(&self) -> &[LocatedFolder] {
        &self.folders
    }

    /// Resolves a repository-backed selector to a request key.
    #[must_use]
    pub fn request_key(&self, selector: &str) -> Option<RequestKey> {
        self.request_keys_by_selector.get(selector).copied()
    }

    /// Resolves a repository-backed selector to a folder key.
    #[must_use]
    pub fn folder_key(&self, selector: &str) -> Option<FolderKey> {
        self.folder_keys_by_selector.get(selector).copied()
    }

    /// Returns the stable selector for a request key.
    #[must_use]
    pub fn request_selector(&self, key: RequestKey) -> Option<&str> {
        self.request_selectors_by_key.get(&key).map(String::as_str)
    }

    /// Returns the stable selector for a folder key.
    #[must_use]
    pub fn folder_selector(&self, key: FolderKey) -> Option<&str> {
        self.folder_selectors_by_key.get(&key).map(String::as_str)
    }

    /// Applies an update in memory and atomically persists its OpenCollection document.
    ///
    /// The save is rejected if the source file no longer exactly matches the bytes
    /// loaded by this repository instance. On persistence failure, the in-memory
    /// request remains updated so callers can report or retry the dirty state.
    pub fn update_request(
        &mut self,
        selector: &str,
        update: &RequestUpdate,
    ) -> Result<(), SaveError> {
        if update.is_empty() {
            return Err(SaveError::EmptyUpdate);
        }

        let located = self
            .requests
            .iter()
            .find(|request| request.selector == selector)
            .cloned()
            .ok_or_else(|| SaveError::RequestNotFound(selector.to_owned()))?;
        let request = self
            .workspace
            .request_mut(located.key)
            .expect("repository request key must resolve");
        update.apply(request);

        let persistence = located.persistence.ok_or(SaveError::ReadOnlySource)?;
        let source = self
            .documents
            .get(&persistence.document_path)
            .expect("filesystem request must retain its source document");
        let _save_lock = SaveLock::acquire(&persistence.document_path)?;
        let current = fs::read(&persistence.document_path).map_err(|source| SaveError::Io {
            path: persistence.document_path.clone(),
            source,
        })?;
        if current != source.original_source {
            return Err(SaveError::ConcurrentModification(
                persistence.document_path.clone(),
            ));
        }

        let mut document: Value =
            serde_yaml_ng::from_slice(&source.original_source).map_err(|error| {
                SaveError::InvalidDocument(format!("retained source cannot be parsed: {error}"))
            })?;
        let request_document = request_document_mut(&mut document, &persistence.item_path)?;
        apply_request_update(request_document, update)?;
        let serialized = serde_yaml_ng::to_string(&document).map_err(SaveError::Serialize)?;
        atomic_write(
            &persistence.document_path,
            serialized.as_bytes(),
            &source.original_source,
        )?;

        self.documents.insert(
            persistence.document_path,
            SourceDocument {
                original_source: serialized.into_bytes(),
            },
        );
        Ok(())
    }

    /// Captures a request save that can be executed away from the UI thread.
    ///
    /// Preparing is in-memory only. [`PreparedRequestSave::execute`] performs the
    /// conflict check and atomic filesystem write.
    pub fn prepare_request_save(
        &self,
        selector: &str,
        update: RequestUpdate,
    ) -> Result<PreparedRequestSave, SaveError> {
        if update.is_empty() {
            return Err(SaveError::EmptyUpdate);
        }
        let persistence = self
            .requests
            .iter()
            .find(|request| request.selector == selector)
            .ok_or_else(|| SaveError::RequestNotFound(selector.to_owned()))?
            .persistence
            .clone()
            .ok_or(SaveError::ReadOnlySource)?;
        let original_source = self
            .documents
            .get(&persistence.document_path)
            .expect("filesystem request must retain its source document")
            .original_source
            .clone();
        Ok(PreparedRequestSave {
            persistence,
            original_source,
            update,
        })
    }

    /// Refreshes the retained conflict baseline after a prepared save succeeds.
    pub fn complete_request_save(&mut self, saved: CompletedRequestSave) {
        self.documents.insert(
            saved.document_path,
            SourceDocument {
                original_source: saved.serialized_source,
            },
        );
    }
}

/// A filesystem save captured from a loaded workspace for background execution.
#[derive(Debug)]
pub struct PreparedRequestSave {
    persistence: RequestPersistence,
    original_source: Vec<u8>,
    update: RequestUpdate,
}

impl PreparedRequestSave {
    /// Performs the exact-source check and atomic write.
    pub fn execute(self) -> Result<CompletedRequestSave, SaveError> {
        let _save_lock = SaveLock::acquire(&self.persistence.document_path)?;
        let current =
            fs::read(&self.persistence.document_path).map_err(|source| SaveError::Io {
                path: self.persistence.document_path.clone(),
                source,
            })?;
        if current != self.original_source {
            return Err(SaveError::ConcurrentModification(
                self.persistence.document_path,
            ));
        }
        let mut document: Value =
            serde_yaml_ng::from_slice(&self.original_source).map_err(|error| {
                SaveError::InvalidDocument(format!("retained source cannot be parsed: {error}"))
            })?;
        let request = request_document_mut(&mut document, &self.persistence.item_path)?;
        apply_request_update(request, &self.update)?;
        let serialized = serde_yaml_ng::to_string(&document)
            .map_err(SaveError::Serialize)?
            .into_bytes();
        atomic_write(
            &self.persistence.document_path,
            &serialized,
            &self.original_source,
        )?;
        Ok(CompletedRequestSave {
            document_path: self.persistence.document_path,
            serialized_source: serialized,
        })
    }
}

/// The refreshed repository baseline produced by a successful prepared save.
#[derive(Debug)]
pub struct CompletedRequestSave {
    document_path: PathBuf,
    serialized_source: Vec<u8>,
}

/// A filesystem environment-variable save captured for background execution.
#[derive(Debug)]
pub struct PreparedEnvironmentSave {
    persistence: EnvironmentPersistence,
    original_source: Vec<u8>,
    mutation: EnvironmentYamlMutation,
}

impl PreparedEnvironmentSave {
    /// Performs the exact-source check and atomic write.
    pub fn execute(self) -> Result<CompletedEnvironmentSave, SaveError> {
        let serialized =
            persist_environment_yaml(&self.persistence, &self.original_source, &self.mutation)?;
        Ok(CompletedEnvironmentSave {
            document_path: self.persistence.document_path,
            serialized_source: serialized,
        })
    }
}

/// The refreshed repository baseline produced by a successful environment save.
#[derive(Debug)]
pub struct CompletedEnvironmentSave {
    document_path: PathBuf,
    serialized_source: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EnvironmentPersistence {
    document_path: PathBuf,
    bundled_index: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum EnvironmentYamlMutation {
    Set { variable: Variable },
    Unset { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestPersistence {
    document_path: PathBuf,
    item_path: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SourceDocument {
    pub(crate) original_source: Vec<u8>,
}

/// Loads a bundled OpenCollection file or an unbundled collection directory.
pub fn load_workspace(path: impl AsRef<Path>) -> Result<LoadedWorkspace, LoadError> {
    let path = path.as_ref();
    let canonical_path = fs::canonicalize(path).map_err(|source| LoadError::Io {
        path: path.to_owned(),
        source,
    })?;
    if canonical_path.is_dir() {
        load_unbundled(&canonical_path)
    } else {
        load_bundled(&canonical_path)
    }
}

/// Loads a bundled OpenCollection workspace from an in-memory YAML document.
///
/// Structural selectors are identical to selectors produced when the same bundled
/// document is loaded from a file.
pub fn load_workspace_from_str(source: &str) -> Result<LoadedWorkspace, LoadError> {
    load_bundled_source(source, None)
}

/// Creates an empty bundled OpenCollection YAML file at `path` and loads it.
///
/// When `path` has no extension, `.yml` is appended. `name` overrides the collection
/// title; otherwise the file stem is used. Existing files are left unchanged unless
/// `replace` is true. Directories are rejected.
pub fn create_bundled_workspace(
    path: impl AsRef<Path>,
    name: Option<&str>,
    replace: bool,
) -> Result<LoadedWorkspace, CreateError> {
    let path = with_yaml_extension(path.as_ref());
    if path.is_dir() {
        return Err(CreateError::IsDirectory(path));
    }
    if path.exists() && !replace {
        return Err(CreateError::AlreadyExists(path));
    }
    let collection_name = collection_name_from_path(&path, name);
    let source = bundled_collection_yaml(&collection_name)?;
    write_collection_file(&path, source.as_bytes())?;
    load_workspace(&path).map_err(CreateError::Load)
}

/// Creates a bundled OpenCollection workspace from a domain collection.
///
/// The destination is written atomically and is never overwritten. The resulting
/// document is loaded again before success is returned, so callers never receive a
/// workspace that the repository itself cannot read.
pub fn create_bundled_workspace_from_collection(
    path: impl AsRef<Path>,
    collection: &probe_core::Collection,
) -> Result<LoadedWorkspace, CreateError> {
    let path = with_yaml_extension(path.as_ref());
    if path.is_dir() {
        return Err(CreateError::IsDirectory(path));
    }
    let document = bundled_collection_value(collection);
    let source = serde_yaml_ng::to_string(&document).map_err(CreateError::Serialize)?;
    let source = source.strip_prefix("---\n").unwrap_or(&source);
    // Validate the complete document before creating the destination. This keeps
    // conversion errors from leaving a newly-created but unusable collection behind.
    load_workspace_from_str(source).map_err(CreateError::Load)?;
    write_new_collection_file(&path, source.as_bytes())?;
    load_workspace(&path).map_err(CreateError::Load)
}

/// Writes a completed document to a previously nonexistent path without ever
/// replacing a concurrently-created destination.
///
/// A temporary file is fully flushed before it is hard-linked into place. Creating
/// the final link is an atomic create-if-absent operation on the destination
/// filesystem, unlike an atomic rename which would replace an existing path.
fn write_new_collection_file(path: &Path, contents: &[u8]) -> Result<(), CreateError> {
    let map_io = |source| CreateError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(map_io)?;
    }

    let (temporary_path, mut file) = create_unique_temporary_file(path)?;
    let write_result = file.write_all(contents).and_then(|()| file.sync_all());
    drop(file);
    let write_result = write_result.and_then(|()| fs::hard_link(&temporary_path, path));
    let cleanup_result = fs::remove_file(&temporary_path);

    match write_result {
        Ok(()) => {
            // The destination is already durable and visible. A stale hidden
            // temporary file is harmless, so never misreport a committed import
            // as failed solely because cleanup was interrupted.
            let _ = cleanup_result;
            Ok(())
        }
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            let _ = cleanup_result;
            Err(CreateError::AlreadyExists(path.to_owned()))
        }
        Err(source) => {
            let _ = cleanup_result;
            Err(map_io(source))
        }
    }
}

fn create_unique_temporary_file(path: &Path) -> Result<(PathBuf, fs::File), CreateError> {
    static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let directory = parent.unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("collection.yml");
    for _ in 0..128 {
        let id = NEXT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let candidate = directory.join(format!(
            ".{filename}.probe-import-{}-{id}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(CreateError::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        }
    }
    Err(CreateError::Io {
        path: path.to_owned(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique import temporary file",
        ),
    })
}

fn with_yaml_extension(path: &Path) -> PathBuf {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension)
            if extension.eq_ignore_ascii_case("yml") || extension.eq_ignore_ascii_case("yaml") =>
        {
            path.to_owned()
        }
        Some(_) => path.to_owned(),
        None => path.with_extension("yml"),
    }
}

fn collection_name_from_path(path: &Path, name: Option<&str>) -> String {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("Collection")
        .to_owned()
}

fn bundled_collection_yaml(name: &str) -> Result<String, CreateError> {
    let document = NewBundledCollection {
        opencollection: "1.0.0",
        info: NewCollectionInfo { name },
        bundled: true,
    };
    let yaml = serde_yaml_ng::to_string(&document).map_err(CreateError::Serialize)?;
    Ok(yaml
        .strip_prefix("---\n")
        .unwrap_or(&yaml)
        .trim_start()
        .to_owned())
}

fn write_collection_file(path: &Path, contents: &[u8]) -> Result<(), CreateError> {
    let map_io = |source| CreateError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(map_io)?;
    }
    let mut file = AtomicWriteFile::open(path).map_err(map_io)?;
    file.write_all(contents).map_err(map_io)?;
    file.sync_all().map_err(map_io)?;
    file.commit().map_err(map_io)?;
    Ok(())
}

#[derive(Serialize)]
struct NewBundledCollection<'a> {
    opencollection: &'a str,
    info: NewCollectionInfo<'a>,
    bundled: bool,
}

#[derive(Serialize)]
struct NewCollectionInfo<'a> {
    name: &'a str,
}

fn bundled_collection_value(collection: &probe_core::Collection) -> Value {
    let mut root = serde_yaml_ng::Mapping::new();
    root.insert(
        string_key("opencollection"),
        Value::String("1.0.0".to_owned()),
    );
    root.insert(
        string_key("info"),
        collection_info_value(&collection.metadata),
    );
    root.insert(string_key("bundled"), Value::Bool(true));
    if !collection.environments.is_empty() {
        root.insert(
            string_key("config"),
            map([(
                "environments",
                Value::Sequence(
                    collection
                        .environments
                        .iter()
                        .map(environment_value)
                        .collect(),
                ),
            )]),
        );
    }
    root.insert(
        string_key("items"),
        Value::Sequence(collection.items.iter().map(collection_item_value).collect()),
    );
    Value::Mapping(root)
}

fn collection_info_value(metadata: &probe_core::CollectionMetadata) -> Value {
    let mut info = serde_yaml_ng::Mapping::new();
    if let Some(name) = &metadata.name {
        info.insert(string_key("name"), Value::String(name.clone()));
    }
    if let Some(summary) = &metadata.summary {
        info.insert(string_key("summary"), Value::String(summary.clone()));
    }
    if let Some(version) = &metadata.version {
        info.insert(string_key("version"), Value::String(version.clone()));
    }
    if !metadata.authors.is_empty() {
        info.insert(
            string_key("authors"),
            Value::Sequence(
                metadata
                    .authors
                    .iter()
                    .map(|author| {
                        let mut value = serde_yaml_ng::Mapping::new();
                        if let Some(name) = &author.name {
                            value.insert(string_key("name"), Value::String(name.clone()));
                        }
                        if let Some(email) = &author.email {
                            value.insert(string_key("email"), Value::String(email.clone()));
                        }
                        if let Some(url) = &author.url {
                            value.insert(string_key("url"), Value::String(url.clone()));
                        }
                        Value::Mapping(value)
                    })
                    .collect(),
            ),
        );
    }
    Value::Mapping(info)
}

fn collection_item_value(item: &CollectionItem) -> Value {
    match item {
        CollectionItem::Folder(folder) => {
            let mut item = item_info_value(&folder.metadata, "folder");
            item.insert(
                string_key("items"),
                Value::Sequence(folder.items.iter().map(collection_item_value).collect()),
            );
            Value::Mapping(item)
        }
        CollectionItem::HttpRequest(request) => {
            let mut item = item_info_value(&request.metadata, "http");
            let mut http = serde_yaml_ng::Mapping::new();
            if let Some(method) = &request.method {
                http.insert(string_key("method"), Value::String(method.clone()));
            }
            if let Some(url) = &request.url {
                http.insert(string_key("url"), Value::String(url.clone()));
            }
            if !request.headers.is_empty() {
                http.insert(
                    string_key("headers"),
                    Value::Sequence(request.headers.iter().map(header_value).collect()),
                );
            }
            let parameters = request
                .query_parameters
                .iter()
                .map(query_parameter_value)
                .chain(request.path_parameters.iter().map(path_parameter_value))
                .collect::<Vec<_>>();
            if !parameters.is_empty() {
                http.insert(string_key("params"), Value::Sequence(parameters));
            }
            if let Some(body) = &request.body {
                http.insert(string_key("body"), request_body_value(body));
            }
            if let Some(authentication) = &request.authentication {
                http.insert(string_key("auth"), authentication_value(authentication));
            }
            item.insert(string_key("http"), Value::Mapping(http));
            if let Some(settings) = request_settings_value(&request.settings) {
                item.insert(string_key("settings"), settings);
            }
            Value::Mapping(item)
        }
    }
}

fn item_info_value(metadata: &probe_core::ItemMetadata, item_type: &str) -> serde_yaml_ng::Mapping {
    let mut info = serde_yaml_ng::Mapping::new();
    info.insert(string_key("type"), Value::String(item_type.to_owned()));
    if let Some(name) = &metadata.name {
        info.insert(string_key("name"), Value::String(name.clone()));
    }
    if let Some(sequence) = metadata.sequence
        && let Ok(value) = serde_yaml_ng::to_value(sequence)
    {
        info.insert(string_key("seq"), value);
    }
    let mut item = serde_yaml_ng::Mapping::new();
    item.insert(string_key("info"), Value::Mapping(info));
    item
}

fn request_settings_value(settings: &probe_core::RequestSettings) -> Option<Value> {
    let mut value = serde_yaml_ng::Mapping::new();
    if let Some(timeout) = settings.timeout
        && let Ok(timeout) = serde_yaml_ng::to_value(timeout.as_secs_f64() * 1000.0)
    {
        value.insert(string_key("timeout"), timeout);
    }
    if let Some(follow_redirects) = settings.follow_redirects {
        value.insert(string_key("followRedirects"), Value::Bool(follow_redirects));
    }
    if let Some(max_redirects) = settings.max_redirects
        && let Ok(max_redirects) = serde_yaml_ng::to_value(max_redirects)
    {
        value.insert(string_key("maxRedirects"), max_redirects);
    }
    (!value.is_empty()).then(|| Value::Mapping(value))
}

fn environment_value(environment: &Environment) -> Value {
    let mut value = serde_yaml_ng::Mapping::new();
    value.insert(string_key("name"), Value::String(environment.name.clone()));
    if let Some(color) = &environment.color {
        value.insert(string_key("color"), Value::String(color.clone()));
    }
    if let Some(extends) = &environment.extends {
        value.insert(string_key("extends"), Value::String(extends.clone()));
    }
    if let Some(path) = &environment.dot_env_file_path {
        value.insert(string_key("dotEnvFilePath"), Value::String(path.clone()));
    }
    if !environment.variables.is_empty() {
        value.insert(
            string_key("variables"),
            Value::Sequence(
                environment
                    .variables
                    .iter()
                    .map(environment_variable_value)
                    .collect(),
            ),
        );
    }
    Value::Mapping(value)
}

fn environment_variable_value(variable: &EnvironmentVariable) -> Value {
    match variable {
        EnvironmentVariable::Plain(variable) => new_environment_variable_value(variable),
        EnvironmentVariable::Secret(variable) => {
            let mut value = serde_yaml_ng::Mapping::new();
            if let Some(name) = &variable.name {
                value.insert(string_key("name"), Value::String(name.clone()));
            }
            value.insert(string_key("secret"), Value::Bool(true));
            if let Some(kind) = &variable.value_type {
                value.insert(string_key("type"), Value::String(kind.as_str().to_owned()));
            }
            if variable.disabled {
                value.insert(string_key("disabled"), Value::Bool(true));
            }
            Value::Mapping(value)
        }
    }
}

/// An error raised while creating an OpenCollection workspace.
#[derive(Debug)]
pub enum CreateError {
    /// The destination already exists and replacement was not requested.
    AlreadyExists(PathBuf),
    /// The destination is a directory.
    IsDirectory(PathBuf),
    /// YAML serialization failed.
    Serialize(serde_yaml_ng::Error),
    /// A filesystem operation failed.
    Io { path: PathBuf, source: io::Error },
    /// The new document could not be loaded after it was written.
    Load(LoadError),
}

impl fmt::Display for CreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(path) => {
                write!(
                    formatter,
                    "refusing to overwrite existing file: {}",
                    path.display()
                )
            }
            Self::IsDirectory(path) => {
                write!(
                    formatter,
                    "cannot create a collection at directory {}",
                    path.display()
                )
            }
            Self::Serialize(source) => {
                write!(formatter, "cannot serialize OpenCollection YAML: {source}")
            }
            Self::Io { path, source } => {
                write!(formatter, "cannot create {}: {source}", path.display())
            }
            Self::Load(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for CreateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialize(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::Load(error) => Some(error),
            Self::AlreadyExists(_) | Self::IsDirectory(_) => None,
        }
    }
}

/// An error raised while loading an OpenCollection workspace.
#[derive(Debug)]
pub enum LoadError {
    /// A filesystem operation failed.
    Io { path: PathBuf, source: io::Error },
    /// A YAML document failed to parse.
    Parse { path: PathBuf, source: ParseError },
    /// An unbundled collection has no root configuration file.
    MissingRoot(PathBuf),
    /// A collection item has an unsupported shape for its filesystem location.
    InvalidItem { path: PathBuf, message: String },
    /// The document mode does not match how the workspace was opened.
    InvalidMode {
        path: PathBuf,
        expected_bundled: bool,
    },
    /// Cross-document workspace semantics are invalid.
    Validation { path: PathBuf, message: String },
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::Parse { path, source } => {
                write!(formatter, "failed to parse {}: {source}", path.display())
            }
            Self::MissingRoot(path) => write!(
                formatter,
                "{} does not contain opencollection.yml or opencollection.yaml",
                path.display()
            ),
            Self::InvalidItem { path, message } => {
                write!(formatter, "invalid item {}: {message}", path.display())
            }
            Self::InvalidMode {
                path,
                expected_bundled,
            } => write!(
                formatter,
                "{} declares bundled: {}, but this workspace requires bundled: {}",
                path.display(),
                !expected_bundled,
                expected_bundled
            ),
            Self::Validation { path, message } => {
                write!(formatter, "invalid workspace {}: {message}", path.display())
            }
        }
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
            Self::MissingRoot(_)
            | Self::InvalidItem { .. }
            | Self::InvalidMode { .. }
            | Self::Validation { .. } => None,
        }
    }
}

/// An error raised while persisting an OpenCollection request update.
#[derive(Debug)]
pub enum SaveError {
    /// No request matched the repository selector.
    RequestNotFound(String),
    /// The requested update did not contain any changed fields.
    EmptyUpdate,
    /// The workspace came from an in-memory source such as stdin.
    ReadOnlySource,
    /// The source changed after it was loaded and was not overwritten.
    ConcurrentModification(PathBuf),
    /// A retained source document no longer has the expected OpenCollection shape.
    InvalidDocument(String),
    /// Domain environment mutation failed.
    Environment(EnvironmentResolutionError),
    /// YAML serialization failed.
    Serialize(serde_yaml_ng::Error),
    /// An atomic filesystem operation failed.
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestNotFound(selector) => {
                write!(formatter, "request selector not found: {selector}")
            }
            Self::EmptyUpdate => formatter.write_str("request update has no changed fields"),
            Self::ReadOnlySource => {
                formatter.write_str("a workspace loaded from stdin cannot be persisted")
            }
            Self::ConcurrentModification(path) => write!(
                formatter,
                "refusing to overwrite externally modified file: {}",
                path.display()
            ),
            Self::InvalidDocument(message) => {
                write!(
                    formatter,
                    "cannot update retained OpenCollection document: {message}"
                )
            }
            Self::Environment(error) => write!(formatter, "{error}"),
            Self::Serialize(source) => {
                write!(formatter, "cannot serialize OpenCollection YAML: {source}")
            }
            Self::Io { path, source } => {
                write!(formatter, "cannot persist {}: {source}", path.display())
            }
        }
    }
}

impl Error for SaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialize(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::RequestNotFound(_)
            | Self::EmptyUpdate
            | Self::ReadOnlySource
            | Self::ConcurrentModification(_)
            | Self::InvalidDocument(_)
            | Self::Environment(_) => None,
        }
    }
}

#[derive(Debug)]
enum LocatorNode {
    Folder {
        selector: String,
        children: Vec<LocatorNode>,
    },
    Request {
        selector: String,
        persistence: Option<RequestPersistence>,
    },
}

fn load_bundled(path: &Path) -> Result<LoadedWorkspace, LoadError> {
    let source = read_to_string(path)?;
    load_bundled_source(&source, Some(path))
}

fn load_bundled_source(
    source: &str,
    document_path: Option<&Path>,
) -> Result<LoadedWorkspace, LoadError> {
    let source_name = document_path.unwrap_or_else(|| Path::new("<memory>"));
    let parsed = parse(source).map_err(|source| LoadError::Parse {
        path: source_name.to_owned(),
        source,
    })?;
    if !parsed.is_bundled() {
        return Err(LoadError::InvalidMode {
            path: source_name.to_owned(),
            expected_bundled: true,
        });
    }
    let nodes = bundled_locator_nodes(parsed.document(), "items", document_path);
    let workspace = Workspace::from_collection(parsed.into_collection());
    let mut loaded = index_locators(workspace, &nodes);
    if let Some(path) = document_path {
        loaded.environment_persistence =
            bundled_environment_persistences(loaded.workspace.environments(), path);
        loaded.documents.insert(
            path.to_owned(),
            SourceDocument {
                original_source: source.as_bytes().to_vec(),
            },
        );
        loaded.source = WorkspaceSource::Bundled(path.to_owned());
    }
    Ok(loaded)
}

fn load_unbundled(root: &Path) -> Result<LoadedWorkspace, LoadError> {
    let root_config = config_file(root, "opencollection")
        .ok_or_else(|| LoadError::MissingRoot(root.to_owned()))?;
    let source = read_to_string(&root_config)?;
    let parsed = parse(&source).map_err(|source| LoadError::Parse {
        path: root_config.clone(),
        source,
    })?;
    if parsed.is_bundled() {
        return Err(LoadError::InvalidMode {
            path: root_config,
            expected_bundled: false,
        });
    }
    let mut collection = parsed.into_collection();
    let mut documents = BTreeMap::new();
    documents.insert(
        root_config.clone(),
        SourceDocument {
            original_source: source.as_bytes().to_vec(),
        },
    );
    let loaded_items = read_items(root, root, "opencollection", &mut documents)?;
    let (items, nodes): (Vec<_>, Vec<_>) = loaded_items.into_iter().unzip();
    collection.items = items;
    let mut environment_persistence =
        bundled_environment_persistences(&collection.environments, &root_config);
    let file_environments = read_environments(root, &mut documents)?;
    for (environment, path) in file_environments {
        environment_persistence.insert(
            environment.name.clone(),
            EnvironmentPersistence {
                document_path: path,
                bundled_index: None,
            },
        );
        collection.environments.push(environment);
    }
    validate_environments(&collection.environments).map_err(|error| LoadError::Validation {
        path: root.to_owned(),
        message: error.to_string(),
    })?;

    let workspace = Workspace::from_collection(collection);
    let mut loaded = index_locators(workspace, &nodes);
    loaded.environment_persistence = environment_persistence;
    loaded.documents = documents;
    loaded.source = WorkspaceSource::Unbundled(root.to_owned());
    Ok(loaded)
}

fn read_items(
    directory: &Path,
    root: &Path,
    reserved_stem: &str,
    documents: &mut BTreeMap<PathBuf, SourceDocument>,
) -> Result<Vec<(CollectionItem, LocatorNode)>, LoadError> {
    let mut entries = read_directory(directory)?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut items = Vec::new();

    for entry in entries {
        let file_type = entry.file_type().map_err(|source| LoadError::Io {
            path: entry.path(),
            source,
        })?;
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();
        if file_type.is_dir() {
            let Some(folder_config) = config_file(&path, "folder") else {
                continue;
            };
            let read = read_item(&folder_config)?;
            documents.insert(
                folder_config.clone(),
                SourceDocument {
                    original_source: read.original_source.clone(),
                },
            );
            let mut folder = match read.item {
                Some(CollectionItem::Folder(folder)) => folder,
                Some(CollectionItem::HttpRequest(_)) => {
                    return Err(LoadError::InvalidItem {
                        path: folder_config,
                        message: "folder.yml must describe a folder".to_owned(),
                    });
                }
                None => continue,
            };
            let children = read_items(&path, root, "folder", documents)?;
            let (child_items, child_nodes): (Vec<_>, Vec<_>) = children.into_iter().unzip();
            folder.items = child_items;
            items.push((
                CollectionItem::Folder(folder),
                LocatorNode::Folder {
                    selector: relative_selector(root, &path),
                    children: child_nodes,
                },
            ));
        } else if is_yaml_file(&path)
            && path.file_stem().and_then(|stem| stem.to_str()) != Some(reserved_stem)
        {
            let read = read_item(&path)?;
            documents.insert(
                path.clone(),
                SourceDocument {
                    original_source: read.original_source.clone(),
                },
            );
            if let Some(item) = read.item {
                match item {
                    CollectionItem::HttpRequest(request) => {
                        let selector = relative_selector(root, &path);
                        items.push((
                            CollectionItem::HttpRequest(request),
                            LocatorNode::Request {
                                selector,
                                persistence: Some(RequestPersistence {
                                    document_path: path,
                                    item_path: Vec::new(),
                                }),
                            },
                        ));
                    }
                    CollectionItem::Folder(_) => {
                        return Err(LoadError::InvalidItem {
                            path,
                            message: "folders must be represented by directories with folder.yml"
                                .to_owned(),
                        });
                    }
                }
            }
        }
    }

    items.sort_by(|(left, left_node), (right, right_node)| {
        item_sequence(left)
            .total_cmp(&item_sequence(right))
            .then_with(|| locator_selector(left_node).cmp(locator_selector(right_node)))
    });
    Ok(items)
}

struct ReadItem {
    item: Option<CollectionItem>,
    original_source: Vec<u8>,
}

fn read_item(path: &Path) -> Result<ReadItem, LoadError> {
    let source = read_to_string(path)?;
    let value: Value = serde_yaml_ng::from_str(&source).map_err(|source| LoadError::Parse {
        path: path.to_owned(),
        source: ParseError::new(source),
    })?;
    let item = project_item(value).map_err(|source| LoadError::Parse {
        path: path.to_owned(),
        source: ParseError::new(source),
    })?;
    Ok(ReadItem {
        item,
        original_source: source.into_bytes(),
    })
}

fn read_environments(
    root: &Path,
    documents: &mut BTreeMap<PathBuf, SourceDocument>,
) -> Result<Vec<(Environment, PathBuf)>, LoadError> {
    let directory = root.join("environments");
    if !directory.is_dir() {
        return Ok(Vec::new());
    }

    let mut entries = read_directory(&directory)?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut environments = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|source| LoadError::Io {
                path: path.clone(),
                source,
            })?
            .is_file()
            || !is_yaml_file(&path)
        {
            continue;
        }
        let source = read_to_string(&path)?;
        let environment: EnvironmentDocument =
            serde_yaml_ng::from_str(&source).map_err(|source| LoadError::Parse {
                path: path.clone(),
                source: ParseError::new(source),
            })?;
        documents.insert(
            path.clone(),
            SourceDocument {
                original_source: source.into_bytes(),
            },
        );
        environments.push((environment.into_domain(), path));
    }
    Ok(environments)
}

fn bundled_environment_persistences(
    environments: &[Environment],
    document_path: &Path,
) -> BTreeMap<String, EnvironmentPersistence> {
    environments
        .iter()
        .enumerate()
        .map(|(index, environment)| {
            (
                environment.name.clone(),
                EnvironmentPersistence {
                    document_path: document_path.to_owned(),
                    bundled_index: Some(index),
                },
            )
        })
        .collect()
}

#[derive(Deserialize)]
struct LocatorItemsDocument {
    #[serde(default)]
    items: Vec<Value>,
}

#[derive(Default, Deserialize)]
struct LocatorItemDocument {
    #[serde(default)]
    info: LocatorInfoDocument,
    #[serde(default)]
    items: Vec<Value>,
}

#[derive(Default, Deserialize)]
struct LocatorInfoDocument {
    #[serde(rename = "type")]
    item_type: Option<String>,
}

fn bundled_locator_nodes(
    document: &Value,
    prefix: &str,
    document_path: Option<&Path>,
) -> Vec<LocatorNode> {
    let document: LocatorItemsDocument = serde_yaml_ng::from_value(document.clone())
        .expect("successfully parsed document must retain an object root");
    locator_nodes_from_items(document.items, prefix, document_path, &[])
}

fn locator_nodes_from_items(
    items: Vec<Value>,
    prefix: &str,
    document_path: Option<&Path>,
    parent_path: &[usize],
) -> Vec<LocatorNode> {
    items
        .into_iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let mut item_path = parent_path.to_vec();
            item_path.push(index);
            let item: LocatorItemDocument = serde_yaml_ng::from_value(value)
                .expect("successfully projected item must retain a valid object shape");
            match item.info.item_type.as_deref() {
                Some("http") => Some(LocatorNode::Request {
                    selector: format!("{prefix}/{index}"),
                    persistence: document_path.map(|path| RequestPersistence {
                        document_path: path.to_owned(),
                        item_path,
                    }),
                }),
                Some("folder") => Some(LocatorNode::Folder {
                    selector: format!("{prefix}/{index}"),
                    children: locator_nodes_from_items(
                        item.items,
                        &format!("{prefix}/{index}/items"),
                        document_path,
                        &item_path,
                    ),
                }),
                _ => None,
            }
        })
        .collect()
}

fn index_locators(workspace: Workspace, nodes: &[LocatorNode]) -> LoadedWorkspace {
    let mut requests = Vec::new();
    let mut folders = Vec::new();
    index_locator_nodes(
        &workspace,
        workspace.root_items(),
        nodes,
        &mut requests,
        &mut folders,
    );
    let request_keys_by_selector = requests
        .iter()
        .map(|request: &LocatedRequest| (request.selector.clone(), request.key))
        .collect();
    let folder_keys_by_selector = folders
        .iter()
        .map(|folder: &LocatedFolder| (folder.selector.clone(), folder.key))
        .collect();
    let request_selectors_by_key = requests
        .iter()
        .map(|request: &LocatedRequest| (request.key, request.selector.clone()))
        .collect();
    let folder_selectors_by_key = folders
        .iter()
        .map(|folder: &LocatedFolder| (folder.key, folder.selector.clone()))
        .collect();
    LoadedWorkspace {
        workspace,
        requests,
        folders,
        request_keys_by_selector,
        folder_keys_by_selector,
        request_selectors_by_key,
        folder_selectors_by_key,
        environment_persistence: BTreeMap::new(),
        documents: BTreeMap::new(),
        source: WorkspaceSource::Memory,
    }
}

fn item_sequence(item: &CollectionItem) -> f64 {
    match item {
        CollectionItem::Folder(folder) => folder.metadata.sequence,
        CollectionItem::HttpRequest(request) => request.metadata.sequence,
    }
    .unwrap_or(f64::INFINITY)
}

fn locator_selector(node: &LocatorNode) -> &str {
    match node {
        LocatorNode::Folder { selector, .. } | LocatorNode::Request { selector, .. } => selector,
    }
}

fn index_locator_nodes(
    workspace: &Workspace,
    items: &[WorkspaceItemRef],
    nodes: &[LocatorNode],
    requests: &mut Vec<LocatedRequest>,
    folders: &mut Vec<LocatedFolder>,
) {
    assert_eq!(
        items.len(),
        nodes.len(),
        "locator tree must match workspace"
    );
    for (item, node) in items.iter().zip(nodes) {
        match (item, node) {
            (
                WorkspaceItemRef::Request(key),
                LocatorNode::Request {
                    selector,
                    persistence,
                },
            ) => {
                requests.push(LocatedRequest {
                    selector: selector.clone(),
                    key: *key,
                    persistence: persistence.clone(),
                });
            }
            (WorkspaceItemRef::Folder(key), LocatorNode::Folder { selector, children }) => {
                folders.push(LocatedFolder {
                    selector: selector.clone(),
                    key: *key,
                });
                let folder = workspace
                    .folder(*key)
                    .expect("workspace folder reference must resolve");
                index_locator_nodes(workspace, &folder.children, children, requests, folders);
            }
            _ => unreachable!("locator node type must match workspace item type"),
        }
    }
}

fn config_file(directory: &Path, stem: &str) -> Option<PathBuf> {
    ["yml", "yaml"]
        .into_iter()
        .map(|extension| directory.join(format!("{stem}.{extension}")))
        .find(|path| path.is_file())
}

fn read_directory(path: &Path) -> Result<Vec<fs::DirEntry>, LoadError> {
    fs::read_dir(path)
        .map_err(|source| LoadError::Io {
            path: path.to_owned(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| LoadError::Io {
            path: path.to_owned(),
            source,
        })
}

fn read_to_string(path: &Path) -> Result<String, LoadError> {
    fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_owned(),
        source,
    })
}

fn is_yaml_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("yml" | "yaml")
    )
}

pub(crate) fn relative_selector(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn persist_environment_yaml(
    persistence: &EnvironmentPersistence,
    original_source: &[u8],
    mutation: &EnvironmentYamlMutation,
) -> Result<Vec<u8>, SaveError> {
    let _save_lock = SaveLock::acquire(&persistence.document_path)?;
    let current = fs::read(&persistence.document_path).map_err(|source| SaveError::Io {
        path: persistence.document_path.clone(),
        source,
    })?;
    if current != original_source {
        return Err(SaveError::ConcurrentModification(
            persistence.document_path.clone(),
        ));
    }
    let mut document: Value = serde_yaml_ng::from_slice(original_source).map_err(|error| {
        SaveError::InvalidDocument(format!("retained source cannot be parsed: {error}"))
    })?;
    let environment = environment_document_mut(&mut document, persistence)?;
    apply_environment_mutation(environment, mutation)?;
    let serialized = serde_yaml_ng::to_string(&document)
        .map_err(SaveError::Serialize)?
        .into_bytes();
    atomic_write(&persistence.document_path, &serialized, original_source)?;
    Ok(serialized)
}

fn persist_bundled_environment_create(
    document_path: &Path,
    original_source: &[u8],
    environment: &Environment,
) -> Result<Vec<u8>, SaveError> {
    let _save_lock = SaveLock::acquire(document_path)?;
    let current = fs::read(document_path).map_err(|source| SaveError::Io {
        path: document_path.to_owned(),
        source,
    })?;
    if current != original_source {
        return Err(SaveError::ConcurrentModification(document_path.to_owned()));
    }
    let mut document: Value = serde_yaml_ng::from_slice(original_source).map_err(|error| {
        SaveError::InvalidDocument(format!("retained source cannot be parsed: {error}"))
    })?;
    let mapping = document.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the collection document is not a mapping".to_owned())
    })?;
    let config = mapping_child(mapping, "config")?;
    let environments = config
        .get_mut(string_key("environments"))
        .and_then(Value::as_sequence_mut);
    let environments = match environments {
        Some(environments) => environments,
        None => {
            let sequence = Value::Sequence(Vec::new());
            config.insert(string_key("environments"), sequence.clone());
            config
                .get_mut(string_key("environments"))
                .and_then(Value::as_sequence_mut)
                .expect("environments sequence must exist after insertion")
        }
    };
    environments.push(environment_value(environment));
    let serialized = serde_yaml_ng::to_string(&document)
        .map_err(SaveError::Serialize)?
        .into_bytes();
    atomic_write(document_path, &serialized, original_source)?;
    Ok(serialized)
}

fn persist_unbundled_environment_create(
    root: &Path,
    environment: &Environment,
) -> Result<(PathBuf, Vec<u8>), SaveError> {
    let directory = root.join("environments");
    if !directory.exists() {
        fs::create_dir_all(&directory).map_err(|source| SaveError::Io {
            path: directory.clone(),
            source,
        })?;
    }
    let document_path = directory.join(format!("{}.yml", environment.name));
    if document_path.exists() {
        return Err(SaveError::ConcurrentModification(document_path));
    }
    let serialized = serde_yaml_ng::to_string(&environment_value(environment))
        .map_err(SaveError::Serialize)?
        .into_bytes();
    write_new_environment_file(&document_path, &serialized)?;
    Ok((document_path, serialized))
}

fn write_new_environment_file(path: &Path, contents: &[u8]) -> Result<(), SaveError> {
    let map_io = |source| SaveError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(map_io)?;
    }

    let (temporary_path, mut file) = create_unique_temporary_save_file(path)?;
    let write_result = file.write_all(contents).and_then(|()| file.sync_all());
    drop(file);
    let write_result = write_result.and_then(|()| fs::hard_link(&temporary_path, path));
    let cleanup_result = fs::remove_file(&temporary_path);

    match write_result {
        Ok(()) => {
            let _ = cleanup_result;
            Ok(())
        }
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            let _ = cleanup_result;
            Err(SaveError::ConcurrentModification(path.to_owned()))
        }
        Err(source) => {
            let _ = cleanup_result;
            Err(map_io(source))
        }
    }
}

fn create_unique_temporary_save_file(path: &Path) -> Result<(PathBuf, fs::File), SaveError> {
    create_unique_temporary_file(path).map_err(create_error_to_save_error)
}

fn create_error_to_save_error(error: CreateError) -> SaveError {
    match error {
        CreateError::Serialize(error) => SaveError::Serialize(error),
        CreateError::Io { path, source } => SaveError::Io { path, source },
        CreateError::AlreadyExists(path) => SaveError::ConcurrentModification(path),
        CreateError::IsDirectory(path) => {
            SaveError::InvalidDocument(format!("{} is a directory", path.display()))
        }
        CreateError::Load(error) => SaveError::InvalidDocument(error.to_string()),
    }
}

fn environment_document_mut<'a>(
    document: &'a mut Value,
    persistence: &EnvironmentPersistence,
) -> Result<&'a mut Value, SaveError> {
    let Some(index) = persistence.bundled_index else {
        return Ok(document);
    };
    let mapping = document.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the collection document is not a mapping".to_owned())
    })?;
    let config = mapping_child(mapping, "config")?;
    let environments = config
        .get_mut(string_key("environments"))
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| {
            SaveError::InvalidDocument("collection config has no environments sequence".to_owned())
        })?;
    environments.get_mut(index).ok_or_else(|| {
        SaveError::InvalidDocument(format!("environment index {index} is out of bounds"))
    })
}

fn apply_environment_mutation(
    environment: &mut Value,
    mutation: &EnvironmentYamlMutation,
) -> Result<(), SaveError> {
    match mutation {
        EnvironmentYamlMutation::Set { variable } => {
            apply_environment_variable_set(environment, variable)
        }
        EnvironmentYamlMutation::Unset { name } => {
            apply_environment_variable_unset(environment, name)
        }
    }
}

fn apply_environment_variable_set(
    environment: &mut Value,
    variable: &Variable,
) -> Result<(), SaveError> {
    let name = variable.name.as_deref().ok_or_else(|| {
        SaveError::InvalidDocument("updated environment variable has no name".to_owned())
    })?;
    let mapping = environment.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the environment document is not a mapping".to_owned())
    })?;
    let variables = sequence_child(mapping, "variables")?;
    if let Some(existing) = variables.iter_mut().find_map(|entry| {
        let mapping = entry.as_mapping_mut()?;
        (yaml_string_field(mapping, "name") == Some(name)).then_some(mapping)
    }) {
        if yaml_bool_field(existing, "secret") == Some(true) {
            return Err(SaveError::InvalidDocument(format!(
                "cannot overwrite secret variable '{name}'"
            )));
        }
        existing.insert(string_key("disabled"), Value::Bool(variable.disabled));
        merge_environment_variable_value(existing, variable);
        return Ok(());
    }
    variables.push(new_environment_variable_value(variable));
    Ok(())
}

fn apply_environment_variable_unset(environment: &mut Value, name: &str) -> Result<(), SaveError> {
    let mapping = environment.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the environment document is not a mapping".to_owned())
    })?;
    let variables = sequence_child(mapping, "variables")?;
    let Some(index) = variables.iter().position(|entry| {
        entry
            .as_mapping()
            .is_some_and(|mapping| yaml_string_field(mapping, "name") == Some(name))
    }) else {
        return Err(SaveError::InvalidDocument(format!(
            "variable '{name}' is missing from the retained environment document"
        )));
    };
    if variables[index]
        .as_mapping()
        .is_some_and(|mapping| yaml_bool_field(mapping, "secret") == Some(true))
    {
        return Err(SaveError::InvalidDocument(format!(
            "cannot unset secret variable '{name}'"
        )));
    }
    variables.remove(index);
    Ok(())
}

fn merge_environment_variable_value(existing: &mut serde_yaml_ng::Mapping, variable: &Variable) {
    match (&variable.value, existing.get_mut(string_key("value"))) {
        (Some(VariableValueSet::Variants(variants)), Some(Value::Sequence(existing_variants))) => {
            merge_variable_variants(existing_variants, variants);
        }
        (Some(value), Some(existing_value)) => {
            merge_yaml_variable_value(existing_value, value);
        }
        (Some(value), None) => {
            existing.insert(string_key("value"), variable_value_set_yaml(value));
        }
        (None, _) => {
            existing.remove(string_key("value"));
        }
    }
}

fn merge_variable_variants(existing: &mut [Value], variants: &[VariableValueVariant]) {
    for (index, variant) in variants.iter().enumerate() {
        let Some(existing) = existing.get_mut(index).and_then(Value::as_mapping_mut) else {
            continue;
        };
        existing.insert(string_key("selected"), Value::Bool(variant.selected));
        match existing.get_mut(string_key("value")) {
            Some(existing_value) => {
                merge_yaml_variable_value(
                    existing_value,
                    &VariableValueSet::Single(variant.value.clone()),
                );
            }
            None => {
                existing.insert(string_key("value"), variable_value_yaml(&variant.value));
            }
        }
    }
}

fn merge_yaml_variable_value(existing: &mut Value, value: &VariableValueSet) {
    match (existing, value) {
        (Value::Mapping(existing), VariableValueSet::Single(VariableValue::Typed { data, .. }))
            if existing.contains_key(string_key("data")) =>
        {
            existing.insert(string_key("data"), Value::String(data.clone()));
        }
        (Value::Mapping(existing), VariableValueSet::Single(VariableValue::String(data)))
            if existing.contains_key(string_key("data")) =>
        {
            existing.insert(string_key("data"), Value::String(data.clone()));
        }
        (existing, value) => *existing = variable_value_set_yaml(value),
    }
}

fn new_environment_variable_value(variable: &Variable) -> Value {
    let mut mapping = serde_yaml_ng::Mapping::new();
    if let Some(name) = &variable.name {
        mapping.insert(string_key("name"), Value::String(name.clone()));
    }
    if let Some(value) = &variable.value {
        mapping.insert(string_key("value"), variable_value_set_yaml(value));
    }
    if variable.disabled {
        mapping.insert(string_key("disabled"), Value::Bool(true));
    }
    Value::Mapping(mapping)
}

fn variable_value_set_yaml(value: &VariableValueSet) -> Value {
    match value {
        VariableValueSet::Single(value) => variable_value_yaml(value),
        VariableValueSet::Variants(variants) => Value::Sequence(
            variants
                .iter()
                .map(|variant| {
                    map([
                        ("title", Value::String(variant.title.clone())),
                        ("selected", Value::Bool(variant.selected)),
                        ("value", variable_value_yaml(&variant.value)),
                    ])
                })
                .collect(),
        ),
    }
}

fn variable_value_yaml(value: &VariableValue) -> Value {
    match value {
        VariableValue::String(value) => Value::String(value.clone()),
        VariableValue::Typed { kind, data } => map([
            ("type", Value::String(kind.as_str().to_owned())),
            ("data", Value::String(data.clone())),
        ]),
    }
}

fn sequence_child<'a>(
    parent: &'a mut serde_yaml_ng::Mapping,
    name: &str,
) -> Result<&'a mut Vec<Value>, SaveError> {
    let key = string_key(name);
    if !parent.contains_key(&key) {
        parent.insert(key.clone(), Value::Sequence(Vec::new()));
    }
    parent
        .get_mut(&key)
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| SaveError::InvalidDocument(format!("'{name}' is not a sequence")))
}

fn yaml_string_field<'a>(mapping: &'a serde_yaml_ng::Mapping, name: &str) -> Option<&'a str> {
    mapping.get(string_key(name)).and_then(Value::as_str)
}

fn yaml_bool_field(mapping: &serde_yaml_ng::Mapping, name: &str) -> Option<bool> {
    mapping.get(string_key(name)).and_then(Value::as_bool)
}

fn request_document_mut<'a>(
    document: &'a mut Value,
    item_path: &[usize],
) -> Result<&'a mut Value, SaveError> {
    let mut current = document;
    for index in item_path {
        let mapping = current.as_mapping_mut().ok_or_else(|| {
            SaveError::InvalidDocument("an item parent is not a mapping".to_owned())
        })?;
        let items = mapping
            .get_mut(Value::String("items".to_owned()))
            .and_then(Value::as_sequence_mut)
            .ok_or_else(|| {
                SaveError::InvalidDocument("an item parent has no items sequence".to_owned())
            })?;
        current = items.get_mut(*index).ok_or_else(|| {
            SaveError::InvalidDocument(format!("item index {index} is out of bounds"))
        })?;
    }
    Ok(current)
}

fn apply_request_update(document: &mut Value, update: &RequestUpdate) -> Result<(), SaveError> {
    let request = document.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the request item is not a mapping".to_owned())
    })?;

    if let Some(name) = &update.name {
        let info = mapping_child(request, "info")?;
        info.insert(
            Value::String("name".to_owned()),
            Value::String(name.clone()),
        );
    }
    if update.method.is_some()
        || update.url.is_some()
        || update.headers.is_some()
        || update.query_parameters.is_some()
        || update.path_parameters.is_some()
        || update.body.is_some()
        || update.authentication.is_some()
    {
        let http = mapping_child(request, "http")?;
        if let Some(method) = &update.method {
            http.insert(
                Value::String("method".to_owned()),
                Value::String(method.clone()),
            );
        }
        if let Some(url) = &update.url {
            http.insert(Value::String("url".to_owned()), Value::String(url.clone()));
        }
        if let Some(headers) = &update.headers {
            merge_sequence_preserving(
                http,
                "headers",
                headers.iter().map(header_value).collect(),
                &[],
            );
        }
        if update.query_parameters.is_some() || update.path_parameters.is_some() {
            merge_parameters(
                http,
                update.query_parameters.as_deref(),
                update.path_parameters.as_deref(),
            );
        }
        if let Some(body) = &update.body {
            set_optional_merged(http, "body", body.as_ref().map(request_body_value));
        }
        if let Some(authentication) = &update.authentication {
            set_optional(
                http,
                "auth",
                authentication.as_ref().map(authentication_value),
            );
        }
    }
    Ok(())
}

fn string_key(name: &str) -> Value {
    Value::String(name.to_owned())
}

fn set_optional(mapping: &mut serde_yaml_ng::Mapping, name: &str, value: Option<Value>) {
    let key = string_key(name);
    if let Some(value) = value {
        mapping.insert(key, value);
    } else {
        mapping.remove(&key);
    }
}

fn set_optional_merged(mapping: &mut serde_yaml_ng::Mapping, name: &str, value: Option<Value>) {
    let key = string_key(name);
    if let Some(mut value) = value {
        if let Some(existing) = mapping.remove(&key) {
            merge_yaml(&mut value, existing);
        }
        mapping.insert(key, value);
    } else {
        mapping.remove(&key);
    }
}

fn merge_yaml(replacement: &mut Value, existing: Value) {
    match (replacement, existing) {
        (Value::Mapping(replacement), Value::Mapping(existing)) => {
            for (key, old_value) in existing {
                if let Some(new_value) = replacement.get_mut(&key) {
                    merge_yaml(new_value, old_value);
                } else {
                    replacement.insert(key, old_value);
                }
            }
        }
        (Value::Sequence(replacement), Value::Sequence(existing)) => {
            for (new_value, old_value) in replacement.iter_mut().zip(existing) {
                merge_yaml(new_value, old_value);
            }
        }
        _ => {}
    }
}

fn map(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Mapping(
        entries
            .into_iter()
            .map(|(key, value)| (string_key(key), value))
            .collect(),
    )
}

fn merge_sequence_preserving(
    parent: &mut serde_yaml_ng::Mapping,
    name: &str,
    replacements: Vec<Value>,
    preserved_keys: &[&str],
) {
    let key = string_key(name);
    let existing = parent
        .get(&key)
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let merged = replacements
        .into_iter()
        .enumerate()
        .map(|(index, replacement)| {
            let Some(Value::Mapping(mut old)) = existing.get(index).cloned() else {
                return replacement;
            };
            let Value::Mapping(new) = replacement else {
                return replacement;
            };
            for (key, value) in new {
                if preserved_keys
                    .iter()
                    .any(|preserved| key == string_key(preserved))
                    && old.contains_key(&key)
                {
                    continue;
                }
                old.insert(key, value);
            }
            Value::Mapping(old)
        })
        .collect();
    parent.insert(key, Value::Sequence(merged));
}

fn header_value(header: &Header) -> Value {
    map([
        ("name", Value::String(header.name.clone())),
        ("value", Value::String(header.value.clone())),
        ("disabled", Value::Bool(header.disabled)),
    ])
}

fn query_parameter_value(parameter: &QueryParameter) -> Value {
    parameter_value(parameter, "query")
}

fn path_parameter_value(parameter: &QueryParameter) -> Value {
    parameter_value(parameter, "path")
}

fn parameter_value(parameter: &QueryParameter, parameter_type: &str) -> Value {
    map([
        ("name", Value::String(parameter.name.clone())),
        ("value", Value::String(parameter.value.clone())),
        ("type", Value::String(parameter_type.to_owned())),
        ("disabled", Value::Bool(parameter.disabled)),
    ])
}

fn merge_parameters(
    parent: &mut serde_yaml_ng::Mapping,
    query: Option<&[QueryParameter]>,
    path: Option<&[QueryParameter]>,
) {
    let key = string_key("params");
    let existing = parent
        .get(&key)
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let mut query = query.map(|values| values.iter().map(query_parameter_value));
    let mut path = path.map(|values| values.iter().map(path_parameter_value));
    let mut merged = Vec::new();

    for old in existing {
        let parameter_type = old
            .as_mapping()
            .and_then(|mapping| mapping.get(string_key("type")))
            .and_then(Value::as_str);
        let replacement = match parameter_type {
            Some("query") => query.as_mut().map(Iterator::next),
            Some("path") => path.as_mut().map(Iterator::next),
            _ => None,
        };
        match replacement {
            Some(Some(mut replacement)) => {
                merge_yaml(&mut replacement, old);
                merged.push(replacement);
            }
            Some(None) => {}
            None => merged.push(old),
        }
    }
    if let Some(values) = query {
        merged.extend(values);
    }
    if let Some(values) = path {
        merged.extend(values);
    }
    parent.insert(key, Value::Sequence(merged));
}

fn request_body_value(body: &RequestBody) -> Value {
    match body {
        RequestBody::Single(body) => body_value(body),
        RequestBody::Variants(variants) => Value::Sequence(
            variants
                .iter()
                .map(|variant| {
                    map([
                        ("title", Value::String(variant.title.clone())),
                        ("selected", Value::Bool(variant.selected)),
                        ("body", body_value(&variant.body)),
                    ])
                })
                .collect(),
        ),
    }
}

fn body_value(body: &Body) -> Value {
    match body {
        Body::Raw(body) => map([
            (
                "type",
                Value::String(
                    match body.kind {
                        RawBodyKind::Json => "json",
                        RawBodyKind::Text => "text",
                        RawBodyKind::Xml => "xml",
                        RawBodyKind::Sparql => "sparql",
                    }
                    .to_owned(),
                ),
            ),
            ("data", Value::String(body.data.clone())),
        ]),
        Body::FormUrlEncoded(fields) => map([
            ("type", Value::String("form-urlencoded".to_owned())),
            (
                "data",
                Value::Sequence(fields.iter().map(form_field_value).collect()),
            ),
        ]),
        Body::Multipart(parts) => map([
            ("type", Value::String("multipart-form".to_owned())),
            (
                "data",
                Value::Sequence(parts.iter().map(multipart_part_value).collect()),
            ),
        ]),
        Body::File(files) => map([
            ("type", Value::String("file".to_owned())),
            (
                "data",
                Value::Sequence(files.iter().map(file_reference_value).collect()),
            ),
        ]),
    }
}

fn form_field_value(field: &FormField) -> Value {
    map([
        ("name", Value::String(field.name.clone())),
        ("value", Value::String(field.value.clone())),
        ("disabled", Value::Bool(field.disabled)),
    ])
}

fn multipart_part_value(part: &MultipartPart) -> Value {
    let mut value = match map([
        ("name", Value::String(part.name.clone())),
        (
            "type",
            Value::String(
                match part.kind {
                    MultipartPartKind::Text => "text",
                    MultipartPartKind::File => "file",
                }
                .to_owned(),
            ),
        ),
        (
            "value",
            match &part.value {
                MultipartValue::Single(value) => Value::String(value.clone()),
                MultipartValue::Multiple(values) => {
                    Value::Sequence(values.iter().cloned().map(Value::String).collect())
                }
            },
        ),
        ("disabled", Value::Bool(part.disabled)),
    ]) {
        Value::Mapping(value) => value,
        _ => unreachable!(),
    };
    value.insert(
        string_key("contentType"),
        part.content_type
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    Value::Mapping(value)
}

fn file_reference_value(file: &FileReference) -> Value {
    map([
        ("filePath", Value::String(file.file_path.clone())),
        ("contentType", Value::String(file.content_type.clone())),
        ("selected", Value::Bool(file.selected)),
    ])
}

fn authentication_value(authentication: &Authentication) -> Value {
    if authentication.kind == AuthenticationKind::Inherit {
        return Value::String("inherit".to_owned());
    }
    let mut value = serde_yaml_ng::Mapping::new();
    value.insert(
        string_key("type"),
        Value::String(authentication.kind.as_str().to_owned()),
    );
    value.extend(
        authentication
            .properties
            .iter()
            .map(|(name, value)| (Value::String(name.clone()), auth_property_value(value))),
    );
    Value::Mapping(value)
}

fn auth_property_value(value: &AuthenticationValue) -> Value {
    match value {
        AuthenticationValue::String(value) => Value::String(value.clone()),
        AuthenticationValue::Number(value) => {
            serde_yaml_ng::from_str(value).unwrap_or_else(|_| Value::String(value.clone()))
        }
        AuthenticationValue::Boolean(value) => Value::Bool(*value),
        AuthenticationValue::Null => Value::Null,
        AuthenticationValue::Sequence(values) => {
            Value::Sequence(values.iter().map(auth_property_value).collect())
        }
        AuthenticationValue::Object(values) => Value::Mapping(
            values
                .iter()
                .map(|(name, value)| (Value::String(name.clone()), auth_property_value(value)))
                .collect(),
        ),
    }
}

fn mapping_child<'a>(
    parent: &'a mut serde_yaml_ng::Mapping,
    name: &str,
) -> Result<&'a mut serde_yaml_ng::Mapping, SaveError> {
    let key = Value::String(name.to_owned());
    if !parent.contains_key(&key) {
        parent.insert(key.clone(), Value::Mapping(serde_yaml_ng::Mapping::new()));
    }
    parent
        .get_mut(&key)
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| SaveError::InvalidDocument(format!("'{name}' is not a mapping")))
}

pub(crate) fn atomic_write(
    path: &Path,
    contents: &[u8],
    expected_source: &[u8],
) -> Result<(), SaveError> {
    let map_io = |source| SaveError::Io {
        path: path.to_owned(),
        source,
    };
    let mut file = AtomicWriteFile::open(path).map_err(map_io)?;
    file.write_all(contents).map_err(map_io)?;
    file.sync_all().map_err(map_io)?;
    let current = fs::read(path).map_err(map_io)?;
    if current != expected_source {
        return Err(SaveError::ConcurrentModification(path.to_owned()));
    }
    file.commit().map_err(map_io)?;
    Ok(())
}

pub(crate) struct SaveLock {
    file: fs::File,
}

impl SaveLock {
    pub(crate) fn acquire(destination: &Path) -> Result<Self, SaveError> {
        let directory = std::env::temp_dir().join("probe-persistence-locks");
        fs::create_dir_all(&directory).map_err(|source| SaveError::Io {
            path: destination.to_owned(),
            source,
        })?;
        let path = directory.join(format!("{:016x}.lock", stable_path_hash(destination)));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|source| SaveError::Io {
                path: destination.to_owned(),
                source,
            })?;
        #[cfg(any(unix, windows))]
        FileExt::lock(&file).map_err(|source| SaveError::Io {
            path: destination.to_owned(),
            source,
        })?;
        Ok(Self { file })
    }
}

fn stable_path_hash(path: &Path) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        for byte in path.as_os_str().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for value in path.as_os_str().encode_wide() {
            for byte in value.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

impl Drop for SaveLock {
    fn drop(&mut self) {
        #[cfg(any(unix, windows))]
        let _ = FileExt::unlock(&self.file);
    }
}
