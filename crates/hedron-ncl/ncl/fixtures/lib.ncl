# Shared builders for in-tree World fixtures. Applies platform law and the 0.9.1
# release overlay so cluster projections match examples/supported-pod.yaml.

let platform = import "../platform.ncl" in
let overlay = import "../overlay/k8s-1.34-h3s-0.9.1.ncl" in
{
  supported_pod = fun args =>
    overlay.restrict_pod (platform.harden_pod {
      apiVersion = "v1",
      kind = "Pod",
      metadata = { name = args.name },
      spec = {
        dnsPolicy = "Default",
        containers = [
          {
            name = "pause",
            image = "registry.k8s.io/pause:3.10",
            securityContext = {
              runAsUser = 65534,
              seccompProfile = { type = "RuntimeDefault" },
            },
          },
        ],
      },
    }),
}
