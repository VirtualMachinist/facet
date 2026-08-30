use std::{io::Read, path::PathBuf};

use probe_opencollection::{LoadedWorkspace, load_workspace, load_workspace_from_str};

use crate::CliError;

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

    pub(crate) fn base_directory(&self) -> Option<PathBuf> {
        match self {
            Self::Path(path) if path.is_dir() => Some(path.clone()),
            Self::Path(path) => path.parent().map(std::path::Path::to_owned),
            Self::Stdin => None,
        }
    }
}

pub(crate) fn load(
    input: &WorkspaceInput,
    stdin: &mut impl Read,
) -> Result<LoadedWorkspace, CliError> {
    match input {
        WorkspaceInput::Path(path) => load_workspace(path),
        WorkspaceInput::Stdin => {
            let mut source = String::new();
            stdin
                .read_to_string(&mut source)
                .map_err(|error| CliError::stdin(&error))?;
            load_workspace_from_str(&source)
        }
    }
    .map_err(|error| CliError::invalid_workspace(error.to_string()))
}
