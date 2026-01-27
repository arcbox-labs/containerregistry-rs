//! OCI layout version file (`oci-layout`).
//!
//! The `oci-layout` file is a JSON object containing the image layout version.
//! Per the OCI Image Layout Specification, this file MUST be present at the root
//! of the layout directory.

use serde::{Deserialize, Serialize};

use crate::{Error, IMAGE_LAYOUT_VERSION, Result};

/// The `oci-layout` file content.
///
/// This file indicates that the directory conforms to the OCI Image Layout
/// Specification and specifies the version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciLayout {
    /// The image layout version. Currently only "1.0.0" is supported.
    #[serde(rename = "imageLayoutVersion")]
    pub image_layout_version: String,
}

impl OciLayout {
    /// Creates a new `oci-layout` with the current version.
    pub fn new() -> Self {
        Self {
            image_layout_version: IMAGE_LAYOUT_VERSION.to_string(),
        }
    }

    /// Parses an `oci-layout` file from JSON bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let layout: Self = serde_json::from_slice(data)?;
        layout.validate()?;
        Ok(layout)
    }

    /// Serializes the `oci-layout` to JSON bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(Error::from)
    }

    /// Validates the layout version.
    pub fn validate(&self) -> Result<()> {
        if self.image_layout_version != IMAGE_LAYOUT_VERSION {
            return Err(Error::InvalidLayout(format!(
                "unsupported imageLayoutVersion: expected {}, got {}",
                IMAGE_LAYOUT_VERSION, self.image_layout_version
            )));
        }
        Ok(())
    }
}

impl Default for OciLayout {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oci_layout_new() {
        let layout = OciLayout::new();
        assert_eq!(layout.image_layout_version, "1.0.0");
    }

    #[test]
    fn test_oci_layout_roundtrip() {
        let layout = OciLayout::new();
        let bytes = layout.to_bytes().unwrap();
        let parsed = OciLayout::from_bytes(&bytes).unwrap();
        assert_eq!(layout, parsed);
    }

    #[test]
    fn test_oci_layout_parse() {
        let json = br#"{"imageLayoutVersion":"1.0.0"}"#;
        let layout = OciLayout::from_bytes(json).unwrap();
        assert_eq!(layout.image_layout_version, "1.0.0");
    }

    #[test]
    fn test_oci_layout_invalid_version() {
        let json = br#"{"imageLayoutVersion":"2.0.0"}"#;
        let result = OciLayout::from_bytes(json);
        assert!(result.is_err());
    }
}
