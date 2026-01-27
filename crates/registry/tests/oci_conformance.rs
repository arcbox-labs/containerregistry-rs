//! OCI Distribution Spec Conformance Tests
//!
//! These tests verify that our registry client implementation conforms to the
//! OCI Distribution Specification:
//! https://github.com/opencontainers/distribution-spec/blob/main/spec.md
//!
//! Run with:
//! ```
//! REGISTRY_INTEGRATION=1 cargo test -p containerregistry-registry --test oci_conformance
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use containerregistry_image::{
    Descriptor, Digest, ImageConfig, ImageIndex, Manifest, MediaType, OciIndex, OciManifest,
    Platform,
};
use containerregistry_registry::{Client, ClientConfig, ManifestOrIndex, Reference};

// ============================================================================
// Test Configuration
// ============================================================================

fn integration_enabled() -> bool {
    std::env::var("REGISTRY_INTEGRATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn registry_addr() -> String {
    std::env::var("REGISTRY_ANON_ADDR").unwrap_or_else(|_| "127.0.0.1:5000".to_string())
}

fn unique_repo(name: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    format!("oci-conformance/{name}-{ts}")
}

async fn wait_for_registry(client: &Client, addr: &str) {
    for _ in 0..50 {
        if client.ping(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("registry {} not ready after 10 seconds", addr);
}

fn create_client() -> Client {
    Client::with_config(ClientConfig::new().with_https(false)).expect("client")
}

// ============================================================================
// OCI-DIST-001: Check API Endpoint
// https://github.com/opencontainers/distribution-spec/blob/main/spec.md#api-version-check
// ============================================================================

/// Verify registry responds to /v2/ with 200 OK
#[tokio::test]
async fn oci_dist_001_api_version_check() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    // GET /v2/ must return 200 OK
    let result = client.ping(&addr).await;
    assert!(result.is_ok(), "ping failed: {:?}", result);
}

// ============================================================================
// OCI-DIST-002: Pull Manifest
// https://github.com/opencontainers/distribution-spec/blob/main/spec.md#pulling-manifests
// ============================================================================

/// Verify manifest can be pulled by tag
#[tokio::test]
async fn oci_dist_002_pull_manifest_by_tag() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    // First push a manifest
    let repo = unique_repo("pull-by-tag");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let (manifest, _config_digest, _layer_digest) = create_test_manifest();
    let config_bytes = create_test_config();
    let layer_bytes = b"test-layer-content".to_vec();

    client.put_blob(&reference, &config_bytes).await.unwrap();
    client.put_blob(&reference, &layer_bytes).await.unwrap();
    let pushed_digest = client.put_manifest(&reference, &manifest).await.unwrap();

    // GET /v2/<name>/manifests/<tag>
    let (fetched, digest) = client.get_manifest(&reference).await.unwrap();
    assert_eq!(digest, pushed_digest);
    assert!(matches!(fetched, ManifestOrIndex::Manifest(_)));
}

/// Verify manifest can be pulled by digest
#[tokio::test]
async fn oci_dist_002_pull_manifest_by_digest() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    // First push a manifest
    let repo = unique_repo("pull-by-digest");
    let tag_ref: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let (manifest, _, _) = create_test_manifest();
    let config_bytes = create_test_config();
    let layer_bytes = b"test-layer-content".to_vec();

    client.put_blob(&tag_ref, &config_bytes).await.unwrap();
    client.put_blob(&tag_ref, &layer_bytes).await.unwrap();
    let pushed_digest = client.put_manifest(&tag_ref, &manifest).await.unwrap();

    // GET /v2/<name>/manifests/<digest>
    let digest_ref: Reference = format!("{addr}/{repo}@{pushed_digest}").parse().unwrap();
    let (fetched, digest) = client.get_manifest(&digest_ref).await.unwrap();
    assert_eq!(digest, pushed_digest);
    assert!(matches!(fetched, ManifestOrIndex::Manifest(_)));
}

// ============================================================================
// OCI-DIST-003: Push Manifest
// https://github.com/opencontainers/distribution-spec/blob/main/spec.md#pushing-manifests
// ============================================================================

/// Verify manifest can be pushed with tag
#[tokio::test]
async fn oci_dist_003_push_manifest_with_tag() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("push-with-tag");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let (manifest, _, _) = create_test_manifest();
    let config_bytes = create_test_config();
    let layer_bytes = b"test-layer-content".to_vec();

    // Push blobs first
    client.put_blob(&reference, &config_bytes).await.unwrap();
    client.put_blob(&reference, &layer_bytes).await.unwrap();

    // PUT /v2/<name>/manifests/<tag>
    let result = client.put_manifest(&reference, &manifest).await;
    assert!(result.is_ok(), "put_manifest failed: {:?}", result);

    // Verify Location header returned digest (implicit via returned value)
    let digest = result.unwrap();
    assert!(digest.to_string().starts_with("sha256:"));
}

/// Verify manifest can be pushed by digest reference
#[tokio::test]
async fn oci_dist_003_push_manifest_with_digest() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("push-with-digest");
    let tag_ref: Reference = format!("{addr}/{repo}:temp").parse().unwrap();

    let (manifest, _, _) = create_test_manifest();
    let config_bytes = create_test_config();
    let layer_bytes = b"test-layer-content".to_vec();

    // Push blobs first
    client.put_blob(&tag_ref, &config_bytes).await.unwrap();
    client.put_blob(&tag_ref, &layer_bytes).await.unwrap();

    // Calculate expected digest
    let manifest_bytes = manifest.to_bytes().unwrap();
    let expected_digest = Digest::sha256(&manifest_bytes);

    // PUT /v2/<name>/manifests/<digest>
    let digest_ref: Reference = format!("{addr}/{repo}@{expected_digest}").parse().unwrap();
    let result = client.put_manifest(&digest_ref, &manifest).await;
    assert!(result.is_ok(), "put_manifest by digest failed: {:?}", result);
    assert_eq!(result.unwrap(), expected_digest);
}

// ============================================================================
// OCI-DIST-004: Content Discovery (Tags List)
// https://github.com/opencontainers/distribution-spec/blob/main/spec.md#content-discovery
// ============================================================================

/// Verify tag listing returns pushed tags
#[tokio::test]
async fn oci_dist_004_list_tags() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("list-tags");
    let (manifest, _, _) = create_test_manifest();
    let config_bytes = create_test_config();
    let layer_bytes = b"test-layer-content".to_vec();

    // Push multiple tags
    for tag in ["v1", "v2", "latest"] {
        let reference: Reference = format!("{addr}/{repo}:{tag}").parse().unwrap();
        client.put_blob(&reference, &config_bytes).await.unwrap();
        client.put_blob(&reference, &layer_bytes).await.unwrap();
        client.put_manifest(&reference, &manifest).await.unwrap();
    }

    // GET /v2/<name>/tags/list
    let reference: Reference = format!("{addr}/{repo}:ignored").parse().unwrap();
    let tags = client.list_tags(&reference).await.unwrap();

    assert!(tags.contains(&"v1".to_string()));
    assert!(tags.contains(&"v2".to_string()));
    assert!(tags.contains(&"latest".to_string()));
}

// ============================================================================
// OCI-DIST-005: Pull Blob
// https://github.com/opencontainers/distribution-spec/blob/main/spec.md#pulling-blobs
// ============================================================================

/// Verify blob can be pulled by digest
#[tokio::test]
async fn oci_dist_005_pull_blob() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("pull-blob");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let blob_content = b"test-blob-content-for-pull".to_vec();
    let blob_digest = Digest::sha256(&blob_content);

    // Push blob first
    client.put_blob(&reference, &blob_content).await.unwrap();

    // GET /v2/<name>/blobs/<digest>
    let fetched = client.get_blob(&reference, &blob_digest).await.unwrap();
    assert_eq!(fetched, blob_content);
}

/// Verify blob HEAD returns correct size
#[tokio::test]
async fn oci_dist_005_head_blob() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("head-blob");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let blob_content = b"test-blob-content-for-head-check".to_vec();
    let blob_digest = Digest::sha256(&blob_content);

    // Push blob first
    client.put_blob(&reference, &blob_content).await.unwrap();

    // HEAD /v2/<name>/blobs/<digest>
    let size = client.head_blob(&reference, &blob_digest).await.unwrap();
    assert_eq!(size, blob_content.len() as u64);
}

// ============================================================================
// OCI-DIST-006: Push Blob
// https://github.com/opencontainers/distribution-spec/blob/main/spec.md#pushing-blobs
// ============================================================================

/// Verify blob can be pushed (monolithic upload)
#[tokio::test]
async fn oci_dist_006_push_blob_monolithic() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("push-blob-mono");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let blob_content = b"monolithic-blob-upload-test".to_vec();
    let expected_digest = Digest::sha256(&blob_content);

    // POST + PUT (monolithic) /v2/<name>/blobs/uploads/
    let digest = client.put_blob(&reference, &blob_content).await.unwrap();
    assert_eq!(digest, expected_digest);

    // Verify blob exists
    let size = client.head_blob(&reference, &digest).await.unwrap();
    assert_eq!(size, blob_content.len() as u64);
}

/// Verify existing blob is not re-uploaded
#[tokio::test]
async fn oci_dist_006_push_blob_exists() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("push-blob-exists");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let blob_content = b"blob-existence-check".to_vec();

    // Push blob first time
    let digest1 = client.put_blob(&reference, &blob_content).await.unwrap();

    // Push same blob again - should succeed without re-upload
    let digest2 = client.put_blob(&reference, &blob_content).await.unwrap();
    assert_eq!(digest1, digest2);
}

// ============================================================================
// OCI-DIST-007: Manifest HEAD
// ============================================================================

/// Verify manifest HEAD returns correct digest header
#[tokio::test]
async fn oci_dist_007_head_manifest() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("head-manifest");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let (manifest, _, _) = create_test_manifest();
    let config_bytes = create_test_config();
    let layer_bytes = b"test-layer-content".to_vec();

    client.put_blob(&reference, &config_bytes).await.unwrap();
    client.put_blob(&reference, &layer_bytes).await.unwrap();
    let pushed_digest = client.put_manifest(&reference, &manifest).await.unwrap();

    // HEAD /v2/<name>/manifests/<tag>
    let (media_type, digest, size) = client.head_manifest(&reference).await.unwrap();

    assert_eq!(digest, pushed_digest);
    assert!(
        media_type == MediaType::OciManifest || media_type == MediaType::DockerManifest,
        "unexpected media type: {:?}",
        media_type
    );
    assert!(size > 0);
}

// ============================================================================
// OCI-DIST-008: Image Index Support
// https://github.com/opencontainers/distribution-spec/blob/main/spec.md#content-management
// ============================================================================

/// Verify image index can be pushed and pulled
#[tokio::test]
async fn oci_dist_008_image_index() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("image-index");

    // Create and push manifests for two platforms
    let (amd64_manifest, amd64_digest) = push_platform_manifest(&client, &addr, &repo, "amd64", "linux").await;
    let (arm64_manifest, arm64_digest) = push_platform_manifest(&client, &addr, &repo, "arm64", "linux").await;

    // Create image index
    let amd64_desc = Descriptor::new(
        MediaType::OciManifest,
        amd64_digest.clone(),
        amd64_manifest.to_bytes().unwrap().len() as u64,
    )
    .with_platform(Platform::new("amd64", "linux"));

    let arm64_desc = Descriptor::new(
        MediaType::OciManifest,
        arm64_digest.clone(),
        arm64_manifest.to_bytes().unwrap().len() as u64,
    )
    .with_platform(Platform::new("arm64", "linux"));

    let index = ImageIndex::Oci(OciIndex::new(vec![amd64_desc, arm64_desc]));

    // Push index
    let reference: Reference = format!("{addr}/{repo}:multi").parse().unwrap();
    let index_digest = client.put_index(&reference, &index).await.unwrap();

    // Pull index
    let (fetched, digest) = client.get_manifest(&reference).await.unwrap();
    assert_eq!(digest, index_digest);
    assert!(matches!(fetched, ManifestOrIndex::Index(_)));

    if let ManifestOrIndex::Index(idx) = fetched {
        assert_eq!(idx.manifests().len(), 2);
    }
}

// ============================================================================
// OCI-DIST-009: Digest Verification
// ============================================================================

/// Verify registry validates digest on PUT
#[tokio::test]
async fn oci_dist_009_digest_verification() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("digest-verify");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let (manifest, _, _) = create_test_manifest();
    let config_bytes = create_test_config();
    let layer_bytes = b"test-layer-content".to_vec();

    client.put_blob(&reference, &config_bytes).await.unwrap();
    client.put_blob(&reference, &layer_bytes).await.unwrap();

    // Calculate correct digest
    let manifest_bytes = manifest.to_bytes().unwrap();
    let correct_digest = Digest::sha256(&manifest_bytes);

    // Push with correct digest reference should succeed
    let digest_ref: Reference = format!("{addr}/{repo}@{correct_digest}").parse().unwrap();
    let result = client.put_manifest(&digest_ref, &manifest).await;
    assert!(result.is_ok());

    // Push with wrong digest reference should fail
    let wrong_digest = Digest::sha256(b"wrong content");
    let wrong_ref: Reference = format!("{addr}/{repo}@{wrong_digest}").parse().unwrap();
    let result = client.put_manifest(&wrong_ref, &manifest).await;
    assert!(result.is_err(), "should reject mismatched digest");
}

// ============================================================================
// OCI-DIST-010: Error Responses
// ============================================================================

/// Verify 404 for non-existent manifest
#[tokio::test]
async fn oci_dist_010_manifest_not_found() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("not-found");
    let reference: Reference = format!("{addr}/{repo}:nonexistent").parse().unwrap();

    let result = client.get_manifest(&reference).await;
    assert!(result.is_err());

    // Should be a NotFound error
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("not found") || err.to_string().contains("NotFound"),
        "expected not found error, got: {}",
        err
    );
}

/// Verify 404 for non-existent blob
#[tokio::test]
async fn oci_dist_010_blob_not_found() {
    if !integration_enabled() {
        eprintln!("skipping (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let client = create_client();
    let addr = registry_addr();
    wait_for_registry(&client, &addr).await;

    let repo = unique_repo("blob-not-found");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();
    let fake_digest = Digest::sha256(b"this blob does not exist");

    let result = client.get_blob(&reference, &fake_digest).await;
    assert!(result.is_err());
}

// ============================================================================
// Helper Functions
// ============================================================================

fn create_test_config() -> Vec<u8> {
    let config = ImageConfig::new("amd64", "linux");
    config.to_bytes().unwrap()
}

fn create_test_manifest() -> (Manifest, Digest, Digest) {
    let config_bytes = create_test_config();
    let config_digest = Digest::sha256(&config_bytes);

    let layer_bytes = b"test-layer-content".to_vec();
    let layer_digest = Digest::sha256(&layer_bytes);

    let config_desc = Descriptor::new(
        MediaType::OciConfig,
        config_digest.clone(),
        config_bytes.len() as u64,
    );
    let layer_desc = Descriptor::new(
        MediaType::OciLayer,
        layer_digest.clone(),
        layer_bytes.len() as u64,
    );

    let oci = OciManifest::new(config_desc, vec![layer_desc]);
    (Manifest::Oci(oci), config_digest, layer_digest)
}

async fn push_platform_manifest(
    client: &Client,
    addr: &str,
    repo: &str,
    arch: &str,
    os: &str,
) -> (Manifest, Digest) {
    let config = ImageConfig::new(arch, os);
    let config_bytes = config.to_bytes().unwrap();
    let config_digest = Digest::sha256(&config_bytes);

    let layer_bytes = format!("layer-for-{}-{}", arch, os).into_bytes();
    let layer_digest = Digest::sha256(&layer_bytes);

    let config_desc = Descriptor::new(
        MediaType::OciConfig,
        config_digest.clone(),
        config_bytes.len() as u64,
    );
    let layer_desc = Descriptor::new(
        MediaType::OciLayer,
        layer_digest.clone(),
        layer_bytes.len() as u64,
    );

    let manifest = Manifest::Oci(OciManifest::new(config_desc, vec![layer_desc]));

    let reference: Reference = format!("{addr}/{repo}:{arch}-{os}").parse().unwrap();
    client.put_blob(&reference, &config_bytes).await.unwrap();
    client.put_blob(&reference, &layer_bytes).await.unwrap();
    let digest = client.put_manifest(&reference, &manifest).await.unwrap();

    (manifest, digest)
}
