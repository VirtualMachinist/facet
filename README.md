# Facet

Hedronite's in-house fork of [Probe](https://github.com/crizant/probe): the same
core, CLI contract, and OpenCollection YAML, cut for the terminal, with
**Lattice** underneath it so every run is remembered. Codename G38.

The `facet` binary coexists with `probe` on `PATH`. Collection files stay YAML;
Git stays the sync layer. Lattice never holds the collection.

```bash
facet --version          # facet 0.5.7 (probe 0.5.7)
probe --version          # probe 0.5.7
```

## What Facet adds

- **TUI** (`facet tui`) — Ratatui, Graphite Honey / Porcelain Honey. Not a
  second GPUI desktop. Upstream `probe-desktop` remains in the workspace but is
  out of the default build.
- **Lattice** — SQLite beside the YAML (`.facet/lattice.db`) plus a machine
  store for secrets, sessions, and a cross-workspace run index.
- **Agent CLI** — metadata is cheap; payloads are explicit:

```bash
facet request run ./api users/list-users.yml --environment development --json
facet history --json
facet blob <sha256>
facet history --sql "SELECT status, count(*) FROM runs GROUP BY status"
facet gc --yes
```

Everything Probe already documents still holds: [CLI](docs/CLI.md),
[Architecture](docs/ARCHITECTURE.md). Facet-only contracts live in
[docs/FACET.md](docs/FACET.md).

## Goals

- TUI-first, agent-optimized
- Same OpenCollection YAML as Probe; cherry-pickable in both directions
- Filesystem-first and Git-friendly
- Run history that survives the terminal
- Deterministic, versioned JSON
- No account required

## Technology

- Rust 1.95 (`rust-toolchain.toml`)
- Ratatui (`crates/facet-tui`)
- Bundled SQLite via `rusqlite` (default Lattice engine)
- Optional `lattice-turso` and `lattice-duckdb` features (off by default)
- OpenCollection YAML

`probe-core` and `probe-cli` are not edited in a Facet slice. Changes that
belong there are written as Probe PRs.

## Development

Install [rustup](https://rustup.rs/). The checked-in toolchain selects 1.95.0
with `rustfmt` and `clippy`.

```bash
cargo run -p facet-cli -- --help
cargo run -p facet-cli -- tui
cargo run -p probe-cli -- --help
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

`cargo build` / `cargo test` skip GPUI. Use `cargo build --workspace` if you
need the upstream desktop.

```bash
cargo build --release -p probe-cli -p facet-cli
# then put target/release/facet and target/release/probe on PATH
```

Read [AGENTS.md](AGENTS.md) before changing code. Facet-only work: [docs/FACET.md](docs/FACET.md).
Index: [docs/README.md](docs/README.md).

## License

- **Facet-original crates** (`facet`, `lattice`, `facet-tui`): [MIT](crates/facet/LICENSE-MIT),
  Copyright 2026 Hedronite.
- **Upstream-derived crates and files**: [Apache License 2.0](LICENSE) (Probe).
- See [NOTICE](NOTICE) for the fork relationship.

Upstream: [crizant/probe](https://github.com/crizant/probe).
This tree: [VirtualMachinist/facet](https://github.com/VirtualMachinist/facet).
