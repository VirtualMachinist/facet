use std::{error::Error, fmt, io, path::PathBuf};

use crate::repository::SaveError;

use super::{ItemKind, StructureResult};

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
