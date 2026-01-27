//! OCI distribution registry client.
//!
//! This crate provides an HTTP client for interacting with OCI distribution
//! registries, implementing the OCI Distribution Specification.
//!
//! # Overview
//!
//! The main types in this crate are:
//! - [`Client`] - The registry client for manifest and blob operations
//! - [`Reference`] - A parsed container image reference
//! - [`ClientConfig`] - Configuration for the client
//!
//! # Example
//!
//! ```no_run
//! use containerregistry_registry::{Client, Reference};
//!
//! # async fn example() -> containerregistry_registry::Result<()> {
//! let client = Client::new()?;
//!
//! // Parse a reference
//! let reference: Reference = "nginx:1.21".parse()?;
//!
//! // Get a manifest
//! let (manifest, digest) = client.get_manifest(&reference).await?;
//! # Ok(())
//! # }
//! ```

mod client;
mod error;
pub mod metrics;
mod reference;

pub use client::{Client, ClientConfig, ManifestOrIndex};
pub use error::Error;
pub use metrics::{MetricsCollector, MetricsSummary, Operation, OperationMetrics, OperationTimer};
pub use reference::{DEFAULT_REGISTRY, DEFAULT_TAG, Reference, Specifier};

/// Result type for registry operations.
pub type Result<T> = std::result::Result<T, Error>;
