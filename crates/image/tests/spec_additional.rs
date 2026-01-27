//! Additional spec-focused tests for image types.

use containerregistry_image::{
    Descriptor, DockerManifest, DockerManifestList, ImageConfig, Manifest, MediaType, OciIndex,
    OciManifest, Platform,
};

#[test]
fn test_media_type_nondistributable_and_foreign() {
    let oci_nd_gzip: MediaType = "application/vnd.oci.image.layer.nondistributable.v1.tar+gzip"
        .parse()
        .unwrap();
    assert!(oci_nd_gzip.is_layer());
    assert!(oci_nd_gzip.is_nondistributable());
    assert_eq!(oci_nd_gzip.to_docker(), MediaType::DockerForeignLayer);
    assert!(oci_nd_gzip.is_docker_compatible());

    let docker_foreign: MediaType = "application/vnd.docker.image.rootfs.foreign.diff.tar.gzip"
        .parse()
        .unwrap();
    assert!(docker_foreign.is_layer());
    assert!(docker_foreign.is_nondistributable());
    assert_eq!(
        docker_foreign.to_oci(),
        MediaType::OciLayerNondistributableGzip
    );

    let oci_zstd = MediaType::OciLayerZstd;
    assert!(!oci_zstd.is_docker_compatible());
    assert_eq!(oci_zstd.to_docker(), MediaType::OciLayerZstd);
}

#[test]
fn test_descriptor_with_data_and_platform_features_roundtrip() {
    let mut desc = Descriptor::new(
        MediaType::OciManifest,
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
            .parse()
            .unwrap(),
        1234,
    )
    .with_url("https://example.com/blob")
    .with_platform(
        Platform::new("amd64", "linux")
            .with_variant("v3")
            .with_os_version("5.10")
            .with_os_feature("sse4")
            .with_feature("sse2"),
    );

    desc.data = Some("ZHVtbXk=".to_string());

    let json = serde_json::to_string(&desc).unwrap();
    let parsed: Descriptor = serde_json::from_str(&json).unwrap();

    assert_eq!(desc, parsed);
    assert!(json.contains("\"os.version\""));
    assert!(json.contains("\"os.features\""));
}

#[test]
fn test_container_config_deterministic_labels() {
    let cfg1 = ImageConfig::new("amd64", "linux").with_config(
        containerregistry_image::ContainerConfig::new()
            .with_label("z-key", "value1")
            .with_label("a-key", "value2"),
    );

    let cfg2 = ImageConfig::new("amd64", "linux").with_config(
        containerregistry_image::ContainerConfig::new()
            .with_label("a-key", "value2")
            .with_label("z-key", "value1"),
    );

    assert_eq!(
        cfg1.digest().unwrap(),
        cfg2.digest().unwrap(),
        "config digest should be deterministic regardless of label insertion order"
    );
}

#[test]
fn test_oci_index_artifact_and_subject_roundtrip() {
    let subject = Descriptor::new(
        MediaType::OciManifest,
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
            .parse()
            .unwrap(),
        1234,
    );

    let index = OciIndex::new(vec![subject.clone()])
        .with_artifact_type(MediaType::Other(
            "application/vnd.example.artifact".to_string(),
        ))
        .with_subject(subject);

    let bytes = index.to_bytes().unwrap();
    let parsed = OciIndex::from_bytes(&bytes).unwrap();

    assert_eq!(index.artifact_type, parsed.artifact_type);
    assert_eq!(index.subject, parsed.subject);
}

#[test]
fn test_try_to_docker_rejects_nondistributable_layer() {
    let manifest = OciManifest::new(
        Descriptor::new(
            MediaType::OciConfig,
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                .parse()
                .unwrap(),
            2,
        ),
        vec![Descriptor::new(
            MediaType::OciLayerNondistributableGzip,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap(),
            0,
        )],
    );

    let err = manifest.try_to_docker().unwrap_err();
    assert!(err.to_string().contains("non-distributable"));
}

#[test]
fn test_manifest_unsupported_media_type() {
    let json = r#"{"schemaVersion":2,"mediaType":"application/example","config":{},"layers":[]}"#;
    let err = Manifest::from_bytes(json.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("unsupported media type"));
}

#[test]
fn test_index_unsupported_media_type() {
    let json = r#"{"schemaVersion":2,"mediaType":"application/example","manifests":[]}"#;
    let err = containerregistry_image::ImageIndex::from_bytes(json.as_bytes()).unwrap_err();
    assert!(err.to_string().contains("unsupported media type"));
}

#[test]
fn test_docker_manifest_list_validation_schema_version() {
    let mut list = DockerManifestList::new(vec![]);
    list.schema_version = 1;

    let err = list.validate().unwrap_err();
    assert!(err.to_string().contains("schemaVersion"));
}

#[test]
fn test_oci_manifest_rejects_docker_media_types() {
    let mut manifest = OciManifest::new(
        Descriptor::new(
            MediaType::DockerConfig,
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                .parse()
                .unwrap(),
            2,
        ),
        vec![Descriptor::new(
            MediaType::DockerLayerGzip,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap(),
            0,
        )],
    );

    let err = manifest.validate().unwrap_err();
    assert!(err.to_string().contains("OCI manifest"));

    // Also reject Docker layers even if config is correct.
    manifest.config.media_type = MediaType::OciConfig;
    let err = manifest.validate().unwrap_err();
    assert!(err.to_string().contains("OCI manifest"));
}

#[test]
fn test_docker_manifest_rejects_oci_media_types() {
    let mut manifest = DockerManifest::new(
        Descriptor::new(
            MediaType::OciConfig,
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
                .parse()
                .unwrap(),
            2,
        ),
        vec![Descriptor::new(
            MediaType::OciLayerGzip,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap(),
            0,
        )],
    );

    let err = manifest.validate().unwrap_err();
    assert!(err.to_string().contains("Docker manifest"));

    // Also reject OCI config even if layers are Docker.
    manifest.layers[0].media_type = MediaType::DockerLayerGzip;
    let err = manifest.validate().unwrap_err();
    assert!(err.to_string().contains("Docker manifest"));
}

#[test]
fn test_docker_manifest_list_rejects_index_descriptors() {
    let index_desc = Descriptor::new(
        MediaType::OciIndex,
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
            .parse()
            .unwrap(),
        123,
    );
    let list = DockerManifestList::new(vec![index_desc]);
    let err = list.validate().unwrap_err();
    assert!(err.to_string().contains("Docker manifest list"));
}

#[test]
fn test_oci_index_try_to_docker_rejects_index_descriptors() {
    let index_desc = Descriptor::new(
        MediaType::OciIndex,
        "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
            .parse()
            .unwrap(),
        123,
    );
    let index = OciIndex::new(vec![index_desc]);
    let err = index.try_to_docker().unwrap_err();
    assert!(err.to_string().contains("cannot be converted"));
}

#[test]
fn test_image_config_rootfs_validate() {
    let mut config = ImageConfig::new("amd64", "linux");
    config.rootfs.fs_type = "not-layers".to_string();
    let err = config.validate().unwrap_err();
    assert!(err.to_string().contains("rootfs.type"));
}
