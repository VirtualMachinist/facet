//! G0b: contract-set marker id and platform law (no `| force` automount).

use hedron_ncl::pack::{
    contract_set_marker_bytes, read_contract_set_marker, write_contract_set_marker,
    CONTRACT_SET_MARKER_FILE,
};
use hedron_ncl::{CONTRACT_SET, PLATFORM_NCL};

#[test]
fn contract_set_marker_is_v091_hold() {
    assert_eq!(CONTRACT_SET, "k8s-1.34-h3s-0.9.1");
    assert_eq!(contract_set_marker_bytes(), b"k8s-1.34-h3s-0.9.1");
    assert_eq!(CONTRACT_SET_MARKER_FILE, "contract-set");
}

#[test]
fn contract_set_marker_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    write_contract_set_marker(dir.path()).unwrap();
    assert_eq!(
        read_contract_set_marker(dir.path()).unwrap(),
        "k8s-1.34-h3s-0.9.1"
    );
    let on_disk = std::fs::read(dir.path().join(CONTRACT_SET_MARKER_FILE)).unwrap();
    assert_eq!(on_disk, b"k8s-1.34-h3s-0.9.1");
}

#[test]
fn contract_set_marker_rejects_wrong_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(CONTRACT_SET_MARKER_FILE), b"k8s-1.34-h3s-0.10.0").unwrap();
    let err = read_contract_set_marker(dir.path()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("0.10.0"), "{err}");
}

fn code_lines(src: &str) -> String {
    src.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn platform_does_not_force_automount() {
    let platform = code_lines(PLATFORM_NCL);
    assert!(
        !platform.contains("automountServiceAccountToken | force"),
        "platform.ncl must not `| force` automount; that is a release overlay gap"
    );
    assert!(
        !platform.contains("enableServiceLinks | force"),
        "platform.ncl must not `| force` enableServiceLinks"
    );
}
