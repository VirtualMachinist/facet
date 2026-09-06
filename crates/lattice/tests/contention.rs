//! Surface 4 fixture: N writers and M readers on one workspace store.
//! WAL + busy_timeout must let every write land and every read succeed.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use lattice::{BodyInput, HistoryQuery, LatticeConfig, NewRun, WorkspaceStore, now_ms};

const WRITERS: usize = 4;
const RUNS_PER_WRITER: usize = 25;
const READERS: usize = 2;

#[test]
fn concurrent_writers_and_readers_never_lose_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = Arc::new(dir.path().to_owned());
    let config = LatticeConfig {
        inline_body_max: 16,
        ..LatticeConfig::default()
    };
    // First open serializes schema creation before the race starts.
    WorkspaceStore::open(&root, config.clone()).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));

    let readers: Vec<_> = (0..READERS)
        .map(|_| {
            let root = Arc::clone(&root);
            let config = config.clone();
            let stop = Arc::clone(&stop);
            let reads = Arc::clone(&reads);
            thread::spawn(move || {
                let store = WorkspaceStore::open(&root, config).unwrap();
                while !stop.load(Ordering::Relaxed) {
                    let rows = store
                        .history(&HistoryQuery {
                            limit: 10,
                            ..HistoryQuery::default()
                        })
                        .expect("readers never block or fail under WAL");
                    for row in &rows {
                        store.response_body(row).unwrap();
                    }
                    reads.fetch_add(1, Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(1));
                }
            })
        })
        .collect();

    let writers: Vec<_> = (0..WRITERS)
        .map(|writer| {
            let root = Arc::clone(&root);
            let config = config.clone();
            thread::spawn(move || {
                let store = WorkspaceStore::open(&root, config).unwrap();
                for index in 0..RUNS_PER_WRITER {
                    let path = format!("writer-{writer}.yml");
                    let body = format!("writer {writer} run {index} body payload");
                    let body_input = if index % 2 == 0 {
                        BodyInput::Bytes(body.as_bytes())
                    } else {
                        BodyInput::Bytes(b"small")
                    };
                    store
                        .record_run(&NewRun {
                            started_at: now_ms(),
                            duration_ms: Some(1),
                            request_path: &path,
                            request_hash: "hash",
                            method: "GET",
                            url: "http://127.0.0.1/",
                            status: Some(200),
                            res_body: body_input,
                            actor: "contention",
                            ..NewRun::default()
                        })
                        .expect("writes retry within busy_timeout");
                }
            })
        })
        .collect();

    for writer in writers {
        writer.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for reader in readers {
        reader.join().unwrap();
    }

    let store = WorkspaceStore::open(&root, config).unwrap();
    assert_eq!(
        store.count_runs().unwrap(),
        (WRITERS * RUNS_PER_WRITER) as i64
    );
    assert!(reads.load(Ordering::Relaxed) > 0, "readers ran");
    let report = store.gc(false).unwrap();
    assert!(report.orphans.is_empty(), "every blob is referenced");
}
