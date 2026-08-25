use std::{error::Error, fmt, io, path::PathBuf};

use probe_core::EnvironmentResolutionError;

use crate::ParseError;

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
