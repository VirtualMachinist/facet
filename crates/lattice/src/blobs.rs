//! Body placement: inline at or under the threshold, otherwise a
//! content-addressed file under `.facet/blobs/<sha256>`.

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{LatticeError, io_error};

/// A body offered to the store.
#[derive(Clone, Copy, Debug)]
pub enum BodyInput<'a> {
    /// No body.
    None,
    /// The body was not retained (for example, the transport bounded it);
    /// only its length is known.
    Unretained {
        /// Length in bytes.
        len: u64,
    },
    /// The body is in memory.
    Bytes(&'a [u8]),
    /// The complete body is in a file.
    File(&'a Path),
}

/// Where a body ended up.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoredBody {
    /// Body length in bytes, when known.
    pub len: Option<u64>,
    /// Inline bytes, when at or under the threshold.
    pub inline: Option<Vec<u8>>,
    /// Blob file hash, when over the threshold.
    pub hash: Option<String>,
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub(crate) fn place(
    blobs_dir: &Path,
    body: BodyInput<'_>,
    threshold: u64,
) -> Result<StoredBody, LatticeError> {
    match body {
        BodyInput::None => Ok(StoredBody::default()),
        BodyInput::Unretained { len } => Ok(StoredBody {
            len: Some(len),
            inline: None,
            hash: None,
        }),
        BodyInput::Bytes(bytes) => {
            let len = bytes.len() as u64;
            if len <= threshold {
                Ok(StoredBody {
                    len: Some(len),
                    inline: Some(bytes.to_vec()),
                    hash: None,
                })
            } else {
                let hash = sha256_hex(bytes);
                write_blob(blobs_dir, &hash, |sink| sink.write_all(bytes))?;
                Ok(StoredBody {
                    len: Some(len),
                    inline: None,
                    hash: Some(hash),
                })
            }
        }
        BodyInput::File(path) => {
            let len = fs::metadata(path)
                .map_err(|error| io_error(path, error))?
                .len();
            if len <= threshold {
                let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
                Ok(StoredBody {
                    len: Some(bytes.len() as u64),
                    inline: Some(bytes),
                    hash: None,
                })
            } else {
                let hash = hash_file(path)?;
                write_blob(blobs_dir, &hash, |sink| {
                    let mut source = fs::File::open(path)?;
                    io::copy(&mut source, sink).map(|_| ())
                })?;
                Ok(StoredBody {
                    len: Some(len),
                    inline: None,
                    hash: Some(hash),
                })
            }
        }
    }
}

fn hash_file(path: &Path) -> Result<String, LatticeError> {
    let mut file = fs::File::open(path).map_err(|error| io_error(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| io_error(path, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Writes a blob atomically: temp file in the blobs directory, then rename.
/// Identical content already on disk is left untouched.
fn write_blob(
    blobs_dir: &Path,
    hash: &str,
    fill: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> Result<PathBuf, LatticeError> {
    let target = blobs_dir.join(hash);
    if target.exists() {
        return Ok(target);
    }
    fs::create_dir_all(blobs_dir).map_err(|error| io_error(blobs_dir, error))?;
    let temp = blobs_dir.join(format!(".tmp-{hash}-{}", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        fill(&mut file)?;
        file.sync_all()?;
        fs::rename(&temp, &target)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        if target.exists() {
            // Another writer landed the same content first; that is fine.
            return Ok(target);
        }
        return Err(io_error(&target, error));
    }
    Ok(target)
}

pub(crate) fn read_blob(blobs_dir: &Path, hash: &str) -> Result<Option<Vec<u8>>, LatticeError> {
    let path = blobs_dir.join(hash);
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path, error)),
    }
}

/// Returns true when `name` looks like a blob file name (64 lowercase hex).
pub(crate) fn is_blob_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::{BodyInput, is_blob_name, place, read_blob, sha256_hex};

    #[test]
    fn hashes_like_sha256sum() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn places_by_threshold_not_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let small = place(dir.path(), BodyInput::Bytes(b"hello"), 5).unwrap();
        assert_eq!(small.inline.as_deref(), Some(&b"hello"[..]));
        assert!(small.hash.is_none());

        let large = place(dir.path(), BodyInput::Bytes(b"hello!"), 5).unwrap();
        assert!(large.inline.is_none());
        let hash = large.hash.unwrap();
        assert!(is_blob_name(&hash));
        assert_eq!(read_blob(dir.path(), &hash).unwrap().unwrap(), b"hello!");

        // Identical content shares one file.
        let again = place(dir.path(), BodyInput::Bytes(b"hello!"), 5).unwrap();
        assert_eq!(again.hash.as_deref(), Some(hash.as_str()));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn places_files_by_size() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("body.bin");
        std::fs::write(&source, vec![7_u8; 100]).unwrap();
        let blobs = dir.path().join("blobs");
        let stored = place(&blobs, BodyInput::File(&source), 10).unwrap();
        assert_eq!(stored.len, Some(100));
        let hash = stored.hash.unwrap();
        assert_eq!(hash, sha256_hex(&[7_u8; 100]));
        assert_eq!(read_blob(&blobs, &hash).unwrap().unwrap().len(), 100);
    }
}
