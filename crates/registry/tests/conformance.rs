//! OCI distribution conformance suite integration.

use std::process::Command;

#[test]
fn test_oci_distribution_conformance() {
    let enabled = std::env::var("OCI_CONFORMANCE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        eprintln!("skipping conformance test (set OCI_CONFORMANCE=1)");
        return;
    }

    let cmd = std::env::var("OCI_CONFORMANCE_CMD")
        .expect("OCI_CONFORMANCE_CMD must be set (e.g., \"./conformance -c config.yml\")");

    let status = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .status()
        .expect("failed to run conformance command");

    assert!(status.success(), "conformance suite failed");
}
