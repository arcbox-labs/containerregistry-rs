//! Parity testing utilities for comparing with go-containerregistry.
//!
//! Provides utilities for running parallel operations in both Rust and Go
//! implementations and comparing results.

use std::path::Path;
use std::process::Command;

use containerregistry_image::{Algorithm, Digest};

/// Result of a parity comparison.
#[derive(Debug)]
pub struct ParityResult {
    /// Whether the comparison passed.
    pub passed: bool,
    /// Rust implementation output.
    pub rust_output: String,
    /// Go implementation output.
    pub go_output: String,
    /// Differences found.
    pub differences: Vec<String>,
}

impl ParityResult {
    /// Creates a passing result.
    pub fn pass(rust_output: String, go_output: String) -> Self {
        Self {
            passed: true,
            rust_output,
            go_output,
            differences: Vec::new(),
        }
    }

    /// Creates a failing result.
    pub fn fail(rust_output: String, go_output: String, differences: Vec<String>) -> Self {
        Self {
            passed: false,
            rust_output,
            go_output,
            differences,
        }
    }
}

/// Runner for parity tests between Rust and Go implementations.
pub struct ParityRunner {
    /// Path to the Go crane binary.
    go_crane_path: Option<String>,
    /// Path to the Rust crane binary.
    rust_crane_path: Option<String>,
    /// Whether to normalize outputs before comparison.
    normalize: bool,
}

impl Default for ParityRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl ParityRunner {
    /// Creates a new parity runner.
    pub fn new() -> Self {
        // Try to find Go crane in PATH and Rust crane via env.
        let go_crane_path = which_crane();
        let rust_crane_path = rust_crane_from_env();
        Self {
            go_crane_path,
            rust_crane_path,
            normalize: true,
        }
    }

    /// Sets the path to the Go crane binary.
    pub fn with_go_crane(mut self, path: impl Into<String>) -> Self {
        self.go_crane_path = Some(path.into());
        self
    }

    /// Sets the path to the Rust crane binary.
    pub fn with_rust_crane(mut self, path: impl Into<String>) -> Self {
        self.rust_crane_path = Some(path.into());
        self
    }

    /// Sets whether to normalize outputs.
    pub fn with_normalize(mut self, normalize: bool) -> Self {
        self.normalize = normalize;
        self
    }

    /// Returns true if Go crane is available.
    pub fn has_go_crane(&self) -> bool {
        self.go_crane_path.is_some()
    }

    /// Returns true if Rust crane is available.
    pub fn has_rust_crane(&self) -> bool {
        self.rust_crane_path.is_some()
    }

    /// Runs parity comparison by invoking both CLI binaries with the same args.
    pub fn test_parity_cli(&self, args: &[&str]) -> Result<ParityResult, String> {
        let go_crane = self
            .go_crane_path
            .as_ref()
            .ok_or("Go crane not available")?;
        let rust_crane = self
            .rust_crane_path
            .as_ref()
            .ok_or("Rust crane not available")?;

        if go_crane == rust_crane {
            return Err("Rust crane path matches Go crane path; set CRANE_RUST_PATH".to_string());
        }

        let go_output = run_cli(go_crane, args)?;
        let rust_output = run_cli(rust_crane, args)?;

        if self.normalize {
            match args.first().copied() {
                Some("digest") => {
                    let go_norm = normalize_digest(&go_output);
                    let rust_norm = normalize_digest(&rust_output);
                    if go_norm == rust_norm {
                        Ok(ParityResult::pass(rust_output, go_output))
                    } else {
                        Ok(ParityResult::fail(
                            rust_output,
                            go_output,
                            vec![format!(
                                "Digest mismatch: go={}, rust={}",
                                go_norm, rust_norm
                            )],
                        ))
                    }
                }
                Some("manifest") => {
                    let go_norm = normalize_json(&go_output)?;
                    let rust_norm = normalize_json(&rust_output)?;
                    if go_norm == rust_norm {
                        Ok(ParityResult::pass(rust_output, go_output))
                    } else {
                        Ok(ParityResult::fail(
                            rust_output,
                            go_output,
                            diff_json(&go_norm, &rust_norm),
                        ))
                    }
                }
                Some("ls") => {
                    let go_norm = normalize_lines(&go_output);
                    let rust_norm = normalize_lines(&rust_output);
                    if go_norm == rust_norm {
                        Ok(ParityResult::pass(rust_output, go_output))
                    } else {
                        Ok(ParityResult::fail(
                            rust_output,
                            go_output,
                            diff_lines(&go_norm, &rust_norm),
                        ))
                    }
                }
                _ => {
                    let go_norm = go_output.trim().to_string();
                    let rust_norm = rust_output.trim().to_string();
                    if go_norm == rust_norm {
                        Ok(ParityResult::pass(rust_output, go_output))
                    } else {
                        Ok(ParityResult::fail(
                            rust_output,
                            go_output,
                            vec!["Output mismatch".to_string()],
                        ))
                    }
                }
            }
        } else if go_output == rust_output {
            Ok(ParityResult::pass(rust_output, go_output))
        } else {
            Ok(ParityResult::fail(
                rust_output,
                go_output,
                vec!["Output mismatch".to_string()],
            ))
        }
    }

    /// Compares manifest digests for a reference.
    pub fn compare_digest(&self, reference: &str) -> Result<ParityResult, String> {
        self.test_parity_cli(&["digest", reference])
    }

    /// Compares manifest JSON for a reference.
    pub fn compare_manifest(&self, reference: &str) -> Result<ParityResult, String> {
        self.test_parity_cli(&["manifest", reference])
    }

    /// Compares tag listings for a repository.
    pub fn compare_tags(&self, repository: &str) -> Result<ParityResult, String> {
        self.test_parity_cli(&["ls", repository])
    }
}

/// Tries to find crane in PATH.
fn which_crane() -> Option<String> {
    Command::new("which")
        .arg("crane")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn rust_crane_from_env() -> Option<String> {
    std::env::var("CRANE_RUST_PATH")
        .ok()
        .or_else(|| std::env::var("CARGO_BIN_EXE_crane").ok())
}

fn run_cli(path: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(path)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {}: {}", path, e))?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            path,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Normalizes a digest string (trims whitespace).
fn normalize_digest(digest: &str) -> String {
    digest.trim().to_string()
}

/// Normalizes JSON by parsing and re-serializing.
fn normalize_json(json: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Invalid JSON: {}", e))?;
    serde_json::to_string_pretty(&value).map_err(|e| format!("Failed to serialize: {}", e))
}

/// Normalizes lines by sorting them.
fn normalize_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().map(|s| s.trim().to_string()).collect();
    lines.sort();
    lines
}

/// Finds differences between two JSON strings.
fn diff_json(expected: &str, actual: &str) -> Vec<String> {
    let mut diffs = Vec::new();

    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();

    for (i, (exp, act)) in exp_lines.iter().zip(act_lines.iter()).enumerate() {
        if exp != act {
            diffs.push(format!("Line {}: expected '{}', got '{}'", i + 1, exp, act));
        }
    }

    if exp_lines.len() != act_lines.len() {
        diffs.push(format!(
            "Line count differs: expected {}, got {}",
            exp_lines.len(),
            act_lines.len()
        ));
    }

    diffs
}

/// Finds differences between two sorted line lists.
fn diff_lines(expected: &[String], actual: &[String]) -> Vec<String> {
    let mut diffs = Vec::new();

    for line in expected {
        if !actual.contains(line) {
            diffs.push(format!("Missing: {}", line));
        }
    }

    for line in actual {
        if !expected.contains(line) {
            diffs.push(format!("Extra: {}", line));
        }
    }

    diffs
}

/// Helper for comparing OCI layout directories.
#[derive(Default)]
pub struct LayoutCompareOptions {
    /// Verify blob content matches the digest.
    pub verify_content: bool,
    /// Compare blob sizes when present in both layouts.
    pub compare_sizes: bool,
}

pub fn compare_layouts(
    expected_path: &Path,
    actual_path: &Path,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    compare_layouts_with_options(expected_path, actual_path, &LayoutCompareOptions::default())
}

pub fn compare_layouts_with_options(
    expected_path: &Path,
    actual_path: &Path,
    options: &LayoutCompareOptions,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut differences = Vec::new();

    // Compare index.json
    let exp_index = std::fs::read_to_string(expected_path.join("index.json"))?;
    let act_index = std::fs::read_to_string(actual_path.join("index.json"))?;

    let exp_json: serde_json::Value = serde_json::from_str(&exp_index)?;
    let act_json: serde_json::Value = serde_json::from_str(&act_index)?;

    if exp_json != act_json {
        differences.push("index.json differs".to_string());
    }

    let exp_blobs_root = expected_path.join("blobs");
    let act_blobs_root = actual_path.join("blobs");

    let exp_algs = list_algorithm_dirs(&exp_blobs_root)?;
    let act_algs = list_algorithm_dirs(&act_blobs_root)?;

    for alg in exp_algs.difference(&act_algs) {
        differences.push(format!("Missing algorithm directory: {}", alg));
    }
    for alg in act_algs.difference(&exp_algs) {
        differences.push(format!("Extra algorithm directory: {}", alg));
    }

    for alg in exp_algs.union(&act_algs) {
        let exp_dir = exp_blobs_root.join(alg);
        let act_dir = act_blobs_root.join(alg);

        if !exp_dir.exists() || !act_dir.exists() {
            continue;
        }

        let exp_files = list_blob_files(&exp_dir)?;
        let act_files = list_blob_files(&act_dir)?;

        for file in exp_files.difference(&act_files) {
            differences.push(format!("Missing blob: {}:{}", alg, file));
        }

        for file in act_files.difference(&exp_files) {
            differences.push(format!("Extra blob: {}:{}", alg, file));
        }

        if options.compare_sizes || options.verify_content {
            for file in exp_files.intersection(&act_files) {
                let exp_path = exp_dir.join(file);
                let act_path = act_dir.join(file);

                if options.compare_sizes {
                    let exp_size = std::fs::metadata(&exp_path)?.len();
                    let act_size = std::fs::metadata(&act_path)?.len();
                    if exp_size != act_size {
                        differences.push(format!(
                            "Blob size mismatch for {}:{} (expected {}, got {})",
                            alg, file, exp_size, act_size
                        ));
                    }
                }

                if options.verify_content {
                    if let Some(expected_digest) = parse_digest(alg, file) {
                        let exp_bytes = std::fs::read(&exp_path)?;
                        let act_bytes = std::fs::read(&act_path)?;
                        if Digest::compute(expected_digest.algorithm(), &exp_bytes)
                            != expected_digest
                        {
                            differences.push(format!(
                                "Expected blob digest mismatch for {}:{}",
                                alg, file
                            ));
                        }
                        if Digest::compute(expected_digest.algorithm(), &act_bytes)
                            != expected_digest
                        {
                            differences
                                .push(format!("Actual blob digest mismatch for {}:{}", alg, file));
                        }
                    } else {
                        differences.push(format!("Invalid digest filename for {}:{}", alg, file));
                    }
                }
            }
        }
    }

    Ok(differences)
}

fn list_algorithm_dirs(
    root: &Path,
) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
    let mut algs = std::collections::HashSet::new();
    if !root.exists() {
        return Ok(algs);
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            algs.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(algs)
}

fn list_blob_files(
    dir: &Path,
) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error>> {
    let mut files = std::collections::HashSet::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(files)
}

fn parse_digest(algorithm: &str, hex: &str) -> Option<Digest> {
    let alg = match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => Algorithm::Sha256,
        "sha384" => Algorithm::Sha384,
        "sha512" => Algorithm::Sha512,
        _ => return None,
    };
    Digest::new(alg, hex).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn write_layout(
        root: &Path,
        blobs: &[(Algorithm, &Digest, &[u8])],
    ) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(root.join("blobs"))?;
        std::fs::write(
            root.join("index.json"),
            r#"{"schemaVersion":2,"manifests":[]}"#,
        )?;

        for (alg, digest, data) in blobs {
            let alg_dir = match alg {
                Algorithm::Sha256 => "sha256",
                Algorithm::Sha384 => "sha384",
                Algorithm::Sha512 => "sha512",
            };
            let dir = root.join("blobs").join(alg_dir);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join(digest.hex()), data)?;
        }
        Ok(())
    }

    #[test]
    fn test_normalize_lines() {
        let text = "c\na\nb";
        let normalized = normalize_lines(text);
        assert_eq!(normalized, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_diff_lines() {
        let expected = vec!["a".to_string(), "b".to_string()];
        let actual = vec!["b".to_string(), "c".to_string()];
        let diffs = diff_lines(&expected, &actual);

        assert!(diffs.iter().any(|d| d.contains("Missing: a")));
        assert!(diffs.iter().any(|d| d.contains("Extra: c")));
    }

    #[test]
    fn test_parity_runner_creation() {
        let runner = ParityRunner::new();
        // Just test that it creates without panicking
        let _ = runner.has_go_crane();
    }

    #[test]
    fn test_compare_layouts_multi_algo_ok() {
        let expected = tempdir().unwrap();
        let actual = tempdir().unwrap();

        let data_a = b"sha256-layer";
        let data_b = b"sha384-layer";
        let data_c = b"sha512-layer";

        let digest_a = Digest::sha256(data_a);
        let digest_b = Digest::sha384(data_b);
        let digest_c = Digest::sha512(data_c);

        write_layout(
            expected.path(),
            &[
                (Algorithm::Sha256, &digest_a, data_a),
                (Algorithm::Sha384, &digest_b, data_b),
                (Algorithm::Sha512, &digest_c, data_c),
            ],
        )
        .unwrap();

        write_layout(
            actual.path(),
            &[
                (Algorithm::Sha256, &digest_a, data_a),
                (Algorithm::Sha384, &digest_b, data_b),
                (Algorithm::Sha512, &digest_c, data_c),
            ],
        )
        .unwrap();

        let options = LayoutCompareOptions {
            verify_content: true,
            compare_sizes: true,
        };
        let diffs = compare_layouts_with_options(expected.path(), actual.path(), &options).unwrap();
        assert!(diffs.is_empty(), "unexpected diffs: {:?}", diffs);
    }

    #[test]
    fn test_compare_layouts_detects_content_mismatch() {
        let expected = tempdir().unwrap();
        let actual = tempdir().unwrap();

        let expected_data = b"expected";
        let digest = Digest::sha256(expected_data);

        write_layout(
            expected.path(),
            &[(Algorithm::Sha256, &digest, expected_data)],
        )
        .unwrap();
        write_layout(actual.path(), &[(Algorithm::Sha256, &digest, b"actual")]).unwrap();

        let options = LayoutCompareOptions {
            verify_content: true,
            compare_sizes: false,
        };
        let diffs = compare_layouts_with_options(expected.path(), actual.path(), &options).unwrap();
        assert!(
            diffs
                .iter()
                .any(|diff| diff.contains("Actual blob digest mismatch")),
            "expected digest mismatch, got {:?}",
            diffs
        );
    }
}
