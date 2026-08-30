# CLI

The `probe` CLI is non-interactive and separates command output on stdout from human
diagnostics on stderr. Add `--json` to commands that return data or structured errors.

## Commands

```text
probe collection create <path> [--name <name>] [--json]
probe collection import postman <source.json> <destination> [--allow-partial] [--json]
probe collection import yaak <source> <destination> [--workspace <id>] [--allow-partial] [--json]
probe collection validate <path> [--json]
probe request list <path> [--json]
probe request get <path> <selector> [--environment <name>] [--json]
probe request run <path> <selector> [--environment <name>] [--var <name=value>]... [--output <file>] [--json]
probe request set <path> <selector> [--name <name>] [--method <method>] [--url <url>] [--json]
probe request create <path> --name <name> [--parent <folder>] [--index <index>] [--method <method>] [--url <url>] [--json]
probe request rename <path> <selector> --name <name> [--json]
probe request delete <path> <selector> [--json]
probe request move <path> <selector> [--parent <folder>] [--index <index>] [--json]
probe request reorder <path> <selector> --index <index> [--json]
probe folder list <path> [--json]
probe folder create <path> --name <name> [--parent <folder>] [--index <index>] [--json]
probe folder rename <path> <selector> --name <name> [--json]
probe folder delete <path> <selector> [--json]
probe folder move <path> <selector> [--parent <folder>] [--index <index>] [--json]
probe folder reorder <path> <selector> --index <index> [--json]
probe environment create <path> --name <name> [--extends <parent>] [--json]
probe environment list <path> [--json]
probe environment set <path> --environment <name> --name <var> --value <value> [--json]
probe environment unset <path> --environment <name> --name <var> [--json]
probe environment delete <path> --environment <name> [--json]
probe environment rename <path> --environment <name> --name <new> [--json]
```

`<path>` may be a bundled OpenCollection YAML file or an unbundled collection
directory containing `opencollection.yml` or `opencollection.yaml`.
`collection create` always writes a new bundled YAML file and refuses to overwrite
an existing path. A missing `.yml` extension is added. `--name` sets `info.name`;
otherwise the file stem is used. Stdin (`-`) is not accepted.

`collection import yaak` accepts either an official Yaak export JSON file (schemas
1–4) or a Yaak Directory/Git Sync directory. It converts one Yaak workspace through
the shared import adapter and writes a new bundled OpenCollection YAML file atomically.
The destination is never overwritten and stdin is not accepted. If a source contains
multiple workspaces, pass the exact Yaak workspace ID with `--workspace`; JSON errors
include the selectable IDs and names.

`collection import postman` accepts one official Postman Collection v2.0 or v2.1 JSON
file. It does not accept Postman environment exports, data dumps, or v3 YAML.
Collection variables are stored in a `Postman Collection Variables` OpenCollection
environment and the returned JSON names that environment when one was created.
`--workspace` is Yaak-only and is rejected for Postman imports.

Import is strict by default. Unsupported or unknown data returns
`unsupported_import` without creating the destination. `--allow-partial` explicitly
permits those omissions and returns every deterministic compatibility diagnostic in
`warnings`. Authentication kinds that OpenCollection can store are preserved even if
Probe's current HTTP engine cannot execute them; those appear as warning diagnostics,
not silent data loss.

For `request get` and `request run`, `--environment <name>` selects an environment,
applies parent environments from `extends`, and interpolates variables in supported
request fields. Without it, `request get` returns the request as stored, including
unresolved `{{variable}}` expressions. That flag is not a write operation; use
`environment set` and `environment unset` to persist variable values.

`request run` resolves the request and executes it through the shared asynchronous HTTP
engine. Pressing Ctrl-C cancels the active execution. `--output <file>` writes the raw
response body to the specified path using bounded streaming; response metadata remains on
stdout. The destination is replaced only after the complete response has been written.

Repeatable `--var <name=value>` arguments provide invocation-only variables for `request run`.
They override selected and inherited environment values before dependent variables are
interpolated, and also work without `--environment`. If a name is repeated, the last value
wins. Runtime variables are never written to the environment or collection source.

Use `-` instead of `<path>` to read a bundled OpenCollection YAML document from
stdin. Stdin does not represent an unbundled directory, and requests loaded this way
use bundled structural selectors.

`request set` is a deliberately small, non-interactive persistence command. At
least one of `--name`, `--method`, or `--url` is required. It updates the in-memory
request first, merges only those fields into the retained YAML document, and then
atomically replaces the source file. It is unavailable for stdin workspaces.

`environment set` and `environment unset` persist OpenCollection environment variables
through the same repository path. `--environment` names the environment to mutate; it
does not resolve a request. `set` writes a plain variable on that environment, updating
it when present or adding an override when the value currently comes from a parent.
`--name` is the variable and `--value` is required. `unset` removes the variable entry
from that environment only, so a parent value can show through. Both commands reject
secrets, empty names, and stdin workspaces.

`environment delete` and `environment rename` use the same `--environment` flag for the
existing environment. `--name` on rename is the new identity, matching `environment create`.
Parent environments cannot be deleted or renamed; that failure is `environment_in_use`.
Stdin workspaces cannot be persisted.

Before committing, Probe compares the source file with the exact bytes that were
loaded. If another process changed it, the command fails with `workspace_modified`
instead of overwriting the external edit. Unknown YAML fields are retained, although
comments and original formatting may change when the YAML document is serialized.

Structural commands are non-interactive and use the same repository operation for bundled and
unbundled workspaces. Omit `--parent` for the collection root and omit `--index` to append.
`reorder` keeps an item in its current parent and requires its new zero-based `--index`.
Bundled selectors are structural and may change when siblings move. Unbundled creation and rename
derive lowercase hyphenated paths from names (requests use `.yml`); an existing destination is
never overwritten.

Bundled edits atomically replace the single collection document. Unbundled ordering is persisted
as `info.seq` in each affected sibling document. Multi-file ordering writes retain rollback
snapshots, and file/directory moves are rolled back if metadata persistence fails. Every retained
source is compared byte-for-byte before a structural write, so external changes fail safely.
Durable recovery directories with manifests are retained if a multi-document rollback cannot
complete.

`--quiet` (or `-q`) suppresses stdout for successful commands, which is useful when
only the exit status matters. Failure diagnostics remain on stderr. `--quiet` and
`--json` are mutually exclusive because structured mode always emits a result.

## Request Selectors

Selectors are repository locators, not session-only `RequestKey` values:

- Unbundled collection: workspace-relative YAML path, such as
  `users/list-users.yml`.
- Bundled collection: structural source path, such as `items/0/items/2`.

Use `request list` to discover valid selectors. Request names are never treated as
identity.

## JSON Output

`collection validate --json` returns:

```json
{
  "schemaVersion": 1,
  "collection": {
    "name": "Example",
    "summary": null,
    "version": null
  },
  "counts": {
    "environments": 0,
    "folders": 1,
    "requests": 2
  },
  "valid": true
}
```

Every JSON success and error document has top-level `schemaVersion: 1`. Fields may be
added compatibly within schema version 1, but documented fields will not be removed or
change type without incrementing the version.

`collection create --json` returns:

```json
{
  "schemaVersion": 1,
  "collection": {
    "name": "pets"
  },
  "counts": {
    "environments": 0,
    "folders": 0,
    "requests": 0
  },
  "created": true,
  "path": "/tmp/pets.yml"
}
```

`collection import yaak --json` returns:

```json
{
  "schemaVersion": 1,
  "counts": { "environments": 1, "folders": 1, "requests": 1 },
  "imported": true,
  "partial": false,
  "path": "/tmp/imported.yml",
  "sourceFormat": "yaak_export",
  "warnings": [],
  "workspace": { "id": "wk_1", "name": "Pets" }
}
```

`collection import postman --json` returns:

```json
{
  "schemaVersion": 1,
  "collection": { "id": "8dcb...", "name": "Pets" },
  "collectionVariablesEnvironment": "Postman Collection Variables",
  "counts": { "environments": 1, "folders": 1, "requests": 2 },
  "imported": true,
  "partial": false,
  "path": "/tmp/imported.yml",
  "sourceFormat": "postman_collection_v2_1",
  "warnings": []
}
```

`collection.id`, `collection.name`, and `collectionVariablesEnvironment` are nullable.
Postman v2.0 uses `postman_collection_v2_0` as `sourceFormat`.

`request list --json` returns a `requests` array. Each entry has nullable `method`,
`name`, and `url` fields plus a string `selector`.

`folder list --json` returns a `folders` array in deterministic collection order.
Each entry has nullable `name` and `parent` fields plus a string `selector`.

`request get --json` returns `authentication`, `body`, `environment`, `headers`,
`method`, `name`, `pathParameters`, `queryParameters`, `selector`, and `url`. `environment` is the
selected name or JSON `null`. Missing optional values are JSON `null`. Headers and
query and path parameters contain stable `disabled`, `name`, and `value` fields. Path
parameters are referenced from URLs with `:variableName` segments.

`request set --json` returns the same request shape after the persisted update,
with `environment` set to JSON `null`.

`environment set --json` and `environment unset --json` return `environment`, `name`,
and `operation`. `set` also returns `value`.

`environment create --json` returns `environment` and `operation`, plus `extends` when a
parent was supplied. `environment delete --json` returns `environment` and `operation`.
`environment rename --json` returns `environment` (the new name), `previousEnvironment`,
and `operation`.

Structural commands return stable fields `operation`, `itemType`, `previousSelector`, `selector`,
`parent`, `index`, and `selectorRemaps`. The remap object contains every surviving known
repository selector, including siblings whose bundled structural selector shifted. `selector`
and `index` are `null` after deletion; `previousSelector` is `null` after creation.

`request run --json` returns:

```json
{
  "schemaVersion": 1,
  "request": {
    "method": "GET",
    "url": "https://api.example.com/users"
  },
  "response": {
    "body": {
      "content": "{\"users\":[]}",
      "encoding": "utf8",
      "omissionReason": null,
      "omitted": false,
      "outputPath": null
    },
    "durationMs": 128,
    "headers": [
      { "name": "content-type", "value": "application/json" }
    ],
    "reason": "OK",
    "sizeBytes": 12,
    "status": 200,
    "url": "https://api.example.com/users"
  }
}
```

UTF-8 bodies up to 16 MiB are included directly. Larger and binary bodies are omitted
from stdout with `omitted: true`; rerun with `--output <file>` to retain them. When an
output file is used, `outputPath` identifies it and `content` remains `null`.

Structured errors use:

```json
{
  "schemaVersion": 1,
  "error": {
    "category": "request_not_found",
    "exitCode": 4,
    "message": "request selector not found: missing.yml"
  }
}
```

`error.category` and `error.exitCode` are the stable programmatic failure contract.
`error.message` is a human diagnostic and may include platform-specific paths or I/O
text, so automation must not parse it. JSON stdout never contains progress output,
terminal escape sequences, or logs.

Environment failures use exit code 5 and stable categories including
`environment_not_found`, `duplicate_environment`, `environment_in_use`,
`missing_variable`, `variable_not_found`,
`secret_variable_unavailable`, and
`environment_resolution`. Secret variables declared by OpenCollection do not contain
their values; until a secure runtime provider is added, referencing one reports
`secret_variable_unavailable` rather than silently substituting an empty value.

HTTP request configuration errors use exit code 5 and category
`request_configuration`. Timeout, cancellation, connection, protocol, and response
body failures use exit code 6 with categories such as `request_timeout`,
`request_cancelled`, and `network_execution`. Output-file failures use `output_error`.
Failure to read a stdin workspace uses `stdin_error` and exit code 3; invalid YAML read
from stdin uses `invalid_workspace` and exit code 3.

Persistence failures use exit code 7. Stable categories are `workspace_modified` for
external-modification conflicts, `persistence_read_only` for stdin sources,
`recovery_required` when a multi-file rollback could not be completed, and
`committed_refresh_failed` when persistence succeeded but the workspace could not be refreshed.
`committed_cleanup_failed` means deletion committed but an out-of-workspace tombstone requires
manual cleanup. Callers must not retry operations reported as committed failures without
reloading the workspace. Other serialization or filesystem failures use `persistence_error`.

Structural validation uses stable categories `folder_not_found`, `destination_not_found`,
`duplicate_destination`, `invalid_destination`, `invalid_name`, and `invalid_index`.
Missing request/folder selectors use exit code 4; invalid destinations, names, duplicates, and
indices use exit code 2.

Postman and Yaak compatibility failures use exit code 8 and category
`unsupported_import`. Malformed or unsupported import schemas use `invalid_import` and
exit code 3; a missing or ambiguous Yaak workspace selection, invalid provider-specific
arguments, and an existing destination use exit code 2. Other destination write
failures use the existing persistence categories and exit code 7.

`collection validate` requires the OpenCollection `1.0.0` marker, explicit collection
metadata, and a `bundled` flag matching whether the source is a bundled file/stdin document or
an unbundled directory. Duplicate environments and invalid inheritance graphs are rejected.

## Exit Codes

| Code | Category |
| ---: | --- |
| 0 | Success |
| 2 | Invalid arguments |
| 3 | Invalid workspace or parse failure |
| 4 | Request or folder not found |
| 5 | Configuration or environment error |
| 6 | Network, cancellation, execution, or response-output error |
| 7 | Persistence failure or external-modification conflict |
