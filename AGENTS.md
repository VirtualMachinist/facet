# Project Instructions

Probe is a native, local-first API client built with Rust, GPUI, Longbridge
gpui-base, and OpenCollection YAML. The CLI and desktop are equal adapters over
the same application and domain implementation.

## Read Only What the Task Needs

After this file, inspect the affected code and read only the matching reference:

| Work | Reference |
| --- | --- |
| Domain, repositories, persistence, HTTP, imports | [Architecture](docs/ARCHITECTURE.md) |
| CLI commands, JSON, selectors, exit codes | [CLI](docs/CLI.md) |
| Desktop interaction, themes, accessibility | [Design](docs/DESIGN.md) |
| Rust workflow, dependencies, GPUI, tests | [Development](docs/DEVELOPMENT.md) |
| Errors or logging | [Errors and logging](docs/ERRORS_AND_LOGGING.md) |
| Benchmarks or optimization | [Performance](docs/PERFORMANCE.md) |
| Future scope | [Roadmap](IMPLEMENTATION_PLAN.md) |

Do not read every document by default. The [documentation index](docs/README.md)
identifies the canonical source for each topic. README is product onboarding, not
required implementation context. For unfamiliar GPUI APIs, inspect the exact pinned
source and examples; pinned revisions are authoritative.

## Priorities

Resolve conflicts in this order: data compatibility, correctness, data safety,
programmatic/API stability, UI responsiveness, cross-platform compatibility,
maintainability, memory efficiency, visual polish.

## Invariants

- Business logic belongs in application/core code, never a frontend. CLI, GPUI, and
  future interfaces must share OpenCollection parsing, environment resolution,
  request construction and execution, authentication, and persistence operations.
- The domain must not depend on GPUI, gpui-base, CLI libraries, YAML, HTTP-client
  types, or filesystem APIs. Keep crate dependencies directed inward.
- OpenCollection YAML is canonical. Do not duplicate collection structure in a
  proprietary database or invent format extensions without approval.
- Preserve unknown YAML where practical. Writes must be atomic, report failures,
  and refuse to overwrite externally modified sources.
- Runtime RequestKey and FolderKey values are session-only. Persistent CLI and
  desktop-session references use repository locators.
- Request selection in an open desktop workspace is an O(1) in-memory operation
  with no filesystem, parsing, database, or network work.
- Filesystem, network, and expensive parsing or highlighting work must not block
  the GPUI thread.
- The CLI is non-interactive by default. Structured stdout is deterministic,
  versioned JSON; diagnostics go to stderr. Keep stable error categories and exit
  codes and bound large output.
- Desktop components consume Probe semantic tokens. Use Longbridge gpui-base
  (crate gpui-base / import gpui_base), never the separate gpui-component crate.
  Do not change pinned GPUI dependencies during unrelated work.

## Data Safety and Tests

Every newly supported OpenCollection feature needs fixture-based tests, including
load → modify → save → reload where relevant. Core behavior must be testable without
launching GPUI or invoking the CLI binary. Preserve unrelated worktree changes and
never use unsafe merely to bypass ownership problems.

Before completing a code change, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

Do not report completion while a required check fails.

## Scope

Implement only the requested feature. Do not add cloud sync, accounts, telemetry,
analytics, plugins, GraphQL, streaming protocols, Git provider APIs, or MCP without
explicit scope. Avoid unrelated refactors, inspect source instead of guessing, and
make the smallest coherent change.
