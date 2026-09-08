//! Actual Rust Turso through both application stores, never libSQL.
#![cfg(feature = "lattice-turso")]

use lattice::{
    BodyInput, Engine, HistoryQuery, LatticeConfig, LatticeError, MachineStore, NewRun,
    SessionQuery, SqlValue, WorkspaceStore,
};

fn config() -> LatticeConfig {
    LatticeConfig {
        engine: Engine::Turso,
        inline_body_max: 8,
        ..LatticeConfig::default()
    }
}

#[tokio::test]
async fn application_history_sessions_bodies_and_reopen_inside_tokio() {
    let dir = tempfile::tempdir().unwrap();
    let machine_path = dir.path().join("machine.db");
    let store = WorkspaceStore::open(dir.path(), config()).unwrap();
    let machine = MachineStore::open_at(&machine_path, &config()).unwrap();
    assert_eq!(store.engine(), Engine::Turso);
    assert_eq!(machine.engine(), Engine::Turso);
    assert_eq!(
        store.schema_version().unwrap(),
        lattice::WORKSPACE_SCHEMA_VERSION
    );
    assert_eq!(
        machine.schema_version().unwrap(),
        lattice::MACHINE_SCHEMA_VERSION
    );
    let session = machine
        .start_session("grok", Some(r#"{"herdr":"pane-1","omp":"session-1"}"#), 100)
        .unwrap();
    machine
        .touch_workspace(
            store.workspace_id(),
            dir.path(),
            Some("Turso contract"),
            101,
        )
        .unwrap();
    machine.set_preference("editor.theme", "dark").unwrap();
    let run = store
        .record_run(&NewRun {
            started_at: 102,
            request_path: "cluster.yml",
            request_hash: "abc",
            method: "POST",
            url: "https://cluster/api",
            status: Some(201),
            actor: "grok",
            session_id: Some(&session.id),
            environment: Some("m1"),
            req_body: BodyInput::Bytes(b"request body"),
            res_body: BodyInput::Bytes(b"tiny"),
            tags: Some(r#"["m1","cluster"]"#),
            ..NewRun::default()
        })
        .unwrap();
    machine.index_run(&run, store.workspace_id()).unwrap();
    let replay = store
        .record_run(&NewRun {
            started_at: 103,
            replayed_from: Some(&run.id),
            res_body: BodyInput::Bytes(b"a large response body"),
            ..NewRun::default()
        })
        .unwrap();
    assert_eq!(store.request_body(&run).unwrap().unwrap(), b"request body");
    assert_eq!(store.response_body(&run).unwrap().unwrap(), b"tiny");
    assert_eq!(
        store.response_body(&replay).unwrap().unwrap(),
        b"a large response body"
    );
    let history = store
        .history(&HistoryQuery {
            limit: 10,
            actor: Some("grok".into()),
            session_id: Some(session.id.clone()),
            environment: Some("m1".into()),
            tags: vec!["m1".into(), "cluster".into()],
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, run.id);
    assert_eq!(
        store
            .query("WITH selected AS (SELECT id FROM runs) SELECT count(*) FROM selected")
            .unwrap()
            .rows,
        vec![vec![SqlValue::Integer(2)]]
    );
    assert!(store.gc(false).unwrap().orphans.is_empty());
    let workspace_id = store.workspace_id().to_owned();
    drop(store);
    drop(machine);
    let store = WorkspaceStore::open(dir.path(), config()).unwrap();
    let machine = MachineStore::open_at(&machine_path, &config()).unwrap();
    assert_eq!(store.workspace_id(), workspace_id);
    assert_eq!(store.run(&run.id).unwrap().unwrap(), run);
    assert_eq!(
        store
            .run(&replay.id)
            .unwrap()
            .unwrap()
            .replayed_from
            .as_deref(),
        Some(run.id.as_str())
    );
    assert_eq!(
        machine.indexed_run(&run.id).unwrap().unwrap().0,
        workspace_id
    );
    assert_eq!(
        machine.preference("editor.theme").unwrap().as_deref(),
        Some("dark")
    );
    assert_eq!(
        machine
            .sessions(&SessionQuery {
                limit: 10,
                actor: Some("grok".into()),
                open_only: true
            })
            .unwrap(),
        vec![session.clone()]
    );
    assert_eq!(
        machine.session(&session.id).unwrap().unwrap().meta,
        session.meta
    );
    assert_eq!(
        machine
            .end_session(&session.id, 200)
            .unwrap()
            .unwrap()
            .ended_at,
        Some(200)
    );
    assert_eq!(
        machine
            .end_session(&session.id, 300)
            .unwrap()
            .unwrap()
            .ended_at,
        Some(200)
    );
}

#[test]
fn read_only_queries_reject_writes_attach_comments_and_multiple_statements() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config()).unwrap();
    for sql in [
        "DELETE FROM runs",
        "WITH x AS (SELECT 1) DELETE FROM runs",
        "PRAGMA user_version = 99",
        "/* prefix */ ATTACH ':memory:' AS other",
        "-- prefix\nATTACH ':memory:' AS other",
        "SELECT 1; DELETE FROM runs",
        "SELECT 1; ATTACH ':memory:' AS other",
        "VACUUM INTO 'escaped.db'",
    ] {
        assert!(
            matches!(store.query(sql), Err(LatticeError::ReadOnlyQuery)),
            "{sql}"
        );
    }
    assert!(
        store
            .query("SELECT absent FROM runs")
            .unwrap_err()
            .is_query_error()
    );
    assert_eq!(store.count_runs().unwrap(), 0);
    assert_eq!(
        store
            .query("/* allowed */ SELECT 1 AS result")
            .unwrap()
            .columns,
        ["result"]
    );
}

#[test]
fn explicit_selection_protects_existing_files_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), LatticeConfig::default()).unwrap();
    store.record_run(&NewRun::default()).unwrap();
    drop(store);
    let path = dir.path().join(".facet/lattice.db");
    let bytes = std::fs::read(&path).unwrap();
    assert!(matches!(
        WorkspaceStore::open(dir.path(), config()),
        Err(LatticeError::Engine(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_file(path.with_added_extension("engine")).unwrap();
    assert!(matches!(
        WorkspaceStore::open(dir.path(), config()),
        Err(LatticeError::Engine(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let other = tempfile::tempdir().unwrap();
    WorkspaceStore::open(other.path(), config()).unwrap();
    assert!(matches!(
        WorkspaceStore::open(other.path(), LatticeConfig::default()),
        Err(LatticeError::Engine(_))
    ));
}

#[test]
fn concurrent_application_writers_retain_every_run() {
    let dir = tempfile::tempdir().unwrap();
    WorkspaceStore::open(dir.path(), config()).unwrap();
    let writers: Vec<_> = (0..4)
        .map(|writer| {
            let root = dir.path().to_owned();
            std::thread::spawn(move || {
                let store = WorkspaceStore::open(&root, config()).unwrap();
                for index in 0..10 {
                    store
                        .record_run(&NewRun {
                            started_at: index,
                            actor: &format!("writer-{writer}"),
                            ..NewRun::default()
                        })
                        .unwrap();
                    assert!(
                        !store
                            .history(&HistoryQuery {
                                limit: 5,
                                ..HistoryQuery::default()
                            })
                            .unwrap()
                            .is_empty()
                    );
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(
        WorkspaceStore::open(dir.path(), config())
            .unwrap()
            .count_runs()
            .unwrap(),
        40
    );
}

#[test]
fn legacy_request_body_migration_preserves_bytes_and_future_schema_is_rejected() {
    use futures::executor::block_on;
    for future in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let facet = dir.path().join(".facet");
        std::fs::create_dir(&facet).unwrap();
        let path = facet.join("lattice.db");
        std::fs::write(path.with_added_extension("engine"), "turso\n").unwrap();
        let db = block_on(
            turso::Builder::new_local(path.to_str().unwrap())
                .experimental_multiprocess_wal(true)
                .build(),
        )
        .unwrap();
        let conn = db.connect().unwrap();
        block_on(conn.execute_batch(include_str!("../migrations/workspace/0001_init.sql")))
            .unwrap();
        block_on(conn.execute("INSERT INTO runs (id, started_at, request_path, request_hash, method, url, req_body) VALUES ('old', 1, 'old.yml', 'hash', 'POST', 'http://local', ?1)", turso::params![b"preserve legacy bytes".to_vec()])).unwrap();
        if future {
            block_on(conn.execute_batch("INSERT INTO schema_version VALUES (100, 1)")).unwrap();
        }
        drop(conn);
        drop(db);
        if future {
            assert!(matches!(
                WorkspaceStore::open(dir.path(), config()),
                Err(LatticeError::Engine(_))
            ));
            assert!(
                !facet.join("blobs").exists(),
                "reject before body hydration"
            );
        } else {
            let store = WorkspaceStore::open(dir.path(), config()).unwrap();
            let run = store.run("old").unwrap().unwrap();
            assert_eq!(
                store.request_body(&run).unwrap().unwrap(),
                b"preserve legacy bytes"
            );
            assert_eq!(store.schema_version().unwrap(), 3);
            assert!(store.query("SELECT req_body FROM runs").is_err());
        }
    }
}

#[test]
fn process_writer() {
    let Some(root) = std::env::var_os("LATTICE_TURSO_TEST_ROOT") else {
        return;
    };
    let store = WorkspaceStore::open(std::path::Path::new(&root), config()).unwrap();
    for _ in 0..10 {
        store.record_run(&NewRun::default()).unwrap();
    }
}

#[test]
fn separate_process_writers_and_reader_share_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let reader = WorkspaceStore::open(dir.path(), config()).unwrap();
    let mut children: Vec<_> = (0..3)
        .map(|_| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_writer", "--nocapture"])
                .env("LATTICE_TURSO_TEST_ROOT", dir.path())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children.drain(..) {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(reader.count_runs().unwrap() >= 10);
    }
    assert_eq!(reader.count_runs().unwrap(), 30);
}
