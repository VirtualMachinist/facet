# facet-lattice

Local run-history store for [Facet](https://github.com/VirtualMachinist/facet): bundled SQLite beside OpenCollection YAML, with an optional DuckDB analytics feature.

The Rust library name remains `lattice` (`use lattice::…`) so Facet adapters keep stable imports.

## Install

```bash
cargo add facet-lattice@0.5.9
```

SQLite-only (default):

```toml
[dependencies]
facet-lattice = "0.5.9"
```

Optional in-process analytics against SQLite files:

```toml
[dependencies]
facet-lattice = { version = "0.5.9", features = ["lattice-duckdb"] }
```

## Features

| Feature | Default | Description |
| --- | --- | --- |
| *(none)* | yes | Bundled SQLite workspace + machine stores |
| `lattice-duckdb` | no | Optional analytics smoke against SQLite files (heavy native build) |

## Docs

- [Facet guide](https://github.com/VirtualMachinist/facet/blob/main/docs/FACET.md)
- [Lattice engines](https://github.com/VirtualMachinist/facet/blob/main/docs/LATTICE-ENGINES.md)

## License

MIT — see [LICENSE-MIT](LICENSE-MIT).
