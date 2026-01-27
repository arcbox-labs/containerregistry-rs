//! Integration tests for the crane CLI against a local registry.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use containerregistry_image::{
    Descriptor, Digest, ImageConfig, Manifest, MediaType, OciIndex, OciManifest, Platform,
};
use containerregistry_layout::Layout;
use containerregistry_registry::{Client, ClientConfig};

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
    format!("integration-cli/{name}-{ts}")
}

fn temp_dir(prefix: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let mut dir = std::env::temp_dir();
    dir.push(format!("crane-{prefix}-{ts}"));
    dir
}

struct BuiltManifest {
    manifest: Manifest,
    digest: Digest,
    config_bytes: Vec<u8>,
    layer_bytes: Vec<u8>,
    platform: Platform,
}

fn build_manifest(os: &str, arch: &str) -> BuiltManifest {
    let config = ImageConfig::new(arch.to_string(), os.to_string());
    let config_bytes = config.to_bytes().expect("config bytes");
    let config_digest = Digest::sha256(&config_bytes);
    let config_desc = Descriptor::new(
        MediaType::OciConfig,
        config_digest,
        config_bytes.len() as u64,
    );

    let layer_bytes = format!("layer-{os}-{arch}").into_bytes();
    let layer_digest = Digest::sha256(&layer_bytes);
    let layer_desc = Descriptor::new(
        MediaType::OciLayer,
        layer_digest,
        layer_bytes.len() as u64,
    );

    let manifest = Manifest::Oci(OciManifest::new(config_desc, vec![layer_desc]));
    let digest = manifest.digest().expect("manifest digest");
    let platform = Platform::new(arch.to_string(), os.to_string());

    BuiltManifest {
        manifest,
        digest,
        config_bytes,
        layer_bytes,
        platform,
    }
}

fn add_manifest_to_layout(
    layout: &Layout,
    built: &BuiltManifest,
) -> Result<Descriptor, Box<dyn std::error::Error>> {
    layout.write_blob_with_digest(&built.config_bytes, &built.manifest.config().digest)?;
    let layer_digest = built.manifest.layers().first().unwrap().digest.clone();
    layout.write_blob_with_digest(&built.layer_bytes, &layer_digest)?;
    let mut descriptor = layout.write_manifest(&built.manifest)?;
    descriptor.platform = Some(built.platform.clone());
    Ok(descriptor)
}

#[tokio::test]
async fn test_crane_multi_arch_push_pull() {
    if !integration_enabled() {
        eprintln!("skipping integration test (set REGISTRY_INTEGRATION=1)");
        return;
    }

    let addr = anon_registry_addr();
    wait_for_registry(&addr).await;

    let layout_dir = temp_dir("layout");
    let pull_dir = temp_dir("pull");

    let layout = Layout::create(&layout_dir).expect("create layout");
    let manifest_amd64 = build_manifest("linux", "amd64");
    let manifest_arm64 = build_manifest("linux", "arm64");

    let desc_amd64 = add_manifest_to_layout(&layout, &manifest_amd64).expect("amd64 manifest");
    let desc_arm64 = add_manifest_to_layout(&layout, &manifest_arm64).expect("arm64 manifest");

    let index = OciIndex::new(vec![desc_amd64, desc_arm64]);
    layout.write_index(&index).expect("write index");

    let repo = unique_repo("multi-arch");
    let image = format!("{addr}/{repo}:v1");

    let output = Command::new(env!("CARGO_BIN_EXE_crane"))
        .args(["--insecure", "push", &image, layout_dir.to_str().unwrap()])
        .output()
        .expect("run crane push");
    assert!(
        output.status.success(),
        "push failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    if pull_dir.exists() {
        let _ = fs::remove_dir_all(&pull_dir);
    }

    let output = Command::new(env!("CARGO_BIN_EXE_crane"))
        .args([
            "--insecure",
            "--platform",
            "linux/amd64",
            "pull",
            "--format",
            "oci",
            "--output",
            pull_dir.to_str().unwrap(),
            &image,
        ])
        .output()
        .expect("run crane pull");
    assert!(
        output.status.success(),
        "pull failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let digest_line = stdout
        .lines()
        .find(|line| !line.trim().is_empty())
        .expect("missing digest output")
        .trim();
    assert_eq!(digest_line, manifest_amd64.digest.to_string());

    let pulled = Layout::open(&pull_dir).expect("open pulled layout");
    let pulled_index = pulled.index().expect("read pulled index");
    let manifest_desc = pulled_index
        .manifests()
        .first()
        .expect("missing manifest descriptor");
    assert_eq!(manifest_desc.digest, manifest_amd64.digest);

    let _ = fs::remove_dir_all(&layout_dir);
    let _ = fs::remove_dir_all(&pull_dir);
}
