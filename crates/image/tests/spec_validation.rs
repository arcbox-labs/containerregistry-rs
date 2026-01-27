//! Spec-oriented validation and compatibility tests.

use containerregistry_image::{
    ContainerConfig, EmptyObject, Healthcheck, History, ImageConfig, MediaType, OciIndex,
    OciManifest,
};

// This test ensures the "unchecked" parser remains permissive for legacy/partial manifests
// where the top-level mediaType is missing but the payload is otherwise well-formed.
#[test]
fn test_oci_manifest_unchecked_allows_missing_media_type() {
    let json = br#"{"schemaVersion":2,"config":{"mediaType":"application/vnd.oci.image.config.v1+json","digest":"sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a","size":100},"layers":[]}"#;
    let manifest = OciManifest::from_bytes_unchecked(json).expect("unchecked parse should succeed");
    assert!(manifest.media_type.is_none());
}

// This test ensures the "unchecked" parser remains permissive for indexes without mediaType,
// which can appear in older fixtures or handcrafted test data.
#[test]
fn test_oci_index_unchecked_allows_missing_media_type() {
    let json = br#"{"schemaVersion":2,"manifests":[]}"#;
    let index = OciIndex::from_bytes_unchecked(json).expect("unchecked parse should succeed");
    assert!(index.media_type.is_none());
}

// This test validates that ContainerConfig uses Docker-compatible PascalCase field names.
#[test]
fn test_container_config_serializes_pascal_case() {
    let config = ContainerConfig::new()
        .with_working_dir("/app")
        .with_user("nobody");
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"WorkingDir\""));
    assert!(json.contains("\"User\""));
}

// This test ensures deterministic config digests when map fields are populated in different orders.
#[test]
fn test_config_digest_deterministic_for_maps() {
    let mut cfg1 = ContainerConfig::new();
    cfg1.labels.insert("z-key".into(), "value1".into());
    cfg1.labels.insert("a-key".into(), "value2".into());
    cfg1.exposed_ports
        .insert("80/tcp".into(), EmptyObject::default());
    cfg1.volumes.insert("/data".into(), EmptyObject::default());

    let mut cfg2 = ContainerConfig::new();
    cfg2.labels.insert("a-key".into(), "value2".into());
    cfg2.labels.insert("z-key".into(), "value1".into());
    cfg2.volumes.insert("/data".into(), EmptyObject::default());
    cfg2.exposed_ports
        .insert("80/tcp".into(), EmptyObject::default());

    let img1 = ImageConfig::new("amd64", "linux").with_config(cfg1);
    let img2 = ImageConfig::new("amd64", "linux").with_config(cfg2);

    assert_eq!(
        img1.digest().unwrap(),
        img2.digest().unwrap(),
        "digest should be deterministic for map fields"
    );
}

// This test verifies that empty_layer is omitted when false and included when true,
// matching the OCI config schema expectations.
#[test]
fn test_history_empty_layer_serialization() {
    let history = History::new().with_created_by("ADD file:abc123 /");
    let json = serde_json::to_string(&history).unwrap();
    assert!(
        !json.contains("empty_layer"),
        "empty_layer=false should be omitted"
    );

    let history = History::new().as_empty_layer();
    let json = serde_json::to_string(&history).unwrap();
    assert!(json.contains("empty_layer"));
}

// This test ensures RootFS uses the required "type" JSON key and defaults to "layers".
#[test]
fn test_rootfs_type_serialized_as_type() {
    let config = ImageConfig::new("amd64", "linux");
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"type\":\"layers\""));
}

// This test ensures invalid diff_ids are rejected during config parsing.
#[test]
fn test_image_config_rejects_invalid_diff_ids() {
    let json = r#"{
        "architecture": "amd64",
        "os": "linux",
        "rootfs": {
            "type": "layers",
            "diff_ids": ["sha256:invalid"]
        }
    }"#;
    let result = ImageConfig::from_bytes(json.as_bytes());
    assert!(result.is_err(), "invalid diff_ids should be rejected");
}

// This test ensures the OCI empty JSON media type is treated as a config type
// and converts to the Docker config media type.
#[test]
fn test_oci_empty_json_media_type_is_config() {
    let mt: MediaType = "application/vnd.oci.empty.v1+json".parse().unwrap();
    assert!(mt.is_config());
    assert_eq!(mt.to_docker(), MediaType::DockerConfig);
}

// This test ensures MediaType parsing and config handling remain stable for unknown types.
#[test]
fn test_media_type_other_roundtrip() {
    let mt: MediaType = "application/vnd.example.custom+json".parse().unwrap();
    let json = serde_json::to_string(&mt).unwrap();
    let parsed: MediaType = serde_json::from_str(&json).unwrap();
    assert_eq!(mt, parsed);
}

// This test ensures ImageConfig.os_features is serialized under "os.features"
// and round-trips correctly.
#[test]
fn test_image_config_os_features_serialization() {
    let mut config = ImageConfig::new("amd64", "linux");
    config.os_features = vec!["sse4".to_string(), "avx".to_string()];

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"os.features\""));

    let parsed: ImageConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.os_features, config.os_features);
}

// This test ensures Healthcheck fields use PascalCase and round-trip properly.
#[test]
fn test_container_config_healthcheck_pascal_case_roundtrip() {
    let mut config = ContainerConfig::new();
    let healthcheck = Healthcheck {
        test: vec!["NONE".to_string()],
        interval: Some(1_000_000_000),
        timeout: Some(2_000_000_000),
        retries: Some(3),
        start_period: Some(4_000_000_000),
        start_interval: Some(5_000_000_000),
    };
    config.healthcheck = Some(healthcheck.clone());

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"Healthcheck\""));
    assert!(json.contains("\"Test\""));
    assert!(json.contains("\"StartPeriod\""));

    let parsed: ContainerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.healthcheck, Some(healthcheck));
}

// This test ensures OnBuild/Shell/ArgsEscaped use PascalCase and ArgsEscaped
// is omitted when false.
#[test]
fn test_container_config_on_build_shell_args_escaped_serialization() {
    let mut config = ContainerConfig::new();
    config.on_build = vec!["RUN echo hello".to_string()];
    config.shell = Some(vec!["/bin/sh".to_string(), "-c".to_string()]);
    config.args_escaped = true;

    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"OnBuild\""));
    assert!(json.contains("\"Shell\""));
    assert!(json.contains("\"ArgsEscaped\""));

    let parsed: ContainerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.on_build, config.on_build);
    assert_eq!(parsed.shell, config.shell);
    assert!(parsed.args_escaped);

    let json_default = serde_json::to_string(&ContainerConfig::new()).unwrap();
    assert!(
        !json_default.contains("ArgsEscaped"),
        "ArgsEscaped=false should be omitted"
    );
}
