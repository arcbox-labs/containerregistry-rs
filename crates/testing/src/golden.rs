//! Golden comparison helpers for testing.
//!
//! Provides utilities for comparing manifests, digests, and other
//! container registry artifacts against expected values.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use containerregistry_image::{Digest, ImageIndex, Manifest};

/// A comparator for golden test data.
pub struct GoldenComparator {
    /// Path to the golden data directory.
    golden_dir: std::path::PathBuf,
    /// Whether to update golden files on mismatch.
    update_mode: bool,
}

impl GoldenComparator {
    /// Creates a new golden comparator.
    pub fn new(golden_dir: impl AsRef<Path>) -> Self {
        let update_mode = std::env::var("UPDATE_GOLDEN").is_ok();
        Self {
            golden_dir: golden_dir.as_ref().to_path_buf(),
            update_mode,
        }
    }

    /// Creates a comparator in update mode (writes expected values).
    pub fn with_update_mode(golden_dir: impl AsRef<Path>) -> Self {
        Self {
            golden_dir: golden_dir.as_ref().to_path_buf(),
            update_mode: true,
        }
    }

    /// Compares a manifest against a golden file.
    pub fn compare_manifest(&self, name: &str, manifest: &Manifest) -> Result<(), String> {
        let golden_path = self.golden_dir.join(format!("{}.manifest.json", name));
        let actual_bytes = manifest.to_bytes().map_err(|e| e.to_string())?;
        let actual = String::from_utf8(actual_bytes).map_err(|e| e.to_string())?;

        self.compare_json(&golden_path, &actual)
    }

    /// Compares an index against a golden file.
    pub fn compare_index(&self, name: &str, index: &ImageIndex) -> Result<(), String> {
        let golden_path = self.golden_dir.join(format!("{}.index.json", name));
        let actual_bytes = index.to_bytes().map_err(|e| e.to_string())?;
        let actual = String::from_utf8(actual_bytes).map_err(|e| e.to_string())?;

        self.compare_json(&golden_path, &actual)
    }

    /// Compares a digest against a golden file.
    pub fn compare_digest(&self, name: &str, digest: &Digest) -> Result<(), String> {
        let golden_path = self.golden_dir.join(format!("{}.digest", name));
        let actual = digest.to_string();

        self.compare_text(&golden_path, &actual)
    }

    /// Compares arbitrary JSON against a golden file.
    fn compare_json(&self, golden_path: &Path, actual: &str) -> Result<(), String> {
        if self.update_mode {
            // Ensure parent directory exists
            if let Some(parent) = golden_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(golden_path, actual).map_err(|e| e.to_string())?;
            return Ok(());
        }

        if !golden_path.exists() {
            return Err(format!(
                "Golden file not found: {}. Run with UPDATE_GOLDEN=1 to create.",
                golden_path.display()
            ));
        }

        let expected = fs::read_to_string(golden_path).map_err(|e| e.to_string())?;

        // Parse both as JSON and compare (ignores whitespace differences)
        let actual_json: serde_json::Value =
            serde_json::from_str(actual).map_err(|e| format!("actual is not valid JSON: {}", e))?;
        let expected_json: serde_json::Value = serde_json::from_str(&expected)
            .map_err(|e| format!("expected is not valid JSON: {}", e))?;

        if actual_json != expected_json {
            Err(format!(
                "Manifest mismatch for {}:\n  Expected: {}\n  Actual: {}",
                golden_path.display(),
                expected.trim(),
                actual.trim()
            ))
        } else {
            Ok(())
        }
    }

    /// Compares text against a golden file.
    fn compare_text(&self, golden_path: &Path, actual: &str) -> Result<(), String> {
        if self.update_mode {
            if let Some(parent) = golden_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(golden_path, actual).map_err(|e| e.to_string())?;
            return Ok(());
        }

        if !golden_path.exists() {
            return Err(format!(
                "Golden file not found: {}. Run with UPDATE_GOLDEN=1 to create.",
                golden_path.display()
            ));
        }

        let expected = fs::read_to_string(golden_path)
            .map_err(|e| e.to_string())?
            .trim()
            .to_string();

        if actual.trim() != expected {
            Err(format!(
                "Text mismatch for {}:\n  Expected: {}\n  Actual: {}",
                golden_path.display(),
                expected,
                actual.trim()
            ))
        } else {
            Ok(())
        }
    }
}

/// Asserts that two digests are equal with a descriptive error message.
pub fn assert_digest_eq(expected: &Digest, actual: &Digest) {
    assert_eq!(
        expected, actual,
        "Digest mismatch:\n  Expected: {}\n  Actual: {}",
        expected, actual
    );
}

/// Asserts that two manifests are byte-equivalent.
pub fn assert_manifest_eq(expected: &Manifest, actual: &Manifest) {
    let expected_bytes = expected.to_bytes().expect("expected manifest serialization");
    let actual_bytes = actual.to_bytes().expect("actual manifest serialization");

    assert_eq!(
        expected_bytes, actual_bytes,
        "Manifest mismatch:\n  Expected digest: {}\n  Actual digest: {}",
        Digest::sha256(&expected_bytes),
        Digest::sha256(&actual_bytes)
    );
}

/// Compares two blob maps for equality.
pub fn compare_blobs(
    expected: &HashMap<Digest, Vec<u8>>,
    actual: &HashMap<Digest, Vec<u8>>,
) -> Result<(), String> {
    // Check for missing blobs
    for digest in expected.keys() {
        if !actual.contains_key(digest) {
            return Err(format!("Missing blob: {}", digest));
        }
    }

    // Check for extra blobs
    for digest in actual.keys() {
        if !expected.contains_key(digest) {
            return Err(format!("Extra blob: {}", digest));
        }
    }

    // Check blob contents
    for (digest, expected_bytes) in expected {
        let actual_bytes = &actual[digest];
        if expected_bytes != actual_bytes {
            return Err(format!(
                "Blob content mismatch for {}: expected {} bytes, got {} bytes",
                digest,
                expected_bytes.len(),
                actual_bytes.len()
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assert_digest_eq() {
        let d1 = Digest::sha256(b"test");
        let d2 = Digest::sha256(b"test");
        assert_digest_eq(&d1, &d2);
    }

    #[test]
    fn test_compare_blobs_equal() {
        let mut map1 = HashMap::new();
        let mut map2 = HashMap::new();

        let digest = Digest::sha256(b"content");
        map1.insert(digest.clone(), b"content".to_vec());
        map2.insert(digest, b"content".to_vec());

        assert!(compare_blobs(&map1, &map2).is_ok());
    }

    #[test]
    fn test_compare_blobs_missing() {
        let mut map1 = HashMap::new();
        let map2 = HashMap::new();

        let digest = Digest::sha256(b"content");
        map1.insert(digest, b"content".to_vec());

        let result = compare_blobs(&map1, &map2);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing blob"));
    }
}
