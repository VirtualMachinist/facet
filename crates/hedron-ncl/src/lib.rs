//! `hedron-ncl` — the Nickel contract pack for h3s, Facet and HedronDB, and the
//! **only** crate on the platform that links the Nickel VM (`nickel-lang-core`).
//!
//! SPEC-nickel product law: Nickel is the configuration algebra; Facet is the only
//! process that evaluates it; h3s and HedronDB consume frozen data through
//! predicates *generated* from this pack (see [`gen`]). Never add
//! `nickel-lang-core` to `h3s-*` or `hedron-core`.
//!
//! Layout:
//! - [`overlay`] — embedded `ncl/platform.ncl` (platform `| force` law) and the
//!   versioned release overlay `ncl/overlay/<CONTRACT_SET>.ncl` (RestrictedV1 as a
//!   priority overlay, not eternal law).
//! - [`eval`] — thin wrapper over `nickel-lang-core`: source in, frozen JSON out.
//! - [`gen`] — generated Rust predicates (backend owns `gen::docs_eod`).
//! - `ncl/contracts/` — `docs_eod.ncl` (backend), `opencollection.ncl` (frontend).

#![forbid(unsafe_code)]

pub mod eval;
pub mod r#gen;
pub mod overlay;

pub use eval::{eval_export, eval_export_with_prelude, Error};
pub use overlay::{contract_set_id, prelude, CONTRACT_SET, OVERLAY_NCL, PLATFORM_NCL};
