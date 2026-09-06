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

use crate::secrets::{
    SecretConfig, StoredSecret, backend_of, delete_secret_with, get_secret_with, put_secret_with,
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

    // ----- Environments (Surface 3) --------------------------------------

    /// Sets one environment value for a workspace. When `secret` is true the
    /// value is routed through the secrets layer (OS keyring by default,
    /// encrypted under `FACET_SECRET_KEY` in headless mode) and the row stores
    /// a `secret_ref`; otherwise the value is stored in plaintext in `value`.
    ///
    /// Replacing an existing entry cleans up the previous secret reference
    /// (keyring entry deleted, encrypted blob dropped with the old row).
    pub fn set_environment(
        &self,
        workspace_id: &str,
        name: &str,
        key: &str,
        value: &str,
        secret: bool,
    ) -> Result<(), LatticeError> {
        self.set_environment_with(
            workspace_id,
            name,
            key,
            value,
            secret,
            &SecretConfig::from_env()?,
        )
    }

    /// [`Self::set_environment`] with an explicit secrets config (for tests).
    pub fn set_environment_with(
        &self,
        workspace_id: &str,
        name: &str,
        key: &str,
        value: &str,
        secret: bool,
        config: &SecretConfig,
    ) -> Result<(), LatticeError> {
        let now = crate::now_ms();
        let old_ref: Option<String> = match self.conn.query_row(
            "SELECT secret_ref FROM environments WHERE workspace_id = ?1 AND name = ?2 AND key = ?3",
            params![workspace_id, name, key],
            |row| row.get::<_, Option<String>>(0),
        ) {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(other) => return Err(other.into()),
        };

        let (stored_value, stored_ref): (Option<String>, Option<String>) = if secret {
            let StoredSecret { reference, .. } = put_secret_with(value, config)?;
            // Drop the old keyring entry if the previous row held one.
            if let Some(previous) = &old_ref
                && backend_of(previous).is_some()
                && previous != &reference
            {
                let _ = delete_secret_with(previous, config);
            }
            (None, Some(reference))
        } else {
            // Downgrading from secret to plain: clean up the old secret.
            if let Some(previous) = &old_ref
                && backend_of(previous).is_some()
            {
                let _ = delete_secret_with(previous, config);
            }
            (Some(value.to_owned()), None)
        };

        self.conn.execute(
            "INSERT INTO environments (workspace_id, name, key, value, secret_ref, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(workspace_id, name, key) DO UPDATE SET \
             value = excluded.value, secret_ref = excluded.secret_ref, updated_at = excluded.updated_at",
            params![workspace_id, name, key, stored_value, stored_ref, now],
        )?;
        Ok(())
    }

    /// Reads one environment value, resolving secrets through the secrets
    /// layer. Returns `None` when the row is absent. A secret whose backend
    /// is unavailable (for example an encrypted ref with no `FACET_SECRET_KEY`)
    /// surfaces as [`SecretError`] wrapped in [`LatticeError::Secret`].
    pub fn environment(
        &self,
        workspace_id: &str,
        name: &str,
        key: &str,
    ) -> Result<Option<String>, LatticeError> {
        self.environment_with(workspace_id, name, key, &SecretConfig::from_env()?)
    }

    /// [`Self::environment`] with an explicit secrets config (for tests).
    pub fn environment_with(
        &self,
        workspace_id: &str,
        name: &str,
        key: &str,
        config: &SecretConfig,
    ) -> Result<Option<String>, LatticeError> {
        let row = match self.conn.query_row(
            "SELECT value, secret_ref FROM environments \
             WHERE workspace_id = ?1 AND name = ?2 AND key = ?3",
            params![workspace_id, name, key],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
        ) {
            Ok(pair) => Some(pair),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(other) => return Err(other.into()),
        };
        let Some((value, secret_ref)) = row else {
            return Ok(None);
        };
        match secret_ref {
            Some(reference) if !reference.is_empty() => {
                Ok(get_secret_with(&reference, config)?)
            }
            _ => Ok(value),
        }
    }

    /// Lists environment entries for a workspace as metadata only: name, key,
    /// whether the value is a secret, and `updated_at`. Secret values are
    /// never returned here; resolve them with [`Self::environment`].
    pub fn environments(&self, workspace_id: &str) -> Result<Vec<EnvironmentRow>, LatticeError> {
        let mut statement = self.conn.prepare(
            "SELECT name, key, secret_ref IS NOT NULL AND secret_ref != '', updated_at \
             FROM environments WHERE workspace_id = ?1 \
             ORDER BY name, key",
        )?;
        let rows = statement
            .query_map(params![workspace_id], |row| {
                Ok(EnvironmentRow {
                    name: row.get(0)?,
                    key: row.get(1)?,
                    secret: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Deletes one environment entry, also removing its secret from the
    /// keyring when it was a keyring-backed secret. Missing rows are not an
    /// error.
    pub fn delete_environment(
        &self,
        workspace_id: &str,
        name: &str,
        key: &str,
    ) -> Result<bool, LatticeError> {
        self.delete_environment_with(workspace_id, name, key, &SecretConfig::from_env()?)
    }

    /// [`Self::delete_environment`] with an explicit secrets config (for tests).
    pub fn delete_environment_with(
        &self,
        workspace_id: &str,
        name: &str,
        key: &str,
        config: &SecretConfig,
    ) -> Result<bool, LatticeError> {
        let secret_ref: Option<String> = match self.conn.query_row(
            "SELECT secret_ref FROM environments \
             WHERE workspace_id = ?1 AND name = ?2 AND key = ?3",
            params![workspace_id, name, key],
            |row| row.get::<_, Option<String>>(0),
        ) {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(other) => return Err(other.into()),
        };
        let removed = self.conn.execute(
            "DELETE FROM environments WHERE workspace_id = ?1 AND name = ?2 AND key = ?3",
            params![workspace_id, name, key],
        )?;
        if removed > 0
            && let Some(reference) = &secret_ref
            && backend_of(reference).is_some()
        {
            // Best effort: a missing keyring entry is not an error.
            let _ = delete_secret_with(reference, config);
        }
        Ok(removed > 0)
    }
}

/// One row of `environments` as metadata (never carries a secret value).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentRow {
    /// Environment name.
    pub name: String,
    /// Variable key.
    pub key: String,
    /// Whether the value is a secret (stored via the secrets layer).
    pub secret: bool,
    /// Unix milliseconds UTC of the last update.
    pub updated_at: i64,
}
