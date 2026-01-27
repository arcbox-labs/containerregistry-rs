//! Integration tests against a real registry (docker-compose stack).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use containerregistry_auth::Credential;
use containerregistry_image::{Descriptor, Digest, ImageConfig, Manifest, MediaType, OciManifest};
use containerregistry_registry::{Client, ClientConfig, Error, ManifestOrIndex, Reference};

fn integration_enabled() -> bool {
    std::env::var("REGISTRY_INTEGRATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn anon_registry_addr() -> String {
    std::env::var("REGISTRY_ANON_ADDR").unwrap_or_else(|_| "127.0.0.1:5000".to_string())
}

fn basic_registry_addr() -> String {
    std::env::var("REGISTRY_BASIC_ADDR").unwrap_or_else(|_| "127.0.0.1:5001".to_string())
}

fn client() -> Client {
    Client::with_config(ClientConfig::new().with_https(false)).expect("client")
}

fn client_with_basic_auth() -> Client {
    Client::with_credential(
        ClientConfig::new().with_https(false),
        Credential::basic("testuser", "testpassword"),
    )
    .expect("client")
}

fn unique_repo(name: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    format!("integration/{name}-{ts}")
}

async fn wait_for_registry(addr: &str) {
    let client = client();
    let mut attempts = 0;
    loop {
        match client.ping(addr).await {
            Ok(()) => return,
            Err(_) => {
                attempts += 1;
                if attempts > 50 {
                    panic!("registry {} not ready", addr);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

async fn wait_for_registry_with_basic(addr: &str) {
    let client = client_with_basic_auth();
    let mut attempts = 0;
    loop {
        match client.ping(addr).await {
            Ok(()) => return,
            Err(_) => {
                attempts += 1;
                if attempts > 50 {
                    panic!("registry {} not ready", addr);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

fn build_minimal_manifest() -> (Vec<u8>, Vec<u8>, Manifest, Digest, Digest) {
    let config = ImageConfig::new("amd64", "linux");
    let config_bytes = config.to_bytes().expect("config bytes");
    let config_digest = Digest::sha256(&config_bytes);

    // Minimal layer payload. Registry does not validate content format.
    let layer_bytes = b"hello-layer".to_vec();
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
    let manifest = Manifest::Oci(oci);

    (
        config_bytes,
        layer_bytes,
        manifest,
        config_digest,
        layer_digest,
    )
}

// This test verifies that the anonymous registry is reachable and responds to /v2/.
#[tokio::test]
async fn test_integration_ping_anonymous() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }
    let addr = anon_registry_addr();
    wait_for_registry(&addr).await;
    client().ping(&addr).await.expect("ping");
}

// This test verifies basic push/pull flow: upload blobs, push manifest, read it back,
// and confirm the tag appears in tag listing.
#[tokio::test]
async fn test_integration_push_pull_manifest() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }
    let addr = anon_registry_addr();
    wait_for_registry(&addr).await;

    let repo = unique_repo("push-pull");
    let tag = "v1";
    let reference: Reference = format!("{addr}/{repo}:{tag}").parse().unwrap();

    let (config_bytes, layer_bytes, manifest, config_digest, layer_digest) =
        build_minimal_manifest();

    let client = client();
    let uploaded_config = client.put_blob(&reference, &config_bytes).await.unwrap();
    let uploaded_layer = client.put_blob(&reference, &layer_bytes).await.unwrap();

    assert_eq!(uploaded_config, config_digest);
    assert_eq!(uploaded_layer, layer_digest);

    let manifest_digest = client.put_manifest(&reference, &manifest).await.unwrap();

    let (got, got_digest) = client.get_manifest(&reference).await.unwrap();
    assert!(matches!(got, ManifestOrIndex::Manifest(_)));
    assert_eq!(got_digest, manifest_digest);

    let tags = client.list_tags(&reference).await.unwrap();
    assert!(tags.contains(&tag.to_string()));
}

// This test verifies head/get blob return the expected size and payload.
#[tokio::test]
async fn test_integration_head_get_blob() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }
    let addr = anon_registry_addr();
    wait_for_registry(&addr).await;

    let repo = unique_repo("blob");
    let reference: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let (_config_bytes, layer_bytes, _manifest, _config_digest, layer_digest) =
        build_minimal_manifest();

    let client = client();
    client.put_blob(&reference, &layer_bytes).await.unwrap();

    let size = client.head_blob(&reference, &layer_digest).await.unwrap();
    assert_eq!(size, layer_bytes.len() as u64);

    let got = client.get_blob(&reference, &layer_digest).await.unwrap();
    assert_eq!(got, layer_bytes);
}

// This test verifies digest-based references succeed when the digest matches.
#[tokio::test]
async fn test_integration_get_manifest_by_digest() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }
    let addr = anon_registry_addr();
    wait_for_registry(&addr).await;

    let repo = unique_repo("by-digest");
    let tag_ref: Reference = format!("{addr}/{repo}:v1").parse().unwrap();

    let (config_bytes, layer_bytes, manifest, _config_digest, _layer_digest) =
        build_minimal_manifest();
    let client = client();

    client.put_blob(&tag_ref, &config_bytes).await.unwrap();
    client.put_blob(&tag_ref, &layer_bytes).await.unwrap();
    let manifest_digest = client.put_manifest(&tag_ref, &manifest).await.unwrap();

    let digest_ref: Reference = format!("{addr}/{repo}@{manifest_digest}").parse().unwrap();
    let (got, got_digest) = client.get_manifest(&digest_ref).await.unwrap();
    assert!(got.is_manifest());
    assert_eq!(got_digest, manifest_digest);
}

// This test verifies the basic-auth registry rejects requests without credentials.
#[tokio::test]
async fn test_integration_basic_auth_requires_auth() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }
    let addr = basic_registry_addr();
    wait_for_registry(&addr).await;

    let err = client().ping(&addr).await.unwrap_err();
    match err {
        Error::Unauthorized(_) => {}
        other => panic!("expected Unauthorized, got {:?}", other),
    }
}

// This test verifies basic-auth registry works when credentials are provided.
#[tokio::test]
async fn test_integration_basic_auth_with_credentials() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }
    let addr = basic_registry_addr();
    wait_for_registry_with_basic(&addr).await;

    client_with_basic_auth()
        .ping(&addr)
        .await
        .expect("ping with credentials");
}
