//! Golden tests for OCI layout operations.

use containerregistry_layout::Layout;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/fixtures")
}

#[test]
fn test_open_minimal_layout() {
    let layout_path = fixtures_dir().join("layout/minimal");
    let layout = Layout::open(&layout_path).expect("failed to open layout");

    assert_eq!(layout.path(), layout_path);
}

#[test]
fn test_read_layout_index() {
    let layout_path = fixtures_dir().join("layout/minimal");
    let layout = Layout::open(&layout_path).expect("failed to open layout");

    let index = layout.oci_index().expect("failed to read index");

    assert_eq!(index.schema_version, 2);
    assert_eq!(index.manifests.len(), 1);

    let manifest_desc = &index.manifests[0];
    assert_eq!(
        manifest_desc.digest.to_string(),
        "sha256:d5592d54da863175e9fa84a93eaaa77539774b716ed974649aaf6c560a42cfa7"
    );
    assert_eq!(manifest_desc.size, 399);
}

#[test]
fn test_read_layout_manifest() {
    use containerregistry_image::{Manifest, MediaType};

    let layout_path = fixtures_dir().join("layout/minimal");
    let layout = Layout::open(&layout_path).expect("failed to open layout");

    let index = layout.oci_index().expect("failed to read index");
    let manifest_desc = &index.manifests[0];

    let manifest = layout
        .read_manifest(&manifest_desc.digest)
        .expect("failed to read manifest");

    match manifest {
        Manifest::Oci(m) => {
            assert_eq!(m.schema_version, 2);
            assert_eq!(m.media_type, Some(MediaType::OciManifest));
            assert_eq!(m.config.media_type, MediaType::OciConfig);
            assert_eq!(m.layers.len(), 1);
            assert_eq!(m.layers[0].media_type, MediaType::OciLayerGzip);
        }
        Manifest::Docker(_) => panic!("expected OCI manifest"),
    }
}

#[test]
fn test_read_layout_config() {
    let layout_path = fixtures_dir().join("layout/minimal");
    let layout = Layout::open(&layout_path).expect("failed to open layout");

    let index = layout.oci_index().expect("failed to read index");
    let manifest_desc = &index.manifests[0];
    let manifest = layout
        .read_manifest(&manifest_desc.digest)
        .expect("failed to read manifest");

    let config_digest = match &manifest {
        containerregistry_image::Manifest::Oci(m) => &m.config.digest,
        containerregistry_image::Manifest::Docker(m) => &m.config.digest,
    };

    let config = layout
        .read_config(config_digest)
        .expect("failed to read config");

    assert_eq!(config.architecture, "amd64");
    assert_eq!(config.os, "linux");
}

#[test]
fn test_read_layout_layer() {
    let layout_path = fixtures_dir().join("layout/minimal");
    let layout = Layout::open(&layout_path).expect("failed to open layout");

    let index = layout.oci_index().expect("failed to read index");
    let manifest_desc = &index.manifests[0];
    let manifest = layout
        .read_manifest(&manifest_desc.digest)
        .expect("failed to read manifest");

    let layer_digest = match &manifest {
        containerregistry_image::Manifest::Oci(m) => &m.layers[0].digest,
        containerregistry_image::Manifest::Docker(m) => &m.layers[0].digest,
    };

    let layer_data = layout
        .read_blob(layer_digest)
        .expect("failed to read layer");

    assert_eq!(layer_data, b"fake layer content for testing");
}

#[test]
fn test_validate_layout() {
    let layout_path = fixtures_dir().join("layout/minimal");
    let layout = Layout::open(&layout_path).expect("failed to open layout");

    // Layout should be valid - all referenced blobs exist
    layout.validate().expect("layout validation failed");
}

#[test]
fn test_layout_blob_integrity() {
    let layout_path = fixtures_dir().join("layout/minimal");
    let layout = Layout::open(&layout_path).expect("failed to open layout");

    let index = layout.oci_index().expect("failed to read index");
    let manifest_desc = &index.manifests[0];

    // All blobs should have correct digests
    assert!(
        layout
            .validate_blob(&manifest_desc.digest)
            .expect("failed to validate blob")
    );
}
