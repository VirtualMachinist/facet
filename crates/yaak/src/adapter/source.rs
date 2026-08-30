use std::{collections::BTreeSet, fs, path::Path};

use serde::Deserialize;
use serde_json::Value;

use super::*;

pub(super) fn inspect(path: &Path) -> Result<YaakImportPreview, YaakImportError> {
    if path.is_dir() {
        inspect_sync_directory(path)
    } else {
        inspect_export_file(path)
    }
}

fn inspect_export_file(path: &Path) -> Result<YaakImportPreview, YaakImportError> {
    let source = fs::read_to_string(path).map_err(|source| YaakImportError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut document: ExportDocument = serde_json::from_str(&source)
        .map_err(|error| YaakImportError::Invalid(format!("invalid Yaak export JSON: {error}")))?;
    if !(1..=4).contains(&document.yaak_schema) {
        return Err(YaakImportError::Invalid(format!(
            "unsupported Yaak export schema {}; supported schemas are 1 through 4",
            document.yaak_schema
        )));
    }
    migrate_export(&mut document);
    let mut diagnostics = Vec::new();
    convert::diagnose_extra_fields("export", None, &document.extra, &mut diagnostics);
    convert::diagnose_extra_fields(
        "resources",
        None,
        &document.resources.extra,
        &mut diagnostics,
    );
    validate_resources(&document.resources)?;
    Ok(YaakImportPreview {
        format: YaakSourceFormat::ExportJson,
        resources: document.resources,
        diagnostics,
    })
}

fn inspect_sync_directory(path: &Path) -> Result<YaakImportPreview, YaakImportError> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| YaakImportError::Io {
            path: path.to_owned(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| YaakImportError::Io {
            path: path.to_owned(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut resources = Resources::default();
    for entry in entries {
        let entry_path = entry.path();
        if !entry
            .file_type()
            .map_err(|source| YaakImportError::Io {
                path: entry_path.clone(),
                source,
            })?
            .is_file()
        {
            continue;
        }
        let extension = entry_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "yaml" | "yml" | "json") {
            continue;
        }
        let value = read_sync_value(&entry_path, &extension)?;
        let Some(model) = value.get("model").and_then(Value::as_str) else {
            continue;
        };
        if value.get("id").and_then(Value::as_str).is_none() {
            return Err(YaakImportError::Invalid(format!(
                "Yaak sync model {} is missing id",
                entry_path.display()
            )));
        }
        match model {
            "workspace" => resources.workspaces.push(from_value(value, &entry_path)?),
            "environment" => resources.environments.push(from_value(value, &entry_path)?),
            "folder" => resources.folders.push(from_value(value, &entry_path)?),
            "http_request" => resources
                .http_requests
                .push(from_value(value, &entry_path)?),
            "grpc_request" => resources
                .grpc_requests
                .push(from_value(value, &entry_path)?),
            "websocket_request" => resources
                .websocket_requests
                .push(from_value(value, &entry_path)?),
            _ => resources
                .unsupported_resources
                .push(from_value(value, &entry_path)?),
        }
    }
    migrate_resources(&mut resources);
    validate_resources(&resources)?;
    Ok(YaakImportPreview {
        format: YaakSourceFormat::SyncDirectory,
        resources,
        diagnostics: Vec::new(),
    })
}

fn read_sync_value(path: &Path, extension: &str) -> Result<Value, YaakImportError> {
    let source = fs::read_to_string(path).map_err(|source| YaakImportError::Io {
        path: path.to_owned(),
        source,
    })?;
    if extension == "json" {
        serde_json::from_str(&source).map_err(|error| invalid_sync_model(path, error))
    } else {
        let yaml: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&source).map_err(|error| invalid_sync_model(path, error))?;
        serde_json::to_value(yaml).map_err(|error| invalid_sync_model(path, error))
    }
}

fn invalid_sync_model(path: &Path, error: impl std::fmt::Display) -> YaakImportError {
    YaakImportError::Invalid(format!(
        "invalid Yaak sync model {}: {error}",
        path.display()
    ))
}

fn from_value<T: for<'de> Deserialize<'de>>(
    value: Value,
    path: &Path,
) -> Result<T, YaakImportError> {
    serde_json::from_value(value).map_err(|error| {
        YaakImportError::Invalid(format!(
            "invalid Yaak sync model {}: {error}",
            path.display()
        ))
    })
}

fn migrate_export(document: &mut ExportDocument) {
    document
        .resources
        .http_requests
        .append(&mut document.resources.requests);
    migrate_resources(&mut document.resources);
}

fn migrate_resources(resources: &mut Resources) {
    let mut generated = Vec::new();
    for workspace in &mut resources.workspaces {
        if !workspace.variables.is_empty() {
            let id = format!("GENERATE_ID::base_env_{}", workspace.id);
            generated.push(YaakEnvironment {
                model: "environment".to_owned(),
                id,
                workspace_id: workspace.id.clone(),
                name: "Global Variables".to_owned(),
                parent_model: "workspace".to_owned(),
                variables: std::mem::take(&mut workspace.variables),
                ..YaakEnvironment::default()
            });
        }
    }
    resources.environments.extend(generated);
    for environment in &mut resources.environments {
        if environment.parent_model.is_empty() {
            environment.parent_model = match environment.base {
                Some(true) => "workspace",
                _ => "environment",
            }
            .to_owned();
        }
    }
}

fn validate_resources(resources: &Resources) -> Result<(), YaakImportError> {
    if resources.workspaces.is_empty() {
        return Err(YaakImportError::Invalid(
            "Yaak source does not contain a workspace".to_owned(),
        ));
    }
    let mut ids = BTreeSet::new();
    for (model, id) in resources.identities() {
        if id.trim().is_empty() {
            return Err(YaakImportError::Invalid(format!(
                "Yaak {model} has an empty id"
            )));
        }
        if !ids.insert(id) {
            return Err(YaakImportError::Invalid(format!(
                "duplicate Yaak resource id: {id}"
            )));
        }
    }
    Ok(())
}
