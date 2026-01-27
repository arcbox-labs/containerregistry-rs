//! Hosted registry integration test for gcrane.

use std::process::Command;

#[test]
fn test_gcrane_hosted_ls() {
    let enabled = std::env::var("HOSTED_REGISTRY_INTEGRATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        eprintln!("skipping hosted integration test (set HOSTED_REGISTRY_INTEGRATION=1)");
        return;
    }

    let repo = std::env::var("HOSTED_REGISTRY_REPO")
        .expect("HOSTED_REGISTRY_REPO must be set (e.g., ghcr.io/org/repo)");

    let output = Command::new(env!("CARGO_BIN_EXE_gcrane"))
        .args(["ls", &repo])
        .output()
        .expect("run gcrane ls");

    assert!(output.status.success(), "gcrane ls failed");
}
