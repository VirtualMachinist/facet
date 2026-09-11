<p align="center">
  <a href="https://github.com/VirtualMachinist/facet">
    <img src="assets/facet-logo.jpeg" alt="Facet" width="220">
  </a>
</p>

<h1 align="center">Facet</h1>

<p align="center">
  <strong>Native, local-first API client.</strong><br>
  The terminal-native fork of Probe, with persistent run history.
</p>

<p align="center">
  <a href="https://github.com/VirtualMachinist/facet/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/VirtualMachinist/facet/ci.yml?branch=main&style=flat&colorA=1A1A1A&colorB=C9A227&label=ci" alt="CI"></a>
  <a href="https://github.com/VirtualMachinist/facet/releases/tag/v0.5.9"><img src="https://img.shields.io/badge/Facet-v0.5.9-C9A227?style=flat&colorA=1A1A1A" alt="Facet v0.5.9"></a>
  <a href="https://crates.io/crates/facet-lattice"><img src="https://img.shields.io/crates/v/facet-lattice?style=flat&colorA=1A1A1A&colorB=C9A227" alt="facet-lattice on crates.io"></a>
  <a href="https://rustup.rs"><img src="https://img.shields.io/badge/Rust-1.95-F46623?style=flat&colorA=1A1A1A&logo=rust&logoColor=white" alt="Rust 1.95"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-C9A227?style=flat&colorA=1A1A1A" alt="License"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="docs/install.md">Install</a> ·
  <a href="#what-you-get">What you get</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="#status">Status</a> ·
  <a href="IMPLEMENTATION_PLAN.md">Roadmap</a> ·
  <a href="#contributing">Contributing</a>
</p>

<p align="center">
  <a href="https://github.com/VirtualMachinist">VirtualMachinist</a> ·
  Fork of <a href="https://github.com/crizant/probe">Probe</a>
</p>

---

Facet is a fast, native, local-first API client. Collections stay OpenCollection YAML on disk; Git stays the sync layer. **facet-lattice** remembers what surrounds them — runs, bodies, timings, sessions — in local SQLite. A Ratatui TUI and an agent-friendly CLI sit on the same core.

It is for people who want a durable terminal workflow: deterministic JSON, content-addressed blobs, and SQL over history, without an account or hosted control plane. The `facet` binary coexists with `probe` on `PATH`.

**0.5.9** · OpenCollection YAML · Ratatui TUI · SQLite run history

## Quick start

### 1. Add the library (crates.io)

```bash
cargo add facet-lattice@0.5.9
```

```toml
[dependencies]
facet-lattice = "0.5.9"
```

```rust
use lattice::WorkspaceStore;
```

The Cargo package is **`facet-lattice`**; the Rust import name is `lattice`. Details: [docs/install.md](docs/install.md) and [crates/lattice/README.md](crates/lattice/README.md).

### 2. Install the CLI

**From a release** (recommended): download `facet` from [GitHub Releases](https://github.com/VirtualMachinist/facet/releases) for your platform.

**From source** (contributors):

```bash
cargo build --release -p probe-cli -p facet-cli
export PATH="$PWD/target/release:$PATH"
```

### 3. Run a request and query history

Facet uses the same OpenCollection YAML as Probe.

```bash
facet request run ./api users/list-users.yml --environment development --json
facet history --json
facet history --sql "SELECT status, count(*) FROM runs GROUP BY status"
facet blob <sha256>
```

For the TUI: `facet tui`.

Full install paths (library, binary, versions): **[docs/install.md](docs/install.md)**.

## What you get

Everything below builds on Probe's core engine, running natively in your terminal.

| | Probe | Facet |
|---|---|---|
| Core Engine | Rust, fast, filesystem-first | Same core engine, cherry-pickable |
| Collections | OpenCollection YAML | Same YAML, 100% compatible |
| Interface | GPUI Desktop + CLI | Ratatui TUI (`facet tui`) + Agent CLI |
| History | In-memory session | **facet-lattice**: SQLite workspace & machine store |
| Binaries | `probe` | `facet` (coexists with `probe` on `PATH`) |
| Data | Ephemeral CLI runs | Persistent, queryable, content-addressed blobs |
| Output | Human-readable & JSON | Deterministic, versioned JSON optimized for agents |

### What stays upstream, on purpose

Facet rebuilds the terminal and history layers. The core domain and HTTP execution belong to Probe, and every place the two meet is written down rather than papered over. The full boundary is documented in [docs/FACET.md](docs/FACET.md).

| Upstream | On Facet |
|---|---|
| `probe-core`, `probe-cli` | Untouched. Fixes go upstream first. |
| GPUI Desktop | Out of default build. `cargo build --workspace` to include. |
| OpenCollection YAML | Canonical format. No proprietary database extensions. |
| CLI Contracts | `facet` delegates standard commands to `probe-cli` verbatim. |

## How it works

One rule drives the design: **YAML holds the collection; facet-lattice holds the history.** Terminal UI and run memory are Facet; shared core stays cherry-pickable.

- The `facet` binary coexists with `probe` on `PATH`. Collection files stay YAML; Git stays the sync layer. Run history never replaces the collection itself.
- **facet-lattice** uses SQLite beside the YAML (`.facet/lattice.db`) plus a machine store for secrets, sessions, and a cross-workspace run index.
- Agent CLI commands (`history`, `blob`, `gc`) are built for machine consumption. Metadata is cheap, and large bodies are content-addressed.
- Secrets are stored securely at rest using the OS keyring or XChaCha20-Poly1305 encryption.

<details>
<summary><strong>Repository map</strong></summary>

```
crates/cli/               # upstream probe-cli
crates/core/              # upstream probe-core
crates/desktop/           # upstream probe-desktop (excluded from default build)
crates/http/              # upstream HTTP client
crates/opencollection/    # upstream YAML parser
crates/facet/             # facet-cli binary
crates/lattice/           # facet-lattice crate (crates.io)
crates/facet-tui/         # Ratatui interface
docs/FACET.md             # Facet-specific documentation
docs/install.md           # install guide (library + binary)
AGENTS.md                 # project instructions
```

</details>

## Status

Development on `main` tracks **0.5.9**. Release binaries ship from matching `v0.5.9` tags; the **`facet-lattice`** crate on crates.io is being advanced to **0.5.9** after the current alignment PR merges.

- **facet-lattice**: Ready. SQLite/rusqlite default. Published on [crates.io](https://crates.io/crates/facet-lattice).
- **TUI**: Ready. Graphite Honey / Porcelain Honey themes via Ratatui.
- **Agent CLI**: Ready. Deterministic JSON and SQL querying.

HTTP is live. Core Facet loops (sessions, recall, replay, diff, secret overlay,
`--expect` / `--dry-run`) are shipped. Still folded from
[Probe's roadmap](https://rusty-probe.pages.dev): WebSocket, GraphQL, gRPC
streaming, user-defined theme files, git HEAD auto-tag, and a TUI env editor.
Details: [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

## Contributing

Issues and pull requests are welcome. Start with [AGENTS.md](AGENTS.md) and [docs/FACET.md](docs/FACET.md). Roadmap: [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). Development practices: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Credits and license

Upstream lineage: [Probe](https://github.com/crizant/probe).

- **Facet-original crates** (`facet-cli`, `facet-lattice`, `facet-tui`): MIT License.
- **Upstream-derived crates and files**: Apache License 2.0 (Probe).

See [LICENSE](LICENSE) and [NOTICE](NOTICE).
