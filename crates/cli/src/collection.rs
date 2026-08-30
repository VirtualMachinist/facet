use std::{io::Read, path::PathBuf};

use probe_opencollection::{
    LoadedWorkspace, create_bundled_workspace, create_bundled_workspace_from_collection,
};
use probe_postman::inspect_postman_source;
use probe_yaak::inspect_yaak_source;
use serde_json::{Value, json};

use crate::{CliError, CommandOutput, WorkspaceInput, error::import_diagnostic_json, load};

pub(crate) fn create(path: PathBuf, name: Option<String>) -> Result<CommandOutput, CliError> {
    let loaded =
        create_bundled_workspace(&path, name.as_deref(), false).map_err(CliError::create)?;
    let workspace = loaded.workspace();
    let collection_name = workspace.metadata().name.as_deref().unwrap_or("<unnamed>");
    let created_path = source_path_or(loaded.source_path(), path);
    Ok(CommandOutput {
        human: format!(
            "Created bundled OpenCollection workspace\nName: {collection_name}\nPath: {}\n",
            created_path.display()
        ),
        json: json!({
            "collection": { "name": workspace.metadata().name },
            "counts": workspace_counts(&loaded),
            "created": true,
            "path": created_path,
        }),
    })
}

pub(crate) fn import_yaak(
    source: PathBuf,
    destination: PathBuf,
    workspace_id: Option<&str>,
    allow_partial: bool,
) -> Result<CommandOutput, CliError> {
    let preview = inspect_yaak_source(&source).map_err(CliError::yaak)?;
    let source_format = preview.format();
    let imported = preview
        .convert(workspace_id, allow_partial)
        .map_err(CliError::yaak)?;
    let loaded = create_bundled_workspace_from_collection(&destination, &imported.collection)
        .map_err(CliError::create)?;
    let path = source_path_or(loaded.source_path(), destination);
    let warning_count = imported.diagnostics.len();
    Ok(CommandOutput {
        human: format!(
            "Imported Yaak workspace\nName: {}\nPath: {}\nRequests: {}\nFolders: {}\nEnvironments: {}\nWarnings: {warning_count}\n",
            imported.workspace.name,
            path.display(),
            loaded.workspace().request_count(),
            loaded.workspace().folder_count(),
            loaded.workspace().environments().len(),
        ),
        json: json!({
            "imported": true,
            "partial": imported.partial,
            "sourceFormat": source_format.as_str(),
            "workspace": {
                "id": imported.workspace.id,
                "name": imported.workspace.name,
            },
            "path": path,
            "counts": workspace_counts(&loaded),
            "warnings": imported.diagnostics.iter().map(import_diagnostic_json).collect::<Vec<_>>(),
        }),
    })
}

pub(crate) fn import_postman(
    source: PathBuf,
    destination: PathBuf,
    allow_partial: bool,
) -> Result<CommandOutput, CliError> {
    let preview = inspect_postman_source(&source).map_err(CliError::postman)?;
    let source_format = preview.format();
    let imported = preview.convert(allow_partial).map_err(CliError::postman)?;
    let loaded = create_bundled_workspace_from_collection(&destination, &imported.collection)
        .map_err(CliError::create)?;
    let path = source_path_or(loaded.source_path(), destination);
    let warning_count = imported.diagnostics.len();
    let environment = imported.collection_variables_environment;
    Ok(CommandOutput {
        human: format!(
            "Imported Postman collection\nName: {}\nPath: {}\nRequests: {}\nFolders: {}\nEnvironments: {}\nCollection variables environment: {}\nWarnings: {warning_count}\n",
            imported.source.name,
            path.display(),
            loaded.workspace().request_count(),
            loaded.workspace().folder_count(),
            loaded.workspace().environments().len(),
            environment.as_deref().unwrap_or("none"),
        ),
        json: json!({
            "imported": true,
            "partial": imported.partial,
            "sourceFormat": source_format.as_str(),
            "collection": {
                "id": imported.source.id,
                "name": imported.source.name,
            },
            "collectionVariablesEnvironment": environment,
            "path": path,
            "counts": workspace_counts(&loaded),
            "warnings": imported.diagnostics.iter().map(import_diagnostic_json).collect::<Vec<_>>(),
        }),
    })
}

pub(crate) fn validate(
    input: &WorkspaceInput,
    stdin: &mut impl Read,
) -> Result<CommandOutput, CliError> {
    let loaded = load(input, stdin)?;
    let workspace = loaded.workspace();
    let name = workspace.metadata().name.as_deref().unwrap_or("<unnamed>");
    Ok(CommandOutput {
        human: format!(
            "Valid OpenCollection workspace\nName: {name}\nRequests: {}\nFolders: {}\nEnvironments: {}\n",
            workspace.request_count(),
            workspace.folder_count(),
            workspace.environments().len()
        ),
        json: json!({
            "collection": {
                "name": workspace.metadata().name,
                "summary": workspace.metadata().summary,
                "version": workspace.metadata().version,
            },
            "counts": workspace_counts(&loaded),
            "valid": true,
        }),
    })
}

fn source_path_or(source_path: Option<&std::path::Path>, fallback: PathBuf) -> PathBuf {
    source_path.map(PathBuf::from).unwrap_or(fallback)
}

fn workspace_counts(loaded: &LoadedWorkspace) -> Value {
    let workspace = loaded.workspace();
    json!({
        "environments": workspace.environments().len(),
        "folders": workspace.folder_count(),
        "requests": workspace.request_count(),
    })
}
