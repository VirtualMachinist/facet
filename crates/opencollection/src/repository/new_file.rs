use std::{
    fs,
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
pub(super) enum NewFileError {
    AlreadyExists,
    Io(io::Error),
}

/// Durably publishes `contents` at a previously nonexistent path.
///
/// The final hard link is an atomic create-if-absent operation. Cleanup failure
/// after publication is deliberately ignored because the destination is already
/// durable and visible.
pub(super) fn write_new_file(path: &Path, contents: &[u8]) -> Result<(), NewFileError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(NewFileError::Io)?;
    }

    let (temporary_path, mut file) = create_unique_temporary_file(path)?;
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
            Err(NewFileError::AlreadyExists)
        }
        Err(source) => {
            let _ = cleanup_result;
            Err(NewFileError::Io(source))
        }
    }
}

fn create_unique_temporary_file(path: &Path) -> Result<(PathBuf, fs::File), NewFileError> {
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
            Err(source) => return Err(NewFileError::Io(source)),
        }
    }
    Err(NewFileError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique import temporary file",
    )))
}
