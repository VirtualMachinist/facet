# Install

Facet ships two install paths: the **`facet-lattice`** library on [crates.io](https://crates.io/crates/facet-lattice), and the **`facet`** CLI from GitHub release assets or a source build.

## facet-lattice (library)

Add the run-history store to a Rust project:

```bash
cargo add facet-lattice@0.5.9
```

SQLite-only (default):

```toml
[dependencies]
facet-lattice = "0.5.9"
```

```rust
use lattice::WorkspaceStore;
```

The Cargo package name is **`facet-lattice`**. The Rust library import remains `lattice` (`use lattice::…`).

Optional in-process analytics against SQLite files (heavy native build):

```toml
[dependencies]
facet-lattice = { version = "0.5.9", features = ["lattice-duckdb"] }
```

See also [crates/lattice/README.md](../crates/lattice/README.md) and [Lattice engines](LATTICE-ENGINES.md).

## facet (CLI binary)

### From a GitHub release (recommended)

1. Open [Facet releases](https://github.com/VirtualMachinist/facet/releases).
2. Download the archive for your platform (`facet-cli-<version>-<platform>.tar.gz`).
3. Extract the `facet` binary and put it on your `PATH`.

Verify:

```bash
facet --version
# facet 0.5.9 (probe 0.5.9)
```

Release assets are built from the matching `v*` tag. The workspace version on `main` should match the tag you install.

### From source (contributors)

Requires Rust **1.95** or newer:

```bash
git clone https://github.com/VirtualMachinist/facet.git
cd facet
cargo build --release -p probe-cli -p facet-cli
export PATH="$PWD/target/release:$PATH"
facet --version
```

`probe` builds alongside `facet` and can coexist on `PATH`.

## Versions

| Artifact | Source | Current target |
| --- | --- | --- |
| `facet-lattice` crate | [crates.io](https://crates.io/crates/facet-lattice) | **0.5.9** (next publish after merge) |
| `facet` / `probe` binaries | GitHub release tag | **v0.5.9** when tagged from `main` |
| Workspace on `main` | this repository | **0.5.9** |

Older crates.io releases remain available; new work targets **0.5.9**.

## Next steps

- Product overview: [README](../README.md)
- Facet commands and JSON contracts: [FACET.md](FACET.md)
- Engine configuration: [LATTICE-ENGINES.md](LATTICE-ENGINES.md)
