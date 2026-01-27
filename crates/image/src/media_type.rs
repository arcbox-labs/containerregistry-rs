//! OCI and Docker media type constants and utilities.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Known media types for OCI and Docker images.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MediaType {
    // OCI types
    /// OCI image manifest.
    OciManifest,
    /// OCI image index (multi-arch).
    OciIndex,
    /// OCI image config.
    OciConfig,
    /// OCI layer (gzip compressed).
    OciLayerGzip,
    /// OCI layer (uncompressed).
    OciLayer,
    /// OCI layer (zstd compressed).
    OciLayerZstd,
    /// OCI non-distributable layer (uncompressed, deprecated).
    OciLayerNondistributable,
    /// OCI non-distributable layer (gzip, deprecated).
    OciLayerNondistributableGzip,
    /// OCI non-distributable layer (zstd, deprecated).
    OciLayerNondistributableZstd,
    /// OCI empty/scratch config descriptor.
    OciEmptyJson,

    // Docker types
    /// Docker v2 schema 2 manifest.
    DockerManifest,
    /// Docker manifest list.
    DockerManifestList,
    /// Docker container config.
    DockerConfig,
    /// Docker layer (gzip compressed).
    DockerLayerGzip,
    /// Docker foreign/non-distributable layer.
    DockerForeignLayer,

    /// Unknown or custom media type.
    Other(String),
}

impl MediaType {
    /// Returns the string representation of the media type.
    pub fn as_str(&self) -> &str {
        match self {
            MediaType::OciManifest => "application/vnd.oci.image.manifest.v1+json",
            MediaType::OciIndex => "application/vnd.oci.image.index.v1+json",
            MediaType::OciConfig => "application/vnd.oci.image.config.v1+json",
            MediaType::OciLayerGzip => "application/vnd.oci.image.layer.v1.tar+gzip",
            MediaType::OciLayer => "application/vnd.oci.image.layer.v1.tar",
            MediaType::OciLayerZstd => "application/vnd.oci.image.layer.v1.tar+zstd",
            MediaType::OciLayerNondistributable => {
                "application/vnd.oci.image.layer.nondistributable.v1.tar"
            }
            MediaType::OciLayerNondistributableGzip => {
                "application/vnd.oci.image.layer.nondistributable.v1.tar+gzip"
            }
            MediaType::OciLayerNondistributableZstd => {
                "application/vnd.oci.image.layer.nondistributable.v1.tar+zstd"
            }
            MediaType::OciEmptyJson => "application/vnd.oci.empty.v1+json",
            MediaType::DockerManifest => "application/vnd.docker.distribution.manifest.v2+json",
            MediaType::DockerManifestList => {
                "application/vnd.docker.distribution.manifest.list.v2+json"
            }
            MediaType::DockerConfig => "application/vnd.docker.container.image.v1+json",
            MediaType::DockerLayerGzip => "application/vnd.docker.image.rootfs.diff.tar.gzip",
            MediaType::DockerForeignLayer => {
                "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip"
            }
            MediaType::Other(s) => s,
        }
    }

    /// Returns true if this is a manifest type (single image).
    pub fn is_manifest(&self) -> bool {
        matches!(self, MediaType::OciManifest | MediaType::DockerManifest)
    }

    /// Returns true if this is an index/manifest list type (multi-arch).
    pub fn is_index(&self) -> bool {
        matches!(self, MediaType::OciIndex | MediaType::DockerManifestList)
    }

    /// Returns true if this is a config type.
    pub fn is_config(&self) -> bool {
        matches!(
            self,
            MediaType::OciConfig | MediaType::DockerConfig | MediaType::OciEmptyJson
        )
    }

    /// Returns true if this is a layer type (distributable or not).
    pub fn is_layer(&self) -> bool {
        matches!(
            self,
            MediaType::OciLayerGzip
                | MediaType::OciLayer
                | MediaType::OciLayerZstd
                | MediaType::OciLayerNondistributable
                | MediaType::OciLayerNondistributableGzip
                | MediaType::OciLayerNondistributableZstd
                | MediaType::DockerLayerGzip
                | MediaType::DockerForeignLayer
        )
    }

    /// Returns true if this is a non-distributable/foreign layer type.
    pub fn is_nondistributable(&self) -> bool {
        matches!(
            self,
            MediaType::OciLayerNondistributable
                | MediaType::OciLayerNondistributableGzip
                | MediaType::OciLayerNondistributableZstd
                | MediaType::DockerForeignLayer
        )
    }

    /// Returns the OCI equivalent of this media type, if applicable.
    pub fn to_oci(&self) -> MediaType {
        match self {
            MediaType::DockerManifest => MediaType::OciManifest,
            MediaType::DockerManifestList => MediaType::OciIndex,
            MediaType::DockerConfig => MediaType::OciConfig,
            MediaType::DockerLayerGzip => MediaType::OciLayerGzip,
            MediaType::DockerForeignLayer => MediaType::OciLayerNondistributableGzip,
            other => other.clone(),
        }
    }

    /// Returns the Docker equivalent of this media type, if applicable.
    ///
    /// Note: This only changes the media type string. It does NOT transform
    /// the actual content. For layers with incompatible compression formats
    /// (uncompressed, zstd), this will return the same media type unchanged
    /// since Docker does not support those formats.
    pub fn to_docker(&self) -> MediaType {
        match self {
            MediaType::OciManifest => MediaType::DockerManifest,
            MediaType::OciIndex => MediaType::DockerManifestList,
            MediaType::OciConfig | MediaType::OciEmptyJson => MediaType::DockerConfig,
            MediaType::OciLayerGzip => MediaType::DockerLayerGzip,
            // Non-distributable gzip can map to Docker foreign layer
            MediaType::OciLayerNondistributableGzip => MediaType::DockerForeignLayer,
            // These cannot be safely converted - return unchanged
            MediaType::OciLayer
            | MediaType::OciLayerZstd
            | MediaType::OciLayerNondistributable
            | MediaType::OciLayerNondistributableZstd => self.clone(),
            other => other.clone(),
        }
    }

    /// Returns true if this media type can be safely converted to Docker format.
    ///
    /// Returns false for OCI-specific compression formats that Docker doesn't support.
    pub fn is_docker_compatible(&self) -> bool {
        !matches!(
            self,
            MediaType::OciLayer
                | MediaType::OciLayerZstd
                | MediaType::OciLayerNondistributable
                | MediaType::OciLayerNondistributableZstd
        )
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for MediaType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "application/vnd.oci.image.manifest.v1+json" => MediaType::OciManifest,
            "application/vnd.oci.image.index.v1+json" => MediaType::OciIndex,
            "application/vnd.oci.image.config.v1+json" => MediaType::OciConfig,
            "application/vnd.oci.image.layer.v1.tar+gzip" => MediaType::OciLayerGzip,
            "application/vnd.oci.image.layer.v1.tar" => MediaType::OciLayer,
            "application/vnd.oci.image.layer.v1.tar+zstd" => MediaType::OciLayerZstd,
            "application/vnd.oci.image.layer.nondistributable.v1.tar" => {
                MediaType::OciLayerNondistributable
            }
            "application/vnd.oci.image.layer.nondistributable.v1.tar+gzip" => {
                MediaType::OciLayerNondistributableGzip
            }
            "application/vnd.oci.image.layer.nondistributable.v1.tar+zstd" => {
                MediaType::OciLayerNondistributableZstd
            }
            "application/vnd.oci.empty.v1+json" => MediaType::OciEmptyJson,
            "application/vnd.docker.distribution.manifest.v2+json" => MediaType::DockerManifest,
            "application/vnd.docker.distribution.manifest.list.v2+json" => {
                MediaType::DockerManifestList
            }
            "application/vnd.docker.container.image.v1+json" => MediaType::DockerConfig,
            "application/vnd.docker.image.rootfs.diff.tar.gzip" => MediaType::DockerLayerGzip,
            "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip" => {
                MediaType::DockerForeignLayer
            }
            other => MediaType::Other(other.to_string()),
        })
    }
}

impl Serialize for MediaType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MediaType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // FromStr is infallible for MediaType (unknown types become Other)
        Ok(s.parse().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_parse() {
        let mt: MediaType = "application/vnd.oci.image.manifest.v1+json"
            .parse()
            .unwrap();
        assert_eq!(mt, MediaType::OciManifest);
    }

    #[test]
    fn test_media_type_unknown() {
        let mt: MediaType = "application/custom".parse().unwrap();
        assert_eq!(mt, MediaType::Other("application/custom".to_string()));
    }

    #[test]
    fn test_media_type_to_oci() {
        assert_eq!(MediaType::DockerManifest.to_oci(), MediaType::OciManifest);
        assert_eq!(MediaType::OciManifest.to_oci(), MediaType::OciManifest);
    }

    #[test]
    fn test_media_type_to_docker_compatible() {
        // gzip layers can convert
        assert_eq!(
            MediaType::OciLayerGzip.to_docker(),
            MediaType::DockerLayerGzip
        );
        assert!(MediaType::OciLayerGzip.is_docker_compatible());
    }

    #[test]
    fn test_media_type_to_docker_incompatible() {
        // zstd and uncompressed cannot convert - return unchanged
        assert_eq!(MediaType::OciLayerZstd.to_docker(), MediaType::OciLayerZstd);
        assert_eq!(MediaType::OciLayer.to_docker(), MediaType::OciLayer);
        assert!(!MediaType::OciLayerZstd.is_docker_compatible());
        assert!(!MediaType::OciLayer.is_docker_compatible());
    }

    #[test]
    fn test_media_type_nondistributable() {
        assert!(MediaType::OciLayerNondistributableGzip.is_nondistributable());
        assert!(MediaType::DockerForeignLayer.is_nondistributable());
        assert!(!MediaType::OciLayerGzip.is_nondistributable());
    }

    #[test]
    fn test_media_type_is_layer() {
        assert!(MediaType::OciLayerGzip.is_layer());
        assert!(MediaType::OciLayerNondistributableGzip.is_layer());
        assert!(MediaType::DockerForeignLayer.is_layer());
        assert!(!MediaType::OciManifest.is_layer());
    }

    #[test]
    fn test_media_type_serde_roundtrip() {
        let mt = MediaType::OciManifest;
        let json = serde_json::to_string(&mt).unwrap();
        let parsed: MediaType = serde_json::from_str(&json).unwrap();
        assert_eq!(mt, parsed);
    }

    #[test]
    fn test_media_type_nondistributable_roundtrip() {
        let mt = MediaType::OciLayerNondistributableGzip;
        let json = serde_json::to_string(&mt).unwrap();
        let parsed: MediaType = serde_json::from_str(&json).unwrap();
        assert_eq!(mt, parsed);
    }
}
