//! G0a: `facet ncl pack --out` materializes the embedded pack byte-for-byte.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use hedron_ncl::pack::{verify, write_to, CONTRACT_SET_MARKER_FILE, FILES, ROOT};
use hedron_ncl::CONTRACT_SET;

fn ncl_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ncl")
}

fn walk(dir: &std::path::Path, out: &mut BTreeSet<String>, base: &std::path::Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "fixtures" || n == ".facet")
            {
                continue;
            }
            walk(&path, out, base);
        } else if path.extension().is_some_and(|e| e == "ncl") {
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel);
        }
    }
}

#[test]
fn written_bytes_equal_embedded_and_on_disk_sources() {
    let dir = tempfile::tempdir().unwrap();
    let written = write_to(dir.path()).unwrap();
    assert_eq!(written.root, dir.path().join(ROOT));
    assert_eq!(written.files.len(), FILES.len());
    for (file, path) in FILES.iter().zip(&written.files) {
        let bytes = fs::read(path).unwrap();
        assert_eq!(
            bytes,
            file.source.as_bytes(),
            "{}: written != include_str!",
            file.path
        );
        let on_disk = fs::read(ncl_dir().join(file.path)).unwrap();
        assert_eq!(bytes, on_disk, "{}: written != ncl/ source tree", file.path);
    }
    assert!(verify(dir.path()).unwrap().is_empty());
}

#[test]
fn inventory_covers_the_ncl_tree_except_fixtures() {
    let mut on_disk = BTreeSet::new();
    walk(&ncl_dir(), &mut on_disk, &ncl_dir());
    let listed: BTreeSet<String> = FILES.iter().map(|f| f.path.to_owned()).collect();
    assert_eq!(
        listed, on_disk,
        "pack::FILES and ncl/ disagree; list new contracts"
    );
}

#[test]
fn marker_names_the_contract_set() {
    let dir = tempfile::tempdir().unwrap();
    let written = write_to(dir.path()).unwrap();
    assert_eq!(
        written.marker,
        dir.path().join(ROOT).join(CONTRACT_SET_MARKER_FILE)
    );
    let text = fs::read_to_string(&written.marker).unwrap();
    assert!(text.contains("k8s-1.34-h3s-0.9.1"), "{text}");
    assert_eq!(written.contract_set, CONTRACT_SET);
    assert_eq!(CONTRACT_SET, "k8s-1.34-h3s-0.9.1");
}

#[test]
fn pack_never_legislates_release_gaps_as_platform_force() {
    let platform = FILES.iter().find(|f| f.path == "platform.ncl").unwrap();
    let code: String = platform
        .source
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    for gap in [
        "automountServiceAccountToken",
        "enableServiceLinks",
        "NodePort",
        "ClusterIP",
    ] {
        assert!(!code.contains(gap), "platform.ncl must not legislate {gap}");
    }
}

#[test]
fn verify_reports_drift_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let written = write_to(dir.path()).unwrap();
    fs::write(&written.files[0], "# tampered\n").unwrap();
    fs::remove_file(&written.files[1]).unwrap();
    fs::write(&written.marker, "k8s-1.34-h3s-9.9.9\n").unwrap();
    let drift = verify(dir.path()).unwrap();
    assert_eq!(
        drift,
        vec![
            FILES[0].path.to_owned(),
            FILES[1].path.to_owned(),
            CONTRACT_SET_MARKER_FILE.to_owned()
        ]
    );
    // Rewriting heals it; writing twice is idempotent.
    write_to(dir.path()).unwrap();
    assert!(verify(dir.path()).unwrap().is_empty());
    let before: Vec<Vec<u8>> = written.files.iter().map(|p| fs::read(p).unwrap()).collect();
    write_to(dir.path()).unwrap();
    let after: Vec<Vec<u8>> = written.files.iter().map(|p| fs::read(p).unwrap()).collect();
    assert_eq!(before, after);
}
