use super::*;

type ReorderPlan<'a> = (&'a Path, Option<(&'a Path, Option<usize>)>);

pub(super) fn reorder_directories(plans: &[ReorderPlan<'_>]) -> Result<Vec<usize>, StructureError> {
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

pub(super) fn write_transaction_with(
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
pub(super) struct DiskChild {
    pub(super) path: PathBuf,
    pub(super) config: PathBuf,
    pub(super) sequence: f64,
}

pub(super) fn direct_children(directory: &Path) -> Result<Vec<DiskChild>, StructureError> {
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

pub(super) fn discover_documents(root: &Path) -> Result<BTreeSet<PathBuf>, StructureError> {
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
