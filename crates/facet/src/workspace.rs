//! Workspace input (mirrors `probe-cli`) plus Lattice store location.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use lattice::{LatticeConfig, Retention, WorkspaceStore};
use probe_opencollection::{LoadedWorkspace, load_workspace, load_workspace_from_str};

use crate::FacetError;

#[derive(Debug)]
pub(crate) enum WorkspaceInput {
    Path(PathBuf),
    Stdin,
}

impl WorkspaceInput {
    pub(crate) fn from_argument(argument: &str) -> Self {
        if argument == "-" {
            Self::Stdin
        } else {
            Self::Path(PathBuf::from(argument))
        }
    }

    /// Directory beside the collection: the workspace root for Lattice.
    pub(crate) fn base_directory(&self) -> Option<PathBuf> {
        match self {
            Self::Path(path) if path.is_dir() => Some(path.clone()),
            Self::Path(path) => Some(path.parent().map_or_else(
                || PathBuf::from("."),
                |parent| {
                    if parent.as_os_str().is_empty() {
                        PathBuf::from(".")
                    } else {
                        parent.to_owned()
                    }
                },
            )),
            Self::Stdin => None,
        }
    }
}

pub(crate) fn load(
    input: &WorkspaceInput,
    stdin: &mut impl Read,
) -> Result<LoadedWorkspace, FacetError> {
    match input {
        WorkspaceInput::Path(path) => load_workspace(path),
        WorkspaceInput::Stdin => {
            let mut source = String::new();
            stdin
                .read_to_string(&mut source)
                .map_err(|error| FacetError::stdin(&error))?;
            load_workspace_from_str(&source)
        }
    }
    .map_err(|error| FacetError::invalid_workspace(error.to_string()))
}

/// CLI overrides applied over the file-based configuration.
#[derive(Clone, Debug, Default)]
pub(crate) struct ConfigOverrides {
    pub(crate) inline_body_max: Option<u64>,
    pub(crate) history_retention: Option<Retention>,
}

impl ConfigOverrides {
    pub(crate) fn from_parsed(parsed: &crate::args::Parsed) -> Result<Self, FacetError> {
        let inline_body_max = parsed
            .value("--inline-body-max")?
            .map(lattice::parse_byte_size)
            .transpose()
            .map_err(|error| FacetError::invalid_arguments(error.to_string()))?;
        let history_retention = parsed
            .value("--history-retention")?
            .map(lattice::parse_retention)
            .transpose()
            .map_err(|error| FacetError::invalid_arguments(error.to_string()))?;
        Ok(Self {
            inline_body_max,
            history_retention,
        })
    }

    fn apply(&self, config: &mut LatticeConfig) {
        if let Some(value) = self.inline_body_max {
            config.inline_body_max = value;
        }
        if let Some(value) = self.history_retention {
            config.history_retention = value;
        }
    }
}

/// Loads configuration for `root` (machine file, then workspace file, then
/// CLI overrides) and opens or creates its store.
pub(crate) fn open_store(
    root: &Path,
    overrides: &ConfigOverrides,
) -> Result<WorkspaceStore, FacetError> {
    let mut config =
        LatticeConfig::load(root).map_err(|error| FacetError::lattice(error.into()))?;
    overrides.apply(&mut config);
    WorkspaceStore::open(root, config).map_err(FacetError::lattice)
}

/// Finds the workspace root for an optional path argument (default: cwd).
pub(crate) fn locate_root(path: Option<&str>) -> (PathBuf, Option<PathBuf>) {
    let start = path.map_or_else(|| PathBuf::from("."), PathBuf::from);
    let root = WorkspaceStore::discover(&start);
    (start, root)
}
