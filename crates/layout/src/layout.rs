//! OCI image layout read/write operations.
//!
//! This module provides the main `Layout` type for working with OCI image layouts
//! on the filesystem.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use containerregistry_image::{
    Descriptor, Digest, ImageConfig, ImageIndex, Manifest, MediaType, OciIndex,
};

use crate::oci_layout::OciLayout;
use crate::{Error, Result};

/// File name for the OCI layout version file.
const OCI_LAYOUT_FILE: &str = "oci-layout";

/// File name for the image index.
const INDEX_FILE: &str = "index.json";

/// Directory name for blobs.
const BLOBS_DIR: &str = "blobs";

/// An OCI image layout on disk.
///
/// The layout follows the OCI Image Layout Specification:
/// - `oci-layout` file containing the layout version
/// - `index.json` containing the image index
/// - `blobs/<algorithm>/<hex>` containing content-addressable blobs
#[derive(Debug)]
pub struct Layout {
    /// Root directory of the layout.
    root: PathBuf,
}

impl Layout {
    /// Opens an existing layout at the given path.
    ///
    /// This validates that the required files exist and the version is supported.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();

        // Verify oci-layout file exists and is valid
        let oci_layout_path = root.join(OCI_LAYOUT_FILE);
        if !oci_layout_path.exists() {
            return Err(Error::MissingFile(OCI_LAYOUT_FILE.to_string()));
        }
        let oci_layout_data = fs::read(&oci_layout_path)?;
        OciLayout::from_bytes(&oci_layout_data)?;

        // Verify index.json exists
        let index_path = root.join(INDEX_FILE);
        if !index_path.exists() {
            return Err(Error::MissingFile(INDEX_FILE.to_string()));
        }

        Ok(Self { root })
    }

    /// Creates a new empty layout at the given path.
    ///
    /// This creates the directory structure and required files.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();

        // Create directory structure
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join(BLOBS_DIR))?;

        // Write oci-layout file
        let oci_layout = OciLayout::new();
        let oci_layout_path = root.join(OCI_LAYOUT_FILE);
        fs::write(&oci_layout_path, oci_layout.to_bytes()?)?;

        // Write empty index.json
        let index = OciIndex::new(vec![]);
        let index_path = root.join(INDEX_FILE);
        fs::write(&index_path, index.to_bytes()?)?;

        Ok(Self { root })
    }

    /// Returns the root path of the layout.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Reads the image index from the layout.
    pub fn index(&self) -> Result<ImageIndex> {
        let index_path = self.root.join(INDEX_FILE);
        let data = fs::read(&index_path)?;
        ImageIndex::from_bytes(&data).map_err(Error::from)
    }

    /// Reads the OCI index from the layout.
    ///
    /// This returns the raw OCI index type rather than the unified ImageIndex.
    pub fn oci_index(&self) -> Result<OciIndex> {
        let index_path = self.root.join(INDEX_FILE);
        let data = fs::read(&index_path)?;
        OciIndex::from_bytes(&data).map_err(Error::from)
    }

    /// Writes the image index to the layout.
    pub fn write_index(&self, index: &OciIndex) -> Result<()> {
        let index_path = self.root.join(INDEX_FILE);
        let data = index.to_bytes()?;
        fs::write(&index_path, data)?;
        Ok(())
    }

    /// Returns the path to a blob given its digest.
    pub fn blob_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join(BLOBS_DIR)
            .join(digest.algorithm().to_string())
            .join(digest.hex())
    }

    /// Checks if a blob exists in the layout.
    pub fn has_blob(&self, digest: &Digest) -> bool {
        self.blob_path(digest).exists()
    }

    /// Reads a blob from the layout.
    pub fn read_blob(&self, digest: &Digest) -> Result<Vec<u8>> {
        let path = self.blob_path(digest);
        if !path.exists() {
            return Err(Error::BlobNotFound(digest.to_string()));
        }
        Ok(fs::read(&path)?)
    }

    /// Reads a blob as a stream.
    pub fn read_blob_stream(&self, digest: &Digest) -> Result<impl Read> {
        let path = self.blob_path(digest);
        if !path.exists() {
            return Err(Error::BlobNotFound(digest.to_string()));
        }
        Ok(fs::File::open(&path)?)
    }

    /// Writes a blob to the layout.
    ///
    /// The blob is stored at `blobs/<algorithm>/<hex>`.
    /// Returns the digest of the written blob.
    pub fn write_blob(&self, data: &[u8]) -> Result<Digest> {
        let digest = Digest::sha256(data);
        self.write_blob_with_digest(data, &digest)?;
        Ok(digest)
    }

    /// Writes a blob with a known digest.
    ///
    /// This is useful when copying blobs from another source where the digest
    /// is already known.
    pub fn write_blob_with_digest(&self, data: &[u8], digest: &Digest) -> Result<()> {
        let path = self.blob_path(digest);

        // Create algorithm directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write blob
        let mut file = fs::File::create(&path)?;
        file.write_all(data)?;

        Ok(())
    }

    /// Reads a manifest from the layout by its digest.
    pub fn read_manifest(&self, digest: &Digest) -> Result<Manifest> {
        let data = self.read_blob(digest)?;
        Manifest::from_bytes(&data).map_err(Error::from)
    }

    /// Reads an image config from the layout by its digest.
    pub fn read_config(&self, digest: &Digest) -> Result<ImageConfig> {
        let data = self.read_blob(digest)?;
        ImageConfig::from_bytes(&data).map_err(Error::from)
    }

    /// Writes a manifest to the layout and returns its descriptor.
    pub fn write_manifest(&self, manifest: &Manifest) -> Result<Descriptor> {
        let data = manifest.to_bytes()?;
        let digest = Digest::sha256(&data);
        let size = data.len() as u64;

        self.write_blob_with_digest(&data, &digest)?;

        Ok(Descriptor::new(manifest.media_type(), digest, size))
    }

    /// Writes an image config to the layout and returns its descriptor.
    pub fn write_config(&self, config: &ImageConfig) -> Result<Descriptor> {
        let data = config.to_bytes()?;
        let digest = Digest::sha256(&data);
        let size = data.len() as u64;

        self.write_blob_with_digest(&data, &digest)?;

        Ok(Descriptor::new(MediaType::OciConfig, digest, size))
    }

    /// Adds a manifest to the index with optional annotations.
    ///
    /// This writes the manifest blob and updates the index to reference it.
    pub fn add_manifest(&self, manifest: &Manifest) -> Result<Descriptor> {
        let descriptor = self.write_manifest(manifest)?;

        // Read current index, add descriptor, write back
        let mut index = self.oci_index()?;
        index.manifests.push(descriptor.clone());
        self.write_index(&index)?;

        Ok(descriptor)
    }

    /// Validates that all blobs referenced by the index exist.
    pub fn validate(&self) -> Result<()> {
        let index = self.oci_index()?;

        for manifest_desc in &index.manifests {
            // Check manifest blob exists
            if !self.has_blob(&manifest_desc.digest) {
                return Err(Error::BlobNotFound(manifest_desc.digest.to_string()));
            }

            // Parse manifest and check its referenced blobs
            let manifest = self.read_manifest(&manifest_desc.digest)?;
            match &manifest {
                Manifest::Oci(m) => {
                    // Check config blob
                    if !self.has_blob(&m.config.digest) {
                        return Err(Error::BlobNotFound(m.config.digest.to_string()));
                    }

                    // Check layer blobs
                    for layer in &m.layers {
                        if !self.has_blob(&layer.digest) {
                            return Err(Error::BlobNotFound(layer.digest.to_string()));
                        }
                    }
                }
                Manifest::Docker(m) => {
                    // Check config blob
                    if !self.has_blob(&m.config.digest) {
                        return Err(Error::BlobNotFound(m.config.digest.to_string()));
                    }

                    // Check layer blobs
                    for layer in &m.layers {
                        if !self.has_blob(&layer.digest) {
                            return Err(Error::BlobNotFound(layer.digest.to_string()));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Validates blob integrity by verifying digests match content.
    pub fn validate_blob(&self, digest: &Digest) -> Result<bool> {
        let data = self.read_blob(digest)?;
        let computed = Digest::compute(digest.algorithm(), &data);
        Ok(computed == *digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use containerregistry_image::OciManifest;
    use tempfile::TempDir;

    fn create_test_config() -> ImageConfig {
        ImageConfig::new("amd64", "linux")
    }

    fn create_test_manifest(config_desc: Descriptor, layer_desc: Descriptor) -> OciManifest {
        OciManifest::new(config_desc, vec![layer_desc])
    }

    #[test]
    fn test_layout_create() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        assert!(dir.path().join("oci-layout").exists());
        assert!(dir.path().join("index.json").exists());
        assert!(dir.path().join("blobs").exists());

        let index = layout.oci_index().unwrap();
        assert_eq!(index.schema_version, 2);
        assert!(index.manifests.is_empty());
    }

    #[test]
    fn test_layout_open() {
        let dir = TempDir::new().unwrap();
        Layout::create(dir.path()).unwrap();

        let layout = Layout::open(dir.path()).unwrap();
        assert_eq!(layout.path(), dir.path());
    }

    #[test]
    fn test_layout_open_missing_oci_layout() {
        let dir = TempDir::new().unwrap();
        let result = Layout::open(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_layout_write_read_blob() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        let data = b"test blob content";
        let digest = layout.write_blob(data).unwrap();

        assert!(layout.has_blob(&digest));

        let read_data = layout.read_blob(&digest).unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_layout_blob_path() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        let digest: Digest =
            "sha256:abc123def456789012345678901234567890123456789012345678901234abcd"
                .parse()
                .unwrap();

        let path = layout.blob_path(&digest);
        assert!(path.ends_with(
            "blobs/sha256/abc123def456789012345678901234567890123456789012345678901234abcd"
        ));
    }

    #[test]
    fn test_layout_read_missing_blob() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        let digest: Digest =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap();

        let result = layout.read_blob(&digest);
        assert!(matches!(result, Err(Error::BlobNotFound(_))));
    }

    #[test]
    fn test_layout_write_read_config() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        let config = create_test_config();
        let descriptor = layout.write_config(&config).unwrap();

        assert!(layout.has_blob(&descriptor.digest));

        let read_config = layout.read_config(&descriptor.digest).unwrap();
        assert_eq!(read_config.architecture, config.architecture);
        assert_eq!(read_config.os, config.os);
    }

    #[test]
    fn test_layout_roundtrip() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        // Create a complete image
        let config = create_test_config();
        let config_desc = layout.write_config(&config).unwrap();

        let layer_data = b"fake layer content";
        let layer_digest = layout.write_blob(layer_data).unwrap();
        let layer_desc = Descriptor::new(
            MediaType::OciLayerGzip,
            layer_digest,
            layer_data.len() as u64,
        );

        let manifest = create_test_manifest(config_desc.clone(), layer_desc.clone());
        let manifest_desc = layout.add_manifest(&Manifest::Oci(manifest)).unwrap();

        // Verify layout structure
        let index = layout.oci_index().unwrap();
        assert_eq!(index.manifests.len(), 1);
        assert_eq!(index.manifests[0].digest, manifest_desc.digest);

        // Re-open and verify
        let layout2 = Layout::open(dir.path()).unwrap();
        let index2 = layout2.oci_index().unwrap();
        assert_eq!(index2.manifests.len(), 1);

        // Read back manifest
        let read_manifest = layout2.read_manifest(&manifest_desc.digest).unwrap();
        match read_manifest {
            Manifest::Oci(m) => {
                assert_eq!(m.config.digest, config_desc.digest);
                assert_eq!(m.layers.len(), 1);
                assert_eq!(m.layers[0].digest, layer_desc.digest);
            }
            _ => panic!("expected OCI manifest"),
        }
    }

    #[test]
    fn test_layout_validate_complete() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        // Create complete image
        let config = create_test_config();
        let config_desc = layout.write_config(&config).unwrap();

        let layer_data = b"layer content";
        let layer_digest = layout.write_blob(layer_data).unwrap();
        let layer_desc = Descriptor::new(
            MediaType::OciLayerGzip,
            layer_digest,
            layer_data.len() as u64,
        );

        let manifest = create_test_manifest(config_desc, layer_desc);
        layout.add_manifest(&Manifest::Oci(manifest)).unwrap();

        // Validation should pass
        layout.validate().unwrap();
    }

    #[test]
    fn test_layout_validate_missing_layer() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        // Create config
        let config = create_test_config();
        let config_desc = layout.write_config(&config).unwrap();

        // Create manifest referencing non-existent layer
        let fake_digest: Digest =
            "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                .parse()
                .unwrap();
        let layer_desc = Descriptor::new(MediaType::OciLayerGzip, fake_digest, 100);

        let manifest = create_test_manifest(config_desc, layer_desc);
        layout.add_manifest(&Manifest::Oci(manifest)).unwrap();

        // Validation should fail
        let result = layout.validate();
        assert!(matches!(result, Err(Error::BlobNotFound(_))));
    }

    #[test]
    fn test_layout_validate_blob_integrity() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();

        let data = b"test content";
        let digest = layout.write_blob(data).unwrap();

        // Blob should be valid
        assert!(layout.validate_blob(&digest).unwrap());
    }

    #[test]
    fn test_layout_blob_path_multi_algo() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();
        let digest_384 = Digest::sha384(b"sha384");
        let digest_512 = Digest::sha512(b"sha512");

        let path_384 = layout.blob_path(&digest_384);
        let path_512 = layout.blob_path(&digest_512);

        assert!(path_384.to_string_lossy().contains("sha384"));
        assert!(path_512.to_string_lossy().contains("sha512"));
    }

    #[test]
    fn test_layout_validate_blob_multi_algo() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();
        let data_384 = b"blob-384";
        let data_512 = b"blob-512";
        let digest_384 = Digest::sha384(data_384);
        let digest_512 = Digest::sha512(data_512);

        layout
            .write_blob_with_digest(data_384, &digest_384)
            .unwrap();
        layout
            .write_blob_with_digest(data_512, &digest_512)
            .unwrap();

        assert!(layout.validate_blob(&digest_384).unwrap());
        assert!(layout.validate_blob(&digest_512).unwrap());
    }

    #[test]
    fn test_layout_validate_blob_detects_mismatch() {
        let dir = TempDir::new().unwrap();
        let layout = Layout::create(dir.path()).unwrap();
        let digest = Digest::sha256(b"expected");

        layout.write_blob_with_digest(b"expected", &digest).unwrap();

        // Corrupt the blob content.
        let path = layout.blob_path(&digest);
        std::fs::write(path, b"actual").unwrap();

        let valid = layout.validate_blob(&digest).unwrap();
        assert!(!valid);
    }
}
