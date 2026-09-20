# TypeSafe Jev collection

Canonical **Jev System One** OpenCollection for product-native gates. Contract:
[Jev native transport](../../jev-native.md).

Secrets via `facet env set --secret` from `$TYPESAFE_API_KEY` — never this YAML,
never Lattice bodies.

| | |
| --- | --- |
| Environment | `typesafe` — hydrate `typesafeApiKey` |
| Shadow | `jevShadow=true` — log/classify only; empty / `ask` / `hold` / `escalate` ≠ approve |

## Selectors (v0.2)

| Selector | Name | Primitive |
| --- | --- | --- |
| `items/0/items/0` | Smoke noul | Noul |
| `items/0/items/1` | Tool gate | Choice allow / ask / deny |
| `items/0/items/2` | Loop stop escalate | Choice continue / stop / escalate |
| `items/0/items/3` | Recipe vs adhoc | Choice recipe / adhoc / hold |

```bash
facet env set docs/examples/typesafe --environment typesafe \
  --name typesafeApiKey --value "$TYPESAFE_API_KEY" --secret

facet request run docs/examples/typesafe/opencollection.yml items/0/items/0 \
  --environment typesafe --expect 2xx
```
