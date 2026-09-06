//! DuckDB in-process analytics smoke (apiary-only; `lattice-duckdb` feature).
//!
//! Proves the DuckDB Rust crate can, in-process, ATTACH the SQLite lattice
//! workspace store (via the `sqlite` extension) and run analytics over
//! `runs`/`blobs`. The default engine stays rusqlite; this is the
//! analytics escape hatch the brief calls for ("DuckDB attaches SQLite files
//! externally for analytics"), here exercised in-process.
//!
//! APIARY-ONLY. Never enable on lathe (heavy native build; "No DuckDB on
//! lathe"). Run on apiary:
//!
//! ```text
//! cargo test -p lattice --features lattice-duckdb --test duckdb
//! ```
//!
//! The default `cargo test -p lattice` (feature off) does not build this
//! file or pull the `duckdb` crate.

#![cfg(feature = "lattice-duckdb")]

use std::path::Path;

use duckdb::Connection;

/// Seeds a workspace store via the system `sqlite3` CLI (separate process,
/// separate SQLite — avoids any in-process rusqlite/duckdb SQLite symbol
/// coupling) and returns the DB path.
fn seed_store(root: &Path) -> std::path::PathBuf {
    let db_path = root.join(lattice::FACET_DIR).join(lattice::DB_FILE);
    std::fs::create_dir_all(root.join(lattice::FACET_DIR)).unwrap();
    let schema = include_str!("../migrations/workspace/0001_init.sql");
    std::process::Command::new("sqlite3")
        .arg(&db_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(schema.as_bytes())?;
            child.wait().map(|_| ())
        })
        .expect("sqlite3 CLI to apply the schema");
    let insert = "INSERT INTO runs (id, started_at, duration_ms, request_path, request_hash, method, url, status, actor, res_body_len) VALUES \
('01JADUCKDB00000001',1757000001000,128,'users/list.yml','h1','GET','http://api/users',200,'agent-a',512),\
('01JADUCKDB00000002',1757000002000,407,'users/list.yml','h1','GET','http://api/users',200,'agent-a',512),\
('01JADUCKDB00000003',1757000003000,503,'items/0','h2','POST','http://api/items',500,'human',2048);";
    std::process::Command::new("sqlite3")
        .arg(&db_path)
        .arg(insert)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()
        .expect("sqlite3 CLI to insert rows");
    db_path
}

#[test]
fn duckdb_attaches_lattice_and_runs_analytics() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = seed_store(dir.path());

    let conn = Connection::open_in_memory().expect("open duckdb");
    conn.execute_batch("INSTALL sqlite; LOAD sqlite;")
        .expect("load duckdb sqlite extension");
    conn.execute_batch(&format!(
        "ATTACH '{}' AS lattice (TYPE SQLITE);",
        db_path.display()
    ))
    .expect("attach lattice store");

    // Analytics over the attached SQLite file: status histogram + avg duration.
    let mut stmt = conn
        .prepare("SELECT status, count(*) AS n, round(avg(duration_ms),1) AS avg_ms FROM lattice.runs GROUP BY status ORDER BY status")
        .unwrap();
    let rows: Vec<(i64, i64, f64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows, vec![(200, 2, 267.5), (500, 1, 503.0)]);

    // Cross-table join: runs x blobs (registry has no rows here, so left join is 0).
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM lattice.runs LEFT JOIN lattice.blobs b ON 1=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(n, 3);

    conn.close().expect("close duckdb");
}
