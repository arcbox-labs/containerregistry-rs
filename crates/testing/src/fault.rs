//! Fault injection utilities for testing network resilience.
//!
//! Provides mock servers and utilities for testing behavior under
//! network failures, timeouts, and partial responses.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest as Sha2Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// Types of faults that can be injected.
#[derive(Clone, Debug)]
pub enum Fault {
    /// Return an HTTP error status code.
    HttpError(u16),
    /// Close connection immediately without response.
    ConnectionReset,
    /// Delay response by specified duration.
    Delay(Duration),
    /// Return partial response then close.
    PartialResponse(usize),
    /// Timeout (never respond).
    Timeout,
    /// Return success.
    Success,
}

/// Controls how Docker-Content-Digest headers are emitted.
#[derive(Clone, Debug)]
pub enum DigestBehavior {
    /// Compute and return the correct digest.
    Correct,
    /// Return an intentionally incorrect digest.
    Wrong(String),
    /// Omit the digest header entirely.
    Omit,
}

impl Default for DigestBehavior {
    fn default() -> Self {
        Self::Correct
    }
}

/// Configuration for fault injection.
#[derive(Clone, Debug)]
pub struct FaultConfig {
    /// Faults to inject in sequence.
    pub faults: Vec<Fault>,
    /// Whether to repeat the fault sequence.
    pub repeat: bool,
}

impl FaultConfig {
    /// Creates a config that always succeeds.
    pub fn always_succeed() -> Self {
        Self {
            faults: vec![Fault::Success],
            repeat: true,
        }
    }

    /// Creates a config that fails N times then succeeds.
    pub fn fail_then_succeed(failures: usize, fault: Fault) -> Self {
        let mut faults = vec![fault; failures];
        faults.push(Fault::Success);
        Self {
            faults,
            repeat: false,
        }
    }

    /// Creates a config that always returns an error.
    pub fn always_fail(fault: Fault) -> Self {
        Self {
            faults: vec![fault],
            repeat: true,
        }
    }

    /// Returns true if the config is valid (has at least one fault).
    pub fn is_valid(&self) -> bool {
        !self.faults.is_empty()
    }

    /// Returns the fault for the given request count, or Success if empty.
    fn fault_for_count(&self, count: usize) -> Fault {
        if self.faults.is_empty() {
            return Fault::Success;
        }
        let idx = if self.repeat {
            count % self.faults.len()
        } else {
            count.min(self.faults.len() - 1)
        };
        self.faults[idx].clone()
    }
}

/// A mock registry server that can inject faults.
pub struct FaultInjectionServer {
    listener: TcpListener,
    addr: SocketAddr,
    config: Arc<Mutex<FaultConfig>>,
    request_count: Arc<AtomicUsize>,
    manifest_digest_behavior: Arc<Mutex<DigestBehavior>>,
    /// Stored blobs for the mock registry.
    blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// Stored manifests for the mock registry.
    manifests: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl FaultInjectionServer {
    /// Creates a new fault injection server.
    pub async fn new(config: FaultConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        Ok(Self {
            listener,
            addr,
            config: Arc::new(Mutex::new(config)),
            request_count: Arc::new(AtomicUsize::new(0)),
            manifest_digest_behavior: Arc::new(Mutex::new(DigestBehavior::default())),
            blobs: Arc::new(Mutex::new(HashMap::new())),
            manifests: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Returns the server address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns the server URL.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Returns the number of requests received.
    pub fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }

    /// Updates the fault configuration.
    pub async fn set_config(&self, config: FaultConfig) {
        *self.config.lock().await = config;
    }

    /// Sets how manifest digest headers are emitted.
    pub async fn set_manifest_digest_behavior(&self, behavior: DigestBehavior) {
        *self.manifest_digest_behavior.lock().await = behavior;
    }

    /// Adds a blob to the mock registry.
    pub async fn add_blob(&self, digest: &str, data: Vec<u8>) {
        self.blobs.lock().await.insert(digest.to_string(), data);
    }

    /// Adds a manifest to the mock registry.
    pub async fn add_manifest(&self, reference: &str, data: Vec<u8>) {
        self.manifests
            .lock()
            .await
            .insert(reference.to_string(), data);
    }

    /// Runs the server, handling one request at a time.
    pub async fn run(&self) -> std::io::Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            let config = self.config.clone();
            let request_count = self.request_count.clone();
            let manifest_digest_behavior = self.manifest_digest_behavior.clone();
            let blobs = self.blobs.clone();
            let manifests = self.manifests.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(
                    stream,
                    config,
                    request_count,
                    manifest_digest_behavior,
                    blobs,
                    manifests,
                )
                .await
                {
                    eprintln!("Connection error: {}", e);
                }
            });
        }
    }

    /// Runs the server for a single request.
    pub async fn handle_one(&self) -> std::io::Result<()> {
        let (stream, _) = self.listener.accept().await?;
        handle_connection(
            stream,
            self.config.clone(),
            self.request_count.clone(),
            self.manifest_digest_behavior.clone(),
            self.blobs.clone(),
            self.manifests.clone(),
        )
        .await
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    config: Arc<Mutex<FaultConfig>>,
    request_count: Arc<AtomicUsize>,
    manifest_digest_behavior: Arc<Mutex<DigestBehavior>>,
    blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    manifests: Arc<Mutex<HashMap<String, Vec<u8>>>>,
) -> std::io::Result<()> {
    // Read request
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buf[..n]);
    let count = request_count.fetch_add(1, Ordering::SeqCst);

    // Determine which fault to inject
    let fault = {
        let cfg = config.lock().await;
        cfg.fault_for_count(count)
    };

    // Parse request path
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let digest_behavior = { manifest_digest_behavior.lock().await.clone() };

    match fault {
        Fault::HttpError(status) => {
            let response = format!(
                "HTTP/1.1 {} Error\r\nContent-Length: 0\r\n\r\n",
                status
            );
            stream.write_all(response.as_bytes()).await?;
        }
        Fault::ConnectionReset => {
            // Just drop the connection
            drop(stream);
        }
        Fault::Delay(duration) => {
            tokio::time::sleep(duration).await;
            send_success_response(&mut stream, path, &blobs, &manifests, &digest_behavior).await?;
        }
        Fault::PartialResponse(bytes) => {
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n";
            stream.write_all(response.as_bytes()).await?;
            stream.write_all(&vec![0u8; bytes]).await?;
            // Close without sending the rest
            drop(stream);
        }
        Fault::Timeout => {
            // Never respond
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        Fault::Success => {
            send_success_response(&mut stream, path, &blobs, &manifests, &digest_behavior).await?;
        }
    }

    Ok(())
}

/// Computes SHA-256 hash and returns hex string.
fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(result)
}

async fn send_success_response(
    stream: &mut TcpStream,
    path: &str,
    blobs: &Arc<Mutex<HashMap<String, Vec<u8>>>>,
    manifests: &Arc<Mutex<HashMap<String, Vec<u8>>>>,
    digest_behavior: &DigestBehavior,
) -> std::io::Result<()> {
    // Handle /v2/ ping
    if path == "/v2/" {
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        stream.write_all(response.as_bytes()).await?;
        return Ok(());
    }

    // Handle blob requests
    if path.contains("/blobs/") {
        if let Some(digest) = path.split("/blobs/").nth(1) {
            let blobs_map = blobs.lock().await;
            if let Some(data) = blobs_map.get(digest) {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nDocker-Content-Digest: {}\r\n\r\n",
                    data.len(),
                    digest
                );
                stream.write_all(response.as_bytes()).await?;
                stream.write_all(data).await?;
                return Ok(());
            }
        }
        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        stream.write_all(response.as_bytes()).await?;
        return Ok(());
    }

    // Handle manifest requests
    if path.contains("/manifests/") {
        if let Some(reference) = path.split("/manifests/").nth(1) {
            let manifests_map = manifests.lock().await;
            if let Some(data) = manifests_map.get(reference) {
                // Compute actual SHA-256 digest
                let digest = compute_sha256(data);
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.oci.image.manifest.v1+json\r\nContent-Length: {}\r\n",
                    data.len()
                );
                match digest_behavior {
                    DigestBehavior::Correct => {
                        response.push_str(&format!(
                            "Docker-Content-Digest: sha256:{}\r\n",
                            digest
                        ));
                    }
                    DigestBehavior::Wrong(bad_digest) => {
                        response.push_str(&format!(
                            "Docker-Content-Digest: {}\r\n",
                            bad_digest
                        ));
                    }
                    DigestBehavior::Omit => {}
                }
                response.push_str("\r\n");
                stream.write_all(response.as_bytes()).await?;
                stream.write_all(data).await?;
                return Ok(());
            }
        }
        let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        stream.write_all(response.as_bytes()).await?;
        return Ok(());
    }

    // Default 404
    let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

/// Helper to test retry behavior.
pub struct RetryCounter {
    count: AtomicUsize,
}

impl RetryCounter {
    /// Creates a new retry counter.
    pub fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }

    /// Increments and returns the count.
    pub fn increment(&self) -> usize {
        self.count.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Returns the current count.
    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// Resets the counter.
    pub fn reset(&self) {
        self.count.store(0, Ordering::SeqCst);
    }
}

impl Default for RetryCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fetch_raw_response(addr: SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            path, addr
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf).to_string()
    }

    #[tokio::test]
    async fn test_fault_server_creation() {
        let server = FaultInjectionServer::new(FaultConfig::always_succeed())
            .await
            .unwrap();
        assert!(server.addr().port() > 0);
    }

    #[tokio::test]
    async fn test_manifest_digest_behavior_omit() {
        let server = FaultInjectionServer::new(FaultConfig::always_succeed())
            .await
            .unwrap();
        server
            .add_manifest("test", b"{}".to_vec())
            .await;
        server
            .set_manifest_digest_behavior(DigestBehavior::Omit)
            .await;

        let addr = server.addr();
        let handle = tokio::spawn(async move {
            server.handle_one().await.unwrap();
        });

        let response = fetch_raw_response(addr, "/v2/repo/manifests/test").await;
        assert!(
            !response.to_ascii_lowercase().contains("docker-content-digest"),
            "expected digest header to be omitted"
        );

        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_manifest_digest_behavior_wrong() {
        let server = FaultInjectionServer::new(FaultConfig::always_succeed())
            .await
            .unwrap();
        server
            .add_manifest("test", b"{}".to_vec())
            .await;
        server
            .set_manifest_digest_behavior(DigestBehavior::Wrong(
                "sha256:badbadbad".to_string(),
            ))
            .await;

        let addr = server.addr();
        let handle = tokio::spawn(async move {
            server.handle_one().await.unwrap();
        });

        let response = fetch_raw_response(addr, "/v2/repo/manifests/test").await;
        assert!(
            response.contains("Docker-Content-Digest: sha256:badbadbad"),
            "expected wrong digest header"
        );

        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_fault_config_empty_defaults_to_success() {
        let config = FaultConfig {
            faults: Vec::new(),
            repeat: false,
        };
        let server = FaultInjectionServer::new(config).await.unwrap();
        server.add_manifest("test", b"{}".to_vec()).await;

        let addr = server.addr();
        let handle = tokio::spawn(async move {
            server.handle_one().await.unwrap();
        });

        let response = fetch_raw_response(addr, "/v2/repo/manifests/test").await;
        assert!(response.starts_with("HTTP/1.1 200 OK"));

        let _ = handle.await;
    }

    #[tokio::test]
    async fn test_fault_timeout_no_response() {
        let server = FaultInjectionServer::new(FaultConfig::always_fail(Fault::Timeout))
            .await
            .unwrap();

        let addr = server.addr();
        let handle = tokio::spawn(async move {
            server.handle_one().await.unwrap();
        });

        let result = tokio::time::timeout(
            Duration::from_millis(50),
            fetch_raw_response(addr, "/v2/"),
        )
        .await;
        assert!(result.is_err(), "expected timeout");
        handle.abort();
        let _ = handle.await;
    }

    #[test]
    fn test_fault_config_fail_then_succeed() {
        let config = FaultConfig::fail_then_succeed(3, Fault::HttpError(500));
        assert_eq!(config.faults.len(), 4);
    }

    #[test]
    fn test_retry_counter() {
        let counter = RetryCounter::new();
        assert_eq!(counter.count(), 0);
        assert_eq!(counter.increment(), 1);
        assert_eq!(counter.increment(), 2);
        assert_eq!(counter.count(), 2);
        counter.reset();
        assert_eq!(counter.count(), 0);
    }
}
