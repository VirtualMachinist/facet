//! Materialized pack layout for `facet ncl pack --out` (Tetra M1 G0).
//!
//! Fullstack owns the CLI; backend owns the **contract-set marker** id and the
//! law that automount-false lives in the release overlay (`| priority 1000`),
//! never as platform `| force` (V010 HOLD keeps id at `k8s-1.34-h3s-0.9.1`).

use std::fs;
use std::io;
use std::path::Path;

use crate::overlay::CONTRACT_SET;

/// Filename written at the pack root: `<out>/contract-set`.
pub const CONTRACT_SET_MARKER_FILE: &str = "contract-set";

/// Bytes of the contract-set marker while V010 is HOLD.
pub fn contract_set_marker_bytes() -> &'static [u8] {
    CONTRACT_SET.as_bytes()
}

/// Write `<out>/contract-set` with [`CONTRACT_SET`]. Creates `out` if absent.
pub fn write_contract_set_marker(out: &Path) -> io::Result<()> {
    fs::create_dir_all(out)?;
    fs::write(out.join(CONTRACT_SET_MARKER_FILE), contract_set_marker_bytes())
}

/// Read and validate `<pack>/contract-set`. Fails if missing or not exactly
/// [`CONTRACT_SET`].
pub fn read_contract_set_marker(pack: &Path) -> io::Result<String> {
    let bytes = fs::read(pack.join(CONTRACT_SET_MARKER_FILE))?;
    let id = std::str::from_utf8(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .trim();
    if id != CONTRACT_SET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "pack contract-set marker is `{id}`; expected `{CONTRACT_SET}`"
            ),
        ));
    }
    Ok(id.to_owned())
}
