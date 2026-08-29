use super::*;

pub(super) fn mutate_unbundled(
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

pub(super) fn reorder_unbundled(
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

pub(super) fn rename_unbundled(
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

pub(super) fn delete_unbundled(
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

pub(super) fn duplicate_unbundled(
    root: &Path,
    selector: String,
) -> Result<StructureResult, StructureError> {
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

pub(super) fn deletion_tombstone(root: &Path) -> Result<PathBuf, StructureError> {
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

pub(super) fn finish_deletion(
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

pub(super) fn move_unbundled(
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

pub(super) fn path_rollback_error(
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
