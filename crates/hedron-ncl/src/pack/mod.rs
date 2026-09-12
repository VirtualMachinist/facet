//! The materialized contract pack (tetra M1 G0a).
//!
//! Out-of-tree worlds cannot `import "../platform.ncl"`. `facet ncl pack --out
//! <dir>` writes the sources embedded in this crate to `<dir>/hedron-ncl/…`
//! byte-for-byte, so a world evaluated with `--import-path <dir>` can write
//! `import "hedron-ncl/platform.ncl"` and get exactly the pack this Facet
//! binary carries. [`verify`] lets `tetractl doctor` prove that later.
//!
//! Inventory rule: every `.ncl` under `ncl/` except `ncl/fixtures/` is in
//! [`FILES`]; `tests/pack.rs` fails when the tree and the list disagree.

/// Contract-set marker API (backend-tetra, G0b). Re-exported so callers keep
/// `hedron_ncl::pack::{write_contract_set_marker, …}`.
pub mod marker;
pub use marker::*;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directory name under `--out`; also the first import segment.
pub const ROOT: &str = "hedron-ncl";

/// One embedded pack file: path relative to [`ROOT`], and its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackFile {
    pub path: &'static str,
    pub source: &'static str,
}

/// The pack, in write order. Platform law and the release overlay reuse the
/// constants `overlay.rs` already embeds, so there is one source per file.
pub const FILES: &[PackFile] = &[
    PackFile {
        path: "platform.ncl",
        source: crate::overlay::PLATFORM_NCL,
    },
    PackFile {
        path: "overlay/k8s-1.34-h3s-0.9.1.ncl",
        source: crate::overlay::OVERLAY_NCL,
    },
    PackFile {
        path: "contracts/docs_eod.ncl",
        source: include_str!("../../ncl/contracts/docs_eod.ncl"),
    },
    PackFile {
        path: "contracts/opencollection.ncl",
        source: include_str!("../../ncl/contracts/opencollection.ncl"),
    },
    PackFile {
        path: "contracts/world.ncl",
        source: include_str!("../../ncl/contracts/world.ncl"),
    },
];

/// What [`write_to`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// `<out>/hedron-ncl`.
    pub root: PathBuf,
    /// The pack files, absolute, in [`FILES`] order.
    pub files: Vec<PathBuf>,
    /// `<out>/hedron-ncl/<CONTRACT_SET_MARKER_FILE>`.
    pub marker: PathBuf,
    /// The contract set the marker names.
    pub contract_set: &'static str,
}

/// Materialize the pack under `out` (created if missing, files overwritten).
pub fn write_to(out: &Path) -> io::Result<Written> {
    let root = out.join(ROOT);
    let mut files = Vec::with_capacity(FILES.len());
    for file in FILES {
        let path = root.join(file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, file.source)?;
        files.push(path);
    }
    marker::write_contract_set_marker(&root)?;
    let marker = root.join(marker::CONTRACT_SET_MARKER_FILE);
    Ok(Written {
        root,
        files,
        marker,
        contract_set: crate::CONTRACT_SET,
    })
}

/// Compare a materialized pack under `out` with the embedded sources. Returns
/// the relative paths that are missing or differ; empty means byte-identical.
pub fn verify(out: &Path) -> io::Result<Vec<String>> {
    let root = out.join(ROOT);
    let mut drift = Vec::new();
    for file in FILES {
        match fs::read(root.join(file.path)) {
            Ok(bytes) if bytes == file.source.as_bytes() => {}
            Ok(_) => drift.push(file.path.to_owned()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => drift.push(file.path.to_owned()),
            Err(e) => return Err(e),
        }
    }
    match fs::read(root.join(marker::CONTRACT_SET_MARKER_FILE)) {
        Ok(bytes) if bytes == marker::contract_set_marker_bytes() => {}
        Ok(_) => drift.push(marker::CONTRACT_SET_MARKER_FILE.to_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            drift.push(marker::CONTRACT_SET_MARKER_FILE.to_owned())
        }
        Err(e) => return Err(e),
    }
    Ok(drift)
}
