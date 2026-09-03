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

#[test]
fn version_flag_honors_json_output() {
    for flag in ["--version", "-V"] {
        let output = probe()
            .args([flag, "--json"])
            .output()
            .expect("probe binary should start");

        assert!(output.status.success(), "{flag} --json should exit 0");
        assert!(output.stderr.is_empty());
        let value: Value =
            serde_json::from_slice(&output.stdout).expect("version JSON should parse");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["name"], "probe");
        assert_eq!(
            value["version"],
            env!("CARGO_PKG_VERSION"),
            "{flag} --json should print the Cargo workspace version"
        );
    }
}

#[test]
fn version_flag_honors_quiet() {
    for flag in ["--version", "-V"] {
        for quiet_flag in ["--quiet", "-q"] {
            let output = probe()
                .args([flag, quiet_flag])
                .output()
                .expect("probe binary should start");

            assert!(output.status.success(), "{flag} {quiet_flag} should exit 0");
            assert!(
                output.stdout.is_empty(),
                "{flag} {quiet_flag} should print nothing"
            );
            assert!(output.stderr.is_empty());
        }
    }
}

#[test]
fn version_flag_still_rejects_json_and_quiet_together() {
    let output = probe()
        .args(["--version", "--json", "--quiet"])
        .output()
        .expect("probe binary should start");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).expect("error JSON should parse");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["error"]["category"], "invalid_arguments");
}
