//! Turso/libSQL smoke (Surface 3/4 future path).
//!
//! Behind the `lattice-turso` cargo feature. Proves Turso (libSQL local mode)
//! can open/write/read a workspace store file. libSQL is a SQLite fork, so the
//! on-disk file format is identical to rusqlite's; flipping the engine flag
//! requires no migration.
//!
//! This test is **libsql-only** on purpose. The lattice test binary also
//! links rusqlite (the default engine); if a test calls rusqlite first, rusqlite
//! initializes the process-global SQLite and libsql's later
//! `sqlite3_config(SERIALIZED)` returns `SQLITE_MISUSE`. Keeping this test
//! libsql-only lets libsql initialize first. Cross-engine interop (a file
//! written by libsql read back by rusqlite) is verified out-of-band with the
//! `sqlite3` CLI — see `agents/backend/notes/`.
//!
//! Run on the build host (apiary), not lathe:
//!
//! ```text
//! cargo test -p lattice --features lattice-turso --test turso
//! ```
//!
//! The default `cargo test -p lattice` (feature off) does not build this
//! file or pull libsql/tokio.

#![cfg(feature = "lattice-turso")]

use std::path::Path;

use libsql::Builder;

/// The workspace-store migration, embedded so the smoke can lay down the
/// schema through libsql without touching rusqlite. Source of truth:
/// `crates/lattice/migrations/workspace/0001_init.sql`.
const WORKSPACE_SCHEMA: &str = include_str!("../migrations/workspace/0001_init.sql");

/// Opens (creating when needed) the workspace store file through Turso
/// (libSQL local mode) and runs the schema migration. Local mode does no
/// networking; it opens the SQLite-compatible file in place.
async fn turso_open(root: &Path) -> libsql::Result<libsql::Connection> {
    let db_path = root.join(lattice::FACET_DIR).join(lattice::DB_FILE);
    std::fs::create_dir_all(root.join(lattice::FACET_DIR)).unwrap();
    let db = Builder::new_local(&db_path).build().await?;
    let conn = db.connect()?;
    // The migration is not idempotent (CREATE TABLE, not IF NOT EXISTS), so
    // only run it when the schema_version table is absent.
    let mut rows = conn
        .query(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
            (),
        )
        .await?;
    let row = rows.next().await?.unwrap();
    if *row.get_value(0).unwrap().as_integer().unwrap() == 0 {
        conn.execute_batch(WORKSPACE_SCHEMA).await?;
    }
    Ok(conn)
}

#[tokio::test]
async fn turso_opens_writes_and_reads_a_workspace_store() {
    // If TURSO_KEEP_DB is set, write the store there and leave it on disk so the
    // file can be inspected with the `sqlite3` CLI (cross-engine interop
    // check). Otherwise use a tempdir that is cleaned up.
    let root: std::path::PathBuf = match std::env::var_os("TURSO_KEEP_DB") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            std::fs::create_dir_all(&path).unwrap();
            path
        }
        None => tempfile::tempdir().unwrap().path().to_owned(),
    };
    let conn = turso_open(&root).await.unwrap();

    // Write a run row through Turso.
    conn
        .execute(
            "INSERT INTO runs (id, started_at, request_path, request_hash, method, url, actor) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            libsql::params!(
                "01JABCTURSO00000000",
                lattice::now_ms(),
                "users/list-users.yml",
                "deadbeef",
                "GET",
                "http://127.0.0.1/users",
                "agent-turso",
            ),
        )
        .await
        .unwrap();

    // Read it back through Turso.
    let mut rows = conn
        .query(
            "SELECT id, request_path, actor, method FROM runs WHERE actor = ?1",
            libsql::params!("agent-turso"),
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get_value(0).unwrap().as_text().unwrap(), "01JABCTURSO00000000");
    assert_eq!(row.get_value(1).unwrap().as_text().unwrap(), "users/list-users.yml");
    assert_eq!(row.get_value(2).unwrap().as_text().unwrap(), "agent-turso");
    assert_eq!(row.get_value(3).unwrap().as_text().unwrap(), "GET");

    // The schema_version row landed.
    let mut meta = conn
        .query("SELECT count(*) FROM schema_version", ())
        .await
        .unwrap();
    let row = meta.next().await.unwrap().unwrap();
    assert_eq!(*row.get_value(0).unwrap().as_integer().unwrap(), 1);

    // Reopening the same file through Turso sees the row (persistence).
    drop(conn);
    let conn2 = turso_open(&root).await.unwrap();
    let mut rows = conn2
        .query("SELECT count(*) FROM runs", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(*row.get_value(0).unwrap().as_integer().unwrap(), 1);
    drop(conn2);

    // Cross-engine interop: when keeping the file, confirm the `sqlite3` CLI
    // (system SQLite, separate from both bundled engines) can read the row
    // libsql wrote. This is run out-of-band after the test, see notes.
    if std::env::var_os("TURSO_KEEP_DB").is_some() {
        eprintln!(
            "TURSO_KEEP_DB at {} — verify with: sqlite3 {}/.facet/lattice.db 'SELECT id, actor FROM runs;'",
            root.display(),
            root.display()
        );
    }
}
