//! Embedded contract sources and the versioned contract set id.
//!
//! Priority ladder (SPEC-nickel):
//!
//! | layer           | Nickel            | lives in                          |
//! |-----------------|-------------------|-----------------------------------|
//! | platform        | `\| force`        | `ncl/platform.ncl`                |
//! | release overlay | `\| priority 1000`| `ncl/overlay/<CONTRACT_SET>.ncl`  |
//! | operator        | `\| priority 100` | operator policy files             |
//! | agent           | plain             | agent `world.ncl`                 |
//! | library         | `\| default`      | contract pack defaults            |
//!
//! Automount-false, no service links, ClusterIP-only and the ConfigMap/Secret
//! volume restriction are **0.9 gaps**: they live in the overlay and leave with
//! the h3s minor that closes them. They are never platform `| force`.

use crate::eval::{eval_export, Error};

/// Contract set id of the current release overlay. Bumps with the h3s minor
/// (`k8s-1.34-h3s-0.10.0` once bound tokens / NodePort land). Lattice tags
/// `ncl:contracts:<CONTRACT_SET>` and HedronDB `source_ref.contract_set` carry it.
pub const CONTRACT_SET: &str = "k8s-1.34-h3s-0.9.1";

/// Platform law: `| force` invariants that survive feature growth.
pub const PLATFORM_NCL: &str = include_str!("../ncl/platform.ncl");

/// RestrictedV1 release overlay for [`CONTRACT_SET`].
pub const OVERLAY_NCL: &str = include_str!("../ncl/overlay/k8s-1.34-h3s-0.9.1.ncl");

/// Nickel prelude binding `platform` and `overlay`, so a body can say
/// `overlay.restrict_pod (platform.harden_pod my_pod)`.
pub fn prelude() -> String {
    format!("let platform = (\n{PLATFORM_NCL}\n) in let overlay = (\n{OVERLAY_NCL}\n) in\n")
}

/// The `id` field of the embedded overlay, as evaluated by the VM. Must equal
/// [`CONTRACT_SET`]; `tests/overlay.rs` pins that.
pub fn contract_set_id() -> Result<String, Error> {
    let v = eval_export("overlay", OVERLAY_NCL)?;
    v.get("id")
        .and_then(|id| id.as_str())
        .map(str::to_owned)
        .ok_or_else(|| Error::Export("overlay has no string `id` field".into()))
}
