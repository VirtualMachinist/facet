#[allow(dead_code, unused_imports)]
mod common;

use common::*;

#[test]
fn version_flag_prints_workspace_version_and_exits_successfully() {
    for flag in ["--version", "-V"] {
        let output = probe()
            .arg(flag)
            .output()
            .expect("probe binary should start");

        assert!(output.status.success(), "{flag} should exit 0");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("probe {}\n", env!("CARGO_PKG_VERSION")),
            "{flag} should print the Cargo workspace version"
        );
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn help_still_works_alongside_version_flag() {
    let output = probe()
        .arg("--help")
        .output()
        .expect("probe binary should start");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("collection validate"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("-V, --version"));
    assert!(output.stderr.is_empty());
}
