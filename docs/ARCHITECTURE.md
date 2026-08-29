# Architecture

## Overview

The application uses two interfaces over shared application and domain layers.

                         Interfaces

                       ┌─────┴─────┐
                       │           │
                      CLI         GPUI
                                Desktop
                       │           │
                       └─────┬─────┘
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
                       ┌─────┴─────┐
                       │           │
                OpenCollection   HTTP
                 Repository      Engine
                       │           │
                       ▼           ▼
                     YAML       Network

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

The CLI is a first-class automation and headless interface. Its responsibilities
are limited to:

- argument parsing
- invoking application operations
- human-readable presentation
- structured presentation
- exit-code mapping
- stdin/stdout integration

It must not implement domain behavior.

### Structured output

The CLI's JSON documents carry an explicit schema version. Automation should branch
on stable error categories and exit codes, never parse human diagnostic messages.
Bundled collections may be supplied through stdin without moving YAML parsing into
the frontend: the OpenCollection repository projects the in-memory document and
builds the same structural selectors used for bundled files. Quiet mode is a
presentation concern and suppresses only successful command output. The complete
public contract is in [CLI.md](CLI.md).


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
colors or parse theme files. Platform presentation may map the same semantic intent
to different macOS, Windows, and Linux conventions.

Built-in themes map Porcelain Honey (light) and Graphite Honey (dark) onto the
semantic token model. [DESIGN.md](DESIGN.md) is the canonical source for platform
behavior, visual testing, themes, and accessibility.


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


## Desktop Runtime

Opening a collection delegates filesystem traversal and parsing to the
OpenCollection repository on a background executor. The desktop retains the resulting
workspace in memory for the life of the window. Request selection is therefore:

    user selection
        ↓
    session-only RequestKey
        ↓
    O(1) in-memory lookup
        ↓
    render notification

The selection path performs no filesystem, YAML, database, or network work. Folder
expansion, tabs, pane state, and environment selection are presentation state.

Request controls mutate the in-memory domain request. Saving is a separate shared
repository operation and never occurs implicitly on each keystroke. Environment
resolution and mutation use core and repository operations rather than desktop-only
logic. Secrets remain unavailable for editing until Probe has a supported runtime
value provider.

Desktop Send resolves the selected environment and executes the same probe-http
engine used by the CLI, away from the UI thread. Cancellation reaches that engine;
generation checks prevent stale completions from replacing newer results. Response
and execution state remain presentation-only.

The response viewer uses virtualized, read-only editing and performs expensive
formatting or highlighting on a background executor. The request tree similarly
virtualizes a flat index of visible in-memory item references.

All create, rename, delete, move, and reorder interactions become repository-owned
StructureOperation values. A successful operation returns a refreshed workspace and
selector remaps. The desktop rebuilds runtime keys and remaps tabs, selection,
collapsed folders, drafts, and session state; dirty request fields are not implicitly
saved. Destructive operations require confirmation when data or drafts would be lost.

### Desktop Session Restoration

Probe stores a small, versioned session document in the platform application-data
directory. It contains presentation metadata such as recent collection paths,
repository selectors, selected environments, pane state, and orientation. It is never
stored in OpenCollection YAML.

Session I/O is atomic and runs off the UI thread. Restoration reloads the collection,
rebuilds runtime keys, and resolves persisted repository selectors. Missing collections,
items, or environments produce recoverable state rather than preventing startup.
Closing a collection clears active session state without deleting collection files.

### Runtime Identity and Persistence Locators

OpenCollection does not define durable request or folder IDs. Each loaded workspace
therefore assigns generational RequestKey and FolderKey values for fast, stale-safe
in-memory lookup. These keys are never serialized and are rebuilt on reload.

Repository adapters separately own persistence locators: workspace-relative paths for
unbundled collections and structural item paths for bundled collections. CLI selectors
and desktop-session references use these locators. Names are presentation data, not
identity.

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
produce typed errors. The same crate also exposes the effective plain variables for a
selected environment, together with the environment that currently defines each name,
so desktop presentation does not reimplement inheritance, overrides, or secret
shadowing.

The resolver returns a cloned, resolved request and leaves the canonical parsed model
unchanged. It currently interpolates method, URL, headers, query and path parameters, supported
body fields, file references, and authentication string/number values. OpenCollection
secret declarations contain no value, so references fail until a separate secure
runtime value provider is introduced. The resolver does not load `dotEnvFilePath`;
the domain remains independent of filesystem APIs.

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


## Persistence and Filesystem Synchronization

Interfaces submit domain or repository operations; only the OpenCollection repository
serializes YAML or mutates collection files. Desktop calls prepare work in memory and
execute filesystem operations away from the UI thread.

For request and environment changes, the repository retains the loaded source and
merges supported fields into that document so unknown YAML survives. Under a stable
workspace writer lock it compares the current source with the loaded bytes, writes and
syncs a temporary file, and atomically replaces the destination. Successful writes
refresh the retained baseline. Symlinked workspaces update their canonical target
without replacing the symlink.

Desktop dirty state compares the live request with its last loaded or successfully
saved snapshot. Save completion acknowledges the captured revision, so edits made
while I/O is running remain dirty. Failures and external-change conflicts retain the
in-memory draft. Closing dirty work requires an explicit save, discard, or cancel
decision.

Structural mutations are also repository-owned. Bundled operations retain unknown
YAML and replace one document atomically. Unbundled multi-document moves and ordering
changes retain rollback data and a recovery manifest; incomplete rollback is reported
as requiring recovery rather than hidden. Every affected retained source is checked
before mutation.

Filesystem notifications are invalidation hints. The desktop debounces them, reloads
through the repository on a background executor, and reconciles baseline, local draft,
and disk state. Non-overlapping field changes merge automatically; overlapping edits,
dirty deletion, and ambiguous rename require an explicit choice. Invalid or partially
written files never replace the last valid workspace.

Accepted reloads rebuild runtime keys and resolve repository selectors for tabs and
presentation state. Confident file renames may remap selectors. Successful Probe writes
refresh the baseline, so their watcher events reconcile as no-ops.

## OpenCollection Validation

Workspace loading requires the OpenCollection `1.0.0` format marker, collection metadata, and
an explicit `bundled` mode matching the source kind. It also validates environment names and
the complete inheritance graph, including duplicate names, missing parents, and cycles. This
validation is shared by `collection validate` and every operation that loads a workspace.


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
