//! Error types for auth operations.

use thiserror::Error;

/// Errors that can occur during auth operations.
#[derive(Debug, Error)]
pub enum Error {
    /// Docker config parse error.
    #[error("config parse error: {0}")]
    ConfigParse(String),

    /// Credential helper execution failed.
    #[error("credential helper failed: {0}")]
    HelperFailure(String),

    /// Credential helper not found.
    #[error("credential helper not found: {0}")]
    HelperNotFound(String),

    /// Invalid credentials format.
    #[error("invalid credentials: {0}")]
    InvalidCredentials(String),

    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
