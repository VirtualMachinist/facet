# k8s-1.34-h3s-0.9.1 — RestrictedV1 release overlay (SPEC-nickel G0b).
#
# This file says what THIS h3s binary can actually run. It is versioned with the
# h3s minor and applied at `| priority 1000`: above operator policy (100) and agent
# intent (0), below platform `| force`. It is NOT eternal law. When 0.10 lands
# bound tokens / NodePort / PVC, the matching lines leave the successor overlay
# (k8s-1.34-h3s-0.10.0) and nothing in ncl/platform.ncl changes.
#
# Blame sentences mirror h3s `pod_profile.rs` / `admission.rs` (422 runtime profile).
{
  id = "k8s-1.34-h3s-0.9.1",
  kubernetes = "1.34",
  h3s = "0.9.1",
  profile = "restricted-v1",

  # 0.9 gaps: no TokenRequest / bearer tokens yet, so nothing is mounted; no
  # service-link env injection. Overlay priority, deliberately not `force`.
  pod = {
    spec = {
      automountServiceAccountToken | priority 1000 = false,
      enableServiceLinks | priority 1000 = false,
    },
  },

  # Contracts and functions below are `not_exported`: exporting the overlay yields
  # only its data (id, versions, allow-lists).

  # Only ClusterIP forwarding exists in 0.9 (ServiceForwarding::UnsupportedType).
  service_types = ["ClusterIP"],

  # Only ConfigMap and Secret volumes are honoured by the 0.9 kubelet.
  volume_kinds = ["configMap", "secret"],

  ServiceType
    | not_exported
    = std.contract.custom
      (
        fun label value =>
          if std.array.elem value service_types then
            'Ok value
          else
            'Error
              {
                message = "%{id}: Service type %{std.to_string value} is not supported by h3s %{h3s}; allowed: %{std.string.join ", " service_types}",
              }
      ),

  Volume
    | not_exported
    = std.contract.custom
      (
        fun label value =>
          if !std.is_record value then
            'Error { message = "%{id}: volume must be a record" }
          else
            let kinds =
              std.record.fields value |> std.array.filter (fun k => k != "name")
            in
            let bad = std.array.filter (fun k => !(std.array.elem k volume_kinds)) kinds in
            if bad == [] then
              'Ok value
            else
              'Error
                {
                  message = "%{id}: volume kind %{std.string.join ", " bad} is not supported by h3s %{h3s}; allowed: %{std.string.join ", " volume_kinds}",
                }
      ),

  Pod | not_exported = {
    spec
      | {
        volumes | Array Volume | optional,
        ..
      },
    ..
  },

  Service | not_exported = {
    spec
      | {
        type | ServiceType | optional,
        ..
      },
    ..
  },

  # Apply the 0.9 overlay to a Pod: overlay values win over agent/operator
  # values, and the release contracts are checked.
  restrict_pod | not_exported = fun p => (p & pod) | Pod,
}
