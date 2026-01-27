//! Parity CLI tests (Go crane vs Rust crane).

use containerregistry_testing::ParityRunner;

fn parity_enabled() -> bool {
    std::env::var("PARITY_CLI")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[test]
fn test_parity_cli_digest_manifest_ls() {
    if !parity_enabled() {
        eprintln!("skipping parity CLI test (set PARITY_CLI=1)");
        return;
    }

    let runner = ParityRunner::new();
    if !runner.has_go_crane() {
        eprintln!("skipping parity CLI test (go crane not found in PATH)");
        return;
    }
    if !runner.has_rust_crane() {
        eprintln!("skipping parity CLI test (rust crane not found in PATH)");
        return;
    }

    // These tests assume a working registry and reference in env.
    let reference = std::env::var("PARITY_REFERENCE")
        .unwrap_or_else(|_| "localhost:5000/library/alpine:latest".to_string());

    let digest = runner
        .compare_digest(&reference)
        .expect("run digest parity");
    assert!(
        digest.passed,
        "digest parity failed: {:?}",
        digest.differences
    );

    let manifest = runner
        .compare_manifest(&reference)
        .expect("run manifest parity");
    assert!(
        manifest.passed,
        "manifest parity failed: {:?}",
        manifest.differences
    );

    let repo = reference
        .split('@')
        .next()
        .unwrap_or(&reference)
        .split(':')
        .next()
        .unwrap_or(&reference)
        .to_string();
    let ls = runner.compare_tags(&repo).expect("run ls parity");
    assert!(ls.passed, "ls parity failed: {:?}", ls.differences);

    // Error parity: missing image should fail similarly.
    let missing_ref = format!("{}/missing:tag", repo);
    let go_crane = std::env::var("GO_CRANE_PATH")
        .ok()
        .unwrap_or_else(|| "crane".to_string());
    let rust_crane = std::env::var("CRANE_RUST_PATH")
        .ok()
        .or_else(|| std::env::var("CARGO_BIN_EXE_crane").ok())
        .unwrap_or_else(|| "crane".to_string());

    let go_out = std::process::Command::new(&go_crane)
        .args(["manifest", &missing_ref])
        .output()
        .expect("run go crane");
    let rust_out = std::process::Command::new(&rust_crane)
        .args(["manifest", &missing_ref])
        .output()
        .expect("run rust crane");

    assert_eq!(go_out.status.success(), rust_out.status.success());
}
