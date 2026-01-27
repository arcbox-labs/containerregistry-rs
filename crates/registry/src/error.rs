//! Error types for registry operations.

use std::time::Duration;

use thiserror::Error;

/// Errors that can occur during registry operations.
#[derive(Debug, Error)]
pub enum Error {
    /// Resource not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// Authentication required or failed.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// Access denied.
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// Invalid reference format.
    #[error("invalid reference: {0}")]
    InvalidReference(String),

    /// Digest mismatch between requested and received content.
    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Expected digest.
        expected: String,
        /// Actual digest.
        actual: String,
    },

    /// Unexpected HTTP status.
    #[error("unexpected status {status}: {message}")]
    UnexpectedStatus {
        /// HTTP status code.
        status: u16,
        /// Error message.
        message: String,
    },

    /// Request timeout.
    #[error("request timed out after {duration:?}")]
    Timeout {
        /// Timeout duration.
        duration: Duration,
        /// Underlying transport error.
        #[source]
        source: reqwest::Error,
    },

    /// Connection failed.
    #[error("connection failed: {message}")]
    ConnectionFailed {
        /// Error message.
        message: String,
        /// Underlying transport error.
        #[source]
        source: reqwest::Error,
    },

    /// Rate limited by the registry.
    #[error("rate limited: {message}")]
    RateLimited {
        /// Suggested retry delay if provided.
        retry_after: Option<Duration>,
        /// Error message.
        message: String,
    },

    /// Network or transport error.
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// Image error.
    #[error("image error: {0}")]
    Image(#[from] containerregistry_image::Error),

    /// Auth error.
    #[error("auth error: {0}")]
    Auth(#[from] containerregistry_auth::Error),

    /// JSON error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// I/O error.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Returns true if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(e) => e.is_timeout() || e.is_connect(),
            Self::Timeout { .. } => true,
            Self::ConnectionFailed { .. } => true,
            Self::RateLimited { .. } => true,
            Self::UnexpectedStatus { status, .. } => {
                // 5xx errors are typically retryable
                *status >= 500 && *status < 600
            }
            _ => false,
        }
    }

    /// Returns the HTTP status code if this is an HTTP error.
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::NotFound(_) => Some(404),
            Self::Unauthorized(_) => Some(401),
            Self::Forbidden(_) => Some(403),
            Self::RateLimited { .. } => Some(429),
            Self::UnexpectedStatus { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Returns a hint message for resolving this error.
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::NotFound(_) => Some("verify the image reference exists and you have access"),
            Self::Unauthorized(_) => Some("check your credentials or run 'docker login'"),
            Self::Forbidden(_) => Some("you may not have permission to access this resource"),
            Self::DigestMismatch { .. } => Some(
                "the content was modified during transfer or the server returned incorrect data",
            ),
            Self::Timeout { .. } => {
                Some("try increasing the timeout or check your network connection")
            }
            Self::ConnectionFailed { .. } => Some("check your network connection and registry URL"),
            Self::RateLimited { .. } => Some("wait and retry, or reduce request frequency"),
            _ => None,
        }
    }

    /// Returns the suggested retry-after duration if present.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// Returns a detailed error message with hint.
    pub fn detailed_message(&self) -> String {
        match self.hint() {
            Some(hint) => format!("{}\n  Hint: {}", self, hint),
            None => self.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable() {
        assert!(!Error::NotFound("test".to_string()).is_retryable());
        assert!(!Error::Unauthorized("test".to_string()).is_retryable());
        assert!(
            Error::RateLimited {
                retry_after: Some(Duration::from_secs(1)),
                message: "test".to_string()
            }
            .is_retryable()
        );
        assert!(
            Error::UnexpectedStatus {
                status: 503,
                message: "test".to_string()
            }
            .is_retryable()
        );
        assert!(
            !Error::UnexpectedStatus {
                status: 400,
                message: "test".to_string()
            }
            .is_retryable()
        );
    }

    #[test]
    fn test_status_code() {
        assert_eq!(Error::NotFound("test".to_string()).status_code(), Some(404));
        assert_eq!(
            Error::Unauthorized("test".to_string()).status_code(),
            Some(401)
        );
        assert_eq!(
            Error::Forbidden("test".to_string()).status_code(),
            Some(403)
        );
        assert_eq!(
            Error::UnexpectedStatus {
                status: 503,
                message: "test".to_string()
            }
            .status_code(),
            Some(503)
        );
    }

    #[test]
    fn test_hint() {
        assert!(Error::NotFound("test".to_string()).hint().is_some());
        assert!(Error::Unauthorized("test".to_string()).hint().is_some());
        assert!(
            Error::Json(serde_json::from_str::<()>("invalid").unwrap_err())
                .hint()
                .is_none()
        );
    }

    #[test]
    fn test_retry_after() {
        let err = Error::RateLimited {
            retry_after: Some(Duration::from_secs(5)),
            message: "test".to_string(),
        };
        assert_eq!(err.retry_after(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_detailed_message() {
        let err = Error::NotFound("manifest".to_string());
        let msg = err.detailed_message();
        assert!(msg.contains("not found"));
        assert!(msg.contains("Hint:"));
    }
}
