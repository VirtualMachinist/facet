# Facet

Facet is Hedronite's in-house fork of Probe: the same core, CLI contract, and
OpenCollection YAML, cut for the terminal, with **Lattice** underneath to
remember every run. Codename G38. Upstream is `crizant/probe`; this fork lives
at `VirtualMachinist/facet` and stays cherry-pickable in both directions.

This document is the canonical reference for everything Facet adds. Everything
Probe already documents ([CLI](CLI.md), [Architecture](ARCHITECTURE.md)) holds
unchanged.

## Boundary with upstream (Surface 7)

| Stays in Facet | Goes upstream first |
| --- | --- |
| `crates/lattice` (run history, blobs, `--sql`, gc) | Agent CLI ergonomics, JSON envelope fixes |
| `crates/facet` (the `facet` binary), `crates/facet-record` (shared recording path) | Performance work in `probe-core` |
| `crates/facet-tui` (ratatui interface) | Bug fixes in any upstream crate |

Rules:

- `probe-core` and `probe-cli` are never edited in a Facet slice. Anything that
  must change there is written in upstream style and offered as a Probe PR.
- No upstream crate is renamed. New crates carry Facet names.
- The `facet` binary delegates every Probe command to `probe-cli` verbatim. Only
  `request run` (to record), `history`, `blob`, `gc`, and `tui` are Facet's.
- `probe-desktop` stays a workspace member but is out of `default-members`, so
  `cargo build` and `cargo test` skip GPUI. `cargo build --workspace` includes it.

### License and attribution

Decided by Evan on 2026-09-06 (Surface 7, closed):

- **Upstream-derived files stay Apache-2.0.** Probe's `LICENSE` (Apache License
  2.0) governs `crates/cli`, `crates/core`, `crates/desktop`, `crates/http`,
  `crates/opencollection`, `crates/postman`, `crates/yaak`, and the docs they
  came with. They are never relicensed, so Probe PRs from this tree stay legal.
  Files modified from upstream carry a change notice (Section 4(b)).
- **Facet-original crates are MIT:** `crates/facet`, `crates/facet-record`,
  `crates/lattice`, `crates/facet-tui`. Each carries `LICENSE-MIT` and declares `license = "MIT"`
  in its own `Cargo.toml`. `crates/facet-tui` adapts palette values from the
  upstream desktop theme; that derivation is noted in `NOTICE`.
- **Copyright:** `Copyright 2026 Hedronite` for the Facet-original work.
  Upstream copyright notices are kept as found; the root `LICENSE` text is
  upstream's and is left byte-identical. Git identity for the fork stays
  VirtualMachinist; the copyright holder is Hedronite.
- The root `NOTICE` file states the fork relationship and the per-crate terms.
  Distributions include `LICENSE`, `NOTICE`, and the per-crate `LICENSE-MIT`.
- The workspace `Cargo.toml` still declares `license = "MIT OR Apache-2.0"`
  for inherited upstream crates; that is upstream's own declaration and is
  left alone.

## Binaries

`facet` and `probe` build from the same workspace and coexist on `PATH`:

```bash
cargo build --release -p probe-cli -p facet-cli
facet --version   # facet 0.5.7 (probe 0.5.7)
probe --version   # probe 0.5.7
```

`facet --version --json` returns `name`, `version`, and `probeVersion`.

## Lattice layout

Two SQLite stores, both via bundled `rusqlite` (the default engine). Turso
and an in-process DuckDB are available behind cargo features (neither is
the default; both are off in a default build). DuckDB is also usable
out-of-process: the `duckdb` CLI ATTACHes the SQLite file for analytics
(see [Engines](#engines)).

| Store | Path | Holds |
| --- | --- | --- |
| Workspace | `.facet/lattice.db` beside the collection | Run history for that workspace |
| Machine | `~/.local/share/facet/lattice.db` (XDG; platform equivalent elsewhere) | Workspace registry, cross-workspace run index, sessions, environments, preferences |

`.facet/` also holds `workspace.toml` (the workspace ULID, commit it),
`config.toml` (optional, commit it), `blobs/` (content-addressed bodies), and a
`.gitignore` that excludes the database and blobs. The workspace root is the
directory for an unbundled collection or the parent directory of a bundled
file.

Override the machine paths with `FACET_DATA_DIR` and `FACET_CONFIG_DIR`.

The schema is `FACET_HANDOFF_BRIEF.md` Section III, applied as numbered SQL
migrations under `crates/lattice/migrations/{workspace,machine}/`. Every
migration inserts its own `schema_version` row. Times are Unix milliseconds
UTC; ids are ULIDs.

### Bodies

Bodies at or under `inline_body_max` (default 64 KiB) are stored in the row.
Larger bodies are written to `.facet/blobs/<sha256>` and the row keeps the
hash, length, and content type. Identical bodies share one file.

Reader rule: **a non-NULL hash means a blob file; otherwise read the inline
column. Never branch on length.** Rows written under an older threshold stay
readable. A body the transport did not retain (over Probe's 16 MiB in-memory
bound with no spool file) records its length only; `retention` is `none`.

### What is recorded

The recording path is one function, `facet_record::record`, shared by the
`facet` CLI and `facet tui` so both adapters write identical rows.

For each `facet request run`: start time, duration, selector, environment,
method, resolved URL, status (NULL on transport failure) or error text,
request and response headers, bodies, content type, actor, session, tags, and
a `requestHash` (SHA-256 of a canonical view of the resolved request; bodies
enter by hash, authentication by scheme only).

Redaction: `Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie`,
`X-Api-Key`, `X-Auth-Token`, `Api-Key`, and `X-Amz-Security-Token` values are
stored as `<redacted>`. URL userinfo (`user:pass@`) is redacted. Request bodies
are stored as sent; do not put secrets in bodies you want recorded.

The machine store gets one pointer row per run (best effort; `indexed: false`
in the output means the run is recorded but not indexed).

Environment: `FACET_ACTOR` names the actor (default `human`);
`FACET_SESSION` sets `sessionId`; `FACET_NO_RECORD=1` disables recording.

### Configuration

```toml
# .facet/config.toml (workspace) | ~/.config/facet/config.toml (machine)
# Precedence: CLI flag > workspace file > machine file > built-in default.
[lattice]
inline_body_max   = "64KiB"     # KiB, MiB, GiB, or bytes; "0" means every body is a blob
history_retention = "unlimited" # or "30d", "90d"
wal               = true
busy_timeout_ms   = 5000
```

Flags: `--inline-body-max`, `--history-retention`. Agents get flags; humans
get files.

### Secrets at rest (Surface 3)

Environment values that are secrets (tokens, keys) never sit in plaintext in
the machine store. Two backends, picked by environment:

- **OS keyring** (default, desktop): the `keyring` crate (macOS Keychain,
  Windows Credential Manager, Linux Secret Service). The `environments` row
  stores a `secret_ref` of the form `kr:<user>`; `value` is NULL. The secret
  itself lives in the keyring under the `facet` service.
- **Encrypted** (headless/CI fallback): XChaCha20-Poly1305 with the master key
  derived from `FACET_SECRET_KEY` via HKDF-SHA256. `secret_ref` stores
  `enc:v1:<base64(nonce||ciphertext)>`; `value` is NULL.

Selection: `FACET_SECRET_KEY` set and non-empty → encrypted; otherwise the OS
keyring. There is never a passphrase prompt (it breaks agent use). If no
keyring backend is available and `FACET_SECRET_KEY` is unset, `set_environment`
fails with a clear error telling the user to set `FACET_SECRET_KEY`.

Reading is by the `secret_ref` prefix, so a secret written under one backend
stays readable under the other as long as its key is available. Non-secret
values stay in `value` with `secret_ref` NULL. Replacing a secret with a
plain value (or deleting the row) cleans up the old keyring entry.

`MachineStore::{set_environment, environment, environments,
delete_environment}` are the API; `environments` returns metadata only
(name, key, `secret`, `updated_at`) and never a secret value.

### Concurrency (Surface 4)

Connections open in WAL mode with `busy_timeout` (5 s default) and use short
immediate write transactions. Readers never block. Schema creation runs inside
one immediate transaction so concurrent first opens serialize.
`crates/lattice/tests/contention.rs` is the N-writer, M-reader fixture.

### Engines

The default engine is **rusqlite** (bundled SQLite). Two optional cargo
features, both off by default, add engines without changing the on-disk
file format:

| Feature | Engine | Notes |
| --- | --- | --- |
| `lattice-turso` | Turso / libSQL (local mode) | Same file format as rusqlite (libSQL is a SQLite fork); flipping the flag requires no migration. Smoke: `crates/lattice/tests/turso.rs` (open/write/read a workspace store through libSQL). The smoke is libsql-only: rusqlite and libsql both bundle SQLite and cannot coexist in one binary (libsql's `sqlite3_config(SERIALIZED)` returns `SQLITE_MISUSE` after rusqlite initializes SQLite; `skip_safety_assert` is `unsafe` and the workspace forbids it). Verified still true 2026-09-06 (libsql 0.9.30). |
| `lattice-duckdb` | DuckDB in-process (bundled) | **Apiary-only; never in lathe default members.** ATTACHes the SQLite lattice file for analytics. Smoke: `crates/lattice/tests/duckdb.rs`. Heavy native build. |

The primary analytics path is the **`duckdb` CLI** attaching the SQLite
file externally (no Rust, no feature flag):

```text
duckdb -c "INSTALL sqlite; LOAD sqlite; \
  ATTACH '/path/to/.facet/lattice.db' AS lattice (TYPE SQLITE); \
  SELECT status, count(*) FROM lattice.runs GROUP BY status;"
```

`INSTALL sqlite` downloads the `sqlite` extension on first use (network
needed once). The in-process `lattice-duckdb` feature is a convenience
for embedding the same ATTACH in Rust; it is not required for analytics.

Feature-gated tests use `required-features` in `crates/lattice/Cargo.toml`,
so a default `cargo test -p lattice` (features off) never pulls libsql,
tokio, or duckdb. See `agents/backend/notes/2026-09-06-facet-engines.md`
in the Lapis vault for the full runbook.


## Commands

```text
facet request run <path> <selector> [<probe request run flags>] [--no-record] [--tag <tag>]... [--inline-body-max <size>] [--json]
facet history [<path>] [--limit <n>] [--request <selector>] [--status <code>] [--actor <name>] [--since <unix-ms>] [--bodies] [--json]
facet history [<path>] --sql "<query>" [--json]
facet blob <hash> [<path>] [--output <file>] [--json]
facet gc [<path>] [--history-retention <r>] [--yes] [--json]
facet tui [<path>] [--appearance graphite|porcelain]
```

Graphite Honey is the TUI default. `:theme` (bare) toggles Porcelain Honey;
`:theme graphite|porcelain` sets one. `:history` and `:sql <query>` open
Lattice overlays. `gg` / `G` jump to the first / last row of the focused
pane. `?` lists the rest.

`<path>` for `history`, `blob`, and `gc` is any file or directory inside the
workspace (default: the current directory); Facet walks up to the nearest
`.facet/lattice.db`. `--json` and `--quiet` follow the upstream rules.

### `request run`

Executes exactly as `probe request run` (same resolution, engine, `--output`
streaming, Ctrl-C cancellation) and returns the upstream JSON with one
additive field:

```json
"lattice": {
  "recorded": true,
  "runId": "01JAB…",
  "workspaceId": "01JAA…",
  "indexed": true,
  "requestHash": "9f86d0…",
  "responseBody": { "sizeBytes": 12, "hash": null, "retention": "inline" }
}
```

When not recorded: `{"recorded": false, "reason": "disabled" | "stdin_workspace" | "error", "message": "…"}`.
A stdin (`-`) workspace has no root and is never recorded. A Lattice failure
never changes the run's exit code; it is reported in `lattice` and on stderr.
A transport failure is recorded with `status: null` and the error text, then
the upstream error envelope is returned with `error.details.lattice`.

### `history`

Metadata only, newest first, default limit 50. Each run:

```json
{
  "id": "01JAB…", "startedAt": 1757160000123, "durationMs": 128,
  "requestPath": "items/0", "requestHash": "9f86d0…", "environment": "local",
  "method": "POST", "url": "http://…", "status": 200, "error": null,
  "actor": "human", "sessionId": null, "tags": [],
  "request":  { "headers": [ { "disabled": false, "name": "X-Probe", "value": "…" } ],
                "body": { "sizeBytes": 16, "hash": null, "retention": "inline" } },
  "response": { "headers": [ { "name": "content-type", "value": "application/json" } ],
                "contentType": "application/json",
                "body": { "sizeBytes": 12, "hash": null, "retention": "inline" } }
}
```

`retention` is `inline`, `blob`, or `none`. With `--bodies`, inline bodies add
the upstream body fields (`content`, `encoding`, `omitted`, `omissionReason`);
blob bodies are always omitted with `omissionReason: "blob"` (pull them with
`blob <hash>`). The envelope carries `workspace: { id, path }`, or `null` with
`runs: []` when no store exists.

### `history --sql`

Runs one read-only statement against the workspace store on a read-only
connection with `query_only` set:

```json
{ "schemaVersion": 1, "workspace": { "id": "…", "path": "…" },
  "columns": ["request_path", "status"], "rows": [["items/0", 200]] }
```

BLOB columns render as `{ "type": "blob", "sizeBytes": n }`. Writes fail with
`sql_read_only`; parse failures with `invalid_sql` (both exit 2). A missing
store is `lattice_not_found` (exit 9).

### `blob`

Writes the raw bytes to stdout. With `--json`:

```json
{ "schemaVersion": 1, "blob": { "hash": "…", "sizeBytes": 12, "contentType": "application/json",
  "body": { "content": "…", "encoding": "utf8", "omitted": false, "omissionReason": null, "outputPath": null } } }
```

`--output <file>` writes the bytes to a file and returns `outputPath`. Bodies
over 16 MiB are omitted from JSON with `too_large`. Unknown hashes are
`blob_not_found` (exit 4).

### `gc`

Mark-and-sweep, manual, dry run by default. Runs older than
`history_retention` expire; blob files no surviving run references are
orphans. Nothing is deleted without `--yes`.

```json
{ "schemaVersion": 1, "workspace": { "id": "…", "path": "…" }, "applied": false,
  "retention": "unlimited", "runsExpired": 0,
  "orphans": [ { "hash": "…", "sizeBytes": 3 } ], "registryOrphans": 0, "bytesReclaimable": 3 }
```

## Exit codes and error categories

Facet extends the upstream table; it never renumbers it.

| Code | Category |
| ---: | --- |
| 0–8 | As in [CLI](CLI.md#exit-codes) |
| 9 | Lattice store failure (`lattice_error`, `lattice_not_found`) |

Additional stable categories: `blob_not_found` (exit 4), `invalid_sql` and
`sql_read_only` (exit 2). Every JSON document carries `schemaVersion: 1`;
fields are added compatibly and never removed or retyped within a version.

## Tests

- `crates/facet/tests/facet_commands.rs`: one golden file per command under
  `crates/facet/tests/golden/` (`UPDATE_GOLDEN=1` rewrites them), plus
  recording, reader-rule, read-only SQL, and gc behavior through the binary.
- `crates/lattice/tests/store.rs`: schema, threshold placement, reader rule
  across threshold changes, gc, machine index, environments (plain + secret).
- `crates/lattice/tests/contention.rs`: Surface 4 fixture.
- `crates/lattice/src/secrets.rs` (unit): encrypted round-trip, wrong-key
  failure, ref-prefix classification, empty/unknown refs.

Facet is compiled and tested on the build host, not on lathe (see
`agents/SQUAD.AGENTS.md` in the Lapis vault).
