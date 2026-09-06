<p align="center">
  <a href="https://github.com/VirtualMachinist/facet">
    <img src="assets/facet-logo.jpeg" alt="Facet" width="220">
  </a>
</p>

<h1 align="center">Facet</h1>

<p align="center">
  <strong>Native, local-first API client.</strong><br>
  The terminal-native fork of Probe, with Lattice run history.
</p>

<p align="center">
  <a href="https://github.com/VirtualMachinist/facet/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/VirtualMachinist/facet/ci.yml?branch=main&style=flat&colorA=1A1A1A&colorB=C9A227&label=ci" alt="CI"></a>
  <a href="https://github.com/VirtualMachinist/facet/releases/tag/v0.5.7"><img src="https://img.shields.io/badge/Facet-v0.5.7-C9A227?style=flat&colorA=1A1A1A" alt="Facet v0.5.7"></a>
  <a href="https://rustup.rs"><img src="https://img.shields.io/badge/Rust-1.95-F46623?style=flat&colorA=1A1A1A&logo=rust&logoColor=white" alt="Rust 1.95"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-C9A227?style=flat&colorA=1A1A1A" alt="License"></a>
  <a href="https://hedronite.com"><img src="https://img.shields.io/badge/Hedronite-hedronite.com-C9A227?style=flat&colorA=1A1A1A" alt="Hedronite"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#what-you-get">What you get</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="#status">Status</a> ·
  <a href="#contributing">Contributing</a>
</p>

<p align="center">
  Built by <a href="https://hedronite.com">Hedronite</a>'s <a href="https://x.com/Hedronite">VirtualMachinist</a>.
  Core engine by <a href="https://github.com/crizant/probe">Probe</a>. Not affiliated with Probe or crizant.<br>
  <em>The Facet mark is the graphite honey hex lattice.</em>
</p>

---

Facet is a fast, native API client that keeps your collections in plain OpenCollection YAML files. It is Hedronite's terminal-optimized fork of [Probe](https://github.com/crizant/probe). The core engine and CLI contracts are identical, but Facet adds a Ratatui interface and **Lattice**, a local SQLite store that remembers every run.

It exists for **developers and agents who want a persistent terminal workflow**: deterministic JSON outputs, content-addressed blobs, and a run history that survives the session. If you love Probe's filesystem-first approach but need to query your request history with SQL or pipe responses into agent workflows, this is the way in.

Facet is not a competing standard and not a rewrite. It is Probe's own core, vendored into a workspace and driven by terminal-first adapters. We are members of the Probe community, and we send core fixes upstream.

**0.5.7** release · **100%** OpenCollection YAML compatible · **1** Ratatui TUI · **SQLite** default engine

## Quick start

Three steps to run your first request and query its history.

**1. Build and install.**

```bash
cargo build --release -p probe-cli -p facet-cli
export PATH="$PWD/target/release:$PATH"
```

**2. Run a request.**

Facet uses the exact same OpenCollection YAML as Probe.

```bash
facet request run ./api users/list-users.yml --environment development --json
```

**3. Explore your history.**

Lattice remembers the run automatically.

```bash
# View recent runs
facet history --json

# Query history with SQL
facet history --sql "SELECT status, count(*) FROM runs GROUP BY status"

# Extract a specific response body by hash
facet blob <sha256>
```

For the TUI, run `facet tui`.

## What you get

Everything below builds on Probe's core engine, running natively in your terminal.

| | Probe | Facet |
|---|---|---|
| Core Engine | Rust, fast, filesystem-first | Same core engine, cherry-pickable |
| Collections | OpenCollection YAML | Same YAML, 100% compatible |
| Interface | GPUI Desktop + CLI | Ratatui TUI (`facet tui`) + Agent CLI |
| History | In-memory session | **Lattice**: SQLite workspace & machine store |
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

One rule drives the whole design: **if it's core business logic, it goes upstream. If it's terminal UI or run history, it's Facet.** Nothing the user touches gets rewritten in Facet if Probe already models it.

- The `facet` binary coexists with `probe` on `PATH`. Collection files stay YAML; Git stays the sync layer. Lattice never holds the collection itself.
- **Lattice** uses SQLite beside the YAML (`.facet/lattice.db`) plus a machine store for secrets, sessions, and a cross-workspace run index.
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
crates/facet/             # the facet binary
crates/lattice/           # run history, blobs, gc, sql
crates/facet-tui/         # Ratatui interface
docs/FACET.md             # Facet-specific documentation
AGENTS.md                 # project instructions
```

</details>

## Status

**G38 / lattice ready / graphite honey**

Facet is the working tree at 0.5.7; track `main`. The Lattice schema is versioned and migrated automatically.

- **Lattice**: Ready. SQLite/rusqlite is the default engine.
- **TUI**: Ready. Graphite Honey / Porcelain Honey themes via Ratatui.
- **Agent CLI**: Ready. Deterministic JSON and SQL querying.

## Contributing

Issues and pull requests are welcome. Start with [AGENTS.md](AGENTS.md) and [docs/FACET.md](docs/FACET.md) for the rules the tree already follows. Two of them matter most: core logic belongs in Probe, and Facet-only crates are strictly separated.

When something we fix turns out to be a Probe bug rather than a terminal-ism, it goes upstream. Being a good citizen of the Probe community is part of the job.

## Credits and license

Facet is built and maintained by [Hedronite](https://hedronite.com). The core engine is [Probe](https://github.com/crizant/probe) by crizant.

- **Facet-original crates** (`facet`, `lattice`, `facet-tui`): MIT License, Copyright 2026 Hedronite.
- **Upstream-derived crates and files**: Apache License 2.0 (Probe).

See [LICENSE](LICENSE) and [NOTICE](NOTICE) for the fork relationship and full terms. The Facet mark is Hedronite's.