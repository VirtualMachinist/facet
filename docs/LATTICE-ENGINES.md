# facet-lattice engines

**facet-lattice** keeps run history beside canonical OpenCollection YAML, and machine
sessions/indexes/preferences in a separate file. **Bundled SQLite (rusqlite) is
the default and only engine in the published [`facet-lattice`](https://crates.io/crates/facet-lattice)
crate (0.5.9).**

## Install the library

```sh
cargo add facet-lattice
# or pin: cargo add facet-lattice@0.5.9
```

SQLite-only (default):

```toml
[dependencies]
facet-lattice = "0.5.9"
```

Optional in-process analytics against SQLite files (heavy native build):

```toml
[dependencies]
facet-lattice = { version = "0.5.9", features = ["lattice-duckdb"] }
```

Install overview: [install.md](install.md).

## Configure Facet

Workspace `.facet/config.toml` overrides the machine file for workspace operations.
Session commands use machine configuration only; keep settings consistent in both
files.

```toml
[lattice]
wal = true
busy_timeout_ms = 5000
```

The workspace database is `.facet/lattice.db`; the machine database is
`$FACET_DATA_DIR/lattice.db` (or the platform XDG equivalent). Library callers
set `LatticeConfig` and inspect `WorkspaceStore::engine()` /
`MachineStore::engine()`.

## Existing files and rollback

Every opened store has a sibling `lattice.db.engine` marker containing `sqlite`.
Legacy unmarked database files belong to SQLite. Marker creation uses exclusive
creation and sync; invalid or incomplete markers fail closed. Keep the marker
with backups and restored files.

Before upgrades, stop Facet writers and preserve both database files, their
markers, any WAL/SHM files, workspace identity and blob directory as one
consistent backup. Rollback restores that complete snapshot and its matching
binary/configuration.

## Read-only SQL

`facet history --sql` opens a separate read-only connection to the workspace
SQLite store. SQLite retains its statement-readonly check and denies ATTACH/DETACH
through the parsed authorizer, including comment-prefixed forms. User SQL cannot
attach the machine database.

## Optional DuckDB analytics

With the `lattice-duckdb` feature, the bundled DuckDB crate can attach SQLite
lattice files in-process for analytics smoke tests. This is not part of the
default installation.

```sh
cargo test -p facet-lattice --features lattice-duckdb --test duckdb --locked
```

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

Tests cover schema, inline/hashed bodies, migrations, transaction rollback,
read-only queries, reopen, sessions/indexes/preferences, and thread/process
contention (`crates/lattice/tests/contention.rs`).
