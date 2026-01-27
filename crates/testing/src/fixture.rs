//! Test fixture builders for deterministic test images.
//!
//! Provides utilities for creating reproducible container images with
//! known digests for testing purposes.

use std::collections::HashMap;
use std::path::Path;

use containerregistry_image::{
    Descriptor, Digest, ImageConfig, ImageIndex, Manifest, MediaType, OciIndex, OciManifest,
    Platform,
};
use containerregistry_layout::Layout;
use flate2::{Compression, GzBuilder};
use std::io::Write;

/// A test image with all its components.
#[derive(Clone, Debug)]
pub struct TestImage {
    /// The image manifest.
    pub manifest: Manifest,
    /// The config blob bytes.
    pub config_bytes: Vec<u8>,
    /// Layer blobs (digest -> bytes).
    pub layers: HashMap<Digest, Vec<u8>>,
    /// The manifest digest.
    pub digest: Digest,
}

impl TestImage {
    /// Creates a minimal test image with a single empty layer.
    pub fn minimal() -> Self {
        FixtureBuilder::new()
            .with_layer(b"minimal layer content")
            .build()
    }

    /// Writes this image to an OCI layout directory.
    pub fn write_to_layout(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let layout = Layout::create(path)?;

        // Write config blob
        layout.write_blob_with_digest(&self.config_bytes, &self.manifest.config().digest)?;

        // Write layer blobs
        for (digest, layer_bytes) in &self.layers {
            layout.write_blob_with_digest(layer_bytes, digest)?;
        }

        // Add manifest to index
        layout.add_manifest(&self.manifest)?;

        Ok(())
    }
}

/// Builder for creating deterministic test images.
pub struct FixtureBuilder {
    layers: Vec<Vec<u8>>,
    platform: Option<Platform>,
    compression: LayerCompression,
}

impl Default for FixtureBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FixtureBuilder {
    /// Creates a new fixture builder.
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            platform: None,
            compression: LayerCompression::Gzip,
        }
    }

    /// Adds a layer with the given content.
    pub fn with_layer(mut self, content: &[u8]) -> Self {
        self.layers.push(content.to_vec());
        self
    }

    /// Sets the platform for the image.
    pub fn with_platform(mut self, os: &str, arch: &str) -> Self {
        self.platform = Some(Platform::new(arch, os));
        self
    }

    /// Sets the layer compression for the image.
    pub fn with_compression(mut self, compression: LayerCompression) -> Self {
        self.compression = compression;
        self
    }

    /// Builds the test image.
    pub fn build(self) -> TestImage {
        // Create layer tarballs and descriptors
        let mut layer_map = HashMap::new();
        let mut layer_descriptors = Vec::new();
        let mut diff_ids = Vec::new();

        for layer_content in &self.layers {
            // Create a minimal tar archive containing the layer content
            let tar_bytes = create_layer_tar(layer_content);
            let diff_id = Digest::sha256(&tar_bytes);
            let (media_type, blob_bytes) = compress_layer(&self.compression, &tar_bytes);
            let digest = Digest::sha256(&blob_bytes);
            let size = blob_bytes.len() as u64;

            layer_descriptors.push(Descriptor::new(media_type, digest.clone(), size));

            diff_ids.push(diff_id);
            layer_map.insert(digest, blob_bytes);
        }

        // Create config
        let platform = self
            .platform
            .unwrap_or_else(|| Platform::new("amd64", "linux"));
        let mut config = ImageConfig::new(platform.architecture.clone(), platform.os.clone());

        for diff_id in diff_ids {
            config = config.with_layer(diff_id);
        }

        let config_bytes = config.to_bytes().expect("config serialization failed");
        let config_digest = Digest::sha256(&config_bytes);
        let config_size = config_bytes.len() as u64;

        let config_descriptor = Descriptor::new(MediaType::OciConfig, config_digest, config_size);

        // Create manifest
        let oci_manifest = OciManifest::new(config_descriptor, layer_descriptors);
        let manifest = Manifest::Oci(oci_manifest);
        let manifest_bytes = manifest.to_bytes().expect("manifest serialization failed");
        let digest = Digest::sha256(&manifest_bytes);

        TestImage {
            manifest,
            config_bytes,
            layers: layer_map,
            digest,
        }
    }
}

/// Compression strategy for fixture layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerCompression {
    None,
    Gzip,
    Zstd,
}

fn compress_layer(compression: &LayerCompression, tar_bytes: &[u8]) -> (MediaType, Vec<u8>) {
    match compression {
        LayerCompression::None => (MediaType::OciLayer, tar_bytes.to_vec()),
        LayerCompression::Gzip => {
            let mut encoder = GzBuilder::new()
                .mtime(0)
                .write(Vec::new(), Compression::default());
            encoder
                .write_all(tar_bytes)
                .expect("gzip compression failed");
            let data = encoder.finish().expect("gzip finish failed");
            (MediaType::OciLayerGzip, data)
        }
        LayerCompression::Zstd => {
            let data = zstd::stream::encode_all(tar_bytes, 0).expect("zstd compression failed");
            (MediaType::OciLayerZstd, data)
        }
    }
}

/// Creates a minimal tar archive containing a single file with the given content.
fn create_layer_tar(content: &[u8]) -> Vec<u8> {
    let mut tar_data = Vec::new();

    // TAR header (512 bytes)
    let mut header = [0u8; 512];

    // File name (100 bytes)
    let name = b"data";
    header[..name.len()].copy_from_slice(name);

    // File mode (8 bytes, octal)
    header[100..108].copy_from_slice(b"0000644\0");

    // Owner UID (8 bytes, octal)
    header[108..116].copy_from_slice(b"0000000\0");

    // Owner GID (8 bytes, octal)
    header[116..124].copy_from_slice(b"0000000\0");

    // File size (12 bytes, octal)
    let size_str = format!("{:011o}\0", content.len());
    header[124..136].copy_from_slice(size_str.as_bytes());

    // Modification time (12 bytes, octal) - fixed for reproducibility
    header[136..148].copy_from_slice(b"00000000000\0");

    // Type flag (1 byte) - regular file
    header[156] = b'0';

    // Magic (6 bytes)
    header[257..263].copy_from_slice(b"ustar\0");

    // Version (2 bytes)
    header[263..265].copy_from_slice(b"00");

    // Calculate checksum
    let checksum: u32 = header.iter().map(|&b| b as u32).sum::<u32>() + 8 * 32; // 8 spaces for checksum field
    let checksum_str = format!("{:06o}\0 ", checksum);
    header[148..156].copy_from_slice(checksum_str.as_bytes());

    tar_data.extend_from_slice(&header);
    tar_data.extend_from_slice(content);

    // Pad to 512-byte boundary
    let padding = (512 - (content.len() % 512)) % 512;
    tar_data.extend(std::iter::repeat_n(0u8, padding));

    // End-of-archive markers (two 512-byte blocks of zeros)
    tar_data.extend(std::iter::repeat_n(0u8, 1024));

    tar_data
}

/// Creates a test image index containing multiple platforms.
pub fn create_test_index(
    images: &[(Platform, TestImage)],
) -> (ImageIndex, HashMap<Digest, Vec<u8>>) {
    let mut manifests = Vec::new();
    let mut all_blobs = HashMap::new();

    for (platform, image) in images {
        let manifest_bytes = image.manifest.to_bytes().expect("manifest serialization");
        let digest = Digest::sha256(&manifest_bytes);
        let size = manifest_bytes.len() as u64;

        let desc = Descriptor::new(MediaType::OciManifest, digest.clone(), size)
            .with_platform(platform.clone());

        manifests.push(desc);

        // Collect all blobs
        all_blobs.insert(digest, manifest_bytes);
        all_blobs.insert(
            image.manifest.config().digest.clone(),
            image.config_bytes.clone(),
        );
        for (digest, bytes) in &image.layers {
            all_blobs.insert(digest.clone(), bytes.clone());
        }
    }

    let oci_index = OciIndex::new(manifests);
    let index = ImageIndex::Oci(oci_index);
    (index, all_blobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimal_image() {
        let image = TestImage::minimal();
        assert!(!image.layers.is_empty());
        assert!(!image.config_bytes.is_empty());
    }

    #[test]
    fn test_fixture_builder() {
        let image = FixtureBuilder::new()
            .with_layer(b"layer 1")
            .with_layer(b"layer 2")
            .with_platform("linux", "arm64")
            .with_compression(LayerCompression::None)
            .build();

        assert_eq!(image.layers.len(), 2);
        assert_eq!(image.manifest.layers().len(), 2);
    }

    #[test]
    fn test_deterministic_digest() {
        // Building the same image twice should produce the same digest
        let image1 = FixtureBuilder::new().with_layer(b"test content").build();

        let image2 = FixtureBuilder::new().with_layer(b"test content").build();

        assert_eq!(image1.digest, image2.digest);
    }

    #[test]
    fn test_create_index() {
        let linux_amd64 = FixtureBuilder::new()
            .with_layer(b"linux amd64")
            .with_platform("linux", "amd64")
            .build();

        let linux_arm64 = FixtureBuilder::new()
            .with_layer(b"linux arm64")
            .with_platform("linux", "arm64")
            .build();

        let (index, blobs) = create_test_index(&[
            (Platform::new("amd64", "linux"), linux_amd64),
            (Platform::new("arm64", "linux"), linux_arm64),
        ]);

        assert_eq!(index.manifests().len(), 2);
        assert!(!blobs.is_empty());
    }

    #[test]
    fn test_layer_compression_media_types() {
        let none = FixtureBuilder::new()
            .with_layer(b"layer")
            .with_compression(LayerCompression::None)
            .build();
        let gzip = FixtureBuilder::new()
            .with_layer(b"layer")
            .with_compression(LayerCompression::Gzip)
            .build();
        let zstd = FixtureBuilder::new()
            .with_layer(b"layer")
            .with_compression(LayerCompression::Zstd)
            .build();

        assert_eq!(none.manifest.layers()[0].media_type, MediaType::OciLayer);
        assert_eq!(
            gzip.manifest.layers()[0].media_type,
            MediaType::OciLayerGzip
        );
        assert_eq!(
            zstd.manifest.layers()[0].media_type,
            MediaType::OciLayerZstd
        );
    }

    #[test]
    fn test_diff_id_vs_blob_digest_for_gzip() {
        let image = FixtureBuilder::new()
            .with_layer(b"layer")
            .with_compression(LayerCompression::Gzip)
            .build();

        let manifest_layer = &image.manifest.layers()[0];
        let config = ImageConfig::from_bytes(&image.config_bytes).unwrap();
        let diff_id = config.rootfs.diff_ids[0].clone();

        let tar_bytes = create_layer_tar(b"layer");
        let expected_diff = Digest::sha256(&tar_bytes);
        assert_eq!(diff_id, expected_diff);

        let blob_bytes = image.layers.get(&manifest_layer.digest).unwrap();
        let expected_blob = Digest::sha256(blob_bytes);
        assert_eq!(manifest_layer.digest, expected_blob);
        assert_ne!(diff_id, manifest_layer.digest);
    }
}
