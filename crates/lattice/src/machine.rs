//! Machine store: `~/.local/share/facet/lattice.db` (XDG data dir).

use std::{
    fs,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, params};

use crate::{
    DB_FILE, LatticeConfig, LatticeError, RunRow, io_error,
    store::{migrate, open_connection},
};

const MACHINE_MIGRATIONS: &[&str] = &[include_str!("../migrations/machine/0001_init.sql")];

/// Machine data directory: `FACET_DATA_DIR` or the platform data dir for `facet`.
#[must_use]
pub fn machine_data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FACET_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "facet").map(|dirs| dirs.data_dir().to_owned())
}

/// Machine config directory: `FACET_CONFIG_DIR` or the platform config dir for `facet`.
#[must_use]
pub fn machine_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FACET_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "facet").map(|dirs| dirs.config_dir().to_owned())
}

/// A workspace registry row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRow {
    /// Workspace ULID.
    pub id: String,
    /// Last known absolute path.
    pub path: String,
    /// Display name.
    pub name: Option<String>,
    /// Last time a run was indexed.
    pub last_seen: i64,
}

/// An open machine store.
#[derive(Debug)]
pub struct MachineStore {
    path: PathBuf,
    conn: Connection,
}

impl MachineStore {
    /// Opens (creating when needed) the machine store in the data directory.
    pub fn open(config: &LatticeConfig) -> Result<Self, LatticeError> {
        let dir = machine_data_dir().ok_or(LatticeError::NoDataDir)?;
        fs::create_dir_all(&dir).map_err(|error| io_error(&dir, error))?;
        Self::open_at(&dir.join(DB_FILE), config)
    }

    /// Opens (creating when needed) a machine store at an explicit path.
    pub fn open_at(path: &Path, config: &LatticeConfig) -> Result<Self, LatticeError> {
        let conn = open_connection(path, config)?;
        migrate(&conn, MACHINE_MIGRATIONS)?;
        Ok(Self {
            path: path.to_owned(),
            conn,
        })
    }

    /// Database file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Highest applied migration.
    pub fn schema_version(&self) -> Result<i64, LatticeError> {
        Ok(self.conn.query_row(
            "SELECT coalesce(max(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?)
    }

    /// Registers or refreshes a workspace.
    pub fn touch_workspace(
        &self,
        id: &str,
        path: &Path,
        name: Option<&str>,
        now: i64,
    ) -> Result<(), LatticeError> {
        self.conn.execute(
            "INSERT INTO workspaces (id, path, name, last_seen) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET path = excluded.path, \
             name = coalesce(excluded.name, workspaces.name), last_seen = excluded.last_seen",
            params![id, path.to_string_lossy(), name, now],
        )?;
        Ok(())
    }

    /// Writes the cross-workspace pointer row for a run.
    pub fn index_run(&self, run: &RunRow, workspace_id: &str) -> Result<(), LatticeError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO run_index (run_id, workspace_id, started_at, request_path, status) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run.id, workspace_id, run.started_at, run.request_path, run.status],
        )?;
        Ok(())
    }

    /// Registered workspaces, most recently seen first.
    pub fn workspaces(&self) -> Result<Vec<WorkspaceRow>, LatticeError> {
        let mut statement = self.conn.prepare(
            "SELECT id, path, name, last_seen FROM workspaces ORDER BY last_seen DESC, id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(WorkspaceRow {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    name: row.get(2)?,
                    last_seen: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Total pointer rows.
    pub fn count_indexed_runs(&self) -> Result<i64, LatticeError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM run_index", [], |row| row.get(0))?)
    }
}
