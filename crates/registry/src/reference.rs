//! Image reference parsing and canonicalization.
//!
//! This module provides types for parsing and manipulating container image
//! references in the format `[registry/]repository[:tag|@digest]`.

use std::fmt;
use std::str::FromStr;

use containerregistry_image::Digest;

use crate::{Error, Result};

/// Default registry when none is specified.
pub const DEFAULT_REGISTRY: &str = "index.docker.io";

/// Default tag when none is specified.
pub const DEFAULT_TAG: &str = "latest";

/// A parsed container image reference.
///
/// References can be in several forms:
/// - `repository` (implies default registry and tag)
/// - `repository:tag`
/// - `repository@digest`
/// - `registry/repository`
/// - `registry/repository:tag`
/// - `registry/repository@digest`
///
/// # Examples
///
/// ```
/// use containerregistry_registry::Reference;
///
/// let r: Reference = "nginx:1.21".parse().unwrap();
/// assert_eq!(r.registry(), "index.docker.io");
/// assert_eq!(r.repository(), "library/nginx");
/// assert_eq!(r.tag(), Some("1.21"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The registry host (e.g., "gcr.io", "index.docker.io").
    registry: String,

    /// The repository path (e.g., "library/nginx", "myorg/myapp").
    repository: String,

    /// The reference specifier - either a tag or a digest.
    specifier: Specifier,
}

/// The reference specifier - either a tag or a digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Specifier {
    /// A tag reference (e.g., "latest", "v1.0").
    Tag(String),

    /// A digest reference (e.g., "sha256:abc...").
    Digest(Digest),
}

impl Reference {
    /// Creates a new reference with the given components.
    pub fn new(
        registry: impl Into<String>,
        repository: impl Into<String>,
        specifier: Specifier,
    ) -> Self {
        Self {
            registry: registry.into(),
            repository: repository.into(),
            specifier,
        }
    }

    /// Creates a reference with a tag.
    pub fn with_tag(
        registry: impl Into<String>,
        repository: impl Into<String>,
        tag: impl Into<String>,
    ) -> Self {
        Self::new(registry, repository, Specifier::Tag(tag.into()))
    }

    /// Creates a reference with a digest.
    pub fn with_digest(
        registry: impl Into<String>,
        repository: impl Into<String>,
        digest: Digest,
    ) -> Self {
        Self::new(registry, repository, Specifier::Digest(digest))
    }

    /// Returns the registry host.
    pub fn registry(&self) -> &str {
        &self.registry
    }

    /// Returns the repository path.
    pub fn repository(&self) -> &str {
        &self.repository
    }

    /// Returns the tag, if this is a tag reference.
    pub fn tag(&self) -> Option<&str> {
        match &self.specifier {
            Specifier::Tag(t) => Some(t),
            Specifier::Digest(_) => None,
        }
    }

    /// Returns the digest, if this is a digest reference.
    pub fn digest(&self) -> Option<&Digest> {
        match &self.specifier {
            Specifier::Tag(_) => None,
            Specifier::Digest(d) => Some(d),
        }
    }

    /// Returns the reference string (tag or digest).
    pub fn reference(&self) -> String {
        match &self.specifier {
            Specifier::Tag(t) => t.clone(),
            Specifier::Digest(d) => d.to_string(),
        }
    }

    /// Returns true if this is a digest reference.
    pub fn is_digest(&self) -> bool {
        matches!(&self.specifier, Specifier::Digest(_))
    }

    /// Returns true if this is a tag reference.
    pub fn is_tag(&self) -> bool {
        matches!(&self.specifier, Specifier::Tag(_))
    }

    /// Returns the full image path (registry/repository).
    pub fn image(&self) -> String {
        format!("{}/{}", self.registry, self.repository)
    }

    /// Converts this reference to use a digest instead of a tag.
    pub fn with_new_digest(self, digest: Digest) -> Self {
        Self {
            registry: self.registry,
            repository: self.repository,
            specifier: Specifier::Digest(digest),
        }
    }

    /// Converts this reference to use a tag instead of a digest.
    pub fn with_new_tag(self, tag: impl Into<String>) -> Self {
        Self {
            registry: self.registry,
            repository: self.repository,
            specifier: Specifier::Tag(tag.into()),
        }
    }
}

impl FromStr for Reference {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_reference(s)
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.specifier {
            Specifier::Tag(t) => write!(f, "{}/{}:{}", self.registry, self.repository, t),
            Specifier::Digest(d) => write!(f, "{}/{}@{}", self.registry, self.repository, d),
        }
    }
}

/// Parses an image reference string.
fn parse_reference(s: &str) -> Result<Reference> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::InvalidReference("empty reference".to_string()));
    }

    // Check for digest reference (@sha256:...)
    let (name_part, specifier) = if let Some(at_pos) = s.rfind('@') {
        let digest_str = &s[at_pos + 1..];
        let digest: Digest = digest_str
            .parse()
            .map_err(|_| Error::InvalidReference(format!("invalid digest: {}", digest_str)))?;
        let name_part = &s[..at_pos];

        // Reject name:tag@digest - both tag and digest is invalid
        if find_tag_separator(name_part).is_some() {
            return Err(Error::InvalidReference(
                "reference cannot have both tag and digest".to_string(),
            ));
        }

        (name_part, Specifier::Digest(digest))
    } else if let Some(colon_pos) = find_tag_separator(s) {
        // Tag reference (:tag)
        let tag = &s[colon_pos + 1..];
        if tag.is_empty() {
            return Err(Error::InvalidReference("empty tag".to_string()));
        }
        (&s[..colon_pos], Specifier::Tag(tag.to_string()))
    } else {
        // No tag or digest, use default tag
        (s, Specifier::Tag(DEFAULT_TAG.to_string()))
    };

    // Parse registry and repository from name_part
    let (registry, repository) = parse_name(name_part)?;

    Ok(Reference {
        registry,
        repository,
        specifier,
    })
}

/// Finds the position of the tag separator (:) that is not part of a port number.
fn find_tag_separator(s: &str) -> Option<usize> {
    // The tag separator is the last colon that is not part of a port number.
    // A port number appears after a registry host, before any slash.
    // So we need to find the last colon that appears after the last slash.
    let last_slash = s.rfind('/');

    if let Some(colon_pos) = s.rfind(':') {
        // If there's a slash after the colon, this colon is part of a port
        if let Some(slash_pos) = last_slash {
            if colon_pos > slash_pos {
                return Some(colon_pos);
            }
        } else {
            // No slash at all - check if this looks like a registry:port
            // A port would be all digits after the colon (and non-empty)
            let after_colon = &s[colon_pos + 1..];
            if after_colon.is_empty() || !after_colon.chars().all(|c| c.is_ascii_digit()) {
                return Some(colon_pos);
            }
        }
    }

    None
}

/// Parses the name part (without tag/digest) into registry and repository.
fn parse_name(name: &str) -> Result<(String, String)> {
    if name.is_empty() {
        return Err(Error::InvalidReference("empty name".to_string()));
    }

    // Check if the first component looks like a registry host
    let first_slash = name.find('/');

    if let Some(slash_pos) = first_slash {
        let first_part = &name[..slash_pos];
        let rest = &name[slash_pos + 1..];

        // Canonicalize well-known Docker Hub aliases before the generic
        // registry-host heuristic. `docker.io`, `registry.hub.docker.com`,
        // and `registry-1.docker.io` all refer to the same registry whose
        // manifest API lives at `index.docker.io`; `docker.io/v2/...` in
        // particular is a marketing-site redirect, not a registry endpoint.
        if is_docker_hub_alias(first_part) {
            if rest.is_empty() {
                return Err(Error::InvalidReference("empty repository".to_string()));
            }
            return Ok((
                DEFAULT_REGISTRY.to_string(),
                normalize_docker_hub_repository(rest),
            ));
        }

        // A registry host contains a dot, colon (port), or is "localhost"
        if first_part.contains('.') || first_part.contains(':') || first_part == "localhost" {
            // It's a registry
            let repository = if rest.is_empty() {
                return Err(Error::InvalidReference("empty repository".to_string()));
            } else {
                rest.to_string()
            };
            return Ok((first_part.to_string(), repository));
        }
    }

    // No explicit registry - use default
    let repository = normalize_docker_hub_repository(name);
    Ok((DEFAULT_REGISTRY.to_string(), repository))
}

/// Normalizes Docker Hub repository names.
///
/// For Docker Hub, single-component names like "nginx" become "library/nginx".
fn normalize_docker_hub_repository(name: &str) -> String {
    if name.contains('/') {
        name.to_string()
    } else {
        format!("library/{}", name)
    }
}

/// Whether `host` is one of the registry hostnames Docker Hub is known by.
///
/// All four forms are accepted by the Docker CLI, podman, containerd, and
/// crane; they all resolve to the same registry whose manifest API is at
/// [`DEFAULT_REGISTRY`].
fn is_docker_hub_alias(host: &str) -> bool {
    matches!(
        host,
        "docker.io" | "index.docker.io" | "registry.hub.docker.com" | "registry-1.docker.io"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_name() {
        let r: Reference = "nginx".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/nginx");
        assert_eq!(r.tag(), Some("latest"));
        assert!(r.is_tag());
    }

    #[test]
    fn test_parse_name_with_tag() {
        let r: Reference = "nginx:1.21".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/nginx");
        assert_eq!(r.tag(), Some("1.21"));
    }

    #[test]
    fn test_parse_name_with_org() {
        let r: Reference = "myorg/myapp:v1.0".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "myorg/myapp");
        assert_eq!(r.tag(), Some("v1.0"));
    }

    #[test]
    fn test_parse_with_registry() {
        let r: Reference = "gcr.io/myproject/myapp:latest".parse().unwrap();
        assert_eq!(r.registry(), "gcr.io");
        assert_eq!(r.repository(), "myproject/myapp");
        assert_eq!(r.tag(), Some("latest"));
    }

    #[test]
    fn test_parse_with_registry_and_port() {
        let r: Reference = "localhost:5000/myapp:v1".parse().unwrap();
        assert_eq!(r.registry(), "localhost:5000");
        assert_eq!(r.repository(), "myapp");
        assert_eq!(r.tag(), Some("v1"));
    }

    #[test]
    fn test_parse_with_digest() {
        let r: Reference =
            "nginx@sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/nginx");
        assert!(r.is_digest());
        assert!(r.digest().is_some());
    }

    #[test]
    fn test_parse_full_with_digest() {
        let r: Reference = "gcr.io/myproject/myapp@sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".parse().unwrap();
        assert_eq!(r.registry(), "gcr.io");
        assert_eq!(r.repository(), "myproject/myapp");
        assert!(r.is_digest());
    }

    #[test]
    fn test_parse_localhost() {
        let r: Reference = "localhost/myapp".parse().unwrap();
        assert_eq!(r.registry(), "localhost");
        assert_eq!(r.repository(), "myapp");
        assert_eq!(r.tag(), Some("latest"));
    }

    #[test]
    fn test_parse_docker_io_alias_canonicalizes_to_index_docker_io() {
        let r: Reference = "docker.io/library/alpine:3.21".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/alpine");
        assert_eq!(r.tag(), Some("3.21"));
    }

    #[test]
    fn test_parse_docker_io_applies_library_prefix_to_single_component_repo() {
        let r: Reference = "docker.io/alpine".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/alpine");
        assert_eq!(r.tag(), Some("latest"));
    }

    #[test]
    fn test_parse_docker_io_preserves_explicit_org_repo() {
        let r: Reference = "docker.io/myorg/myapp:v1".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "myorg/myapp");
        assert_eq!(r.tag(), Some("v1"));
    }

    #[test]
    fn test_parse_registry_hub_docker_com_alias() {
        let r: Reference = "registry.hub.docker.com/nginx".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/nginx");
    }

    #[test]
    fn test_parse_registry_one_docker_io_alias() {
        let r: Reference = "registry-1.docker.io/library/busybox".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/busybox");
    }

    #[test]
    fn test_parse_index_docker_io_single_component_gets_library_prefix() {
        // Previously produced `repo=alpine` with no prefix; now consistent
        // with `docker.io/alpine` and the bare `alpine` form.
        let r: Reference = "index.docker.io/alpine".parse().unwrap();
        assert_eq!(r.registry(), DEFAULT_REGISTRY);
        assert_eq!(r.repository(), "library/alpine");
    }

    #[test]
    fn test_parse_docker_io_empty_repository_rejected() {
        let r: Result<Reference> = "docker.io/".parse();
        assert!(r.is_err());
    }

    #[test]
    fn test_display() {
        let r: Reference = "gcr.io/myproject/myapp:v1.0".parse().unwrap();
        assert_eq!(r.to_string(), "gcr.io/myproject/myapp:v1.0");

        let r2: Reference =
            "nginx@sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap();
        assert_eq!(
            r2.to_string(),
            "index.docker.io/library/nginx@sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_invalid_empty() {
        let r: Result<Reference> = "".parse();
        assert!(r.is_err());
    }

    #[test]
    fn test_invalid_empty_tag() {
        let r: Result<Reference> = "nginx:".parse();
        assert!(r.is_err());
    }

    // This test ensures references containing both tag and digest are rejected
    // to avoid ambiguous or invalid specifiers like "name:tag@sha256:...".
    #[test]
    fn test_invalid_tag_and_digest() {
        let r: Result<Reference> =
            "nginx:1.0@sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse();
        assert!(r.is_err());
    }

    #[test]
    fn test_with_new_digest() {
        let r: Reference = "nginx:1.21".parse().unwrap();
        let digest: Digest =
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap();
        let r2 = r.with_new_digest(digest.clone());
        assert!(r2.is_digest());
        assert_eq!(r2.digest(), Some(&digest));
    }

    #[test]
    fn test_with_new_tag() {
        let r: Reference =
            "nginx@sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap();
        let r2 = r.with_new_tag("v2.0");
        assert!(r2.is_tag());
        assert_eq!(r2.tag(), Some("v2.0"));
    }
}
