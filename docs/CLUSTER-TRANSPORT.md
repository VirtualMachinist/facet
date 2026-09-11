# Authenticated cluster requests

Facet can use a project kubeconfig for verified client-certificate authentication.
Set `FACET_KUBECONFIG` to one explicit kubeconfig file; optionally set
`FACET_KUBE_CONTEXT` to a context name in that file. Otherwise its `current-context`
is selected. Facet does not discover ambient `KUBECONFIG` or execute credential
plugins.

```sh
export FACET_KUBECONFIG=/absolute/project/runtime/facet.kubeconfig
export FACET_KUBE_CONTEXT=m1
facet mcp
```

The same shared transport applies to CLI `request run`, replay, TUI execution and
MCP `request_run`/`run_replay`. Ordinary Facet behavior is unchanged when the
variable is absent. Canonical OpenCollection YAML holds URLs, methods and request
bodies; certificate and private-key material stays in the runtime kubeconfig or
referenced files. Do not place these credentials in Nix expressions or stores,
collection YAML, command arguments, or Lattice fields.

## Supported profile and enforcement

This profile supports a kubeconfig `v1`/`Config` with a selected context, cluster
and user; a CA bundle; and a client certificate/private key. Each material may be
embedded as base64 `*-data` or referenced by a file path relative to the
kubeconfig. Exactly one source per material is required. Missing/duplicate selected
names, malformed input, and files over 4 MiB fail closed with credential-safe
diagnostics.

The configured server must be an HTTPS origin without userinfo, a path prefix,
query or fragment. Its CA and hostname are verified. The final request URL,
after substitutions, must have that exact origin. Same-origin redirects obey the
request's limit; cross-origin or downgrade redirects fail before connecting to
the destination. Non-default redirect settings preserve the client certificate
and origin restrictions. The profile disables ambient HTTP proxies and TLS key
logging.

The profile rejects insecure TLS, TLS-name overrides, proxy settings, exec and
auth-provider plugins, bearer/token-file/password authentication and impersonation.
Other unsupported cluster/user fields are rejected rather than silently ignored.
It also rejects explicit Authorization, Proxy-Authorization, Host and
Impersonate-* request headers to avoid conflicting identity/routing. This is a
bounded certificate profile, not a claim to implement every kubeconfig feature.
Unset `FACET_KUBECONFIG` to use ordinary non-cluster Facet requests.

Use a dedicated identity with only the necessary RBAC permissions in the intended
namespace. Loading a kubeconfig does not itself grant permissions; h3s authorizes
every API action. Dry runs continue to avoid loading credentials or sending requests.
TLS/transport configuration failures are recorded as failed actions when recording
is enabled. HTTP 403 is a real response; use `--expect`/MCP `expect` to have Facet
report the rejected action as a tool failure while retaining its history.

## Session integration

Use private `FACET_CONFIG_DIR` and `FACET_DATA_DIR`; keep workspace and machine
configuration consistent. See [Lattice engines](LATTICE-ENGINES.md).

`session_start` returns an ID. The harness sets `FACET_SESSION` on the MCP process
used for subsequent requests, and `FACET_ACTOR` identifies the actor. Restarting
MCP with that same session environment preserves the lineage. The transport never
adds credential material to the request model or to recorded headers/body metadata.

## Tower acceptance probe

`integration/tower/check-cluster-mcp.py` drives the real Facet MCP protocol against
an existing h3s API. It requires explicit project tool, operator kubeconfig, PKI
bundle, artifact-root and non-system namespace paths. Run it through the pinned
Facet Nix shell, with remote builders disabled and job/core limits of two. The
shell supplies OpenSSL for the operator-controlled, three-day test certificate.

The probe issues an unprivileged test identity, installs a temporary ConfigMap-only
Role and RoleBinding, then verifies create/read/delete, Secret and cross-namespace
RBAC denials, Turso recording/indexing, session history, and MCP process reopen.
The operator CA key is materialized only in a private temporary signing directory
and removed immediately after issuance. The client credentials are removed at the
end. Fixture cleanup checks ownership and uses UID preconditions; sanitized MCP
transcripts, canonical YAML, Turso databases and a result manifest remain under a
unique project artifact directory.

This probe does not run containers, OMP, Herdr or a model. It supports the Facet
integration checkpoint; the complete six-component workflow, HQL, runtime recovery
and portability retain separate M1 acceptance requirements.
