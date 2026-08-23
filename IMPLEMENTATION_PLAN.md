# Implementation Plan

## Phase 0 — Rust Workspace Foundation

Create the Rust workspace.

Suggested structure:

crates/
├── core/
├── opencollection/
├── http/
├── cli/
└── desktop/

Required:

- Rust workspace
- formatting
- Clippy
- test infrastructure
- fixture directory
- CI
- logging/error strategy

GPUI and Longbridge gpui-base may be added now or when desktop development begins,
but desktop implementation must not begin yet.

Exit criteria:

- cargo fmt --check passes
- cargo clippy passes
- cargo test passes
- CLI binary starts


## Phase 1 — OpenCollection Reader

Implement OpenCollection YAML parsing independently of every frontend.

Support:

- Collection
- Folder
- HTTP request
- Headers
- Query parameters
- Request body
- Authentication
- Environments

Use specification-based fixtures.

Preserve unknown fields whenever practical.

Exit criteria:

- OpenCollection fixtures load successfully
- models contain expected values
- round-trip serialization works
- unsupported fields are not silently destroyed


## Phase 2 — Workspace Domain Model

Implement the in-memory workspace.

Conceptually:

Workspace
├── requests keyed by runtime RequestKey
├── folders keyed by runtime FolderKey
├── environments
└── metadata

Requirements:

- O(1) request lookup where appropriate
- generational runtime keys are in-memory only and are rebuilt when a workspace is loaded
- deleting and reusing a slot must not make a stale key resolve to a new item
- persistent locators are owned by repository adapters, not domain keys
- domain has no dependency on CLI or GPUI
- domain has no dependency on YAML representation
- simple Rust ownership

Avoid unnecessary:

- Arc
- Mutex
- RwLock
- global mutable state

Exit criteria:

- 1,000+ request fixture loads
- workspace operations have unit tests
- domain can be used without UI or CLI-specific code


## Phase 3 — Agent-Friendly CLI Foundation

Build the first real frontend over the core.

Initial commands:

<app> collection validate <path>
<app> request list <path>
<app> request get <path> <request-selector>
<app> request run <path> <request-selector>

Support:

--json
--help

Human output should be readable.

JSON output should be stable and machine-readable.

Requirements:

- deterministic output
- request selectors use repository locators, never session-only runtime keys
- meaningful exit codes
- no unnecessary prompts
- errors available as structured JSON when --json is used

Exit criteria:

- agent can discover requests
- agent can inspect a request
- agent can validate a workspace


## Phase 4 — Environment Resolution

Implement:

- environments
- variable lookup
- variable interpolation
- environment selection
- missing-variable errors

Expose through the CLI.

Example:

<app> request run ./api req_users \
  --environment development

Tests must cover resolution behavior independently of CLI parsing.

Environment creation, deletion, and rename are out of scope for this phase. They belong to
Phase 18.

Exit criteria:

- environments resolve with inheritance and interpolation
- missing-variable and invalid-environment errors are stable and testable without CLI parsing


## Phase 5 — HTTP Execution

Implement the HTTP engine.

Support:

- GET
- POST
- PUT
- PATCH
- DELETE
- headers
- query parameters
- JSON body
- text body
- form body
- multipart
- Basic auth
- Bearer auth
- redirects
- timeout
- cancellation

Response metadata:

- status
- duration
- size
- headers

Expose through:

<app> request run

Human example:

GET https://api.example.com/users

200 OK
128 ms
4.2 KB

{ ... }

Machine output:

<app> request run ./api req_users --json

Large bodies must not create unreasonable stdout behavior.

Support explicit body output to a file when appropriate.

Exit criteria:

- real OpenCollection HTTP requests execute correctly
- HTTP engine is completely independent of CLI
- CLI only adapts arguments/output to the shared engine


## Phase 6 — CLI Automation Hardening

Make the CLI suitable for AI agents and CI.

Add:

- stable JSON schemas
- documented exit codes
- stdin support where useful
- explicit output-file support
- quiet mode where useful
- deterministic error representation

Avoid output that depends on terminal animation or cursor control when
structured mode is enabled.

Add integration tests exercising the binary.

Exit criteria:

An external program can reliably:

1. discover requests
2. inspect requests
3. execute requests
4. interpret success/failure

without parsing human-readable text.


## Phase 7 — OpenCollection Persistence

Implement safe writes.

Required:

- serialization
- atomic writes
- unknown-field preservation
- safe error handling

Add CLI operations only where useful.

Do not turn the CLI into a full interactive request editor.

Include environment-variable set and unset persistence for bundled `config.environments`
and unbundled `environments/*.yml` documents. Environment creation belongs to Phase 18.

Exit criteria:

load
→ modify
→ save
→ reload

preserves expected OpenCollection semantics.


## Phase 8 — Performance Baseline

Before building the UI, establish core benchmarks.

Workspace sizes:

- 100 requests
- 1,000 requests
- 10,000 requests

Measure:

- parsing
- workspace construction
- request lookup
- CLI startup
- memory consumption where practical

Create representative benchmark fixtures.

Do not optimize without measurements.


## Phase 9 — GPUI + gpui-base Foundation

Begin desktop implementation only after the core and CLI architecture
is established.

Add/pin:

- GPUI (Zed revision required by the pinned gpui-base commit)
- Longbridge gpui-base (`gpui-base` from `longbridge/gpui-component`)

Do not add `gpui-component` (styled façade). Inspect the pinned `gpui-base`
source; desktop chrome follows gpui-base's default gallery tokens.

Before using unfamiliar APIs:

- inspect exact pinned source
- inspect examples/tests
- do not guess APIs

Build:

- application window
- semantic theme foundation from gpui-base default light and dark tokens
- basic Longbridge gpui-base primitives styled with those tokens

Follow `docs/DESIGN.md`. Phase 9 establishes design tokens and built-in themes only;
loading user-authored theme files remains a future capability.

Exit criteria:

- desktop application launches
- Longbridge gpui-base works
- shared core is accessible from desktop crate
- macOS window structure and interaction conventions align with the current Apple HIG
- Windows behavior aligns with the current Microsoft Windows App Design and Fluent
  conventions
- Linux behavior follows a documented cross-desktop baseline reviewed against both
  GNOME and KDE guidance rather than imitating either toolkit
- built-in light and dark appearances use semantic tokens without component-level
  hard-coded colors
- keyboard focus, keyboard navigation, system appearance changes, and display scaling
  have been verified on each supported platform


### Deferred follow-up — User-Defined Themes

After the desktop design system and component vocabulary are stable, support themes
stored in plain-text, human-editable configuration files. This is not a Phase 9 exit
criterion and should be scheduled separately.

Required:

- a documented, versioned theme schema
- deterministic parsing and validation outside individual components
- semantic tokens rather than component-specific styling keys
- safe fallback for missing, invalid, or unsupported values
- built-in themes remain available when custom themes fail
- live theme application without changing component behavior
- contrast and accessibility validation
- useful diagnostics that identify the source file and invalid field

Do not store themes in OpenCollection YAML or a proprietary database. Do not choose
the configuration syntax until the schema is designed; YAML, TOML, and JSON are not
implicitly approved by this requirement.


## Phase 10 — Desktop Shell

Implement:

- workspace sidebar
- title-bar workspace switcher with recent, new, open, and close actions
- folder/request tree
- tabs
- request editor area
- response panel
- resizable panes
- vertical and horizontal editor/response layouts
- local desktop-session persistence
- automatic restoration of the last active collection
- recent collections and explicit close-without-delete behavior

No duplicate OpenCollection parsing logic is allowed.

The desktop application consumes the same core used by the CLI.

Exit criteria:

- open collection
- switch or open a workspace from the title bar
- browse requests
- switch requests instantly
- open/close tabs
- relaunch without choosing the active collection again
- restore tabs using repository selectors rather than runtime keys
- restore collapsed folders and pane sizes
- restore the selected editor/response layout and its size in each orientation
- restore the last selected environment for each collection
- recover safely when a remembered collection is missing or invalid

Desktop session state is local presentation metadata. Store it in the platform
application-data directory, not in OpenCollection YAML or a proprietary collection
database. Writes must be atomic and must not block the GPUI thread. Persist collection
paths plus repository-owned request/folder selectors; never serialize `RequestKey` or
`FolderKey`. Opening a remembered collection rebuilds the workspace and resolves fresh
runtime keys on every launch. Closing a collection forgets it as active without
deleting its files.


## Phase 11 — Desktop Request Editor

Implement:

- method
- URL
- query parameters
- headers
- body
- authentication
- environment selection

Environment creation belongs to Phase 18. Desktop UI for creating environments may follow
after the shared repository operations exist.

Changes update the in-memory model immediately.

Persistence happens separately.


## Phase 12 — Desktop HTTP Execution

Connect the existing shared HTTP engine to GPUI.

Do not implement a second HTTP stack.

Required:

- Send
- Cancel
- status
- duration
- size
- headers
- response body


## Phase 13 — Response Viewer

Implement:

- Pretty
- Raw
- Headers
- Search

Benchmark Longbridge gpui-base/GPUI text components before assuming suitability
for large responses.

For large responses consider:

- virtualization
- chunked rendering
- lazy loading
- background parsing
- specialized read-only rendering

Exit criteria:

- normal responses render immediately
- large responses do not freeze navigation
- request switching remains responsive


## Phase 14 — Desktop Request Persistence

Connect the desktop request editor to the existing shared OpenCollection
persistence boundary.

Implement:

- dirty-state tracking against the last loaded or successfully saved request
- explicit Save and Save All actions
- platform-appropriate Save keyboard shortcuts
- persistence for every request field editable by the desktop
- atomic background saves that never block the GPUI thread
- visible save failures that retain the dirty in-memory request
- close-tab, close-workspace, and quit protection for unsaved changes

Desktop code must not serialize YAML or write collection files directly. It invokes
the same application/repository operations available to other interfaces. Saving must
preserve unsupported OpenCollection fields and retain the existing exact-source conflict
check so an external modification is never overwritten.

This phase saves existing requests only. General request/file creation, folder creation,
deletion, moves, and reordering belong to Phase 16.

Exit criteria:

- every desktop-editable request field survives save and reload
- dirty state clears only after the corresponding revision is saved successfully
- edits made while a background save is running remain dirty
- save errors and external-modification conflicts do not discard local edits
- closing dirty work requires an explicit Save, Discard, or Cancel decision


## Phase 15 — Filesystem Synchronization

Implement:

- file watching
- external modifications
- file creation/deletion
- rename handling
- conflict detection

Never silently overwrite externally modified data.

Filesystem notifications are invalidation hints, not authoritative workspace changes.
Debounce event bursts, reload and validate through the OpenCollection repository on a
background executor, and reconcile the last loaded version, dirty in-memory version,
and new filesystem version. Clean external changes may update the desktop automatically;
overlapping local and external edits require explicit conflict resolution.

Probe's own successful saves must update the repository baseline so their resulting
watcher events reconcile as no-ops. Invalid or partially written external files must not
replace the last valid in-memory workspace.

This phase detects requests and folders created by external tools. It does not add a
general request-creation command or desktop creation workflow.

Exit criteria:

- valid external modifications appear without reopening the collection
- external creation, deletion, and confidently identified renames update the workspace
- open tabs and presentation state are remapped through repository selectors rather than
  stale runtime keys
- dirty local edits survive non-conflicting external changes
- overlapping changes, ambiguous renames, and external deletion of dirty items cannot
  cause silent data loss
- filesystem parsing and reconciliation do not block GPUI


## Phase 16 — Workspace Structure Operations

Implement shared domain, application, and repository operations for:

- request creation
- folder creation
- rename
- deletion
- moving requests and folders between folders
- ordering requests and folders within a parent

Frontend adapters must invoke these operations rather than edit YAML or move collection
files directly.

Bundled collections persist hierarchy and ordering in their item structure. Unbundled
collections use the OpenCollection filesystem representation and ordering metadata.
Moves and renames must update repository selectors while preserving open tabs, dirty
edits, and session restoration where identity can be established safely.

Required:

- duplicate-path and invalid-destination protection
- atomic writes where one document is affected
- recoverable behavior for multi-file moves
- exact-source conflict checks before structural writes
- fixture-based load, modify, save, and reload tests for bundled and unbundled collections

Exit criteria:

- requests and folders can be created, renamed, moved, reordered, and deleted
- persisted order and hierarchy survive reload
- runtime keys are never serialized or treated as persistent identity
- external modifications during a structural operation cannot be silently overwritten


## Phase 17 — CLI Workspace Structure Editing

Expose the Phase 16 operations through automation-safe CLI commands where each operation
is meaningful.

Required:

- request and folder creation, rename, deletion, move, and reorder commands
- non-interactive arguments for parent and insertion position
- deterministic JSON output
- stable error categories and exit codes
- repository selectors in every command and result
- no YAML serialization or filesystem move logic in the CLI

Exit criteria:

- an external program can perform every supported structural operation without parsing
  human-readable output
- CLI operations produce the same persisted domain and repository results as direct
  Phase 16 operations
- duplicate paths, invalid destinations, missing selectors, read-only sources, and
  external-modification conflicts are distinguishable programmatically


## Phase 18 — Environment Management Operations

Implement shared domain and repository operations for creating environments.

Environment edits are not structural item-tree operations. They follow the same
environment-mutation path as variable set and unset rather than `StructureOperation`.

Implement in this order:

1. **Core** — in-memory environment creation and validation
2. **OpenCollection repository** — YAML mutation and persistence
3. **Frontend adapters** — CLI first, then desktop

Required core behavior:

- create an environment with a unique name
- optional `extends` parent
- duplicate-name, missing-parent, and inheritance-cycle protection
- reuse `validate_environments` after mutation

Required repository behavior:

- bundled collections append to `config.environments`
- unbundled collections write a new `environments/<name>.yml` document
- update `environment_persistence` after creation
- exact-source conflict checks and atomic writes
- fixture-based load, create, save, and reload tests for bundled and unbundled collections

Frontend adapters must invoke these operations rather than edit YAML directly.

Exit criteria:

- environments can be created in memory with stable validation errors
- created environments survive save and reload in bundled and unbundled workspaces
- runtime environment state is never serialized as a persistent identity outside OpenCollection


## Phase 19 — CLI Environment Management

Expose Phase 18 operations through automation-safe CLI commands where each operation
is meaningful.

Initial commands:

<app> environment create <path> --name <name> [--extends <parent>]
<app> environment set <path> --environment <name> --name <var> --value <value>
<app> environment unset <path> --environment <name> --name <var>

Required:

- non-interactive arguments for environment name and optional parent
- deterministic JSON output
- stable error categories and exit codes
- no YAML serialization or filesystem logic in the CLI

Consider `environment list` when it helps agents discover available environments without
parsing the collection file.

Exit criteria:

- an external program can create environments and manage variables without parsing
  human-readable output
- CLI operations produce the same persisted domain and repository results as direct
  Phase 18 operations
- duplicate names, missing parents, inheritance cycles, read-only sources, and
  external-modification conflicts are distinguishable programmatically


## Phase 20 — Desktop Workspace Structure Editing

Expose the Phase 16 operations through keyboard-accessible desktop interactions.

Implement:

- request and folder creation
- inline or dialog-based rename
- deletion with data-loss protection
- moving and reordering through explicit controls
- focusable tree rows and keyboard navigation
- selector-based remapping after structural changes

Structural filesystem work must run away from the GPUI thread. Open tabs, dirty edits,
collapsed folders, and restored session state must follow safely established selector
changes rather than runtime keys.

Exit criteria:

- every structural operation is available without drag and drop
- dirty requests are never silently discarded by rename, move, or deletion
- tabs and presentation state survive selector changes where identity is safely
  established
- save and filesystem-watch events caused by Probe reconcile as no-ops


## Phase 21 — Desktop Tree Drag and Drop

Add drag-and-drop movement and reordering to the desktop request tree using the same
Phase 16 operations as the CLI and keyboard workflows.

Required:

- clear insertion indicators
- valid folder and sibling drop targets
- rejection of self, descendant, duplicate-path, and otherwise invalid destinations
- autoscroll where needed for large virtualized trees
- equivalent keyboard-accessible operations from Phase 20

Exit criteria:

- requests and folders can be moved and reordered by drag and drop
- drag-and-drop and CLI operations produce identical domain and repository results
- large virtualized trees remain responsive while dragging
- failed persistence or conflict checks leave the previous valid workspace visible and
  recoverable


## Phase 22 — Streaming Protocol Architecture

Design a shared event/session abstraction suitable for:

- WebSocket
- SSE
- gRPC streaming

Do not design this specifically around terminal interaction.

Conceptually:

Protocol Engine
      ↓
Session/Event Stream
      ↓
 ┌────┴────┐
 CLI      GPUI


## Phase 23 — WebSocket

Implement shared WebSocket support.

CLI should support agent-friendly non-interactive operation, including
structured streaming output such as JSONL.

Interactive terminal mode may be added but is not the architectural
foundation.


## Phase 24 — gRPC

Implement:

- unary
- server streaming
- client streaming
- bidirectional streaming

The shared protocol implementation must work for both CLI and GPUI.


## Phase 25 — Git Integration

Filesystem remains the primary Git integration boundary.

Optional built-in functionality:

- status
- changed files
- current branch
- diff
- commit
- pull
- push

Keep basic Git behavior provider-independent.


## Phase 26 — MCP

Consider exposing the shared application layer through an MCP server.

Do not reimplement business logic.

Conceptually:

                 Application/Core
                ┌──────┼──────┐
                │      │      │
               CLI    GPUI    MCP


## Phase 27 — Yaak Import

Add a shared inbound adapter for official Yaak export JSON and Directory/Git Sync
models. Convert the selected Yaak workspace into the domain model, then persist it as
a new bundled OpenCollection document through the shared repository.

Requirements:

- CLI and GPUI use the same import conversion and persistence path
- schemas 1–4 are migrated before conversion
- multiple workspaces require explicit selection
- strict mode rejects lossy conversion before creating a destination
- partial mode requires explicit user intent and reports deterministic diagnostics
- supported authentication remains stored even when current execution is unavailable
- destination writes are atomic and never overwrite an existing path
- fixture tests cover export, sync, strict/partial behavior, persistence, CLI JSON, and exit codes


## Phase 28 — Postman Collection Import

Add an inbound adapter for official Postman Collection v2.0 and v2.1 JSON. Normalize
their authentication encodings, convert supported collection data into the shared
domain model, and persist through the existing bundled OpenCollection writer.

Requirements:

- CLI and GPUI share conversion, strict/partial diagnostics, and persistence
- preserve nested order, structured URL/query/path data, supported bodies, inherited
  authentication, templates, and collection variables
- create `Postman Collection Variables` when collection-scoped variables exist
- diagnose scripts, examples, scoped variables, unsupported settings, dynamic
  variables, unknown data, and execution-unsupported authentication
- strict mode creates no destination; partial mode requires explicit user intent
- CLI exposes `collection import postman` with deterministic JSON and exit codes
- workspace switcher, empty sidebar, and macOS `File > Import` share one
  provider-aware desktop workflow
- fixture and integration tests cover v2/v2.1 conversion, persistence, CLI contracts,
  desktop provider menus, and native system-menu ordering
