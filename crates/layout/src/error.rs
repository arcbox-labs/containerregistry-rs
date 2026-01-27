//! Error types for layout operations.

use thiserror::Error;

/// Errors that can occur during layout operations.
#[derive(Debug, Error)]
pub enum Error {
    /// Invalid layout structure.
    #[error("invalid layout: {0}")]
    InvalidLayout(String),

    /// Missing required file.
    #[error("missing file: {0}")]
    MissingFile(String),

    /// Blob not found.
    #[error("blob not found: {0}")]
    BlobNotFound(String),

    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Image error.
    #[error("image error: {0}")]
    Image(#[from] containerregistry_image::Error),

    /// JSON error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
