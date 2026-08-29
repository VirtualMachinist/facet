# Documentation

Read only the document relevant to the task. `AGENTS.md` contains the invariants that
apply to every code change and routes task-specific work here.

| Document | Canonical subject | Read when |
| --- | --- | --- |
| [Architecture](ARCHITECTURE.md) | Crate boundaries, data flow, workspace identity, persistence, synchronization, HTTP, imports, and desktop runtime behavior | Changing shared behavior or boundaries |
| [CLI](CLI.md) | Commands, selectors, JSON schemas, error categories, and exit codes | Changing or consuming the CLI contract |
| [Desktop design](DESIGN.md) | Platform behavior, components, semantic tokens, themes, and accessibility | Changing desktop presentation or interaction |
| [Development](DEVELOPMENT.md) | Rust ownership, async work, dependencies, pinned GPUI guidance, tests, and completion checks | Implementing code or changing dependencies |
| [Errors and logging](ERRORS_AND_LOGGING.md) | Typed-error ownership and interface logging boundaries | Adding or mapping failures and diagnostics |
| [Performance](PERFORMANCE.md) | Benchmark commands, fixtures, measurement policy, and reference results | Measuring or optimizing performance |
| [Roadmap](../IMPLEMENTATION_PLAN.md) | Implemented foundation and explicitly deferred product work | Planning scope or starting a deferred feature |

[README.md](../README.md) is the user-facing product, installation, and development
quick start. It should link to canonical documents rather than repeat their contracts.

## Documentation Rules

- State each durable rule in one canonical document and link to it elsewhere.
- Keep `AGENTS.md` limited to always-applicable constraints and task routing.
- Document current behavior in architecture, CLI, or design references; keep future
  scope in the roadmap.
- Prefer code and fixture names over copying large implementation inventories.
- Remove completed phase checklists after their behavior is captured in canonical
  documentation and tests.
