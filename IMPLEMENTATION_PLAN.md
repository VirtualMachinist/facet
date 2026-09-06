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

### Facet-first

These require explicit task scope before implementation:

- Lattice engine options behind cargo features (`lattice-turso`; analytics via external
  DuckDB ATTACH — not a default product path);
- TUI depth (collections browser, run inspector, vim-modal command surface polish);
- Public docs/brand seating and release tagging aligned with workspace version;
- Anything that changes Facet-only contracts in [docs/FACET.md](docs/FACET.md).

### Inherited deferred (still open upstream)

The following were intentionally deferred in Probe's plan and remain out of Facet's
default scope until explicitly tasked. Prefer contributing shared core work upstream
when it belongs in `probe-core` / `probe-cli`.

#### User-Defined Themes

Add versioned, human-editable theme files after the semantic token model is stable.
Parsing and validation must remain outside components, invalid themes must fall back
safely to built-ins, and theme configuration must remain local presentation data rather
than OpenCollection content. Design contract:
[docs/DESIGN.md](docs/DESIGN.md#future-plain-text-themes).

#### Streaming Protocols

Design a shared protocol session/event abstraction before adding WebSocket, SSE, or
gRPC. Protocol implementations must be independent of stdin/stdout and GPUI; the CLI
may adapt events to JSONL while a desktop/TUI adapts the same events to visual sessions.

#### Git Integration

The filesystem remains the primary Git boundary. Optional built-in status, diff,
branch, commit, pull, and push workflows may be added later without coupling core
collection behavior to a hosting provider.

#### MCP Interface

An MCP server may eventually become another adapter over the shared application layer.
It must not duplicate business logic or depend on parsing CLI output.

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
