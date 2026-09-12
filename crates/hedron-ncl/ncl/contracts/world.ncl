# World contract — one agent file, three frozen projections (SPEC-nickel R8 / G5a).
# `cluster` is tightened to Pod objects this h3s minor admits (Service is a follow-on).

let DocsEod = {
  kind | force = "docs_eod",
  date | String,
  required_briefs | Array String,
} in
let OpenCollection = {
  opencollection | force = "1.0.0",
  info | { name | String, summary | String | optional, .. } | optional,
  config | {
    environments | Array {
      name | String,
      variables | Array {
        name | String,
        secret_ref | String | optional,
        value | String | not_exported | optional,
        ..
      } | optional,
      ..
    } | optional,
    ..
  } | optional,
  ..
} in
let K8sContainer = {
  name | String,
  image | String,
  securityContext | Dyn | optional,
  ..
} in
let H3sPod = {
  apiVersion | force = "v1",
  kind | force = "Pod",
  metadata | { name | String, .. },
  spec | {
    automountServiceAccountToken | Bool | optional,
    enableServiceLinks | Bool | optional,
    dnsPolicy | String | optional,
    containers | Array K8sContainer,
    volumes | Array Dyn | optional,
    securityContext | Dyn | optional,
    ..
  },
  ..
} in
let DesiredState = {
  name | String,
  importance | Number,
  spec | DocsEod,
} in
{
  cluster | Array H3sPod,
  intent | Array DesiredState,
  calls | OpenCollection,
}
