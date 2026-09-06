//! Workspace-store behavior: schema, threshold placement, the reader rule,
//! read-only SQL, and mark-and-sweep gc.

use std::fs;

use lattice::{
    BodyInput, HistoryQuery, LatticeConfig, LatticeError, MachineStore, NewRun, Retention,
    SecretConfig, SqlValue, WORKSPACE_SCHEMA_VERSION, WorkspaceStore, now_ms, sha256_hex,
};

fn config(threshold: u64) -> LatticeConfig {
    LatticeConfig {
        inline_body_max: threshold,
        ..LatticeConfig::default()
    }
}

fn run<'a>(path: &'a str, body: &'a [u8]) -> NewRun<'a> {
    NewRun {
        started_at: now_ms(),
        duration_ms: Some(12),
        request_path: path,
        request_hash: "deadbeef",
        method: "GET",
        url: "http://127.0.0.1/x",
        status: Some(200),
        res_body: BodyInput::Bytes(body),
        res_content_type: Some("text/plain"),
        ..NewRun::default()
    }
}

#[test]
fn open_connection_honors_wal_flag() {
    let wal_dir = tempfile::tempdir().unwrap();
    WorkspaceStore::open(wal_dir.path(), LatticeConfig::default()).unwrap();
    let wal_db = wal_dir.path().join(".facet/lattice.db");
    let wal_mode: String = rusqlite::Connection::open(&wal_db)
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(wal_mode, "wal");

    let rollback_dir = tempfile::tempdir().unwrap();
    WorkspaceStore::open(
        rollback_dir.path(),
        LatticeConfig {
            wal: false,
            ..LatticeConfig::default()
        },
    )
    .unwrap();
    let rollback_db = rollback_dir.path().join(".facet/lattice.db");
    let rollback_mode: String = rusqlite::Connection::open(&rollback_db)
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rollback_mode, "delete");
}

#[test]
fn open_lays_down_the_facet_directory_and_schema() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), LatticeConfig::default()).unwrap();

    let facet = dir.path().join(".facet");
    assert!(facet.join("lattice.db").is_file());
    assert!(facet.join(".gitignore").is_file());
    assert!(facet.join("workspace.toml").is_file());
    assert!(lattice::is_ulid(store.workspace_id()));
    assert_eq!(store.schema_version().unwrap(), WORKSPACE_SCHEMA_VERSION);

    // Reopening keeps the identity and does not re-run migrations.
    let again = WorkspaceStore::open(dir.path(), LatticeConfig::default()).unwrap();
    assert_eq!(again.workspace_id(), store.workspace_id());
    assert_eq!(again.schema_version().unwrap(), WORKSPACE_SCHEMA_VERSION);

    let tables = again
        .query("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    let names: Vec<String> = tables
        .rows
        .iter()
        .map(|row| match &row[0] {
            SqlValue::Text(text) => text.clone(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(names, ["blobs", "runs", "schema_version"]);
}

#[test]
fn discover_walks_up_from_a_nested_path() {
    let dir = tempfile::tempdir().unwrap();
    WorkspaceStore::open(dir.path(), LatticeConfig::default()).unwrap();
    let nested = dir.path().join("users/deep");
    fs::create_dir_all(&nested).unwrap();
    let file = nested.join("list.yml");
    fs::write(&file, "x").unwrap();

    let found = WorkspaceStore::discover(&file).unwrap();
    assert_eq!(found, std::path::absolute(dir.path()).unwrap());
    assert!(WorkspaceStore::discover(&std::env::temp_dir().join("nope-facet")).is_none());
}

#[test]
fn bodies_land_inline_or_as_blobs_by_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config(8)).unwrap();

    let small = store.record_run(&run("a.yml", b"tiny")).unwrap();
    assert_eq!(small.res_body.retention(), "inline");
    assert_eq!(small.res_body.len, Some(4));
    assert!(small.res_body.hash.is_none());
    assert_eq!(store.response_body(&small).unwrap().unwrap(), b"tiny");

    let big = store.record_run(&run("a.yml", b"not tiny at all")).unwrap();
    assert_eq!(big.res_body.retention(), "blob");
    let hash = big.res_body.hash.clone().unwrap();
    assert_eq!(hash, sha256_hex(b"not tiny at all"));
    assert!(store.blobs_dir().join(&hash).is_file());
    assert_eq!(
        store.response_body(&big).unwrap().unwrap(),
        b"not tiny at all"
    );

    let blob = store.blob(&hash).unwrap().unwrap();
    assert_eq!(blob.len, 15);
    assert_eq!(blob.content_type.as_deref(), Some("text/plain"));
    assert!(store.blob("0000").unwrap().is_none());

    // Identical bodies share one file and one registry row.
    let again = store.record_run(&run("b.yml", b"not tiny at all")).unwrap();
    assert_eq!(again.res_body.hash.as_deref(), Some(hash.as_str()));
    assert_eq!(fs::read_dir(store.blobs_dir()).unwrap().count(), 1);
    let registry = store.query("SELECT count(*) FROM blobs").unwrap();
    assert_eq!(registry.rows[0][0], SqlValue::Integer(1));

    // Unretained bodies keep their length and nothing else.
    let unretained = store
        .record_run(&NewRun {
            res_body: BodyInput::Unretained { len: 99 },
            ..run("c.yml", b"")
        })
        .unwrap();
    assert_eq!(unretained.res_body.retention(), "none");
    assert_eq!(unretained.res_body.len, Some(99));
    assert!(store.response_body(&unretained).unwrap().is_none());
}

#[test]
fn reader_rule_never_branches_on_length() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config(1 << 20)).unwrap();
    let recorded = store.record_run(&run("a.yml", b"short")).unwrap();
    assert_eq!(recorded.res_body.retention(), "inline");

    // A stale or misleading length must not steer reads away from the inline column.
    let db = dir.path().join(".facet/lattice.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE runs SET res_body_len = 999999 WHERE id = ?1",
        rusqlite::params![recorded.id],
    )
    .unwrap();
    drop(conn);

    let row = store.run(&recorded.id).unwrap().unwrap();
    assert_eq!(row.res_body.len, Some(999999));
    assert!(row.res_body.hash.is_none());
    assert_eq!(
        store.response_body(&row).unwrap().unwrap(),
        b"short"
    );
}

#[test]
fn reader_rule_survives_a_threshold_change() {
    let dir = tempfile::tempdir().unwrap();
    // Written with a 4-byte threshold: 10 bytes becomes a blob.
    let blob_run = WorkspaceStore::open(dir.path(), config(4))
        .unwrap()
        .record_run(&run("a.yml", b"0123456789"))
        .unwrap();
    // Written with a 1 MiB threshold: the same 10 bytes stay inline.
    let inline_run = WorkspaceStore::open(dir.path(), config(1 << 20))
        .unwrap()
        .record_run(&run("a.yml", b"0123456789"))
        .unwrap();

    // A reader with yet another threshold reads both by the hash column.
    let reader = WorkspaceStore::open(dir.path(), config(0)).unwrap();
    let blob_row = reader.run(&blob_run.id).unwrap().unwrap();
    let inline_row = reader.run(&inline_run.id).unwrap().unwrap();
    assert_eq!(blob_row.res_body.len, inline_row.res_body.len);
    assert_eq!(blob_row.res_body.retention(), "blob");
    assert_eq!(inline_row.res_body.retention(), "inline");
    assert_eq!(
        reader.response_body(&blob_row).unwrap().unwrap(),
        b"0123456789"
    );
    assert_eq!(
        reader.response_body(&inline_row).unwrap().unwrap(),
        b"0123456789"
    );
}

#[test]
fn history_filters_and_orders_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config(1 << 20)).unwrap();
    for (index, path) in ["a.yml", "b.yml", "a.yml"].iter().enumerate() {
        store
            .record_run(&NewRun {
                started_at: 1_000 + index as i64,
                status: Some(if index == 1 { 500 } else { 200 }),
                actor: if index == 2 { "agent-x" } else { "human" },
                ..run(path, b"body")
            })
            .unwrap();
    }

    let all = store
        .history(&HistoryQuery {
            limit: 10,
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].started_at, 1_002);
    assert_eq!(all[2].started_at, 1_000);

    let only_a = store
        .history(&HistoryQuery {
            limit: 10,
            request_path: Some("a.yml".into()),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(only_a.len(), 2);

    let failures = store
        .history(&HistoryQuery {
            limit: 10,
            status: Some(500),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].request_path, "b.yml");

    let agent = store
        .history(&HistoryQuery {
            limit: 10,
            actor: Some("agent-x".into()),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(agent.len(), 1);

    let limited = store
        .history(&HistoryQuery {
            limit: 1,
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(limited.len(), 1);
}

#[test]
fn sql_escape_hatch_is_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config(1 << 20)).unwrap();
    store.record_run(&run("a.yml", b"body")).unwrap();

    let result = store
        .query("SELECT request_path, status, res_body_len FROM runs")
        .unwrap();
    assert_eq!(result.columns, ["request_path", "status", "res_body_len"]);
    assert_eq!(result.rows[0][1], SqlValue::Integer(200));

    let write = store.query("DELETE FROM runs");
    assert!(matches!(write, Err(LatticeError::ReadOnlyQuery)));
    assert_eq!(store.count_runs().unwrap(), 1);

    let multi = store.query("SELECT 1; DELETE FROM runs");
    assert!(multi.is_err());
    assert_eq!(store.count_runs().unwrap(), 1);

    let bad = store.query("SELEC nope").unwrap_err();
    assert!(bad.is_query_error(), "{bad}");
}

#[test]
fn gc_is_mark_and_sweep_with_a_dry_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config(2)).unwrap();
    let kept = store.record_run(&run("a.yml", b"kept body")).unwrap();
    let orphan_hash = sha256_hex(b"orphan body");
    fs::write(store.blobs_dir().join(&orphan_hash), b"orphan body").unwrap();

    let dry = store.gc(false).unwrap();
    assert!(!dry.applied);
    assert_eq!(dry.retention, Retention::Unlimited);
    assert_eq!(dry.runs_expired, 0);
    assert_eq!(dry.orphans, [(orphan_hash.clone(), 11)]);
    assert_eq!(dry.bytes_reclaimable, 11);
    assert!(store.blobs_dir().join(&orphan_hash).is_file());

    let applied = store.gc(true).unwrap();
    assert!(applied.applied);
    assert!(!store.blobs_dir().join(&orphan_hash).exists());
    assert!(
        store
            .blobs_dir()
            .join(kept.res_body.hash.as_deref().unwrap())
            .is_file()
    );
    assert_eq!(store.count_runs().unwrap(), 1);
}

#[test]
fn gc_expires_runs_by_retention_and_sweeps_their_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(
        dir.path(),
        LatticeConfig {
            inline_body_max: 2,
            history_retention: Retention::Days(1),
            ..LatticeConfig::default()
        },
    )
    .unwrap();
    let old = store
        .record_run(&NewRun {
            started_at: now_ms() - 3 * 86_400_000,
            ..run("old.yml", b"old body")
        })
        .unwrap();
    let fresh = store.record_run(&run("new.yml", b"new body")).unwrap();

    let dry = store.gc(false).unwrap();
    assert_eq!(dry.runs_expired, 1);
    assert_eq!(dry.orphans.len(), 1);
    assert_eq!(dry.orphans[0].0, old.res_body.hash.clone().unwrap());

    store.gc(true).unwrap();
    assert_eq!(store.count_runs().unwrap(), 1);
    assert!(store.run(&old.id).unwrap().is_none());
    assert!(store.run(&fresh.id).unwrap().is_some());
    assert!(
        !store
            .blobs_dir()
            .join(old.res_body.hash.as_deref().unwrap())
            .exists()
    );
}

#[test]
fn machine_store_indexes_runs_by_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config(1 << 20)).unwrap();
    let recorded = store.record_run(&run("a.yml", b"body")).unwrap();

    let machine = MachineStore::open_at(&dir.path().join("machine.db"), store.config()).unwrap();
    assert_eq!(
        machine.schema_version().unwrap(),
        lattice::MACHINE_SCHEMA_VERSION
    );
    machine
        .touch_workspace(store.workspace_id(), store.root(), Some("Pets"), now_ms())
        .unwrap();
    machine.index_run(&recorded, store.workspace_id()).unwrap();
    machine.index_run(&recorded, store.workspace_id()).unwrap();

    assert_eq!(machine.count_indexed_runs().unwrap(), 1);
    let workspaces = machine.workspaces().unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id, store.workspace_id());
    assert_eq!(workspaces[0].name.as_deref(), Some("Pets"));
}


#[test]
fn environments_round_trip_plain_and_secret() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::open(dir.path(), config(1 << 20)).unwrap();
    let machine = MachineStore::open_at(&dir.path().join("machine.db"), store.config()).unwrap();
    let wsid = store.workspace_id();

    // Plain value: stored in the value column, secret_ref NULL.
    machine
        .set_environment_with(wsid, "local", "API_URL", "http://x", false, &SecretConfig::keyring())
        .unwrap();
    assert_eq!(
        machine.environment_with(wsid, "local", "API_URL", &SecretConfig::keyring()).unwrap().as_deref(),
        Some("http://x")
    );

    // Secret value: stored via the encrypted backend, value NULL.
    let key_cfg = SecretConfig::encrypted(b"test-master-key");
    machine
        .set_environment_with(wsid, "local", "API_TOKEN", "tok-123", true, &key_cfg)
        .unwrap();
    assert_eq!(
        machine.environment_with(wsid, "local", "API_TOKEN", &key_cfg).unwrap().as_deref(),
        Some("tok-123")
    );

    // Listing returns metadata only; secrets are flagged, never returned.
    let mut rows = machine.environments(wsid).unwrap();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].key, "API_TOKEN");
    assert!(rows[0].secret);
    assert_eq!(rows[1].key, "API_URL");
    assert!(!rows[1].secret);

    // A secret cannot be read without the right key.
    assert!(machine
        .environment_with(wsid, "local", "API_TOKEN", &SecretConfig::encrypted(b"wrong"))
        .is_err());

    // Replacing a secret with a plain value drops the old secret_ref.
    machine
        .set_environment_with(wsid, "local", "API_TOKEN", "plain-now", false, &key_cfg)
        .unwrap();
    assert_eq!(
        machine.environment_with(wsid, "local", "API_TOKEN", &key_cfg).unwrap().as_deref(),
        Some("plain-now")
    );
    assert!(!machine.environments(wsid).unwrap().iter().any(|r| r.key == "API_TOKEN" && r.secret));

    // Delete removes the row.
    assert!(machine.delete_environment_with(wsid, "local", "API_TOKEN", &key_cfg).unwrap());
    assert!(machine.environment_with(wsid, "local", "API_TOKEN", &key_cfg).unwrap().is_none());
    assert!(!machine.delete_environment_with(wsid, "local", "API_TOKEN", &key_cfg).unwrap());
}
