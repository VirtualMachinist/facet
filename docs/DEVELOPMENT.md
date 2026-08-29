# Development Practices

Read this document for implementation workflow, dependency changes, GPUI work, or
test design. Project-wide architectural invariants remain in `AGENTS.md`.

## Rust and Async Work

- Prefer normal ownership, GPUI entity ownership, message passing, task results, and
  immutable shared data before `Arc`, `Mutex`, `RwLock`, `RefCell`, or global mutable
  state.
- Keep shared networking and application APIs asynchronous where the desktop needs
  them. Do not create duplicate synchronous business logic for the CLI.
- Run filesystem I/O, HTTP, large YAML or JSON parsing, Git work, and expensive
  highlighting away from the GPUI thread.
- Never use `unsafe` solely to bypass ownership problems. Any unsafe code requires
  explicit justification.

## Dependencies

Before adding a crate, check the standard library and existing dependencies. Prefer
actively maintained, cross-platform crates and avoid large dependencies for trivial
work. Do not replace dependencies without a task-specific reason.

Probe uses Longbridge `gpui-base` (`gpui-base` / `gpui_base`, from
`longbridge/gpui-component`'s `crates/base`). Never add the separate, pre-styled
`gpui-component` crate or copy its APIs. Initialize the theme once through
`probe_desktop::theme::Theme::init(cx)`.

Keep GPUI and `gpui-base` on compatible pinned sources. If their types conflict,
inspect the pinned `gpui-base` lockfile and correct the GPUI pin rather than mixing
revisions. Do not upgrade either dependency during unrelated work. Inspect the exact
pinned source and examples before using unfamiliar APIs.

Do not introduce Electron, Tauri, WebView, React, Flutter, or another GUI framework
without explicit approval.

## Tests

Keep shared fixtures under `tests/fixtures/`. CLI integration tests cover command
behavior, JSON output, and exit codes.

Do not automate visual constants such as spacing, radii, typography sizes, palette
values, or contrast ratios. Review those visually against [DESIGN.md](DESIGN.md).
Desktop tests may cover behavior such as appearance selection, pane constraints,
focus, and highlight ranges. Extend an existing desktop test when it already builds
the same surface and the new assertion is a follow-on interaction.

Before completing a code change, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

Do not report completion while any required check fails.

## Working Style

- Inspect relevant architecture and pinned dependency source instead of guessing.
- Preserve unrelated user changes in a dirty worktree.
- Make the smallest coherent change and add or update tests.
- Summarize architectural decisions and remaining limitations.
