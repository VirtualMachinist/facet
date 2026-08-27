# Architecture

## Overview

The application uses a layered architecture with multiple interfaces
over a shared core.

                         Interfaces

                ┌────────────┼────────────┐
                │            │            │
               CLI          GPUI         MCP
                │         Desktop       Future
                │            │            │
                └────────────┼────────────┘
                             │
                    ┌────────▼────────┐
                    │   Application   │
                    │                │
                    │ Workspace      │
                    │ Request Exec   │
                    │ Environment    │
                    │ Persistence    │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │     Domain      │
                    │                │
                    │ Workspace      │
                    │ Request        │
                    │ Environment    │
                    │ Response       │
                    └────────▲────────┘
                             │
          ┌──────────────────┼──────────────────┐
          │                  │                  │
   OpenCollection          HTTP              History
    Repository             Engine              Store
          │                  │                  │
          ▼                  ▼                  ▼
        YAML              Network           Files/DB

Portable import formats are inbound adapters. The Postman adapter reads official
Collection v2.0/v2.1 JSON, while the Yaak adapter reads official export JSON or
directory-sync models. Both produce the same domain `Collection` used by every
interface. OpenCollection remains the only canonical persistence representation:

    Postman JSON        Yaak export / sync directory
          ↓                         ↓
    Postman adapter             Yaak adapter
          └────────────┬────────────┘
                       ↓
              Domain Collection
                       ↓
    OpenCollection repository
                       ↓
       Bundled YAML file

The adapters do not own CLI prompts, GPUI state, or filesystem persistence. Both
frontends invoke them and pass the converted domain value to the shared atomic writer.
Shared import diagnostics live in the core so strict and partial behavior remains
identical across providers and interfaces.


## Fundamental Rule

CLI and GPUI are interfaces.

They do not own business logic.

If behavior differs between CLI and GPUI, determine whether the
difference belongs to presentation or represents an architectural bug.

For example:

CLI:
request run
    ↓
Application
    ↓
HTTP Engine

GPUI:
Send button
    ↓
Application
    ↓
same HTTP Engine


## Dependency Rule

Dependencies point inward.

Interfaces may depend on Application and Domain.

Application may depend on Domain abstractions.

Infrastructure implements capabilities required by the application.

Domain must not depend on:

- GPUI
- gpui-base (Longbridge)
- CLI frameworks
- YAML
- reqwest
- filesystem implementation


## CLI

The CLI is a first-class interface optimized for:

- AI agents
- automation
- CI
- shell usage
- debugging
- headless environments

Its responsibilities are limited to:

- argument parsing
- invoking application operations
- human-readable presentation
- structured presentation
- exit-code mapping
- stdin/stdout integration

It must not implement domain behavior.

### Structured output

Programmatic commands should support JSON.

Streaming protocols should support JSONL where appropriate.

Structured stdout must contain only structured command output.

Logs and diagnostics belong on stderr.

The CLI's JSON documents carry an explicit schema version. Automation should branch
on stable error categories and exit codes, never parse human diagnostic messages.
Bundled collections may be supplied through stdin without moving YAML parsing into
the frontend: the OpenCollection repository projects the in-memory document and
builds the same structural selectors used for bundled files. Quiet mode is a
presentation concern and suppresses only successful command output.


## Desktop Presentation

GPUI owns:

- windows
- rendering
- entities
- events
- application lifecycle

gpui-base (Longbridge `gpui-base` from `longbridge/gpui-component`) provides
reusable component behavior and default chrome tokens. Do not use
`gpui-component`.

The application provides:

- visual identity
- themes
- colors
- typography
- spacing
- component composition

Preferred composition:

Longbridge gpui-base primitive
        ↓
application styled component
        ↓
feature UI

Desktop components consume semantic design tokens. They must not hard-code theme
colors or parse theme files. Do not add automated tests for UI spacing or color
values; review appearance visually against [docs/DESIGN.md](DESIGN.md). Platform presentation may map the same semantic intent to
different macOS, Windows, and Linux conventions. macOS follows Apple HIG behavior,
Windows follows Microsoft Windows App Design and Fluent conventions, and Linux uses a
cross-desktop baseline informed by GNOME and KDE guidance plus applicable
freedesktop.org standards. The Linux adapter must not become separate GTK and Qt
imitations.

Built-in themes map Porcelain Honey (light) and Graphite Honey (dark) onto the
semantic token model. Future user-defined themes use a separate
presentation-infrastructure boundary:

Plain-text theme file
        ↓
theme parser and validator
        ↓
semantic theme model
        ↓
GPUI presentation

Theme configuration is local presentation state. It does not belong in the domain
workspace, OpenCollection YAML, or the canonical collection repository. Invalid or
incomplete custom themes fall back safely to built-in semantic values. See
`docs/DESIGN.md` for the design-system contract.


## Application Layer

The application layer coordinates use cases.

Examples:

- load workspace
- list requests
- get request
- execute request
- resolve environment
- save request
- validate collection
- inspect and convert an imported collection

These operations should be usable from both CLI and GPUI without
knowledge of either frontend.


## Workspace

Opening a workspace:

OpenCollection files
        ↓
OpenCollectionRepository
        ↓
Domain Workspace
        ↓
Application layer
        ↓
CLI or GPUI

For the desktop application, the resulting workspace remains in memory
for fast navigation.


## Desktop Request Selection

Fast path:

User click
    ↓
RequestKey (session-only)
    ↓
in-memory lookup
    ↓
selected request state
    ↓
GPUI notification/render

It must not include:

- filesystem reads
- YAML parsing
- database queries
- network operations

The desktop shell retains the repository-loaded workspace for the lifetime of the
window. Its tree is rendered from `WorkspaceItemRef` values, and tabs retain only
session-local `RequestKey` values. Opening a collection delegates parsing and
filesystem traversal to the OpenCollection repository on a background executor;
the GPUI adapter never parses YAML. Folder expansion, active tabs, and pane sizes are
presentation state and do not modify the domain workspace.

The window title bar owns workspace-level navigation. Its workspace switcher exposes
recent collections plus explicit new, open, and close actions, while collection content
begins directly beneath the title bar without a duplicate application toolbar. New
Collection uses the platform save panel to create an empty bundled OpenCollection YAML
file through the shared repository, then loads it with the same path used to open an
existing collection. The empty sidebar offers the same new and open actions before a
collection is loaded. The
request editor and response viewer can be stacked vertically or placed side by side;
the orientation and independent response-pane dimensions are presentation state.

Request editor controls mutate the repository-loaded in-memory `HttpRequest` directly
through its runtime `RequestKey`; they do not reload or parse collection files. The tab
is the single request-name label above the editor, so the URL bar is not preceded by a
duplicate title. Persistence remains a separate repository operation and is not
implicitly coupled to individual keystrokes. Hovering a `{{variable}}` span in the
request editor (URL, headers, and body) shows only that variable, aligned to that
span, with an input for copying or updating the value on the selected environment.
An undefined name on a selected environment uses the same input to create the
variable. The popup stays open while the pointer moves from the span onto the popup.
Those edits update the in-memory environment used by Send, then persist through the
same OpenCollection repository operation used by the CLI: merge the changed variable
into the retained YAML, compare source bytes, and atomically write. Secrets remain
read-only. After a successful save, the value survives collection reload.
Environment selection lives at the fixed right edge of the request tab bar and is
workspace-scoped presentation state, so every
open request shares the same selected environment. The switcher can create a new
environment: Probe prompts for a name, updates the in-memory workspace, then persists
through the same OpenCollection repository create used by the CLI, off the UI thread.
The last selection for each
collection is restored from the desktop session. Unsaved body representations are
retained as local editor drafts per request, allowing users to switch body types without
losing work.

Desktop HTTP execution resolves the selected environment against an in-memory request,
then runs the shared `probe-http` engine away from the GPUI thread. Cancellation is
forwarded into that engine. Execution and response state is retained per runtime request
key, while generation checks prevent a superseded request from replacing a newer result.
Response state is presentation-only and is not written into OpenCollection YAML.

The response viewer renders Pretty, Raw, and Headers through Longbridge
gpui-base `Editor` in read-only mode. Probe retains the original response text,
searches the active representation, applies language highlighting through
gpui-base's highlighter seam (Syntect for JSON on the Pretty tab), overlays
search matches as decorations, and lets the editor virtualize the viewport.
JSON larger than 64 KiB is pretty-printed on a background executor. Syntax
highlighting scans buffers up to 16 MiB, matching the in-memory response page.

The request tree keeps a flat list of lightweight references for currently visible
expanded nodes. Its fixed-height GPUI list is virtualized, so scrolling constructs and
paints only the viewport range rather than every request in a large workspace. Folder
expansion rebuilds this in-memory visible-row index without filesystem access.

Tree rows are focusable collection items with directional navigation and explicit
create, rename, delete, move, and reorder controls, so drag and drop is never required.
The same repository-owned `StructureOperation` values are used when a tree row is
dragged onto a valid folder or sibling insertion target. The desktop rejects drops onto
the dragged item, its descendants, duplicate unbundled paths, and other invalid
destinations, and shows an insertion line or folder highlight only for accepted
targets. Large virtualized trees autoscroll while a drag is near the viewport edge.
Failed persistence or conflict checks leave the previous valid workspace visible.
The desktop converts these interactions into repository-owned `StructureOperation`
values and executes them on GPUI's background executor. A successful operation returns
a complete old-to-new selector map and a refreshed repository workspace. The desktop
uses that map to rebuild runtime keys for open tabs, the active tree item, collapsed
folders, editor drafts, and session state. Dirty request fields are overlaid onto their
remapped request after the structural refresh; a structural move or rename does not
implicitly save unrelated request edits. Folder and request deletion always requires
confirmation, with a stronger warning when dirty requests would be discarded.

### Desktop Session Restoration

Probe stores a small, versioned desktop-session document in the operating system's
local application-data directory. The document may contain the active and recent collection paths, open and active
request selectors, collapsed folder selectors, the last selected environment for
each recent collection, and pane sizes and orientation. It is local presentation
metadata and is never written into an OpenCollection workspace.

Session writes are atomic and run off the GPUI thread. On launch, the desktop loads the
session and active collection on a background executor, rebuilds the in-memory
workspace, and resolves repository selectors to fresh runtime keys. Invalid selectors
and environment names are ignored because the underlying item may have been removed.
A missing or invalid collection produces a recoverable diagnostic and remains available
in the recent list; it does not prevent the application from opening. Explicitly closing
a collection clears the active session without deleting collection files. The last
selected environment for that collection remains available when it is opened again.

### Runtime Identity and Persistence Locators

OpenCollection does not define durable IDs for requests or folders. The active
workspace therefore assigns compact generational `RequestKey` and `FolderKey` values
while it is loaded. These keys index in-memory storage directly, are never serialized,
and are rebuilt the next time the workspace opens. When a deleted slot is reused, its
generation changes so stale selections and asynchronous results cannot resolve to the
replacement item.

Repository adapters separately own persistence locators. An unbundled collection can
use a workspace-relative file path; a bundled collection may use a structural item
path. CLI selectors are derived from repository locators rather than runtime keys.
Request names are presentation data and must not be treated as identity.


## CLI Request Execution

Typical path:

CLI arguments
     ↓
Application operation
     ↓
WorkspaceRepository
     ↓
Request + Environment
     ↓
EnvironmentResolver
     ↓
HTTP Engine
     ↓
Response
     ↓
CLI formatter
     ↓
human text / JSON

## Environment Resolution

Environment selection and interpolation live in `probe-core`, so CLI, desktop, and
future interfaces share exactly the same behavior. Resolution operates on the loaded
in-memory workspace: parent environments are applied before children, child variables
override by name, and variable values may reference other variables. Cyclic
inheritance, cyclic interpolation, missing variables, and invalid variant selection
produce typed errors.

The resolver returns a cloned, resolved request and leaves the canonical parsed model
unchanged. It currently interpolates method, URL, headers, query and path parameters, supported
body fields, file references, and authentication string/number values. OpenCollection
secret declarations contain no value, so references fail until a separate secure
runtime value provider is introduced. Loading `dotEnvFilePath` is also outside Phase 4;
the domain resolver remains independent of filesystem APIs.

## HTTP Execution

`probe-http` owns the single asynchronous HTTP implementation. It converts resolved
domain requests into network requests, substitutes enabled `:variableName` path parameters,
applies enabled headers and query parameters,
selects body/file variants, implements Basic and Bearer authentication, and enforces
OpenCollection timeout and redirect settings. Neither CLI nor desktop constructs HTTP
requests independently.

The engine accepts a caller-provided cancellation future. Completion of that future—or
dropping the execution future—cancels the request without coupling the engine to
terminal signals or a GUI framework. The CLI adapts Ctrl-C to this boundary; desktop
can later adapt task or view cancellation to the same API.

Completed responses contain status, reason, final URL, duration, size, deterministically
sorted headers, and at most 16 MiB of in-memory body data. Once that bound is crossed, the
engine keeps the leading 16 MiB as the first presentation page and, when requested by the caller,
streams the complete body to an automatically managed spool file. Cloned response handles share
ownership of that file, and
the final owner removes it. Frontends that need the complete body provide a cache directory;
callers such as the CLI can drain the remainder without retaining it. The desktop reads subsequent
16 MiB pages off the UI thread, searches only the resident page, and renders those pages as
unwrapped Raw text without retaining a duplicate Pretty representation. Pretty is hidden for
file-backed responses because formatting an isolated page would not produce a valid document.
Inspect remains available and scans file-backed JSON and XML through streaming parsers without
constructing a complete document tree. `--output`
remains distinct: it streams chunks to a temporary file and replaces the requested user-owned
destination only after the complete response is written and synced.
Response retention and history policies remain outside the frontend and can evolve without
changing request construction.

The desktop response cache has a 512 MiB global quota. Cache sessions hold filesystem leases so
multiple Probe processes do not recover or delete one another's live responses. Initialization and
subsequent reservations remove session directories whose lease was released by a crash. Quota
accounting includes live response files from every active session. If a response cannot fit in the
remaining quota, Probe deletes its partial spool, continues draining the network response, and
returns the 16 MiB preview with a retention warning; existing retained responses are not evicted.


## Persistence

Desktop editing:

User edit
    ↓
in-memory update
    ↓
immediate render
    ↓
persistence operation
    ↓
OpenCollectionRepository
    ↓
atomic filesystem write

CLI mutation:

CLI command
    ↓
application operation
    ↓
domain update
    ↓
same repository
    ↓
atomic filesystem write

The OpenCollection repository retains the exact source bytes for every editable
request document. A request update is applied to the domain workspace first, then
merged into the retained YAML so unsupported fields survive. Before the temporary file
is committed, the repository compares the current file with its loaded bytes and
rejects externally modified sources. The temporary file is synced and atomically
committed on Unix, Windows, and WASI through a focused filesystem dependency.
Successful saves refresh the retained byte snapshot so subsequent edits from the same
loaded workspace remain safe.

Workspace structure editing is also repository-owned. Interfaces submit one typed create,
rename, delete, move, or reorder operation using repository selectors; the repository validates
the destination, applies the corresponding in-memory/domain semantics, persists it, reloads the
workspace, and returns refreshed selectors. Bundled edits retain unknown YAML and atomically
replace one document. Unbundled edits use paths as locators and `info.seq` as sibling ordering
metadata. Multi-document ordering changes retain rollback bytes, while directory moves are
reversed if metadata persistence fails. Ordering transactions write a hidden recovery directory
and manifest before mutation and retain them when rollback cannot complete. Folder deletion moves
the source to an out-of-workspace tombstone before sibling metadata changes, so failed cleanup
cannot expose partial data in the canonical collection. Every retained source document is checked
byte-for-byte under the workspace writer lock before structural mutation begins.

Filesystem paths are canonicalized when the workspace opens, so saving through a symlink
updates its target without replacing the symlink. A stable sidecar advisory lock serializes
Probe writers across processes; the exact source-byte comparison and atomic replacement both
happen while that lock is held. Non-cooperating third-party writers cannot be forced to honor
the advisory lock, so the final compare-to-rename interval remains the smallest portable race
window.

The repository exposes a prepared request save that captures the retained source baseline and
supported-field update in memory. Environment-variable set and unset use the same retained-source
merge, exact-byte conflict check, save lock, and atomic replacement for both bundled
`config.environments` and unbundled `environments/*.yml` documents. The desktop executes prepared
saves on GPUI's background executor, then returns the successful source snapshot to the loaded
repository so later saves use the refreshed conflict baseline. This does not create a second
persistence path; CLI and desktop use the same implementation. Request and environment writes
are serialized so a bundled collection file is not saved concurrently with itself.

Desktop dirty state compares each live request with its last loaded or successfully saved snapshot.
One request save runs at a time, so close and quit protection can safely save several requests
stored in the same bundled document. Completion acknowledges the captured request snapshot rather than the current
request; edits made while I/O is running therefore remain dirty. Save failures leave the in-memory
request unchanged, and destructive tab, workspace, and window closes require an explicit Save,
Discard, or Cancel decision.

### Filesystem Synchronization

The desktop watches an unbundled collection recursively and watches the containing directory
of a bundled collection so atomic file replacement does not detach the watch. Notifications are
debounced and treated only as invalidation hints. Probe reloads and validates the collection
through `OpenCollectionRepository` on a background executor before changing live state.

Reconciliation is three-way: the repository's last loaded or saved request is the baseline, the
editor owns a potentially dirty local request, and a fresh repository load supplies the disk
request. Changes to different supported fields merge automatically. Changes to the same field,
external deletion of a dirty request, and ambiguous dirty renames require an explicit Use Disk or
Keep Local decision. Invalid or partially written files leave the last valid workspace open.

Runtime keys are rebuilt after every accepted reload. Tabs and collapsed folders are captured as
repository selectors and resolved to new keys, with paired filesystem renames and unique unchanged
content used as rename evidence. A successful Probe save refreshes the repository baseline, so its
watcher notification reconciles as a no-op instead of appearing as an external conflict.

## OpenCollection Validation

Workspace loading requires the OpenCollection `1.0.0` format marker, collection metadata, and
an explicit `bundled` mode matching the source kind. It also validates environment names and
the complete inheritance graph, including duplicate names, missing parents, and cycles. This
validation is shared by `collection validate` and every operation that loads a workspace.


## Repository Abstraction

The application should not know that the workspace is represented by
YAML.

Conceptually:

                  WorkspaceRepository
                         ▲
              ┌──────────┴──────────┐
              │                     │
 OpenCollectionRepository      other repositories

Only OpenCollection is required initially.


## HTTP Engine

HTTP execution is shared.

                         HTTP Engine
                         ▲         ▲
                         │         │
                       CLI       GPUI

The engine accepts domain/application request representations rather
than CLI arguments or GPUI state.

It returns structured response data.


## Streaming Protocols

Long-lived protocols require a session/event abstraction.

Conceptually:

WebSocket / SSE / gRPC
          ↓
    Protocol Session
          ↓
       Event Stream
       ┌──────────┐
       ▼          ▼
      CLI        GPUI
       │          │
 JSONL/stdin     visual
 terminal        session UI

Terminal interaction must not leak into the protocol implementation.


## Responses

Responses are separate from request definitions.

Request metadata
→ generally resident in desktop memory

Response metadata
→ cache/history

Response bodies
→ bounded memory and/or filesystem

Large response bodies must not dictate workspace memory consumption.


## Concurrency

Application/UI-facing state has clear ownership.

Slow operations execute outside the GPUI render/event path:

- HTTP
- filesystem
- YAML parsing
- JSON processing
- Git
- streaming network work

Completed operations return structured results/events.

Avoid shared mutable global state.


## Future MCP Interface

MCP should be another adapter over the application layer.

It should not wrap CLI commands unless there is a compelling reason.

Prefer:

MCP
 ↓
Application/Core

rather than:

MCP
 ↓
shell
 ↓
CLI
 ↓
Application/Core

This keeps tool semantics structured and avoids unnecessary process and
text-parsing boundaries.
