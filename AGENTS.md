# Project Instructions

## Product

This project is a fast, native, local-first API client.

Primary platforms:

- macOS
- Windows
- Linux

Technology:

- Rust
- GPUI
- Longbridge `gpui-base` (crate `gpui-base` from `longbridge/gpui-component`)
- OpenCollection YAML

The product has two first-class interfaces:

- CLI for developers, automation, CI, and AI agents
- GPUI desktop application for humans

Both interfaces MUST use the same application/domain implementation.

A future MCP interface may also use the same core.

---

# Priority Order

When requirements conflict, follow this order:

1. Data compatibility
2. Correctness
3. Data safety
4. Programmatic/API stability
5. UI responsiveness
6. Cross-platform compatibility
7. Maintainability
8. Memory efficiency
9. Visual polish

---

# Core Architectural Rule

Business logic must never belong to a frontend.

Conceptually:

                  Application/Core
                    ▲         ▲
                    │         │
                   CLI       GPUI

Future:

                  Application/Core
                ┌─────┼─────┐
                │     │     │
               CLI   GPUI   MCP

CLI, GPUI, and future MCP code are adapters.

They must not independently implement:

- OpenCollection parsing
- environment resolution
- request construction
- HTTP execution
- authentication behavior
- persistence rules

---

# Crate Boundaries

Prefer explicit crate boundaries.

Suggested structure:

crates/
├── core/
├── opencollection/
├── http/
├── cli/
└── desktop/

Responsibilities:

core:
- domain models
- workspace
- environment resolution
- application operations

opencollection:
- YAML representation
- parsing
- serialization
- filesystem repository

http:
- HTTP execution
- response handling

cli:
- command parsing
- human output
- structured output
- exit codes

desktop:
- GPUI
- Longbridge gpui-base
- desktop presentation

Do not introduce circular dependencies.

---

# CLI

The CLI is a permanent product interface, not a temporary test harness.

It is specifically intended to support:

- AI coding agents
- CI
- shell automation
- developers
- headless environments

Design commands for programmatic use first.

## Structured Output

Commands that return meaningful data should support:

--json

Structured output must:

- be valid JSON
- be deterministic
- avoid terminal escape sequences
- avoid progress animations
- avoid unrelated log output on stdout
- use stable documented fields

stdout is for command output.

stderr is for diagnostics/logging when appropriate.

Do not mix logs into JSON stdout.

## Exit Codes

Commands must use meaningful, stable exit codes.

At minimum distinguish:

- success
- invalid arguments
- invalid workspace
- request not found
- configuration/environment error
- network/execution error

Do not require agents to parse English error messages to determine the
error category.

## Interactivity

Do not require interactive prompts for normal operations.

If information is required, prefer explicit arguments.

Interactive convenience features may exist for humans but must not be
required for automation.

## Large Output

Do not blindly write arbitrarily large response bodies to structured
stdout.

Support explicit file output or other bounded behavior for large
responses.

Streaming protocols should support machine-readable streaming output,
such as JSONL, where appropriate.

---

# GPUI

GPUI is the desktop application and rendering framework.

The macOS application must feel native and follow the current Apple Human
Interface Guidelines. This includes platform-appropriate window structure,
menus, terminology, keyboard shortcuts, focus behavior, accessibility, and
system appearance. Inspect the current official guidance before implementing
an unfamiliar interaction pattern.

The Windows application must follow the current Microsoft Windows App Design
guidance and Fluent Design conventions. Respect standard Windows windowing,
menus, keyboard behavior, input methods, system appearance, high-contrast mode,
display scaling, and shell integration.

Linux has no single universal desktop HIG. Follow common cross-desktop Linux
conventions and applicable freedesktop.org standards. Use the current GNOME and
KDE Human Interface Guidelines as references, and verify behavior on both GNOME
and KDE Plasma where practical. Do not dynamically imitate GTK/Adwaita or
Qt/Breeze styling; maintain one coherent Probe design while respecting system
fonts, appearance preferences, scaling, accessibility, input, and desktop
integration.

Do not impose one platform's conventions on another. Share application behavior
and semantic design tokens while allowing presentation and interaction details
to vary by platform.

Follow [docs/DESIGN.md](docs/DESIGN.md) for the desktop design system. Components
must consume semantic design tokens rather than hard-code colors or read theme
configuration directly. The visual system must remain compatible with future
user-defined themes loaded from plain-text files, but do not implement theme-file
loading before its planned phase.

GPUI APIs may evolve.

Always treat the exact version/revision pinned by this repository as
authoritative.

Before using an unfamiliar GPUI API:

1. Inspect the pinned source.
2. Inspect examples matching the pinned revision.
3. Prefer patterns already used in this repository.
4. Do not rely on remembered APIs from other GPUI versions.

Do not copy large portions of Zed architecture merely because Zed
uses GPUI.

Do not introduce:

- Electron
- Tauri
- WebView
- React
- Flutter
- another GUI framework

without explicit approval.

---

# gpui-base

Probe uses **Longbridge `gpui-base`**, the foundation crate in
[`longbridge/gpui-component`](https://github.com/longbridge/gpui-component).
Docs: https://longbridge.github.io/gpui-component/base/

Cargo dependency (always pin a git revision):

```toml
gpui-base = { git = "https://github.com/longbridge/gpui-component", rev = "…" }
```

Rust crate and import path: `gpui-base` / `gpui_base`.

Call `probe_desktop::theme::Theme::init(cx)` once before opening windows
(this calls `gpui_base::init` and installs Probe's Catppuccin semantic tokens).

## Do not confuse these libraries

These are different crates. Do not substitute one for another:

| Name | Repository | Role in Probe |
| --- | --- | --- |
| **gpui-base** | `longbridge/gpui-component` (`crates/base`) | Required. Primitives plus default chrome tokens. |
| **gpui-component** | `longbridge/gpui-component` (`crates/ui`) | Forbidden. Pre-styled façade. |

Never depend on `gpui-component`. Never copy `gpui_component::` APIs or
`.primary()` theme helpers into Probe. Desktop chrome uses Catppuccin Latte
(light) and Mocha (dark) through Probe's semantic theme model.

Pin GPUI to the Zed revision required by the pinned `gpui-base` commit.
If Cargo reports incompatible GPUI types, the GPUI pin drifted from
gpui-base — fix the pin, do not mix two GPUI revisions. Do not add
`rev` to Probe's GPUI git URL; gpui-base uses the same unpinned source
and a mismatched revision compiles two GPUIs. Pin the Zed commit in
`Cargo.lock` (`cargo update -p gpui --precise <sha>`) to the revision
in that gpui-base commit's `Cargo.lock`.

## Behavior vs appearance

Treat behavior and appearance separately.

gpui-base:
- reusable primitives (`Button`, `Input`/`InputState`, `Select`, `Switch`,
  `Toggle`, `Popover`, `Popup`, …)
- interaction behavior
- accessibility
- component mechanics

application (Probe):
- maps Catppuccin Latte/Mocha into semantic roles
- domain colors (methods, status, syntax)
- compositions in `crates/desktop/src/components.rs`

Project Probe's `Theme` into `gpui_base::Theme` so primitives and
infrastructure (scrollbars, resize handles) share the same tokens.

Before using an unfamiliar gpui-base API:

1. Inspect the exact pinned source under the git revision in
   `crates/desktop/Cargo.toml`.
2. Inspect `crates/base` examples/tests in that revision.
3. Verify the API exists on `gpui_base::…`.
4. Do not guess from `gpui-component` or remembered APIs.

Do not fork gpui-base merely to change styling. Style primitives with
GPUI `Styled` methods, `.styles(|s| …)` semantic states, and Probe tokens.

Custom GPUI components are appropriate when gpui-base does not provide
the required behavior or specialized performance is necessary. The
response viewer and request-body editor are examples: they need
virtualization and overlay highlighting that the stock single-line
`Input` does not provide.

---

# Domain Independence

The domain layer must not depend on:

- GPUI
- gpui-base
- CLI libraries
- YAML
- HTTP client implementation details
- filesystem APIs

Domain models represent application concepts, not serialization
formats or widgets.

---

# Workspace Repository

Do not couple the domain directly to OpenCollection YAML.

Use a repository boundary conceptually equivalent to:

WorkspaceRepository
        ▲
        │
OpenCollectionRepository

This allows future adapters such as:

- Yaak
- other open formats

without rewriting the CLI, GPUI application, or HTTP engine.

Only OpenCollection is required initially.

Do not implement future formats without explicit instruction.

---

# Filesystem

OpenCollection YAML is the canonical collection representation.

Do not introduce a proprietary database for:

- requests
- folders
- environments
- collection structure

A database may later store local-only data such as:

- history
- response metadata
- UI state
- caches

Do not duplicate the canonical collection into a database.

---

# In-Memory Workspace

Request metadata for an active desktop workspace should normally remain
in memory.

Selecting a request must not:

- read YAML
- access the filesystem
- query a database
- access the network
- synchronously parse data

Request lookup should normally be O(1).

---

# Rust Ownership

Prefer simple ownership.

Do not introduce:

- Arc
- Mutex
- RwLock
- RefCell
- shared global mutable state

merely to silence ownership errors.

Before introducing synchronization primitives, consider:

- normal ownership
- GPUI Entity ownership
- message passing
- task results
- immutable shared data

Never use unsafe Rust solely to work around ownership problems.

Unsafe code requires explicit justification.

---

# Async and Background Work

Never block the GPUI UI thread with:

- filesystem operations
- HTTP requests
- large YAML parsing
- large JSON parsing
- Git operations
- expensive syntax highlighting

The CLI may await operations normally because it has no UI event loop,
but networking/domain APIs must remain suitable for use asynchronously
by the desktop application.

Do not create separate synchronous and asynchronous business logic for
CLI and GPUI merely for convenience.

---

# Persistence

Changes update the in-memory model first where applicable.

Filesystem persistence happens afterward.

Writes should:

- be atomic
- report failures
- preserve unknown data where practical
- avoid overwriting externally modified files

Desktop persistence should not block the UI.

---

# OpenCollection Compatibility

Follow the current OpenCollection specification.

Do not invent proprietary extensions unless explicitly approved.

Unknown YAML fields should be preserved whenever practical.

Reading and writing a supported file must not silently destroy
information the application does not understand.

Every newly supported OpenCollection feature requires fixture-based
tests.

Important tests should include:

load
→ modify
→ save
→ reload

and verify semantic equivalence.

Where useful, compatibility may also be checked against Bruno tooling.

---

# HTTP

There must be one shared HTTP implementation.

CLI:

request run
      ↓
shared HTTP engine

Desktop:

Send button
      ↓
same shared HTTP engine

Never create separate request-building behavior for the two
interfaces.

HTTP functionality should be testable without CLI or GPUI.

---

# Streaming Protocols

WebSocket, SSE, and gRPC streaming must be represented by shared
protocol/session abstractions.

Do not design the networking layer around stdin/stdout.

Instead:

Protocol Engine
      ↓
events/session
      ↓
 ┌────┴────┐
 CLI      GPUI

The CLI adapts events to:

- JSONL
- stdout
- stdin
- optional interactive terminal behavior

GPUI adapts the same events to visual components.

---

# Performance

Performance is a core product feature.

Desktop target:

- 10,000 requests
- visually instantaneous request switching
- responsive while requests execute
- responsive while files save
- large responses do not freeze navigation

CLI target:

- fast startup
- minimal unnecessary initialization
- efficient headless operation

Measure before introducing complex optimizations.

---

# Response Handling

Do not assume every response should exist indefinitely in memory.

Prefer:

active/recent responses
→ bounded memory

older/large responses
→ filesystem where appropriate

Avoid unnecessary copies of large bodies.

The desktop response viewer may use:

- virtualization
- chunked rendering
- lazy loading
- background parsing

The CLI must not accidentally duplicate huge bodies simply to format
output.

---

# Dependencies

Before adding a crate:

1. Check existing dependencies and the standard library.
2. Prefer actively maintained crates.
3. Verify platform support where relevant.
4. Avoid large dependencies for trivial functionality.
5. Do not replace dependencies without a clear reason.

Pin GPUI and Longbridge gpui-base together. The GPUI git revision must
match the revision required by the pinned `gpui-base` commit.

Do not upgrade GPUI or gpui-base as part of unrelated work. Do not add
`gpui-component`.

---

# Testing

Core functionality must be testable without launching GPUI or invoking
the CLI binary.

Maintain fixtures under:

tests/fixtures/

Test:

- OpenCollection parsing
- serialization
- workspace behavior
- environment resolution
- HTTP request construction
- persistence
- protocol behavior

CLI-specific integration tests should additionally verify:

- command behavior
- JSON output
- exit codes

Do not add automated tests for UI spacing, padding, radii, typography sizes,
palette values, or contrast ratios. Review appearance visually against
[docs/DESIGN.md](docs/DESIGN.md). Tests may still cover behavior such as
light/dark appearance selection, pane resize constraints, focus, and
highlight span ranges without asserting pixel or color values.

Before adding a desktop test, check whether an existing test already builds
the same workspace, selection, and UI surface. If the new assertion is a
follow-on interaction on that surface, extend that test. Keep a separate
test when the fixture, setup, or failure mode is different.

Before completing a task run:

cargo fmt --check
cargo clippy --all-targets --all-features
cargo test

Do not mark work complete if these fail.

---

# Scope Control

Implement only the requested phase or feature.

Do not spontaneously add:

- cloud sync
- accounts
- telemetry
- analytics
- plugin systems
- GraphQL
- WebSocket
- gRPC
- GitHub/GitLab APIs
- MCP

before their planned phase or explicit request.

Do not perform broad refactors during unrelated tasks.

---

# Working Style

Before substantial implementation:

1. Read README.md.
2. Read IMPLEMENTATION_PLAN.md.
3. Read docs/ARCHITECTURE.md.
4. Inspect existing architecture.
5. Inspect relevant GPUI/gpui-base source when necessary.
6. Identify affected files.
7. Make the smallest coherent change.
8. Add/update tests.
9. Run formatting, Clippy, and tests.
10. Summarize architectural decisions and remaining limitations.

When uncertain about GPUI or gpui-base APIs, inspect source rather than
guessing.
