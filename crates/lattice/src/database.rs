//! Narrow synchronous storage boundary shared by both Lattice stores.
//!
//! SQL, migrations, bodies and application semantics stay in store/machine.

use std::{fs, io::Write, ops::Deref, path::Path, time::Duration};

use rusqlite::{
    ToSql,
    types::{FromSql, FromSqlError, ToSqlOutput, Value, ValueRef},
};

use crate::{Engine, LatticeConfig, LatticeError, io_error};

type Result<T> = std::result::Result<T, LatticeError>;

pub(crate) enum Connection {
    Sqlite(rusqlite::Connection),
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("engine", &self.engine())
            .finish_non_exhaustive()
    }
}

impl Connection {
    pub(crate) fn open(path: &Path, config: &LatticeConfig, read_only: bool) -> Result<Self> {
        if config.engine == Engine::Turso {
            return Err(LatticeError::Engine(
                "Turso engine is not supported in this build".into(),
            ));
        }
        verify_engine(path, config.engine, read_only)?;
        let flags = if read_only {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        };
        let conn = rusqlite::Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(Duration::from_millis(config.busy_timeout_ms))?;
        if read_only {
            conn.pragma_update(None, "query_only", true)?;
            // ATTACH is considered read-only by SQLite, even with a
            // comment prefix. Authorize the parsed action, not a token.
            conn.authorizer(Some(|ctx: rusqlite::hooks::AuthContext<'_>| {
                match ctx.action {
                    rusqlite::hooks::AuthAction::Attach { .. }
                    | rusqlite::hooks::AuthAction::Detach { .. } => {
                        rusqlite::hooks::Authorization::Deny
                    }
                    _ => rusqlite::hooks::Authorization::Allow,
                }
            }))?;
        } else if config.wal {
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
        }
        Ok(Self::Sqlite(conn))
    }

    pub(crate) fn engine(&self) -> Engine {
        Engine::Sqlite
    }

    pub(crate) fn execute(&self, sql: &str, params: impl Bind) -> Result<usize> {
        let values = params.values()?;
        match self {
            Self::Sqlite(conn) => Ok(conn.execute(sql, rusqlite::params_from_iter(values))?),
        }
    }

    pub(crate) fn execute_batch(&self, sql: &str) -> Result<()> {
        match self {
            Self::Sqlite(conn) => Ok(conn.execute_batch(sql)?),
        }
    }

    pub(crate) fn prepare(&self, sql: &str) -> Result<Statement<'_>> {
        match self {
            Self::Sqlite(conn) => Ok(Statement::Sqlite(conn.prepare_cached(sql)?)),
        }
    }

    pub(crate) fn prepare_read_only(&self, sql: &str) -> Result<Statement<'_>> {
        let statement = self.prepare(sql).map_err(|error| {
            if let LatticeError::Sqlite(rusqlite::Error::SqliteFailure(failure, _)) = &error
                && failure.code == rusqlite::ErrorCode::AuthorizationForStatementDenied
            {
                return LatticeError::ReadOnlyQuery;
            }
            error
        })?;
        if let Statement::Sqlite(inner) = &statement
            && !inner.readonly()
        {
            return Err(LatticeError::ReadOnlyQuery);
        }
        Ok(statement)
    }

    pub(crate) fn query_row<T>(
        &self,
        sql: &str,
        params: impl Bind,
        f: impl FnOnce(&Row) -> rusqlite::Result<T>,
    ) -> Result<T> {
        let mut statement = self.prepare(sql)?;
        let mut rows = statement.query(params)?;
        let row = rows.next()?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        Ok(f(row)?)
    }

    pub(crate) fn transaction(&self) -> Result<Transaction<'_>> {
        self.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Transaction {
            conn: self,
            committed: false,
        })
    }
}

/// Never infer Turso compatibility from a SQLite header. Legacy unmarked files
/// belong to SQLite; changing configuration cannot silently convert them.
fn verify_engine(path: &Path, engine: Engine, read_only: bool) -> Result<()> {
    let marker = path.with_added_extension("engine");
    match fs::read_to_string(&marker) {
        Ok(value) if value == format!("{}\n", engine.as_str()) => return Ok(()),
        Ok(value) => {
            return Err(LatticeError::Engine(format!(
                "{} belongs to {:?}, requested {}; use a separate store or explicit conversion",
                path.display(),
                value.trim(),
                engine.as_str()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(&marker, error)),
    }
    if path.exists() && engine != Engine::Sqlite {
        return Err(LatticeError::Engine(format!(
            "{} is an unmarked SQLite store; refusing a Turso open",
            path.display()
        )));
    }
    if read_only {
        if engine == Engine::Sqlite {
            return Ok(());
        }
        return Err(LatticeError::Engine(
            "Turso store is missing its engine marker".into(),
        ));
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(mut file) => {
            writeln!(file, "{}", engine.as_str()).map_err(|error| io_error(&marker, error))?;
            file.sync_all().map_err(|error| io_error(&marker, error))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            verify_engine(path, engine, true)
        }
        Err(error) => Err(io_error(&marker, error)),
    }
}

pub(crate) trait Bind {
    fn values(self) -> rusqlite::Result<Vec<Value>>;
}
impl Bind for [Value; 0] {
    fn values(self) -> rusqlite::Result<Vec<Value>> {
        Ok(Vec::new())
    }
}
impl Bind for Vec<Value> {
    fn values(self) -> rusqlite::Result<Vec<Value>> {
        Ok(self)
    }
}
impl Bind for &[&dyn ToSql] {
    fn values(self) -> rusqlite::Result<Vec<Value>> {
        self.iter()
            .map(|value| match value.to_sql()? {
                ToSqlOutput::Borrowed(value) => Value::column_result(value)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error))),
                ToSqlOutput::Owned(value) => Ok(value),
                _ => Err(rusqlite::Error::InvalidParameterName(
                    "unsupported Lattice parameter type".into(),
                )),
            })
            .collect()
    }
}

pub(crate) enum Statement<'conn> {
    Sqlite(rusqlite::CachedStatement<'conn>),
}
impl Statement<'_> {
    pub(crate) fn column_names(&self) -> Vec<String> {
        match self {
            Self::Sqlite(stmt) => stmt.column_names().into_iter().map(str::to_owned).collect(),
        }
    }
    pub(crate) fn query(&mut self, params: impl Bind) -> Result<Rows<'_>> {
        let values = params.values()?;
        let inner = match self {
            Self::Sqlite(stmt) => {
                RowsInner::Sqlite(stmt.query(rusqlite::params_from_iter(values))?)
            }
        };
        Ok(Rows {
            inner,
            current: None,
        })
    }
    pub(crate) fn query_map<'a, T, F: FnMut(&Row) -> rusqlite::Result<T> + 'a>(
        &'a mut self,
        params: impl Bind,
        f: F,
    ) -> Result<impl Iterator<Item = Result<T>> + 'a> {
        let mut rows = self.query(params)?;
        let mut f = f;
        let mut ended = false;
        Ok(std::iter::from_fn(move || {
            if ended {
                return None;
            }
            match rows.next() {
                Ok(Some(row)) => Some(f(row).map_err(Into::into)),
                Ok(None) => {
                    ended = true;
                    None
                }
                Err(error) => {
                    ended = true;
                    Some(Err(error))
                }
            }
        }))
    }
}

enum RowsInner<'stmt> {
    Sqlite(rusqlite::Rows<'stmt>),
}
pub(crate) struct Rows<'stmt> {
    inner: RowsInner<'stmt>,
    current: Option<Row>,
}
impl Rows<'_> {
    pub(crate) fn next(&mut self) -> Result<Option<&Row>> {
        self.current = match &mut self.inner {
            RowsInner::Sqlite(rows) => rows
                .next()?
                .map(|row| {
                    (0..row.as_ref().column_count())
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<Value>>>()
                        .map(Row)
                })
                .transpose()?,
        };
        Ok(self.current.as_ref())
    }
}
pub(crate) struct Row(Vec<Value>);
impl Row {
    pub(crate) fn get<T: FromSql>(&self, index: usize) -> rusqlite::Result<T> {
        let value = self
            .0
            .get(index)
            .ok_or(rusqlite::Error::InvalidColumnIndex(index))?;
        T::column_result(ValueRef::from(value)).map_err(|error| match error {
            FromSqlError::InvalidType => {
                rusqlite::Error::InvalidColumnType(index, index.to_string(), value.data_type())
            }
            FromSqlError::OutOfRange(value) => {
                rusqlite::Error::IntegralValueOutOfRange(index, value)
            }
            other => {
                rusqlite::Error::FromSqlConversionFailure(index, value.data_type(), Box::new(other))
            }
        })
    }
}

pub(crate) struct Transaction<'conn> {
    conn: &'conn Connection,
    committed: bool,
}
impl Deref for Transaction<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}
impl Transaction<'_> {
    pub(crate) fn commit(mut self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.committed = true;
        Ok(())
    }
}
impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_transaction_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(
            &dir.path().join("transaction.db"),
            &LatticeConfig::default(),
            false,
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TABLE entries (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
        )
        .unwrap();
        {
            let tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO entries VALUES (1, 'first')", [])
                .unwrap();
            assert!(
                tx.execute("INSERT INTO entries VALUES (1, 'duplicate')", [])
                    .is_err()
            );
        }
        assert_eq!(
            conn.query_row("SELECT count(*) FROM entries", [], |row| row.get::<i64>(0))
                .unwrap(),
            0
        );
        let tx = conn.transaction().unwrap();
        tx.execute("INSERT INTO entries VALUES (2, 'committed')", [])
            .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM entries", [], |row| row.get::<i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn sqlite_authorizer_blocks_commented_attach() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::WorkspaceStore::open(dir.path(), LatticeConfig::default()).unwrap();
        for sql in [
            "/* prefix */ ATTACH ':memory:' AS other",
            "-- prefix\nATTACH ':memory:' AS other",
        ] {
            assert!(matches!(store.query(sql), Err(LatticeError::ReadOnlyQuery)));
        }
    }
}
