# platform.ncl — platform law. Only `| force` invariants that survive feature
# growth belong here (SPEC-nickel R3 / G0b):
#
#   * containers run non-root, drop ALL capabilities, cannot escalate privilege
#   * Hedron `extra` carries no secret-looking fields
#   * OpenCollection secret slots are references, never plaintext values
#
# NOT here, on purpose: automountServiceAccountToken=false, enableServiceLinks=false,
# ClusterIP-only / NodePort refusal, ConfigMap+Secret-only volumes. Those are gaps of
# one h3s release and live in ncl/overlay/<contract-set>.ncl as a priority overlay,
# so that 0.10 (bound tokens, NodePort, PVC) removes them without a contract break.
let secret_field_re =
  "(?i)(^|[_-])(token|password|passwd|secret|api[_-]?key|private[_-]?key|bearer|credential)s?($|[_-])"
in
{
  id = "platform",

  # ---------------------------------------------------------------------------
  # Contracts
  # ---------------------------------------------------------------------------

  # A record none of whose field names look like a secret. Applied to Hedron `extra`.
  NoSecretFields
    | not_exported
    = std.contract.custom
      (
        fun label value =>
          if !std.is_record value then
            'Error { message = "platform: expected a record" }
          else
            let bad =
              std.record.fields value
              |> std.array.filter (fun k => std.string.is_match secret_field_re k)
            in
            if bad == [] then
              'Ok value
            else
              'Error
                {
                  message = "platform: secret-looking fields are not allowed here: %{std.string.join ", " bad}",
                  notes = ["Store the value in the vault and reference it; never copy secrets into Hedron `extra`."],
                }
      ),

  # A secret slot in an OpenCollection projection. Only `{ secret_ref = "<name>" }`
  # may cross the export boundary. Plaintext values are `| not_exported` in
  # ncl/contracts/opencollection.ncl and hydrated by Facet outside the document.
  SecretSlot
    | not_exported
    = std.contract.custom
      (
        fun label value =>
          if std.is_record value && std.record.has_field "secret_ref" value then
            'Ok value
          else
            'Error
              {
                message = "platform: secret slots must be { secret_ref = \"<name>\" }, never a plaintext value",
                notes = ["Facet hydrates secret_ref at run time; Nickel never sees the raw token."],
              }
      ),

  # ---------------------------------------------------------------------------
  # Pod law as merge overlays. `| force` wins over operator / agent / library.
  # ---------------------------------------------------------------------------

  container = {
    securityContext = {
      runAsNonRoot | force = true,
      allowPrivilegeEscalation | force = false,
      capabilities = { drop | force = ["ALL"] },
    },
  },

  pod = {
    spec = {
      securityContext = { runAsNonRoot | force = true },
    },
  },

  # Apply platform law to a whole Pod. Containers are arrays, so the container
  # law is merged element-wise (Nickel does not merge arrays).
  harden_pod
    | not_exported
    = fun p =>
      let harden = std.array.map (fun c => c & container) in
      let inits =
        if std.record.has_field "initContainers" p.spec then
          { spec = { initContainers | force = harden p.spec.initContainers } }
        else
          {}
      in
      p & pod & { spec = { containers | force = harden p.spec.containers } } & inits,
}
