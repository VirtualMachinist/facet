# Implementation Status and Roadmap

This document records Facet's current scope and future product work. It is not
required reading for ordinary implementation tasks; use the task-specific
references in [AGENTS.md](AGENTS.md) and the [documentation index](docs/README.md).

Upstream Probe's planning history informed the inherited core; this file is the
Facet-owned roadmap going forward.

## Current Foundation

### Inherited core (Probe)

The workspace still carries the product foundation Probe established for the first
desktop release:

- a Rust workspace with separate core, OpenCollection, HTTP, CLI, desktop, Postman,
  and Yaak crates;
- bundled and unbundled OpenCollection loading, validation, retained YAML, atomic
  persistence, external-change detection, and recovery-aware structural writes;
- an indexed in-memory workspace with repository-owned persistent selectors;
- shared environment resolution and management;
- one asynchronous HTTP engine used by both interfaces, with cancellation and bounded
  response handling;
- a deterministic, non-interactive CLI with versioned JSON, stable exit codes, request
  and workspace editing, and Postman and Yaak import;
- performance fixtures and benchmarks for workspaces up to 10,000 requests.

The public CLI contract is documented in [docs/CLI.md](docs/CLI.md). Shared
architecture and upstream desktop design remain in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and [docs/DESIGN.md](docs/DESIGN.md).
Code and tests remain authoritative when a document falls behind.

### Facet today

Facet-original surface on top of that core (see [docs/FACET.md](docs/FACET.md)):

- the `facet` binary beside `probe` on `PATH` (packages `facet-cli` / `probe-cli`);
- **Lattice** — workspace `.facet/lattice.db` plus a machine store for run history,
  bodies, sessions, and cross-workspace index;
- agent CLI: `history`, `blob`, `gc`, and recording on `request run`;
- Ratatui TUI (`facet tui`) with Graphite Honey / Porcelain Honey; vim-modal keys
  (default); `probe-desktop` remains a workspace member but is out of the default build;
- Facet-original crates (`facet`, `lattice`, `facet-tui`) under MIT; upstream-derived
  crates remain Apache-2.0.

## Planned Work

Folded from Probe's public roadmap
([rusty-probe.pages.dev](https://rusty-probe.pages.dev)): HTTP is live; next is
WebSocket, GraphQL, gRPC streaming, custom themes, git integration, secret
storage, and *and more*. Facet ships these independently where we already own
the layer, and contributes shared core upstream. Protocol work in `probe-core`
is written as an upstream PR; Facet TUI/CLI/Lattice adapters land in this tree
in parallel.

### Shipped (Facet)

| Probe item | Facet |
| --- | --- |
| 01 HTTP requests | Live. Same engine as Probe. Lattice records every `request run` / TUI send. |
| 05 Custom theme support | **Partial.** Graphite Honey (default) and Porcelain Honey, `:theme` toggle, `--appearance`. User-defined theme files are still open (see below). |
| 07 Secret storage | **Facet machine store.** OS keyring or `FACET_SECRET_KEY` XChaCha20-Poly1305. Probe desktop secret UX remains upstream. |

### Next (Probe-aligned)

Ship in Facet; offer the shared core upstream first when it touches
`probe-core` / `probe-cli`. Thread A (`protocol-session` on
`repos/probe-upstream`) stays parked until unparked — adapters can still be
designed against the event shape.

| # | Item | Facet slice | Upstream |
| --- | --- | --- | --- |
| 02 | WebSocket | TUI session pane + Lattice events + `facet` JSONL | Protocol session/event abstraction in `probe-core` |
| 03 | GraphQL | Collection item + TUI editor + history | Shared operation/variables model |
| 04 | gRPC streaming | Same session adapter as WebSocket | Streaming on the protocol session |
| 05 | Custom themes (rest) | Versioned theme files for `facet-tui` | Desktop theme files per [docs/DESIGN.md](docs/DESIGN.md#future-plain-text-themes) |
| 06 | Git integration | Optional status/diff/commit in TUI; filesystem stays the Git boundary | No provider coupling in core |
| 07 | Secret storage (rest) | Already in Lattice; wire TUI env editor to the machine store | Probe desktop |

### Facet-only — next slice (*and more*)

Probe's last public item is open-ended. Facet's is Lattice doing work:
send → see → compare → send again, with a ULID an agent can hold. Canonical
write-up: [docs/FACET.md](docs/FACET.md#next-slice). Product note:
`agents/fullstack/notes/2026-09-07-and-more.md` in the Lapis vault.

**Ship next** (ranked):

1. Session lifecycle — `facet session start/end`, mint-if-missing, `history --session`
2. Recall — `history --id` plus `--session/--environment/--tag/--hash`; TUI history grid hydrates the response pane
3. Replay — current YAML at the recorded env; warn on hash change; `--frozen` refuses
4. Hash-first diff — `facet diff <a> <b>`; exit 1 on mismatch
5. Secret hydration — Lattice env as `--var` before resolve
6. Dry-run + `--expect` — upstream-first; `--expect` is exit 6 `expect_failed`

**Load-bearing picks** for that slice: replay = current YAML; overlay secrets
now; MCP after 1–6. Still open: `--expect` exit **6 vs 1** (explore pass
argues 1 so agents do not retry assertion misses); git HEAD skip / auto-tag
rather than 0003.

MCP / harness adapter over Lattice must not parse CLI output. Engines stay
rusqlite default; DuckDB ATTACH is `scripts/duckdb-attach-demo.sh`. TUI
`ctrl+u` / `ctrl+d` deferred.

### Facet-only (later)

- TUI depth: collections browser, run inspector, vim-modal polish
- Public docs/brand seating and release tagging aligned with workspace version
- `history --follow --jsonl`, fixtures from blobs, watch, FTS5

### Inherited notes (still load-bearing)

#### User-defined theme files

Versioned, human-editable theme files after the semantic token model is stable.
Parsing and validation stay outside components; invalid themes fall back to
built-ins. Local presentation data, not OpenCollection content.
[docs/DESIGN.md](docs/DESIGN.md#future-plain-text-themes).

#### Streaming protocols

Shared protocol session/event abstraction before WebSocket, SSE, or gRPC.
Implementations independent of stdin/stdout and GPUI; CLI adapts events to
JSONL, TUI/desktop to visual sessions.

#### Git

Filesystem remains the primary Git boundary. Optional built-in status, diff,
branch, commit, pull, and push later, without coupling collections to a host.

#### MCP

Another adapter over the shared application layer. Must not duplicate business
logic or depend on parsing CLI output.

## Planning Rules

- Add a roadmap item only when it expresses product scope not already documented by
  current behavior or tests.
- Move implemented behavior to its canonical product or architecture document instead
  of retaining a completed phase checklist here.
- Do not use historical phase numbers as dependencies. Describe concrete prerequisites
  and affected architectural boundaries.
- Keep speculative provider integrations, cloud services, accounts, telemetry,
  analytics, plugins, and unsupported protocols out of scope until explicitly approved.
- Facet-only work stays in Facet crates; shared-core fixes are offered upstream first.
