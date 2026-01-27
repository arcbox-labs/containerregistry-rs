//! Registry client for OCI distribution operations.
//!
//! This module provides the main client for interacting with OCI distribution
//! registries, implementing manifest and blob operations.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, LINK, RANGE, RETRY_AFTER,
    WWW_AUTHENTICATE,
};
use reqwest::{Client as HttpClient, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use tracing::{debug, instrument, trace, warn};

use containerregistry_auth::Credential;
use containerregistry_image::{Algorithm, Digest, ImageIndex, Manifest, MediaType};

use crate::metrics::{MetricsCollector, Operation};
use crate::reference::Reference;
use crate::{Error, Result};

/// A manifest or image index returned from a registry.
///
/// Since a tag can point to either a single manifest or a multi-platform
/// image index, this enum represents both possibilities.
#[derive(Clone, Debug)]
pub enum ManifestOrIndex {
    /// A single manifest (OCI or Docker).
    Manifest(Box<Manifest>),
    /// An image index / manifest list (OCI or Docker).
    Index(Box<ImageIndex>),
}

impl ManifestOrIndex {
    /// Parses bytes as either a manifest or index, auto-detecting the format.
    pub fn from_bytes(data: &[u8]) -> crate::Result<Self> {
        // Try to detect based on media type field
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct MediaTypeProbe {
            media_type: Option<String>,
        }

        let probe: MediaTypeProbe = serde_json::from_slice(data)?;

        match probe.media_type.as_deref() {
            Some("application/vnd.oci.image.index.v1+json")
            | Some("application/vnd.docker.distribution.manifest.list.v2+json") => Ok(
                ManifestOrIndex::Index(Box::new(ImageIndex::from_bytes(data)?)),
            ),
            Some("application/vnd.oci.image.manifest.v1+json")
            | Some("application/vnd.docker.distribution.manifest.v2+json") => Ok(
                ManifestOrIndex::Manifest(Box::new(Manifest::from_bytes(data)?)),
            ),
            _ => {
                // Try index first (has "manifests" array), then manifest
                if let Ok(index) = ImageIndex::from_bytes(data) {
                    Ok(ManifestOrIndex::Index(Box::new(index)))
                } else {
                    Ok(ManifestOrIndex::Manifest(Box::new(Manifest::from_bytes(
                        data,
                    )?)))
                }
            }
        }
    }

    /// Returns true if this is a manifest.
    pub fn is_manifest(&self) -> bool {
        matches!(self, ManifestOrIndex::Manifest(_))
    }

    /// Returns true if this is an index.
    pub fn is_index(&self) -> bool {
        matches!(self, ManifestOrIndex::Index(_))
    }

    /// Returns the manifest if this is a manifest, or None.
    pub fn as_manifest(&self) -> Option<&Manifest> {
        match self {
            ManifestOrIndex::Manifest(m) => Some(m),
            ManifestOrIndex::Index(_) => None,
        }
    }

    /// Returns the index if this is an index, or None.
    pub fn as_index(&self) -> Option<&ImageIndex> {
        match self {
            ManifestOrIndex::Manifest(_) => None,
            ManifestOrIndex::Index(i) => Some(i),
        }
    }

    /// Converts into the manifest if this is a manifest.
    pub fn into_manifest(self) -> Option<Manifest> {
        match self {
            ManifestOrIndex::Manifest(m) => Some(*m),
            ManifestOrIndex::Index(_) => None,
        }
    }

    /// Converts into the index if this is an index.
    pub fn into_index(self) -> Option<ImageIndex> {
        match self {
            ManifestOrIndex::Manifest(_) => None,
            ManifestOrIndex::Index(i) => Some(*i),
        }
    }
}

/// Default timeout for HTTP requests.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default number of retry attempts.
const DEFAULT_RETRIES: u32 = 3;

/// User-Agent header value.
const USER_AGENT_VALUE: &str = concat!("containerregistry-rs/", env!("CARGO_PKG_VERSION"));

/// OCI distribution API version prefix.
const API_PREFIX: &str = "/v2";

/// Configuration for the registry client.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Request timeout.
    pub timeout: Duration,

    /// Number of retry attempts for transient errors.
    pub retries: u32,

    /// Whether to use HTTPS (default: true).
    pub https: bool,

    /// Whether to accept invalid TLS certificates.
    pub insecure: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            retries: DEFAULT_RETRIES,
            https: true,
            insecure: false,
        }
    }
}

impl ClientConfig {
    /// Creates a new client configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the number of retry attempts.
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Sets whether to use HTTPS.
    pub fn with_https(mut self, https: bool) -> Self {
        self.https = https;
        self
    }

    /// Sets whether to accept invalid TLS certificates.
    pub fn with_insecure(mut self, insecure: bool) -> Self {
        self.insecure = insecure;
        self
    }
}

/// A client for interacting with OCI distribution registries.
#[derive(Clone)]
pub struct Client {
    http: HttpClient,
    config: ClientConfig,
    metrics: Option<Arc<MetricsCollector>>,
    credential: Option<Credential>,
    bearer_token: Arc<RwLock<Option<String>>>,
}

impl Client {
    /// Creates a new client with default configuration.
    pub fn new() -> Result<Self> {
        Self::with_config(ClientConfig::default())
    }

    /// Creates a new client with the given configuration.
    pub fn with_config(config: ClientConfig) -> Result<Self> {
        let mut builder = HttpClient::builder()
            .timeout(config.timeout)
            .user_agent(USER_AGENT_VALUE);

        if config.insecure {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let http = builder.build()?;

        Ok(Self {
            http,
            config,
            metrics: None,
            credential: None,
            bearer_token: Arc::new(RwLock::new(None)),
        })
    }

    /// Creates a new client with metrics collection enabled.
    pub fn with_metrics(config: ClientConfig, metrics: Arc<MetricsCollector>) -> Result<Self> {
        let mut client = Self::with_config(config)?;
        client.metrics = Some(metrics);
        Ok(client)
    }

    /// Creates a new client with a static credential.
    pub fn with_credential(config: ClientConfig, credential: Credential) -> Result<Self> {
        let mut client = Self::with_config(config)?;
        client.credential = Some(credential);
        Ok(client)
    }

    /// Returns the metrics collector if metrics are enabled.
    pub fn metrics(&self) -> Option<&MetricsCollector> {
        self.metrics.as_ref().map(|m| m.as_ref())
    }

    /// Records a successful operation with metrics.
    fn record_success(&self, op: Operation, start: Instant, bytes: u64) {
        if let Some(metrics) = &self.metrics {
            metrics.record_success(op, start.elapsed(), bytes);
        }
    }

    /// Records a failed operation with metrics.
    fn record_failure(&self, op: Operation, start: Instant) {
        if let Some(metrics) = &self.metrics {
            metrics.record_failure(op, start.elapsed());
        }
    }

    /// Records a retry with metrics.
    fn record_retry(&self, op: Operation) {
        if let Some(metrics) = &self.metrics {
            metrics.record_retry(op);
        }
    }

    fn auth_header_value(&self) -> Option<String> {
        if let Ok(guard) = self.bearer_token.read()
            && let Some(token) = guard.as_ref()
        {
            return Some(format!("Bearer {}", token));
        }
        self.credential
            .as_ref()
            .and_then(|c| c.authorization_header())
    }

    fn apply_auth(&self, builder: RequestBuilder) -> RequestBuilder {
        if let Some(value) = self.auth_header_value() {
            builder.header(AUTHORIZATION, value)
        } else {
            builder
        }
    }

    fn set_bearer_token(&self, token: String) {
        if let Ok(mut guard) = self.bearer_token.write() {
            *guard = Some(token);
        }
    }

    /// Records a successful upload operation with metrics.
    fn record_upload(&self, op: Operation, start: Instant, bytes: u64) {
        if let Some(metrics) = &self.metrics {
            metrics.record_upload(op, start.elapsed(), bytes);
        }
    }

    /// Returns the scheme to use for requests.
    fn scheme(&self) -> &str {
        if self.config.https { "https" } else { "http" }
    }

    /// Builds a URL for the given registry and path.
    fn url(&self, registry: &str, path: &str) -> String {
        format!("{}://{}{}{}", self.scheme(), registry, API_PREFIX, path)
    }

    /// Pings the registry to check if it supports the OCI distribution API.
    ///
    /// Returns `Ok(())` if the registry responds with a 200 OK to `/v2/`.
    #[instrument(skip(self), level = "debug")]
    pub async fn ping(&self, registry: &str) -> Result<()> {
        let start = Instant::now();
        let url = self.url(registry, "/");
        let response = self
            .execute_with_retry_op(
                || async { self.apply_auth(self.http.get(&url)).send().await },
                Some(Operation::Ping),
            )
            .await;

        match response {
            Ok(resp) => match resp.status() {
                StatusCode::OK => {
                    self.record_success(Operation::Ping, start, 0);
                    Ok(())
                }
                StatusCode::UNAUTHORIZED => {
                    self.record_failure(Operation::Ping, start);
                    Err(Error::Unauthorized(format!(
                        "registry {} requires authentication",
                        registry
                    )))
                }
                status => {
                    self.record_failure(Operation::Ping, start);
                    Err(Error::UnexpectedStatus {
                        status: status.as_u16(),
                        message: format!("ping failed for {}", registry),
                    })
                }
            },
            Err(e) => {
                self.record_failure(Operation::Ping, start);
                Err(e)
            }
        }
    }

    /// Gets a manifest or index from the registry.
    ///
    /// Returns the manifest/index and its content digest. Note that multi-architecture
    /// tags may return an ImageIndex rather than a Manifest.
    #[instrument(skip(self), level = "debug", fields(reference = %reference))]
    pub async fn get_manifest(&self, reference: &Reference) -> Result<(ManifestOrIndex, Digest)> {
        let start = Instant::now();
        let path = format!(
            "/{}/manifests/{}",
            reference.repository(),
            reference.reference()
        );
        let url = self.url(reference.registry(), &path);

        let response = self
            .execute_with_retry_op(
                || async {
                    self.apply_auth(self.http.get(&url).header(ACCEPT, accept_manifest_types()))
                        .send()
                        .await
                },
                Some(Operation::GetManifest),
            )
            .await;

        match response {
            Ok(resp) => {
                let result = self
                    .handle_manifest_response(resp, reference.digest())
                    .await;
                match &result {
                    Ok((_, _, body_size)) => {
                        self.record_success(Operation::GetManifest, start, *body_size)
                    }
                    Err(_) => self.record_failure(Operation::GetManifest, start),
                }
                result.map(|(manifest, digest, _)| (manifest, digest))
            }
            Err(e) => {
                self.record_failure(Operation::GetManifest, start);
                Err(e)
            }
        }
    }

    /// Gets a manifest's metadata without downloading the full body.
    ///
    /// Returns the content type, digest, and size.
    #[instrument(skip(self), level = "debug", fields(reference = %reference))]
    pub async fn head_manifest(&self, reference: &Reference) -> Result<(MediaType, Digest, u64)> {
        let start = Instant::now();
        let path = format!(
            "/{}/manifests/{}",
            reference.repository(),
            reference.reference()
        );
        let url = self.url(reference.registry(), &path);

        let response = self
            .execute_with_retry_op(
                || async {
                    self.apply_auth(self.http.head(&url).header(ACCEPT, accept_manifest_types()))
                        .send()
                        .await
                },
                Some(Operation::HeadManifest),
            )
            .await;

        match response {
            Ok(resp) => match resp.status() {
                StatusCode::OK => {
                    let content_type = resp
                        .headers()
                        .get(CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("application/vnd.oci.image.manifest.v1+json");

                    // Strip Content-Type parameters (e.g., "; charset=utf-8")
                    let media_type_str = content_type
                        .split(';')
                        .next()
                        .unwrap_or(content_type)
                        .trim();

                    let media_type: MediaType =
                        media_type_str.parse().unwrap_or(MediaType::OciManifest);

                    let status = resp.status().as_u16();
                    let digest = Self::parse_optional_digest_header(resp.headers(), status)?
                        .ok_or_else(|| Error::UnexpectedStatus {
                            status,
                            message: "missing docker-content-digest header".to_string(),
                        })?;

                    let size = resp
                        .headers()
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);

                    self.record_success(Operation::HeadManifest, start, 0);
                    Ok((media_type, digest, size))
                }
                StatusCode::NOT_FOUND => {
                    self.record_failure(Operation::HeadManifest, start);
                    Err(Error::NotFound(reference.to_string()))
                }
                StatusCode::UNAUTHORIZED => {
                    self.record_failure(Operation::HeadManifest, start);
                    Err(Error::Unauthorized(reference.to_string()))
                }
                StatusCode::FORBIDDEN => {
                    self.record_failure(Operation::HeadManifest, start);
                    Err(Error::Forbidden(reference.to_string()))
                }
                status => {
                    self.record_failure(Operation::HeadManifest, start);
                    Err(Error::UnexpectedStatus {
                        status: status.as_u16(),
                        message: format!("HEAD manifest failed for {}", reference),
                    })
                }
            },
            Err(e) => {
                self.record_failure(Operation::HeadManifest, start);
                Err(e)
            }
        }
    }

    /// Puts a manifest to the registry.
    ///
    /// Returns the digest of the uploaded manifest.
    #[instrument(skip(self, manifest), level = "debug", fields(reference = %reference))]
    pub async fn put_manifest(&self, reference: &Reference, manifest: &Manifest) -> Result<Digest> {
        let start = Instant::now();
        let path = format!(
            "/{}/manifests/{}",
            reference.repository(),
            reference.reference()
        );
        let url = self.url(reference.registry(), &path);

        let body = manifest.to_bytes()?;
        let body_size = body.len() as u64;
        let computed_sha256 = Digest::sha256(&body);

        // Validate digest if reference is by digest
        if let Some(expected_digest) = reference.digest() {
            let computed =
                Self::digest_for_algorithm(expected_digest.algorithm(), &body, &computed_sha256);
            if &computed != expected_digest {
                self.record_failure(Operation::PutManifest, start);
                return Err(Error::DigestMismatch {
                    expected: expected_digest.to_string(),
                    actual: computed.to_string(),
                });
            }
        }

        let media_type = manifest.media_type();
        let content_type = media_type.as_str();

        let response = match self
            .apply_auth(
                self.http
                    .put(&url)
                    .header(CONTENT_TYPE, content_type)
                    .body(body.clone()),
            )
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                self.record_failure(Operation::PutManifest, start);
                return Err(self.map_transport_error(e));
            }
        };

        match response.status() {
            StatusCode::CREATED | StatusCode::OK => {
                let status = response.status().as_u16();
                // Verify digest header if present
                if let Some(header_digest) =
                    Self::parse_optional_digest_header(response.headers(), status)?
                {
                    let computed = Self::digest_for_algorithm(
                        header_digest.algorithm(),
                        &body,
                        &computed_sha256,
                    );
                    if header_digest != computed {
                        self.record_failure(Operation::PutManifest, start);
                        return Err(Error::DigestMismatch {
                            expected: computed.to_string(),
                            actual: header_digest.to_string(),
                        });
                    }
                }
                self.record_upload(Operation::PutManifest, start, body_size);
                Ok(computed_sha256)
            }
            StatusCode::UNAUTHORIZED => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::Unauthorized(reference.to_string()))
            }
            StatusCode::FORBIDDEN => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::Forbidden(reference.to_string()))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::RateLimited {
                    retry_after: Self::parse_retry_after(response.headers()),
                    message: format!("rate limited while putting manifest {}", reference),
                })
            }
            status => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::UnexpectedStatus {
                    status: status.as_u16(),
                    message: format!("PUT manifest failed for {}", reference),
                })
            }
        }
    }

    /// Puts an image index (manifest list) to the registry.
    ///
    /// Returns the digest of the uploaded index.
    #[instrument(skip(self, index), level = "debug", fields(reference = %reference))]
    pub async fn put_index(&self, reference: &Reference, index: &ImageIndex) -> Result<Digest> {
        let start = Instant::now();
        let path = format!(
            "/{}/manifests/{}",
            reference.repository(),
            reference.reference()
        );
        let url = self.url(reference.registry(), &path);

        let body = index.to_bytes()?;
        let body_size = body.len() as u64;
        let computed_sha256 = Digest::sha256(&body);

        // Validate digest if reference is by digest
        if let Some(expected_digest) = reference.digest() {
            let computed =
                Self::digest_for_algorithm(expected_digest.algorithm(), &body, &computed_sha256);
            if &computed != expected_digest {
                self.record_failure(Operation::PutManifest, start);
                return Err(Error::DigestMismatch {
                    expected: expected_digest.to_string(),
                    actual: computed.to_string(),
                });
            }
        }

        let media_type = index.media_type();
        let content_type = media_type.as_str();

        let response = match self
            .apply_auth(
                self.http
                    .put(&url)
                    .header(CONTENT_TYPE, content_type)
                    .body(body.clone()),
            )
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                self.record_failure(Operation::PutManifest, start);
                return Err(self.map_transport_error(e));
            }
        };

        match response.status() {
            StatusCode::CREATED | StatusCode::OK => {
                let status = response.status().as_u16();
                // Verify digest header if present
                if let Some(header_digest) =
                    Self::parse_optional_digest_header(response.headers(), status)?
                {
                    let computed = Self::digest_for_algorithm(
                        header_digest.algorithm(),
                        &body,
                        &computed_sha256,
                    );
                    if header_digest != computed {
                        self.record_failure(Operation::PutManifest, start);
                        return Err(Error::DigestMismatch {
                            expected: computed.to_string(),
                            actual: header_digest.to_string(),
                        });
                    }
                }
                self.record_upload(Operation::PutManifest, start, body_size);
                Ok(computed_sha256)
            }
            StatusCode::UNAUTHORIZED => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::Unauthorized(reference.to_string()))
            }
            StatusCode::FORBIDDEN => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::Forbidden(reference.to_string()))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::RateLimited {
                    retry_after: Self::parse_retry_after(response.headers()),
                    message: format!("rate limited while putting index {}", reference),
                })
            }
            status => {
                self.record_failure(Operation::PutManifest, start);
                Err(Error::UnexpectedStatus {
                    status: status.as_u16(),
                    message: format!("PUT index failed for {}", reference),
                })
            }
        }
    }

    /// Gets a blob from the registry.
    ///
    /// Returns the blob data. For large blobs, consider using `get_blob_stream`.
    #[instrument(skip(self), level = "debug", fields(reference = %reference, digest = %digest))]
    pub async fn get_blob(&self, reference: &Reference, digest: &Digest) -> Result<Vec<u8>> {
        let start = Instant::now();
        let path = format!("/{}/blobs/{}", reference.repository(), digest);
        let url = self.url(reference.registry(), &path);

        let response = match self
            .execute_with_retry_op(
                || async { self.apply_auth(self.http.get(&url)).send().await },
                Some(Operation::GetBlob),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                self.record_failure(Operation::GetBlob, start);
                return Err(e);
            }
        };

        match response.status() {
            StatusCode::OK => {
                let data = response
                    .bytes()
                    .await
                    .map_err(|e| self.map_transport_error(e))?;
                // Verify digest using the requested algorithm
                let computed_sha256 = Digest::sha256(&data);
                let computed =
                    Self::digest_for_algorithm(digest.algorithm(), &data, &computed_sha256);
                if computed != *digest {
                    self.record_failure(Operation::GetBlob, start);
                    return Err(Error::DigestMismatch {
                        expected: digest.to_string(),
                        actual: computed.to_string(),
                    });
                }
                self.record_success(Operation::GetBlob, start, data.len() as u64);
                Ok(data.to_vec())
            }
            StatusCode::NOT_FOUND => {
                self.record_failure(Operation::GetBlob, start);
                Err(Error::NotFound(digest.to_string()))
            }
            StatusCode::UNAUTHORIZED => {
                self.record_failure(Operation::GetBlob, start);
                Err(Error::Unauthorized(reference.to_string()))
            }
            StatusCode::FORBIDDEN => {
                self.record_failure(Operation::GetBlob, start);
                Err(Error::Forbidden(reference.to_string()))
            }
            status => {
                self.record_failure(Operation::GetBlob, start);
                Err(Error::UnexpectedStatus {
                    status: status.as_u16(),
                    message: format!("GET blob failed for {}", digest),
                })
            }
        }
    }

    /// Checks if a blob exists in the registry.
    ///
    /// Returns the size if the blob exists.
    #[instrument(skip(self), level = "debug", fields(reference = %reference, digest = %digest))]
    pub async fn head_blob(&self, reference: &Reference, digest: &Digest) -> Result<u64> {
        let start = Instant::now();
        let path = format!("/{}/blobs/{}", reference.repository(), digest);
        let url = self.url(reference.registry(), &path);

        let response = self
            .execute_with_retry_op(
                || async { self.apply_auth(self.http.head(&url)).send().await },
                Some(Operation::HeadBlob),
            )
            .await;

        match response {
            Ok(resp) => match resp.status() {
                StatusCode::OK => {
                    let size = resp
                        .headers()
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    self.record_success(Operation::HeadBlob, start, 0);
                    Ok(size)
                }
                StatusCode::NOT_FOUND => {
                    self.record_failure(Operation::HeadBlob, start);
                    Err(Error::NotFound(digest.to_string()))
                }
                StatusCode::UNAUTHORIZED => {
                    self.record_failure(Operation::HeadBlob, start);
                    Err(Error::Unauthorized(reference.to_string()))
                }
                StatusCode::FORBIDDEN => {
                    self.record_failure(Operation::HeadBlob, start);
                    Err(Error::Forbidden(reference.to_string()))
                }
                status => {
                    self.record_failure(Operation::HeadBlob, start);
                    Err(Error::UnexpectedStatus {
                        status: status.as_u16(),
                        message: format!("HEAD blob failed for {}", digest),
                    })
                }
            },
            Err(e) => {
                self.record_failure(Operation::HeadBlob, start);
                Err(e)
            }
        }
    }

    /// Uploads a blob to the registry.
    ///
    /// This uses the monolithic upload method for simplicity.
    /// For large blobs, consider using chunked upload.
    #[instrument(skip(self, data), level = "debug", fields(reference = %reference, size = data.len()))]
    pub async fn put_blob(&self, reference: &Reference, data: &[u8]) -> Result<Digest> {
        let digest = Digest::sha256(data);
        self.put_blob_with_digest(reference, data, &digest).await
    }

    /// Uploads a blob to the registry with a specified digest.
    ///
    /// Returns the digest of the uploaded blob.
    #[instrument(skip(self, data), level = "debug", fields(reference = %reference, digest = %digest, size = data.len()))]
    pub async fn put_blob_with_digest(
        &self,
        reference: &Reference,
        data: &[u8],
        digest: &Digest,
    ) -> Result<Digest> {
        let start = Instant::now();
        let data_size = data.len() as u64;
        let computed_sha256 = Digest::sha256(data);
        let computed = Self::digest_for_algorithm(digest.algorithm(), data, &computed_sha256);
        if &computed != digest {
            self.record_failure(Operation::PutBlob, start);
            return Err(Error::DigestMismatch {
                expected: digest.to_string(),
                actual: computed.to_string(),
            });
        }

        // Check if blob already exists (head_blob has its own metrics)
        if self.head_blob(reference, digest).await.is_ok() {
            // Blob already exists, count as success with 0 uploaded bytes
            self.record_success(Operation::PutBlob, start, 0);
            return Ok(digest.clone());
        }

        // Initiate upload
        let path = format!("/{}/blobs/uploads/", reference.repository());
        let url = self.url(reference.registry(), &path);

        let response = match self.apply_auth(self.http.post(&url)).send().await {
            Ok(resp) => resp,
            Err(e) => {
                self.record_failure(Operation::PutBlob, start);
                return Err(self.map_transport_error(e));
            }
        };

        let upload_url = match response.status() {
            StatusCode::ACCEPTED => response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| {
                    if s.starts_with('/') {
                        format!("{}://{}{}", self.scheme(), reference.registry(), s)
                    } else {
                        s.to_string()
                    }
                })
                .ok_or_else(|| Error::UnexpectedStatus {
                    status: 202,
                    message: "missing Location header in upload response".to_string(),
                })?,
            StatusCode::UNAUTHORIZED => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::Unauthorized(reference.to_string()));
            }
            StatusCode::FORBIDDEN => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::Forbidden(reference.to_string()));
            }
            StatusCode::TOO_MANY_REQUESTS => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::RateLimited {
                    retry_after: Self::parse_retry_after(response.headers()),
                    message: format!(
                        "rate limited while initiating blob upload for {}",
                        reference
                    ),
                });
            }
            status => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::UnexpectedStatus {
                    status: status.as_u16(),
                    message: format!("POST blob upload failed for {}", reference),
                });
            }
        };

        // Complete monolithic upload
        let separator = if upload_url.contains('?') { "&" } else { "?" };
        let final_url = format!("{}{}digest={}", upload_url, separator, digest);

        let response = match self
            .apply_auth(
                self.http
                    .put(&final_url)
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(data.to_vec()),
            )
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                self.record_failure(Operation::PutBlob, start);
                return Err(self.map_transport_error(e));
            }
        };

        match response.status() {
            StatusCode::CREATED => {
                self.record_upload(Operation::PutBlob, start, data_size);
                Ok(digest.clone())
            }
            StatusCode::UNAUTHORIZED => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::Unauthorized(reference.to_string()))
            }
            StatusCode::FORBIDDEN => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::Forbidden(reference.to_string()))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::RateLimited {
                    retry_after: Self::parse_retry_after(response.headers()),
                    message: format!("rate limited while uploading blob for {}", reference),
                })
            }
            status => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::UnexpectedStatus {
                    status: status.as_u16(),
                    message: format!("PUT blob upload failed for {}", reference),
                })
            }
        }
    }

    /// Uploads a blob using chunked PATCH requests.
    pub async fn put_blob_chunked(
        &self,
        reference: &Reference,
        data: &[u8],
        chunk_size: usize,
    ) -> Result<Digest> {
        let digest = Digest::sha256(data);
        self.put_blob_chunked_with_digest(reference, data, &digest, chunk_size)
            .await
    }

    /// Uploads a blob using chunked PATCH requests with a specified digest.
    pub async fn put_blob_chunked_with_digest(
        &self,
        reference: &Reference,
        data: &[u8],
        digest: &Digest,
        chunk_size: usize,
    ) -> Result<Digest> {
        let start = Instant::now();
        let data_size = data.len() as u64;
        let computed_sha256 = Digest::sha256(data);
        let computed = Self::digest_for_algorithm(digest.algorithm(), data, &computed_sha256);
        if &computed != digest {
            self.record_failure(Operation::PutBlob, start);
            return Err(Error::DigestMismatch {
                expected: digest.to_string(),
                actual: computed.to_string(),
            });
        }

        let path = format!("/{}/blobs/uploads/", reference.repository());
        let url = self.url(reference.registry(), &path);

        let response = match self.apply_auth(self.http.post(&url)).send().await {
            Ok(resp) => resp,
            Err(e) => {
                self.record_failure(Operation::PutBlob, start);
                return Err(self.map_transport_error(e));
            }
        };

        let mut upload_url = match response.status() {
            StatusCode::ACCEPTED => response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| {
                    if s.starts_with('/') {
                        format!("{}://{}{}", self.scheme(), reference.registry(), s)
                    } else {
                        s.to_string()
                    }
                })
                .ok_or_else(|| Error::UnexpectedStatus {
                    status: 202,
                    message: "missing Location header in upload response".to_string(),
                })?,
            StatusCode::UNAUTHORIZED => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::Unauthorized(reference.to_string()));
            }
            StatusCode::FORBIDDEN => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::Forbidden(reference.to_string()));
            }
            StatusCode::TOO_MANY_REQUESTS => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::RateLimited {
                    retry_after: Self::parse_retry_after(response.headers()),
                    message: format!(
                        "rate limited while initiating blob upload for {}",
                        reference
                    ),
                });
            }
            status => {
                self.record_failure(Operation::PutBlob, start);
                return Err(Error::UnexpectedStatus {
                    status: status.as_u16(),
                    message: format!("POST blob upload failed for {}", reference),
                });
            }
        };

        let mut offset = 0usize;
        let chunk_size = chunk_size.max(1);

        while offset < data.len() {
            let end = (offset + chunk_size).min(data.len());
            let chunk = &data[offset..end];

            let response = match self
                .apply_auth(
                    self.http
                        .patch(&upload_url)
                        .header(CONTENT_TYPE, "application/octet-stream")
                        .header("Content-Range", format!("{}-{}", offset, end - 1))
                        .body(chunk.to_vec()),
                )
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    self.record_failure(Operation::PutBlob, start);
                    return Err(self.map_transport_error(e));
                }
            };

            match response.status() {
                StatusCode::ACCEPTED => {
                    if let Some(location) = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                    {
                        upload_url = if location.starts_with('/') {
                            format!("{}://{}{}", self.scheme(), reference.registry(), location)
                        } else {
                            location.to_string()
                        };
                    }
                    if let Some(range_end) = Self::parse_range_end(response.headers()) {
                        offset = range_end.saturating_add(1);
                    } else {
                        offset = end;
                    }
                }
                StatusCode::UNAUTHORIZED => {
                    self.record_failure(Operation::PutBlob, start);
                    return Err(Error::Unauthorized(reference.to_string()));
                }
                StatusCode::FORBIDDEN => {
                    self.record_failure(Operation::PutBlob, start);
                    return Err(Error::Forbidden(reference.to_string()));
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    self.record_failure(Operation::PutBlob, start);
                    return Err(Error::RateLimited {
                        retry_after: Self::parse_retry_after(response.headers()),
                        message: format!("rate limited while uploading blob for {}", reference),
                    });
                }
                status => {
                    self.record_failure(Operation::PutBlob, start);
                    return Err(Error::UnexpectedStatus {
                        status: status.as_u16(),
                        message: format!("PATCH blob upload failed for {}", reference),
                    });
                }
            }
        }

        let separator = if upload_url.contains('?') { "&" } else { "?" };
        let final_url = format!("{}{}digest={}", upload_url, separator, digest);

        let response = match self
            .apply_auth(
                self.http
                    .put(&final_url)
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(data.to_vec()),
            )
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                self.record_failure(Operation::PutBlob, start);
                return Err(self.map_transport_error(e));
            }
        };

        match response.status() {
            StatusCode::CREATED => {
                self.record_upload(Operation::PutBlob, start, data_size);
                Ok(digest.clone())
            }
            StatusCode::UNAUTHORIZED => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::Unauthorized(reference.to_string()))
            }
            StatusCode::FORBIDDEN => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::Forbidden(reference.to_string()))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::RateLimited {
                    retry_after: Self::parse_retry_after(response.headers()),
                    message: format!("rate limited while uploading blob for {}", reference),
                })
            }
            status => {
                self.record_failure(Operation::PutBlob, start);
                Err(Error::UnexpectedStatus {
                    status: status.as_u16(),
                    message: format!("PUT blob upload failed for {}", reference),
                })
            }
        }
    }

    /// Attempts to mount a blob from another repository.
    ///
    /// Returns true if the blob was mounted, false if the registry requested an upload.
    pub async fn mount_blob(
        &self,
        reference: &Reference,
        from_repo: &str,
        digest: &Digest,
    ) -> Result<bool> {
        let path = format!("/{}/blobs/uploads/", reference.repository());
        let url = self.url(reference.registry(), &path);

        let response = match self
            .apply_auth(self.http.post(&url).query(&[
                ("mount", digest.to_string()),
                ("from", from_repo.to_string()),
            ]))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => return Err(self.map_transport_error(e)),
        };

        match response.status() {
            StatusCode::CREATED => Ok(true),
            StatusCode::ACCEPTED => Ok(false),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized(reference.to_string())),
            StatusCode::FORBIDDEN => Err(Error::Forbidden(reference.to_string())),
            status => Err(Error::UnexpectedStatus {
                status: status.as_u16(),
                message: format!("mount blob failed for {}", reference),
            }),
        }
    }

    /// Deletes a manifest by tag or digest reference.
    pub async fn delete_manifest(&self, reference: &Reference) -> Result<()> {
        let path = format!(
            "/{}/manifests/{}",
            reference.repository(),
            reference.reference()
        );
        let url = self.url(reference.registry(), &path);

        let response = match self.apply_auth(self.http.delete(&url)).send().await {
            Ok(resp) => resp,
            Err(e) => return Err(self.map_transport_error(e)),
        };

        match response.status() {
            StatusCode::ACCEPTED | StatusCode::OK => Ok(()),
            StatusCode::NOT_FOUND => Err(Error::NotFound(reference.to_string())),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized(reference.to_string())),
            StatusCode::FORBIDDEN => Err(Error::Forbidden(reference.to_string())),
            status => Err(Error::UnexpectedStatus {
                status: status.as_u16(),
                message: format!("delete manifest failed for {}", reference),
            }),
        }
    }

    /// Deletes a blob by digest.
    pub async fn delete_blob(&self, reference: &Reference, digest: &Digest) -> Result<()> {
        let path = format!("/{}/blobs/{}", reference.repository(), digest);
        let url = self.url(reference.registry(), &path);

        let response = match self.apply_auth(self.http.delete(&url)).send().await {
            Ok(resp) => resp,
            Err(e) => return Err(self.map_transport_error(e)),
        };

        match response.status() {
            StatusCode::ACCEPTED | StatusCode::OK => Ok(()),
            StatusCode::NOT_FOUND => Err(Error::NotFound(digest.to_string())),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized(reference.to_string())),
            StatusCode::FORBIDDEN => Err(Error::Forbidden(reference.to_string())),
            status => Err(Error::UnexpectedStatus {
                status: status.as_u16(),
                message: format!("delete blob failed for {}", reference),
            }),
        }
    }

    /// Gets referrers for the given subject digest.
    pub async fn get_referrers(
        &self,
        reference: &Reference,
        digest: &Digest,
    ) -> Result<ImageIndex> {
        let path = format!("/{}/referrers/{}", reference.repository(), digest);
        let url = self.url(reference.registry(), &path);

        let response = self
            .execute_with_retry(|| async {
                self.apply_auth(
                    self.http
                        .get(&url)
                        .header(ACCEPT, "application/vnd.oci.image.index.v1+json"),
                )
                .send()
                .await
            })
            .await?;

        match response.status() {
            StatusCode::OK => {
                let body = response
                    .bytes()
                    .await
                    .map_err(|e| self.map_transport_error(e))?;
                ImageIndex::from_bytes(&body).map_err(Error::from)
            }
            StatusCode::NOT_FOUND => Err(Error::NotFound(digest.to_string())),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized(reference.to_string())),
            StatusCode::FORBIDDEN => Err(Error::Forbidden(reference.to_string())),
            status => Err(Error::UnexpectedStatus {
                status: status.as_u16(),
                message: "get referrers failed".to_string(),
            }),
        }
    }

    /// Lists tags for a repository.
    #[instrument(skip(self), level = "debug", fields(reference = %reference))]
    pub async fn list_tags(&self, reference: &Reference) -> Result<Vec<String>> {
        let start = Instant::now();
        let path = format!("/{}/tags/list", reference.repository());
        let mut url = self.url(reference.registry(), &path);
        let mut tags = Vec::new();

        loop {
            let response = self
                .execute_with_retry_op(
                    || async { self.apply_auth(self.http.get(&url)).send().await },
                    Some(Operation::ListTags),
                )
                .await;

            match response {
                Ok(resp) => match resp.status() {
                    StatusCode::OK => {
                        let next_link = Self::parse_link_next(resp.headers())
                            .map(|link| self.resolve_link(reference.registry(), &link));
                        let body: TagList =
                            resp.json().await.map_err(|e| self.map_transport_error(e))?;
                        // Handle null tags from some registries
                        tags.extend(body.tags.unwrap_or_default());
                        if let Some(next) = next_link {
                            url = next;
                            continue;
                        }
                        self.record_success(Operation::ListTags, start, 0);
                        return Ok(tags);
                    }
                    StatusCode::NOT_FOUND => {
                        self.record_failure(Operation::ListTags, start);
                        return Err(Error::NotFound(reference.repository().to_string()));
                    }
                    StatusCode::UNAUTHORIZED => {
                        self.record_failure(Operation::ListTags, start);
                        return Err(Error::Unauthorized(reference.to_string()));
                    }
                    StatusCode::FORBIDDEN => {
                        self.record_failure(Operation::ListTags, start);
                        return Err(Error::Forbidden(reference.to_string()));
                    }
                    status => {
                        self.record_failure(Operation::ListTags, start);
                        return Err(Error::UnexpectedStatus {
                            status: status.as_u16(),
                            message: format!("list tags failed for {}", reference),
                        });
                    }
                },
                Err(e) => {
                    self.record_failure(Operation::ListTags, start);
                    return Err(e);
                }
            }
        }
    }

    /// Handles a manifest response, parsing it into a ManifestOrIndex and extracting the digest.
    ///
    /// Returns the manifest/index, digest, and the response body size in bytes.
    async fn handle_manifest_response(
        &self,
        response: Response,
        expected_digest: Option<&Digest>,
    ) -> Result<(ManifestOrIndex, Digest, u64)> {
        match response.status() {
            StatusCode::OK => {
                let status = response.status().as_u16();
                // Parse digest header if present (error on malformed)
                let header_digest = Self::parse_optional_digest_header(response.headers(), status)?;

                let body = response
                    .bytes()
                    .await
                    .map_err(|e| self.map_transport_error(e))?;
                let computed_sha256 = Digest::sha256(&body);

                // Verify digest if header is present
                if let Some(ref header) = header_digest {
                    let computed =
                        Self::digest_for_algorithm(header.algorithm(), &body, &computed_sha256);
                    if header != &computed {
                        return Err(Error::DigestMismatch {
                            expected: header.to_string(),
                            actual: computed.to_string(),
                        });
                    }
                }

                // Verify digest if reference was by digest
                if let Some(expected) = expected_digest {
                    let computed =
                        Self::digest_for_algorithm(expected.algorithm(), &body, &computed_sha256);
                    if &computed != expected {
                        return Err(Error::DigestMismatch {
                            expected: expected.to_string(),
                            actual: computed.to_string(),
                        });
                    }
                }

                let body_size = body.len() as u64;
                let manifest_or_index = ManifestOrIndex::from_bytes(&body)?;

                let result_digest = if let Some(header) = header_digest {
                    header
                } else if let Some(expected) = expected_digest {
                    Self::digest_for_algorithm(expected.algorithm(), &body, &computed_sha256)
                } else {
                    computed_sha256
                };

                Ok((manifest_or_index, result_digest, body_size))
            }
            StatusCode::NOT_FOUND => Err(Error::NotFound("manifest not found".to_string())),
            StatusCode::UNAUTHORIZED => Err(Error::Unauthorized("unauthorized".to_string())),
            StatusCode::FORBIDDEN => Err(Error::Forbidden("forbidden".to_string())),
            status => Err(Error::UnexpectedStatus {
                status: status.as_u16(),
                message: "failed to get manifest".to_string(),
            }),
        }
    }

    fn parse_digest_header_value(header_value: &HeaderValue, status: u16) -> Result<Digest> {
        let header_str = header_value.to_str().map_err(|_| Error::UnexpectedStatus {
            status,
            message: "malformed docker-content-digest header".to_string(),
        })?;
        let header_str = header_str.trim();
        if header_str.is_empty() {
            return Err(Error::UnexpectedStatus {
                status,
                message: "invalid docker-content-digest header: empty".to_string(),
            });
        }
        header_str.parse().map_err(|_| Error::UnexpectedStatus {
            status,
            message: format!("invalid docker-content-digest header: {}", header_str),
        })
    }

    fn parse_optional_digest_header(headers: &HeaderMap, status: u16) -> Result<Option<Digest>> {
        match headers.get("docker-content-digest") {
            Some(value) => Ok(Some(Self::parse_digest_header_value(value, status)?)),
            None => Ok(None),
        }
    }

    fn digest_for_algorithm(algorithm: Algorithm, data: &[u8], computed_sha256: &Digest) -> Digest {
        match algorithm {
            Algorithm::Sha256 => computed_sha256.clone(),
            Algorithm::Sha384 => Digest::sha384(data),
            Algorithm::Sha512 => Digest::sha512(data),
        }
    }

    /// Executes a request with retry logic for transient errors.
    #[allow(dead_code)]
    async fn execute_with_retry<F, Fut>(&self, f: F) -> Result<Response>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Response, reqwest::Error>>,
    {
        self.execute_with_retry_op(f, None).await
    }

    /// Executes a request with retry logic for transient errors, tracking retries for the given operation.
    async fn execute_with_retry_op<F, Fut>(&self, f: F, op: Option<Operation>) -> Result<Response>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<Response, reqwest::Error>>,
    {
        let mut last_error = None;

        for attempt in 0..=self.config.retries {
            let mut delay_override = None;
            let will_retry;
            match f().await {
                Ok(response) => {
                    let status = response.status();

                    // Handle bearer auth challenges (401)
                    if status == StatusCode::UNAUTHORIZED {
                        if let Some(challenge) = Self::parse_bearer_challenge(response.headers()) {
                            match self.fetch_bearer_token(&challenge).await {
                                Ok(token) => {
                                    self.set_bearer_token(token);
                                    if attempt == self.config.retries {
                                        return Ok(response);
                                    }
                                    last_error = Some(Error::Unauthorized(
                                        "bearer auth challenge".to_string(),
                                    ));
                                    will_retry = true;
                                }
                                Err(_) => {
                                    return Ok(response);
                                }
                            }
                        } else {
                            return Ok(response);
                        }
                    // Handle rate limiting (429)
                    } else if status.as_u16() == 429 {
                        let retry_after = Self::parse_retry_after(response.headers());
                        let err = Error::RateLimited {
                            retry_after,
                            message: "too many requests".to_string(),
                        };
                        if attempt == self.config.retries {
                            return Err(err);
                        }
                        warn!(attempt, retry_after = ?retry_after, "rate limited, will retry");
                        last_error = Some(err);
                        delay_override = retry_after;
                        will_retry = true;
                    } else if status.is_success() || status.is_client_error() {
                        // Success or client error (non-retryable)
                        if attempt > 0 {
                            debug!(attempt, "request succeeded after retry");
                        }
                        return Ok(response);
                    } else if attempt == self.config.retries {
                        // Final attempt, return the error
                        return Ok(response);
                    } else {
                        // Server error - retry
                        warn!(
                            attempt,
                            status = status.as_u16(),
                            "server error, will retry"
                        );
                        last_error = Some(Error::UnexpectedStatus {
                            status: status.as_u16(),
                            message: "server error, will retry".to_string(),
                        });
                        will_retry = true;
                    }
                }
                Err(e) => {
                    // Classify the error type
                    let err = self.map_transport_error(e);
                    if attempt == self.config.retries {
                        warn!(attempt, error = %err, "request failed, no more retries");
                        return Err(err);
                    }
                    trace!(attempt, error = %err, "network error, will retry");
                    last_error = Some(err);
                    will_retry = true;
                }
            }

            // Record retry if we're about to retry and we have an operation
            if will_retry && let Some(operation) = op {
                self.record_retry(operation);
            }

            // Exponential backoff: 100ms, 200ms, 400ms, ...
            if attempt < self.config.retries {
                let delay =
                    delay_override.unwrap_or_else(|| Duration::from_millis(100 * (1 << attempt)));
                trace!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    "backing off before retry"
                );
                tokio::time::sleep(delay).await;
            }
        }

        Err(last_error.unwrap_or_else(|| Error::UnexpectedStatus {
            status: 0,
            message: "retry exhausted".to_string(),
        }))
    }

    /// Parses Retry-After header value.
    ///
    /// Supports both formats:
    /// - Numeric seconds: "120"
    /// - HTTP-date: "Wed, 21 Oct 2015 07:28:00 GMT"
    fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
        let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
        let trimmed = value.trim();

        // Try parsing as numeric seconds first
        if let Ok(seconds) = trimmed.parse::<u64>() {
            return Some(Duration::from_secs(seconds));
        }

        // Try parsing as HTTP-date
        if let Ok(date) = httpdate::parse_http_date(trimmed) {
            let now = SystemTime::now();
            // Calculate duration until the specified time
            if let Ok(duration) = date.duration_since(now) {
                return Some(duration);
            }
            // If the date is in the past, return zero duration
            return Some(Duration::ZERO);
        }

        None
    }

    fn parse_bearer_challenge(headers: &HeaderMap) -> Option<BearerChallenge> {
        let value = headers.get(WWW_AUTHENTICATE)?.to_str().ok()?;
        let value = value.trim();
        if !value.to_ascii_lowercase().starts_with("bearer ") {
            return None;
        }
        let params = value[7..].split(',');
        let mut realm = None;
        let mut service = None;
        let mut scope = None;
        for param in params {
            let param = param.trim();
            let (key, val) = match param.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            let val = val.trim().trim_matches('"');
            match key {
                "realm" => realm = Some(val.to_string()),
                "service" => service = Some(val.to_string()),
                "scope" => scope = Some(val.to_string()),
                _ => {}
            }
        }
        realm.map(|realm| BearerChallenge {
            realm,
            service,
            scope,
        })
    }

    async fn fetch_bearer_token(&self, challenge: &BearerChallenge) -> Result<String> {
        #[derive(Deserialize)]
        struct TokenResponse {
            token: Option<String>,
            access_token: Option<String>,
        }

        let mut request = self.http.get(&challenge.realm);
        if let Some(service) = &challenge.service {
            request = request.query(&[("service", service)]);
        }
        if let Some(scope) = &challenge.scope {
            request = request.query(&[("scope", scope)]);
        }
        if let Some(cred) = &self.credential
            && let Some(header) = cred.authorization_header()
        {
            request = request.header(AUTHORIZATION, header);
        }

        let response = request
            .send()
            .await
            .map_err(|e| self.map_transport_error(e))?;
        if !response.status().is_success() {
            return Err(Error::Unauthorized("token request failed".to_string()));
        }
        let body: TokenResponse = response
            .json()
            .await
            .map_err(|e| self.map_transport_error(e))?;
        body.token
            .or(body.access_token)
            .ok_or_else(|| Error::Unauthorized("token response missing token".to_string()))
    }

    fn parse_link_next(headers: &HeaderMap) -> Option<String> {
        let header = headers.get(LINK)?;
        let value = header.to_str().ok()?;
        for part in value.split(',') {
            let part = part.trim();
            let url_start = part.find('<')?;
            let url_end = part.find('>')?;
            let url = &part[url_start + 1..url_end];
            if part.contains("rel=\"next\"") || part.contains("rel=next") {
                return Some(url.to_string());
            }
        }
        None
    }

    fn resolve_link(&self, registry: &str, link: &str) -> String {
        if link.starts_with("http://") || link.starts_with("https://") {
            link.to_string()
        } else if link.starts_with('/') {
            format!("{}://{}{}", self.scheme(), registry, link)
        } else {
            format!("{}://{}/{}", self.scheme(), registry, link)
        }
    }

    fn parse_range_end(headers: &HeaderMap) -> Option<usize> {
        let value = headers.get(RANGE)?.to_str().ok()?;
        let trimmed = value.trim();
        let trimmed = trimmed.strip_prefix("bytes=").unwrap_or(trimmed);
        let end = trimmed.split('-').nth(1)?;
        end.parse::<usize>().ok()
    }

    /// Maps reqwest errors into semantic error variants.
    fn map_transport_error(&self, e: reqwest::Error) -> Error {
        if e.is_timeout() {
            Error::Timeout {
                duration: self.config.timeout,
                source: e,
            }
        } else if e.is_connect() {
            let message = e.to_string();
            Error::ConnectionFailed { message, source: e }
        } else {
            Error::Transport(e)
        }
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new().expect("failed to create default client")
    }
}

struct BearerChallenge {
    realm: String,
    service: Option<String>,
    scope: Option<String>,
}

/// Tag list response from the registry.
#[derive(Debug, Deserialize)]
struct TagList {
    #[allow(dead_code)]
    name: String,
    /// Tags can be null for empty repositories on some registries.
    tags: Option<Vec<String>>,
}

/// Returns the Accept header value for manifest requests.
fn accept_manifest_types() -> HeaderValue {
    HeaderValue::from_static(
        "application/vnd.oci.image.manifest.v1+json, \
         application/vnd.oci.image.index.v1+json, \
         application/vnd.docker.distribution.manifest.v2+json, \
         application/vnd.docker.distribution.manifest.list.v2+json, \
         */*",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
        assert_eq!(config.retries, DEFAULT_RETRIES);
        assert!(config.https);
        assert!(!config.insecure);
    }

    #[test]
    fn test_client_config_builder() {
        let config = ClientConfig::new()
            .with_timeout(Duration::from_secs(60))
            .with_retries(5)
            .with_https(false)
            .with_insecure(true);

        assert_eq!(config.timeout, Duration::from_secs(60));
        assert_eq!(config.retries, 5);
        assert!(!config.https);
        assert!(config.insecure);
    }

    #[test]
    fn test_url_building() {
        let client = Client::new().unwrap();
        let url = client.url("gcr.io", "/myrepo/manifests/latest");
        assert_eq!(url, "https://gcr.io/v2/myrepo/manifests/latest");
    }

    #[test]
    fn test_url_building_http() {
        let client = Client::with_config(ClientConfig::new().with_https(false)).unwrap();
        let url = client.url("localhost:5000", "/myrepo/manifests/latest");
        assert_eq!(url, "http://localhost:5000/v2/myrepo/manifests/latest");
    }
}
