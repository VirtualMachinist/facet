//! Lattice: the Facet run-history store.
//!
//! Lattice sits beside an OpenCollection workspace and remembers every run.
//! It never holds the collection itself; OpenCollection YAML on disk stays
//! canonical and Git stays the sync layer. Two SQLite files:
//!
//! - **Workspace store** `.facet/lattice.db` next to the YAML. Run history
//!   for that workspace. Bodies are inline at or under `inline_body_max`,
//!   otherwise content-addressed files under `.facet/blobs/<sha256>`.
//! - **Machine store** `~/.local/share/facet/lattice.db` (XDG data dir).
//!   Cross-workspace state: workspace registry, run index, sessions,
//!   environments, preferences.
//!
//! Decisions and defaults come from `FACET_HANDOFF_BRIEF.md` (Surfaces 1,
//! 3, 4, 5). Engine order: bundled SQLite now, Turso behind a feature later.
//! DuckDB ATTACHes the SQLite file out of process (`scripts/duckdb-attach-demo.sh`);
//! the in-process `lattice-duckdb` feature is apiary-only, never lathe.
//!
//! **Next slice** (sessions, recall, replay, hash-diff, secret hydration,
//! `--expect`): `docs/FACET.md` § Next slice. The `sessions` table and
//! [`WorkspaceStore::run`] exist; they have no CLI yet. `FACET_SESSION` is
//! written onto the run row without minting a parent session.

#![forbid(unsafe_code)]

mod blobs;
mod config;
mod machine;
mod secrets;
mod store;
mod ulid;

use std::{fmt, io, path::PathBuf};

pub use blobs::{BodyInput, StoredBody, sha256_hex};
pub use config::{ConfigError, LatticeConfig, Retention, parse_byte_size, parse_retention};
pub use machine::{
    EnvironmentRow, MachineStore, SessionQuery, SessionRow, machine_config_dir, machine_data_dir,
};
pub use rusqlite::types::Value as SqlValue;
pub use secrets::{
    ENCRYPTED_REF_PREFIX, KEYRING_REF_PREFIX, SECRET_KEY_ENV, SecretBackend, SecretConfig,
    SecretError, StoredSecret, backend_of, delete_secret, delete_secret_with, get_secret,
    get_secret_with, put_secret, put_secret_with,
};
pub use store::{
    BLOBS_DIR, Blob, BodyRef, CONFIG_FILE, DB_FILE, FACET_DIR, GcReport, HistoryQuery, NewRun,
    RunRow, SqlResult, WORKSPACE_FILE, WorkspaceStore,
};
pub use ulid::{is_ulid, ulid};

/// Current workspace-store schema version (highest numbered migration).
pub const WORKSPACE_SCHEMA_VERSION: i64 = 3;
/// Current machine-store schema version (highest numbered migration).
pub const MACHINE_SCHEMA_VERSION: i64 = 2;

/// Failures raised by Lattice.
#[derive(Debug)]
pub enum LatticeError {
    /// SQLite reported an error.
    Sqlite(rusqlite::Error),
    /// A filesystem operation failed.
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// Configuration could not be parsed.
    Config(ConfigError),
    /// A secret at rest (Surface 3) operation failed.
    Secret(SecretError),
    /// A `--sql` query attempted to write.
    ReadOnlyQuery,
    /// No machine data directory could be resolved.
    NoDataDir,
}

impl fmt::Display for LatticeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "lattice store error: {error}"),
            Self::Io { path, source } => {
                write!(f, "lattice I/O error at {}: {source}", path.display())
            }
            Self::Config(error) => write!(f, "lattice configuration error: {error}"),
            Self::Secret(error) => write!(f, "lattice secret error: {error}"),
            Self::ReadOnlyQuery => write!(
                f,
                "--sql queries are read-only; use facet commands to write"
            ),
            Self::NoDataDir => write!(
                f,
                "cannot resolve the machine data directory; set FACET_DATA_DIR"
            ),
        }
    }
}

impl std::error::Error for LatticeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Config(error) => Some(error),
            Self::Secret(error) => Some(error),
            Self::ReadOnlyQuery | Self::NoDataDir => None,
        }
    }
}

impl From<rusqlite::Error> for LatticeError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<ConfigError> for LatticeError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<SecretError> for LatticeError {
    fn from(error: SecretError) -> Self {
        Self::Secret(error)
    }
}

impl LatticeError {
    /// Returns true when the failure is a SQLite parse or prepare failure
    /// caused by the caller's SQL rather than by the store.
    #[must_use]
    pub fn is_query_error(&self) -> bool {
        match self {
            Self::ReadOnlyQuery => true,
            Self::Sqlite(rusqlite::Error::SqliteFailure(failure, _)) => {
                failure.code == rusqlite::ErrorCode::Unknown
                    || failure.code == rusqlite::ErrorCode::ReadOnly
            }
            Self::Sqlite(rusqlite::Error::SqlInputError { .. }) => true,
            Self::Sqlite(rusqlite::Error::MultipleStatement) => true,
            _ => false,
        }
    }
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: io::Error) -> LatticeError {
    LatticeError::Io {
        path: path.into(),
        source,
    }
}

/// Current time as Unix milliseconds UTC.
#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
