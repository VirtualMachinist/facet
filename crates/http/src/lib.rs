//! Shared asynchronous HTTP construction and execution for Probe.

#![forbid(unsafe_code)]

mod cache;
mod engine;
mod error;
mod request;
mod response;

pub use cache::{ResponseBodyFile, ResponseCache};
pub use engine::HttpEngine;
pub use error::HttpError;
pub use response::{HttpResponse, ResponseHeader};

use std::path::PathBuf;

/// Maximum response body retained by the default in-memory execution methods.
pub const MAX_IN_MEMORY_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Filesystem context used while constructing request bodies.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionOptions {
    /// Base directory for relative request-body file paths.
    pub base_directory: Option<PathBuf>,
    /// Shared cache used for automatically managed large-response spool files.
    ///
    /// When absent, the complete large body is drained without being retained.
    pub response_cache: Option<ResponseCache>,
}
