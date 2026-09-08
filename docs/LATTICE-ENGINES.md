# Lattice engines

Lattice keeps run history beside canonical OpenCollection YAML, and machine
sessions/indexes/preferences in a separate file. SQLite is the default. The
`lattice-turso` feature selects the actual Rust Turso driver at runtime when
`[lattice] engine = "turso"` is configured. Both stores and all their application
operations use that engine. There is no libSQL dependency or SQLite execution
fallback in the Turso branch.

## Build and configure

```sh
cargo build -p facet-cli --features lattice-turso --locked
```

For a new project, choose private configuration and data directories. Put this in
`$FACET_CONFIG_DIR/config.toml` (or the platform's normal Facet configuration
file):

```toml
[lattice]
engine = "turso"
wal = true
busy_timeout_ms = 5000
```

Set `FACET_DATA_DIR` to the project's machine-data directory and run Facet from
the project workspace. Workspace `.facet/config.toml` overrides the machine file
for workspace operations. Session commands intentionally use machine configuration
only; keep the engine setting consistent in both files. A mismatched engine
produces an explicit store error, which request recording reports in its existing
`recorded`/`indexed` outcome. Verify these fields; HTTP success alone does not
prove history was persisted.

The workspace database remains `.facet/lattice.db`; the machine database remains
`$FACET_DATA_DIR/lattice.db`. Existing library callers can set `LatticeConfig.engine`
and inspect `WorkspaceStore::engine()` / `MachineStore::engine()`. A build without
`lattice-turso` rejects a Turso selection rather than opening SQLite.

## Driver and runtime configuration

- Rust Turso source: `https://github.com/tursodatabase/turso`, commit
  `87c7a8516511c3ad2745c25b1079ea5c90112692` (`0.8.0-pre.8`). The same commit
  supplies the SQL parser used to authorize user queries. Cargo.lock pins the
  resolved dependency set. This reviewed revision is used for M1's native engine
  integration; it is not the former `libsql` package.
- Default driver features are disabled: no FTS, optional network sync, or global
  mimalloc allocator. Upstream still includes its sync support crates as ordinary
  dependencies; the network sync feature is not enabled.
- Local database mode, WAL required, experimental multiprocess WAL enabled, and
  the configured busy timeout. The multiprocess flag is needed for independent
  Facet CLI processes sharing history. It is an upstream experimental capability,
  so thread and process contention tests are part of the compatibility contract.
- The existing synchronous storage API uses `futures::executor::block_on` for
  local driver operations. The driver polls its own I/O; no nested Tokio runtime
  is constructed. As with SQLite, callers must keep storage I/O off the UI thread.
- Each write transaction begins IMMEDIATE, commits explicitly, and attempts
  rollback on error/drop. Migrations and run/blob registry inserts share the same
  application code for both engines. Body files retain the existing hash/threshold
  placement policy.

## Existing files and rollback

Every opened store has a sibling `lattice.db.engine` marker containing `sqlite`
or `turso`. Legacy unmarked database files belong to SQLite. Selecting Turso for
one is rejected before opening that database, and a marked Turso file is rejected
by the SQLite path. Marker creation uses exclusive creation and sync; invalid or
incomplete markers fail closed. Keep the marker with backups and restored files.
Do not edit/delete it to force an engine switch.

This implementation supports new Turso stores and numbered Lattice migrations
within the same engine. It does not yet provide a cross-engine import/export
command. Preserve existing SQLite history and select a separate workspace and
machine-data directory when evaluating Turso. Merely sharing a SQLite-format
header does not establish safe interchangeability, especially with active WAL.
Unknown future schema versions are rejected before request-body hydration.

Before upgrades, stop the project's Facet writers and preserve both database
files, their markers, any WAL/SHM files, workspace identity and blob directory as
one consistent backup. Keep the previous tested executable and configuration.
Rollback restores that complete snapshot and its matching binary/configuration;
it never changes only the engine setting on a live database. Remove only the
explicit project installation/data paths when uninstalling. Credentials stay in
approved runtime configuration, outside source, build artifacts and logs.

## Read-only SQL

`facet history --sql` opens a separate read-only connection to the selected
workspace engine. The Turso path accepts one SELECT (including WITH, VALUES and
compound selects), EXPLAIN SELECT or EXPLAIN QUERY PLAN SELECT. It rejects other
statements, PRAGMA commands and multiple statements before preparation. This is
a deliberately documented subset of the SQLite escape hatch. Invalid SQL remains
a query error in the CLI contract; storage failures retain their driver errors.

SQLite retains its statement-readonly check and now denies ATTACH/DETACH through
the parsed authorizer, including comment-prefixed forms. User SQL cannot attach
the machine database. Turso attachment and load-extension capabilities are not
enabled. Neither engine's SQL escape hatch replaces the authorized HedronDB
Store or HQL adapter.

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
cargo test -p lattice --features lattice-turso --locked
cargo test -p facet-cli --features lattice-turso --test facet_commands \
  turso_records_real_http_requests_and_recalls_sessions_across_cli_processes --locked
```

Tests cover the existing SQLite contracts and actual Turso application recording,
JSON tag filtering, inline/hashed bodies, v1 migration, future-schema rejection,
transaction rollback, read-only queries, reopen, sessions/indexes/preferences,
thread/process writers, and real HTTP recording/recall through separate Facet CLI
processes. These do not establish the complete Hedronetes M1 workflow: NixOS
integration, authenticated cluster actions, Herdr/OMP/Grok, HQL, system recovery,
and distro portability have their own acceptance gates.

## NixOS development environment

`flake.nix` / `flake.lock` provide the native C toolchain, CMake, pkg-config and
Nix's bindgen hook (including libclang and header search paths) needed by the
actual Turso dependency. Nixpkgs is pinned to
`c25784012c9982bca5b3e0de87e90bbdac8927d3`, rust-overlay to
`ca7f624be3935a5bc46d2c240515491ab8675503`, and the Nix development compiler to
Rust 1.98.1. The existing rustup toolchain and declared minimum remain 1.95;
local checks exercise that minimum and NixOS checks exercise the newer compiler.
This is a development shell, not yet a redistributable Facet Nix package.

On Tower, use only the explicitly named project Lima guest and disable remote
builders for every Nix invocation. Extract a clean `git archive` for the tested
Facet commit; use that immutable source as the flake input. Build from a separate
working copy so `target/` never enters Nix's input store:

```sh
nix develop path:/absolute/project/sources/facet-COMMIT \
  --option builders '' --option max-jobs 2 --option cores 2 \
  -c cargo test -p lattice --features lattice-turso --locked
nix develop path:/absolute/project/sources/facet-COMMIT \
  --option builders '' --option max-jobs 2 --option cores 2 \
  -c cargo test -p facet-cli --features lattice-turso --test facet_commands \
  turso_records_real_http_requests_and_recalls_sessions_across_cli_processes --locked
```

The shell limits Cargo to two jobs and disables debug information in dev/test
builds to bound guest disk consumption. It does not change shared host tools or
Nix configuration, and it must never dispatch to the protected `builder` lab.
The feature-enabled CLI executable is `target/debug/facet`; an explicit
project-local wrapper may enter this same pinned shell when launching it. Keep
such a wrapper and its required store closure with the installation provenance.
