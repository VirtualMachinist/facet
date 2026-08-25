use std::{fs, fs::OpenOptions, path::Path};

#[cfg(any(unix, windows))]
use fs4::FileExt;

use super::SaveError;

pub(crate) struct SaveLock {
    file: fs::File,
}

impl SaveLock {
    pub(crate) fn acquire(destination: &Path) -> Result<Self, SaveError> {
        let directory = std::env::temp_dir().join("probe-persistence-locks");
        fs::create_dir_all(&directory).map_err(|source| SaveError::Io {
            path: destination.to_owned(),
            source,
        })?;
        let path = directory.join(format!("{:016x}.lock", stable_path_hash(destination)));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|source| SaveError::Io {
                path: destination.to_owned(),
                source,
            })?;
        #[cfg(any(unix, windows))]
        FileExt::lock(&file).map_err(|source| SaveError::Io {
            path: destination.to_owned(),
            source,
        })?;
        Ok(Self { file })
    }
}

fn stable_path_hash(path: &Path) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        for byte in path.as_os_str().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for value in path.as_os_str().encode_wide() {
            for byte in value.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

impl Drop for SaveLock {
    fn drop(&mut self) {
        #[cfg(any(unix, windows))]
        let _ = FileExt::unlock(&self.file);
    }
}
