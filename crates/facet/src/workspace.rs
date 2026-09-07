//! Workspace input (mirrors `probe-cli`) plus Lattice store location.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use lattice::WorkspaceStore;
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

pub(crate) use facet_record::ConfigOverrides;

/// Reads `--inline-body-max` / `--history-retention` from parsed arguments.
pub(crate) fn overrides_from_parsed(
    parsed: &crate::args::Parsed,
) -> Result<ConfigOverrides, FacetError> {
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
    Ok(ConfigOverrides {
        inline_body_max,
        history_retention,
    })
}

/// Opens or creates the store for `root` with the shared configuration path.
pub(crate) fn open_store(
    root: &Path,
    overrides: &ConfigOverrides,
) -> Result<WorkspaceStore, FacetError> {
    facet_record::open_store(root, overrides).map_err(FacetError::lattice)
}

/// Finds the workspace root for an optional path argument (default: cwd).
pub(crate) fn locate_root(path: Option<&str>) -> (PathBuf, Option<PathBuf>) {
    let start = path.map_or_else(|| PathBuf::from("."), PathBuf::from);
    let root = WorkspaceStore::discover(&start);
    (start, root)
}
