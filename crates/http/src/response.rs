use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use reqwest::Response;
use tokio::io::AsyncWriteExt;

use crate::{
    HttpError, MAX_IN_MEMORY_RESPONSE_BYTES, ResponseBodyFile, ResponseCache,
    cache::{ResponseCacheReservation, ResponseCacheReservationError},
};

static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A response header retained independently of the HTTP implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseHeader {
    /// Header name.
    pub name: String,
    /// Header value. Invalid UTF-8 bytes are represented lossily.
    pub value: String,
}

/// A completed HTTP response and its execution metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    /// Numeric HTTP status code.
    pub status: u16,
    /// Canonical reason phrase when one is defined.
    pub reason: String,
    /// Final URL after redirects.
    pub url: String,
    /// Total time through receipt of the complete response body.
    pub duration: Duration,
    /// Decoded response-body size in bytes.
    pub size: usize,
    /// Response headers sorted deterministically by name and value.
    pub headers: Vec<ResponseHeader>,
    /// Raw response body, bounded to [`MAX_IN_MEMORY_RESPONSE_BYTES`].
    ///
    /// When incomplete, this is the leading preview of the complete body.
    pub body: Vec<u8>,
    /// Whether `body` contains the complete response body.
    pub body_complete: bool,
    /// Complete automatically managed body when it exceeded the in-memory bound.
    pub body_file: Option<ResponseBodyFile>,
    /// Why the complete body could not be retained, when applicable.
    pub body_retention_error: Option<String>,
}

pub(crate) struct CollectedBody {
    pub(crate) preview: Vec<u8>,
    pub(crate) size: usize,
    pub(crate) complete: bool,
    pub(crate) file: Option<ResponseBodyFile>,
    pub(crate) retention_error: Option<String>,
}

pub(crate) async fn collect_bounded(
    response: &mut Response,
    cache: Option<&ResponseCache>,
    expected_size: Option<u64>,
) -> Result<CollectedBody, HttpError> {
    let mut preview = Vec::new();
    let mut size = 0_usize;
    let mut spool: Option<ActiveSpool> = None;
    let mut exceeded_limit = false;
    let mut retention_error = None;

    while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
        size = checked_response_size(size, chunk.len())?;

        if let Some(active) = &mut spool {
            let next_size = active.written.saturating_add(chunk.len() as u64);
            if next_size > active.max_bytes {
                retention_error = Some(quota_message(active.cache_quota_bytes));
                spool = None;
            } else if let Err(error) = active.file.write_all(&chunk).await {
                retention_error = Some(retention_failure(error));
                spool = None;
            } else {
                active.written = next_size;
            }
            continue;
        }
        if exceeded_limit {
            continue;
        }

        let preview_length = (MAX_IN_MEMORY_RESPONSE_BYTES - preview.len()).min(chunk.len());
        preview.extend_from_slice(&chunk[..preview_length]);
        if preview_length == chunk.len() {
            continue;
        }

        exceeded_limit = true;
        let Some(cache) = cache else {
            continue;
        };
        match reserve(cache, expected_size, size as u64).await {
            Ok(reservation) => {
                let mut active = ActiveSpool::new(reservation, cache.quota_bytes());
                if let Err(error) = active
                    .write_initial(&preview, &chunk[preview_length..])
                    .await
                {
                    retention_error = Some(retention_failure(error));
                } else {
                    active.written = size as u64;
                    spool = Some(active);
                }
            }
            Err(ReserveFailure::QuotaExceeded) => {
                retention_error = Some(quota_message(cache.quota_bytes()));
            }
            Err(ReserveFailure::Initialization(message)) => {
                retention_error = Some(format!(
                    "Could not initialize the response cache: {message}"
                ));
            }
        }
    }

    let file = match spool {
        Some(active) => match active.finish().await {
            Ok(file) => Some(file),
            Err(error) => {
                retention_error = Some(retention_failure(error));
                None
            }
        },
        None => None,
    };
    Ok(CollectedBody {
        preview,
        size,
        complete: !exceeded_limit,
        file,
        retention_error,
    })
}

enum ReserveFailure {
    QuotaExceeded,
    Initialization(String),
}

async fn reserve(
    cache: &ResponseCache,
    expected_size: Option<u64>,
    minimum_size: u64,
) -> Result<ResponseCacheReservation, ReserveFailure> {
    let cache = cache.clone();
    match tokio::task::spawn_blocking(move || cache.reserve(expected_size, minimum_size)).await {
        Ok(Ok(reservation)) => Ok(reservation),
        Ok(Err(ResponseCacheReservationError::QuotaExceeded)) => Err(ReserveFailure::QuotaExceeded),
        Ok(Err(ResponseCacheReservationError::Io(error))) => {
            Err(ReserveFailure::Initialization(error.to_string()))
        }
        Err(error) => Err(ReserveFailure::Initialization(error.to_string())),
    }
}

struct ActiveSpool {
    file: tokio::fs::File,
    body_file: ResponseBodyFile,
    written: u64,
    max_bytes: u64,
    cache_quota_bytes: u64,
}

impl ActiveSpool {
    fn new(reservation: ResponseCacheReservation, cache_quota_bytes: u64) -> Self {
        Self {
            file: tokio::fs::File::from_std(reservation.file),
            body_file: reservation.body_file,
            written: 0,
            max_bytes: reservation.max_bytes,
            cache_quota_bytes,
        }
    }

    async fn write_initial(&mut self, preview: &[u8], remainder: &[u8]) -> std::io::Result<()> {
        self.file.write_all(preview).await?;
        self.file.write_all(remainder).await
    }

    async fn finish(mut self) -> std::io::Result<ResponseBodyFile> {
        self.file.set_len(self.written).await?;
        self.file.flush().await?;
        Ok(self.body_file)
    }
}

fn checked_response_size(size: usize, chunk_size: usize) -> Result<usize, HttpError> {
    size.checked_add(chunk_size).ok_or_else(|| {
        HttpError::Transport("response body size exceeds platform limits".to_owned())
    })
}

fn retention_failure(error: std::io::Error) -> String {
    format!("Could not retain the complete response: {error}")
}

fn quota_message(quota_bytes: u64) -> String {
    const MEBIBYTE: u64 = 1024 * 1024;
    let quota = if quota_bytes.is_multiple_of(MEBIBYTE) {
        format!("{} MiB", quota_bytes / MEBIBYTE)
    } else {
        format!("{quota_bytes} byte")
    };
    format!(
        "The complete response was not retained because the {quota} response cache quota was reached."
    )
}

pub(crate) async fn stream_to_file(
    response: &mut Response,
    output: &Path,
) -> Result<usize, HttpError> {
    let (mut file, mut temporary) = create_temporary_output(output).await?;
    let mut size = 0_usize;
    while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
        size = checked_response_size(size, chunk.len())?;
        file.write_all(&chunk)
            .await
            .map_err(|error| output_error(output, error))?;
    }
    file.flush()
        .await
        .map_err(|error| output_error(output, error))?;
    file.sync_all()
        .await
        .map_err(|error| output_error(output, error))?;
    drop(file);
    replace_output(&temporary.path, output).await?;
    temporary.committed = true;
    Ok(size)
}

async fn create_temporary_output(
    output: &Path,
) -> Result<(tokio::fs::File, TemporaryOutput), HttpError> {
    let file_name = output
        .file_name()
        .ok_or_else(|| HttpError::ResponseOutput {
            path: output.to_owned(),
            message: "output path has no file name".to_owned(),
        })?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let sequence = OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".probe-{}-{sequence}.part", std::process::id()));
        let path = parent.join(temporary_name);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => {
                return Ok((
                    file,
                    TemporaryOutput {
                        path,
                        committed: false,
                    },
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(output_error(output, error)),
        }
    }
}

async fn replace_output(temporary: &Path, output: &Path) -> Result<(), HttpError> {
    #[cfg(windows)]
    if tokio::fs::try_exists(output)
        .await
        .map_err(|error| output_error(output, error))?
    {
        tokio::fs::remove_file(output)
            .await
            .map_err(|error| output_error(output, error))?;
    }
    tokio::fs::rename(temporary, output)
        .await
        .map_err(|error| output_error(output, error))
}

fn output_error(output: &Path, error: std::io::Error) -> HttpError {
    HttpError::ResponseOutput {
        path: output.to_owned(),
        message: error.to_string(),
    }
}

struct TemporaryOutput {
    path: PathBuf,
    committed: bool,
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(crate) fn map_reqwest_error(error: reqwest::Error) -> HttpError {
    if error.is_timeout() {
        HttpError::Timeout
    } else if error.is_builder() {
        HttpError::InvalidRequest(error.to_string())
    } else {
        HttpError::Transport(error.to_string())
    }
}
