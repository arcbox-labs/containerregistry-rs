//! Integration test: registry <-> layout roundtrip.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use containerregistry_image::{Descriptor, Digest, ImageConfig, Manifest, MediaType, OciManifest};
use containerregistry_layout::Layout;
use containerregistry_registry::{Client, ClientConfig, ManifestOrIndex, Reference};

fn integration_enabled() -> bool {
    std::env::var("REGISTRY_INTEGRATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn anon_registry_addr() -> String {
    std::env::var("REGISTRY_ANON_ADDR").unwrap_or_else(|_| "127.0.0.1:5000".to_string())
}

async fn wait_for_registry(addr: &str) {
    let client = Client::with_config(ClientConfig::new().with_https(false)).expect("client");
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

fn unique_repo(name: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    format!("integration-layout/{name}-{ts}")
}

fn build_minimal_manifest() -> (Vec<u8>, Vec<u8>, Manifest, Digest, Digest) {
    let config = ImageConfig::new("amd64", "linux");
    let config_bytes = config.to_bytes().expect("config bytes");
    let config_digest = Digest::sha256(&config_bytes);

    let layer_bytes = b"layout-roundtrip-layer".to_vec();
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

#[tokio::test]
async fn test_registry_layout_roundtrip() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let addr = anon_registry_addr();
    wait_for_registry(&addr).await;

    let repo = unique_repo("roundtrip");
    let tag = "v1";
    let reference: Reference = format!("{addr}/{repo}:{tag}").parse().unwrap();

    let (config_bytes, layer_bytes, manifest, config_digest, layer_digest) =
        build_minimal_manifest();

    let layout_src = tempfile::tempdir().unwrap();
    let layout = Layout::create(layout_src.path()).unwrap();
    layout
        .write_blob_with_digest(&config_bytes, &config_digest)
        .unwrap();
    layout
        .write_blob_with_digest(&layer_bytes, &layer_digest)
        .unwrap();
    let manifest_desc = layout.write_manifest(&manifest).unwrap();
    layout.add_manifest(&manifest).unwrap();

    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();
    client.put_blob(&reference, &config_bytes).await.unwrap();
    client.put_blob(&reference, &layer_bytes).await.unwrap();
    let manifest_digest = client.put_manifest(&reference, &manifest).await.unwrap();
    assert_eq!(manifest_digest, manifest_desc.digest);

    let (fetched, fetched_digest) = client.get_manifest(&reference).await.unwrap();
    assert!(matches!(fetched, ManifestOrIndex::Manifest(_)));
    assert_eq!(fetched_digest, manifest_digest);

    let fetched_manifest = match fetched {
        ManifestOrIndex::Manifest(m) => m,
        _ => unreachable!(),
    };
    let fetched_config = client
        .get_blob(&reference, &fetched_manifest.config().digest)
        .await
        .unwrap();
    let fetched_layer = client
        .get_blob(&reference, &fetched_manifest.layers()[0].digest)
        .await
        .unwrap();

    let layout_pull = tempfile::tempdir().unwrap();
    let pulled = Layout::create(layout_pull.path()).unwrap();
    pulled
        .write_blob_with_digest(&fetched_config, &fetched_manifest.config().digest)
        .unwrap();
    pulled
        .write_blob_with_digest(&fetched_layer, &fetched_manifest.layers()[0].digest)
        .unwrap();
    pulled.write_manifest(&fetched_manifest).unwrap();
    pulled.add_manifest(&fetched_manifest).unwrap();

    pulled.validate().unwrap();

    let repo2 = unique_repo("roundtrip-copy");
    let reference2: Reference = format!("{addr}/{repo2}:{tag}").parse().unwrap();
    client.put_blob(&reference2, &fetched_config).await.unwrap();
    client.put_blob(&reference2, &fetched_layer).await.unwrap();
    let digest2 = client
        .put_manifest(&reference2, &fetched_manifest)
        .await
        .unwrap();
    assert_eq!(digest2, fetched_digest);
}
