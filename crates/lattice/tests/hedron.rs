//! HedronDB knowledge-graph engine smoke (apiary-only; `lattice-hedron` feature).
//!
//! Proves `hedron-core` (rusqlite 0.32, bundled) and this crate's default
//! engine (rusqlite 0.40, bundled) **coexist in one binary**: the test
//! links both bundled-SQLite copies and runs a real HedronDB operation (open
//! a store, bootstrap a vault + agent, put a document node, read it back
//! through HQL) inside the lattice test binary. This is the same coexistence
//! class as `lattice-turso` (two bundled SQLite copies in one binary); a green
//! smoke is the green light for the `lattice-hedron` feature.
//!
//! It does **not** open `lattice.db`. `hedron_core::Store::open` bootstraps its
//! own schema (`nodes`/`edges`/`desired_states`/`events`) on the file, so it
//! cannot read a Lattice workspace store — HedronDB is a **projection
//! complement**, not a same-file read engine like Turso. See
//! `agents/backend/notes/2026-09-07-evaluate-hedrondb.md` and
//! `docs/FACET.md` § Engines.
//!
//! APIARY-ONLY. Never enable on lathe (the `hedron-core` git dep + a second
//! bundled SQLite build is heavy; "No Facet cargo on lathe"). Run on apiary:
//!
//! ```text
//! cargo test -p lattice --features lattice-hedron --test hedron
//! ```
//!
//! The default `cargo test -p lattice` (feature off) does not build this
//! file or pull `hedron-core`.

#![cfg(feature = "lattice-hedron")]

use hedron_core::hql::Query;
use hedron_core::{Bootstrap, Node, Store};

/// Opens a HedronDB store in a temp dir, bootstraps a vault + agent, writes a
/// brief document, and reads it back through HQL — all inside the lattice
/// test binary that also links rusqlite 0.40 (the default engine). Green
/// proves the two engines coexist; the projection mapping is a later slice.
#[test]
fn hedron_core_coexists_with_rusqlite_default_engine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("hedron.db");

    // Write path: bootstrap the store and put a document node.
    let mut store = Store::open(&db).expect("open hedron store");
    let Bootstrap { vault, agent: _, token } = store
        .bootstrap("lattice", "facet", "/tmp/htec")
        .expect("bootstrap vault + agent");
    let doc = Node::brief_document(vault.id, "smoke", "2026-09-07").expect("build doc node");
    store.put_node(&token, doc).expect("put_node");
    drop(store);

    // Read path: HQL over the same file, read-only. `RoStore::open` uses
    // `SQLITE_OPEN_READ_ONLY`, so it does not mutate the file.
    let rows = Query::open(&db)
        .expect("open hql query")
        .vault("lattice")
        .run()
        .expect("hql vault read");
    let paths: Vec<String> = rows.into_iter().filter_map(|row| row.path).collect();
    assert!(
        paths.contains(&"briefs/2026-09-07/smoke".to_owned()),
        "document node present: {paths:?}"
    );
}
