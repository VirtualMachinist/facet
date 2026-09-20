# Jev native transport

Facet is the product-native HTTP transport for [TypeSafe](https://api.typesafe.ai)
Jev System One. Later suite members (Lapis, HedronDB, Geode) call these recipes
instead of linking an SDK. This is **gates, not writers**: Jev classifies; Facet
does not generate vault text, API bodies, or approvals.

The example collection lives at
[`docs/examples/typesafe/`](examples/typesafe/opencollection.yml)
(OpenCollection 1.0.0, bundled). Vault SoT is `foundry/facet/collections/typesafe`.

## Law

1. Jev HTTP goes through `facet request run` or MCP `request_run` (1:1). Do not
   curl TypeSafe with a second key copy.
2. Jev **gates** (Choice / Noul). It does not write code, vault text, or API bodies.
3. Empty / low-confidence / `ask` / `hold` / `escalate` **≠ approve**.
4. Fail-closed if Facet or hydration is down (`secret_variable_unavailable`, exit 5).
5. Never put secrets, GTOK, or private org bodies in `state`.
6. Key via `$TYPESAFE_API_KEY` → `facet env set --secret` only. **Never** Lattice
   bodies, YAML values, `--var` in MCP, or session/history JSON.

## Shadow

The collection default is `jevShadow=true`. Shadow means **log and classify
only**. A Choice or Noul answer is not an authorization.

| Outcome | Meaning |
| --- | --- |
| empty / missing / low confidence | Not an approval. Fail closed or escalate. |
| `ask` | Needs a human. Not allow. |
| `hold` | Do not send yet. Not recipe / not allow. |
| `escalate` | Need a human. Not continue. |
| `deny` / `stop` | Refuse. Code enforces. |
| `allow` / `continue` / `recipe` / Noul `true` | Informative under shadow. Callers must not treat this as a write grant. |

`jevShadow=false` is a later, explicit product decision. Until then, treat every
recipe as classify-only. Facet records the HTTP exchange; it does not interpret
Jev answers as policy.

## Secret hydration

`typesafeApiKey` is declared `secret: true` with **no value** in the YAML.
Probe-core refuses to interpolate it empty.

```bash
# Once per workspace (machine store / keyring or FACET_SECRET_KEY).
facet env set docs/examples/typesafe --environment typesafe \
  --name typesafeApiKey --value "$TYPESAFE_API_KEY" --secret
```

`facet env list` returns metadata only (name, secret flag, updated). Values never
appear in `history`, `--sql`, `session`, or `env list`. Authorization headers
are stored as `<redacted>`. Do not put the key in request `state` or a Lattice
body.

`--var typesafeApiKey=…` on the CLI still works as a runtime override and is
still a secret (scrubbed from Lattice text columns). MCP **refuses** a `var`
whose name is already stored in the machine store; hydrate, do not pass the key
as a tool argument.

## CLI — `facet request run`

Same engine as `probe request run`, plus Lattice recording.

```bash
cd docs/examples/typesafe

# Reachability (Noul)
facet request run opencollection.yml items/0/items/0 \
  --environment typesafe --expect 2xx

# Loop-stop / escalate (Choice), override state for the step
facet request run opencollection.yml items/0/items/2 \
  --environment typesafe --var state='…step context…' --expect 2xx

# Preview resolve + redact; no network, no row
facet request run opencollection.yml items/0/items/1 \
  --environment typesafe --dry-run
```

| Selector | Name | Use |
| --- | --- | --- |
| `items/0/items/0` | Smoke noul | Reachability |
| `items/0/items/1` | Tool gate | allow / ask / deny |
| `items/0/items/2` | Loop stop escalate | continue / stop / escalate |
| `items/0/items/3` | Recipe vs adhoc | recipe / adhoc / hold |

`--expect 2xx` is HTTP success, not a Jev approval. Read the Choice / Noul
in the response body; apply the shadow table above.

## MCP — `request_run` (1:1)

`facet mcp` exposes the same function the CLI calls. Tool `request_run` is
byte-for-byte `facet request run --json`. No second API, no stdout parse.

```json
{
  "name": "request_run",
  "arguments": {
    "path": "docs/examples/typesafe/opencollection.yml",
    "selector": "items/0/items/0",
    "environment": "typesafe",
    "expect": "2xx"
  }
}
```

`var` is an object of non-secret overrides (`state`, not `typesafeApiKey`).
Hydration supplies the key from `facet env set`. See [Facet MCP](FACET.md#mcp).

## Out of scope

Stanley CLI, Pi SDK, Facet-as-chat, and in-crate TypeSafe SDKs are not this
spike. Overlay loop-stop (omapi-overlay harness) is a parallel track.
