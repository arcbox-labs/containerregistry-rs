//! Parity tests between Rust gcrane and Go gcrane (cp command).

use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use containerregistry_image::{Descriptor, Digest, ImageConfig, Manifest, MediaType, OciManifest};
use containerregistry_registry::{Client, ClientConfig, Reference};

fn parity_enabled() -> bool {
    std::env::var("GCRANE_PARITY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

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
    format!("parity-gcrane/{name}-{ts}")
}

fn build_minimal_manifest() -> (Vec<u8>, Vec<u8>, Manifest) {
    let config = ImageConfig::new("amd64", "linux");
    let config_bytes = config.to_bytes().expect("config bytes");
    let config_digest = Digest::sha256(&config_bytes);

    let layer_bytes = b"gcrane-parity-layer".to_vec();
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

    (config_bytes, layer_bytes, manifest)
}

#[tokio::test]
async fn test_gcrane_cp_parity() {
    if !parity_enabled() || !integration_enabled() {
        eprintln!("skipping gcrane parity test (set GCRANE_PARITY=1 and REGISTRY_INTEGRATION=1)");
        return;
    }

    let go_gcrane = match std::env::var("GO_GCRANE_PATH") {
        Ok(path) => path,
        Err(_) => {
            eprintln!("skipping gcrane parity test (set GO_GCRANE_PATH)");
            return;
        }
    };

    let addr = anon_registry_addr();
    wait_for_registry(&addr).await;

    let repo = unique_repo("src");
    let tag = "v1";
    let image = format!("{addr}/{repo}:{tag}");
    let reference: Reference = image.parse().unwrap();

    let (config_bytes, layer_bytes, manifest) = build_minimal_manifest();
    let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();
    client.put_blob(&reference, &config_bytes).await.unwrap();
    client.put_blob(&reference, &layer_bytes).await.unwrap();
    client.put_manifest(&reference, &manifest).await.unwrap();

    let dst_repo = unique_repo("dst");
    let image_dst = format!("{addr}/{dst_repo}:{tag}");

    let rust_output = Command::new(env!("CARGO_BIN_EXE_gcrane"))
        .args(["--insecure", "cp", &image, &image_dst])
        .output()
        .expect("run rust gcrane");

    let go_output = Command::new(go_gcrane)
        .args(["cp", &image, &image_dst])
        .output()
        .expect("run go gcrane");

    assert_eq!(rust_output.status.success(), go_output.status.success());
}
