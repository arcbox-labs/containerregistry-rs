//! Golden tests for manifest parsing and digest stability.

use containerregistry_image::{
    DockerManifest, DockerManifestList, ImageConfig, ImageIndex, Manifest, MediaType, OciIndex,
    OciManifest,
};
use std::fs;
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
fn test_parse_oci_manifest_fixture() {
    let path = fixtures_dir().join("oci/manifest.json");
    let data = fs::read(&path).expect("failed to read fixture");

    let manifest = OciManifest::from_bytes(&data).expect("failed to parse OCI manifest");

    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.media_type, Some(MediaType::OciManifest));
    assert_eq!(manifest.config.media_type, MediaType::OciConfig);
    assert_eq!(manifest.layers.len(), 1);
    assert_eq!(manifest.layers[0].media_type, MediaType::OciLayerGzip);
}

#[test]
fn test_parse_docker_manifest_fixture() {
    let path = fixtures_dir().join("docker/manifest.json");
    let data = fs::read(&path).expect("failed to read fixture");

    let manifest = DockerManifest::from_bytes(&data).expect("failed to parse Docker manifest");

    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.media_type, MediaType::DockerManifest);
    assert_eq!(manifest.config.media_type, MediaType::DockerConfig);
    assert_eq!(manifest.layers.len(), 1);
    assert_eq!(manifest.layers[0].media_type, MediaType::DockerLayerGzip);
}

#[test]
fn test_parse_oci_config_fixture() {
    let path = fixtures_dir().join("oci/config.json");
    let data = fs::read(&path).expect("failed to read fixture");

    let config = ImageConfig::from_bytes(&data).expect("failed to parse config");

    assert_eq!(config.architecture, "amd64");
    assert_eq!(config.os, "linux");
    assert_eq!(config.rootfs.fs_type, "layers");
    assert_eq!(config.rootfs.diff_ids.len(), 1);

    // Check container config
    let container_config = config.config.as_ref().expect("missing container config");
    assert_eq!(
        container_config.entrypoint,
        Some(vec!["/bin/sh".to_string()])
    );
    assert_eq!(container_config.working_dir, Some("/app".to_string()));
    assert_eq!(
        container_config
            .labels
            .get("org.opencontainers.image.title"),
        Some(&"test-image".to_string())
    );
}

#[test]
fn test_parse_oci_index_fixture() {
    let path = fixtures_dir().join("index/oci-index.json");
    let data = fs::read(&path).expect("failed to read fixture");

    let index = OciIndex::from_bytes(&data).expect("failed to parse OCI index");

    assert_eq!(index.schema_version, 2);
    assert_eq!(index.media_type, Some(MediaType::OciIndex));
    assert_eq!(index.manifests.len(), 2);

    // Check platform selection
    let amd64 = index.find_platform("amd64", "linux", None);
    assert!(amd64.is_some());
    assert_eq!(
        amd64.unwrap().platform.as_ref().unwrap().architecture,
        "amd64"
    );

    let arm64 = index.find_platform("arm64", "linux", None);
    assert!(arm64.is_some());
}

#[test]
fn test_parse_docker_manifest_list_fixture() {
    let path = fixtures_dir().join("index/docker-manifest-list.json");
    let data = fs::read(&path).expect("failed to read fixture");

    let list = DockerManifestList::from_bytes(&data).expect("failed to parse Docker manifest list");

    assert_eq!(list.schema_version, 2);
    assert_eq!(list.media_type, MediaType::DockerManifestList);
    assert_eq!(list.manifests.len(), 2);
}

#[test]
fn test_manifest_auto_detect() {
    // OCI manifest
    let oci_path = fixtures_dir().join("oci/manifest.json");
    let oci_data = fs::read(&oci_path).expect("failed to read fixture");
    let oci = Manifest::from_bytes(&oci_data).expect("failed to parse");
    assert!(matches!(oci, Manifest::Oci(_)));
    assert_eq!(oci.media_type(), MediaType::OciManifest);

    // Docker manifest
    let docker_path = fixtures_dir().join("docker/manifest.json");
    let docker_data = fs::read(&docker_path).expect("failed to read fixture");
    let docker = Manifest::from_bytes(&docker_data).expect("failed to parse");
    assert!(matches!(docker, Manifest::Docker(_)));
    assert_eq!(docker.media_type(), MediaType::DockerManifest);
}

#[test]
fn test_image_index_auto_detect() {
    // OCI index
    let oci_path = fixtures_dir().join("index/oci-index.json");
    let oci_data = fs::read(&oci_path).expect("failed to read fixture");
    let oci = ImageIndex::from_bytes(&oci_data).expect("failed to parse");
    assert!(matches!(oci, ImageIndex::Oci(_)));
    assert_eq!(oci.media_type(), MediaType::OciIndex);

    // Docker manifest list
    let docker_path = fixtures_dir().join("index/docker-manifest-list.json");
    let docker_data = fs::read(&docker_path).expect("failed to read fixture");
    let docker = ImageIndex::from_bytes(&docker_data).expect("failed to parse");
    assert!(matches!(docker, ImageIndex::Docker(_)));
    assert_eq!(docker.media_type(), MediaType::DockerManifestList);
}

#[test]
fn test_oci_manifest_digest_stability() {
    // Create a manifest and compute its digest multiple times
    let path = fixtures_dir().join("oci/manifest.json");
    let data = fs::read(&path).expect("failed to read fixture");
    let manifest = OciManifest::from_bytes(&data).expect("failed to parse");

    let digest1 = manifest.digest().expect("failed to compute digest");
    let digest2 = manifest.digest().expect("failed to compute digest");

    assert_eq!(digest1, digest2, "digest should be stable across calls");

    // Re-serialize and parse should produce same digest
    let bytes = manifest.to_bytes().expect("failed to serialize");
    let reparsed = OciManifest::from_bytes(&bytes).expect("failed to reparse");
    let digest3 = reparsed.digest().expect("failed to compute digest");

    // Note: The digest may differ from digest1 because we're comparing
    // the original file (with formatting) to the re-serialized version (compact).
    // What matters is that digest3 equals subsequent computations.
    let digest4 = reparsed.digest().expect("failed to compute digest");
    assert_eq!(digest3, digest4, "digest should be stable after roundtrip");
}

#[test]
fn test_oci_to_docker_conversion() {
    let path = fixtures_dir().join("oci/manifest.json");
    let data = fs::read(&path).expect("failed to read fixture");
    let oci = OciManifest::from_bytes(&data).expect("failed to parse");

    let docker = oci.try_to_docker().expect("failed to convert");

    assert_eq!(docker.media_type, MediaType::DockerManifest);
    assert_eq!(docker.config.media_type, MediaType::DockerConfig);
    assert_eq!(docker.layers[0].media_type, MediaType::DockerLayerGzip);
    assert_eq!(docker.config.digest, oci.config.digest);
    assert_eq!(docker.layers[0].digest, oci.layers[0].digest);
}

#[test]
fn test_docker_to_oci_conversion() {
    let path = fixtures_dir().join("docker/manifest.json");
    let data = fs::read(&path).expect("failed to read fixture");
    let docker = DockerManifest::from_bytes(&data).expect("failed to parse");

    let oci = docker.to_oci();

    assert_eq!(oci.media_type, Some(MediaType::OciManifest));
    assert_eq!(oci.config.media_type, MediaType::OciConfig);
    assert_eq!(oci.layers[0].media_type, MediaType::OciLayerGzip);
    assert_eq!(oci.config.digest, docker.config.digest);
    assert_eq!(oci.layers[0].digest, docker.layers[0].digest);
}

#[test]
fn test_index_to_docker_conversion() {
    let path = fixtures_dir().join("index/oci-index.json");
    let data = fs::read(&path).expect("failed to read fixture");
    let oci = OciIndex::from_bytes(&data).expect("failed to parse");

    let docker = oci.try_to_docker().expect("failed to convert");

    assert_eq!(docker.media_type, MediaType::DockerManifestList);
    assert_eq!(docker.manifests.len(), oci.manifests.len());
    for (d, o) in docker.manifests.iter().zip(oci.manifests.iter()) {
        assert_eq!(d.media_type, MediaType::DockerManifest);
        assert_eq!(d.digest, o.digest);
    }
}

#[test]
fn test_manifest_digest_matches_raw_sha256() {
    use containerregistry_image::Digest;

    // Parse and serialize the manifest
    let path = fixtures_dir().join("oci/manifest.json");
    let data = fs::read(&path).expect("failed to read fixture");
    let manifest = OciManifest::from_bytes(&data).expect("failed to parse");

    // Get digest via struct method
    let struct_digest = manifest.digest().expect("failed to compute digest");

    // Get digest by manually hashing serialized bytes
    let bytes = manifest.to_bytes().expect("failed to serialize");
    let raw_digest = Digest::sha256(&bytes);

    // They must match - this proves deterministic serialization
    assert_eq!(
        struct_digest, raw_digest,
        "struct digest must match raw SHA256 of serialized bytes"
    );
}

#[test]
fn test_index_digest_matches_raw_sha256() {
    use containerregistry_image::Digest;

    let path = fixtures_dir().join("index/oci-index.json");
    let data = fs::read(&path).expect("failed to read fixture");
    let index = OciIndex::from_bytes(&data).expect("failed to parse");

    let struct_digest = index.digest().expect("failed to compute digest");
    let bytes = index.to_bytes().expect("failed to serialize");
    let raw_digest = Digest::sha256(&bytes);

    assert_eq!(
        struct_digest, raw_digest,
        "index digest must match raw SHA256 of serialized bytes"
    );
}

#[test]
fn test_config_digest_matches_raw_sha256() {
    use containerregistry_image::Digest;

    let path = fixtures_dir().join("oci/config.json");
    let data = fs::read(&path).expect("failed to read fixture");
    let config = ImageConfig::from_bytes(&data).expect("failed to parse");

    let struct_digest = config.digest().expect("failed to compute digest");
    let bytes = config.to_bytes().expect("failed to serialize");
    let raw_digest = Digest::sha256(&bytes);

    assert_eq!(
        struct_digest, raw_digest,
        "config digest must match raw SHA256 of serialized bytes"
    );
}

#[test]
fn test_validation_oci_manifest_schema_version() {
    // Valid manifest with schemaVersion 2
    let valid = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":100},"layers":[]}"#;
    assert!(OciManifest::from_bytes(valid).is_ok());

    // Invalid schemaVersion 1
    let invalid = br#"{"schemaVersion":1,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":100},"layers":[]}"#;
    assert!(OciManifest::from_bytes(invalid).is_err());
}

#[test]
fn test_validation_oci_index_schema_version() {
    // Valid index with schemaVersion 2
    let valid = br#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[]}"#;
    assert!(OciIndex::from_bytes(valid).is_ok());

    // Invalid schemaVersion 1
    let invalid = br#"{"schemaVersion":1,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[]}"#;
    assert!(OciIndex::from_bytes(invalid).is_err());
}

#[test]
fn test_auto_detect_fails_without_media_type() {
    // Manifest without mediaType should fail auto-detection
    let manifest_no_media_type = br#"{"schemaVersion":2,"config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":100},"layers":[]}"#;
    let result = Manifest::from_bytes(manifest_no_media_type);
    assert!(result.is_err(), "auto-detect should fail without mediaType");

    // Index without mediaType should fail auto-detection
    let index_no_media_type = br#"{"schemaVersion":2,"manifests":[]}"#;
    let result = ImageIndex::from_bytes(index_no_media_type);
    assert!(result.is_err(), "auto-detect should fail without mediaType");
}
