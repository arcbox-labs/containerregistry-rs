//! Docker config file parsing.
//!
//! This module parses Docker's `config.json` file which contains registry
//! credentials and credential helper configuration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Credential, Error, Result};

/// Docker config file structure.
///
/// This represents the contents of `~/.docker/config.json` or the file
/// pointed to by `DOCKER_CONFIG`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerConfig {
    /// Direct authentication entries (base64-encoded or plaintext).
    #[serde(default)]
    pub auths: HashMap<String, AuthEntry>,

    /// Default credential store for all registries.
    #[serde(default, rename = "credsStore")]
    pub creds_store: Option<String>,

    /// Per-registry credential helpers.
    #[serde(default, rename = "credHelpers")]
    pub cred_helpers: HashMap<String, String>,
}

/// An auth entry from the Docker config's "auths" section.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AuthEntry {
    /// Base64-encoded "username:password" string.
    #[serde(default)]
    pub auth: Option<String>,

    /// Username (if stored separately).
    #[serde(default)]
    pub username: Option<String>,

    /// Password (if stored separately).
    #[serde(default)]
    pub password: Option<String>,

    /// Identity token for OAuth2.
    #[serde(default, rename = "identitytoken")]
    pub identity_token: Option<String>,

    /// Registry token (deprecated).
    #[serde(default, rename = "registrytoken")]
    pub registry_token: Option<String>,
}

impl DockerConfig {
    /// Loads the Docker config from the default location.
    ///
    /// Checks `DOCKER_CONFIG` environment variable first, then falls back
    /// to `~/.docker/config.json`.
    pub fn load() -> Result<Self> {
        let path = Self::default_config_path()?;
        Self::load_from(&path)
    }

    /// Loads the Docker config from a specific path.
    pub fn load_from(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                // Return empty config if file doesn't exist
                return Error::Io(e);
            }
            Error::Io(e)
        })?;

        Self::from_json(&contents)
    }

    /// Parses a Docker config from JSON string.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| Error::ConfigParse(e.to_string()))
    }

    /// Returns the default Docker config file path.
    ///
    /// Checks `DOCKER_CONFIG` environment variable first, then uses
    /// `~/.docker/config.json`.
    pub fn default_config_path() -> Result<PathBuf> {
        // Check DOCKER_CONFIG env var
        if let Ok(docker_config) = std::env::var("DOCKER_CONFIG") {
            let path = PathBuf::from(docker_config);
            return Ok(path.join("config.json"));
        }

        // Fall back to ~/.docker/config.json
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| Error::ConfigParse("could not determine home directory".to_string()))?;

        Ok(PathBuf::from(home).join(".docker").join("config.json"))
    }

    /// Gets the credential helper name for a registry.
    ///
    /// Returns the registry-specific helper from `credHelpers` if present,
    /// otherwise falls back to `credsStore`.
    pub fn get_credential_helper(&self, registry: &str) -> Option<&str> {
        // Check registry-specific helper first
        if let Some(helper) = self.cred_helpers.get(registry) {
            return Some(helper.as_str());
        }

        // Normalize registry and try again
        let normalized = normalize_registry(registry);
        if let Some(helper) = self.cred_helpers.get(&normalized) {
            return Some(helper.as_str());
        }

        // Fall back to global credential store
        self.creds_store.as_deref()
    }

    /// Gets credentials from the `auths` section for a registry.
    pub fn get_auth(&self, registry: &str) -> Option<Credential> {
        // Try exact match first
        if let Some(entry) = self.auths.get(registry)
            && let Some(cred) = entry.to_credential()
        {
            return Some(cred);
        }

        // Normalize the input registry
        let normalized = normalize_registry(registry);

        // Try to find a matching entry by normalizing each auths key
        for (key, entry) in &self.auths {
            let normalized_key = normalize_registry(key);
            if normalized_key == normalized
                && let Some(cred) = entry.to_credential()
            {
                return Some(cred);
            }
        }

        None
    }
}

impl AuthEntry {
    /// Converts this auth entry to a Credential.
    pub fn to_credential(&self) -> Option<Credential> {
        // Check for identity token first
        if let Some(ref token) = self.identity_token
            && !token.is_empty()
        {
            return Some(Credential::identity_token(token));
        }

        // Check for registry token
        if let Some(ref token) = self.registry_token
            && !token.is_empty()
        {
            return Some(Credential::bearer(token));
        }

        // Check for base64-encoded auth
        if let Some(ref auth) = self.auth
            && !auth.is_empty()
        {
            return Self::decode_auth(auth);
        }

        // Check for separate username/password
        if let (Some(username), Some(password)) = (&self.username, &self.password)
            && !username.is_empty()
        {
            return Some(Credential::basic(username, password));
        }

        None
    }

    /// Decodes a base64-encoded "username:password" auth string.
    fn decode_auth(auth: &str) -> Option<Credential> {
        use base64::Engine;

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(auth)
            .ok()?;
        let decoded_str = String::from_utf8(decoded).ok()?;

        // Split on first colon (password may contain colons)
        let (username, password) = decoded_str.split_once(':')?;

        if username.is_empty() {
            None
        } else {
            Some(Credential::basic(username, password))
        }
    }
}

/// Normalizes a registry hostname for lookup.
///
/// Docker Hub can be referenced as "docker.io", "index.docker.io",
/// "registry-1.docker.io", etc.
fn normalize_registry(registry: &str) -> String {
    let registry = registry
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let registry = registry.trim_end_matches('/');

    // Remove common path suffixes like /v1, /v2
    let registry = registry
        .trim_end_matches("/v1")
        .trim_end_matches("/v2");

    // Normalize Docker Hub references
    match registry {
        "docker.io" | "registry-1.docker.io" => "index.docker.io".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::ENV_LOCK;

    #[test]
    fn test_parse_empty_config() {
        let config: DockerConfig = serde_json::from_str("{}").unwrap();
        assert!(config.auths.is_empty());
        assert!(config.creds_store.is_none());
        assert!(config.cred_helpers.is_empty());
    }

    #[test]
    fn test_parse_auths_base64() {
        let json = r#"{
            "auths": {
                "https://index.docker.io/v1/": {
                    "auth": "dXNlcjpwYXNz"
                }
            }
        }"#;

        let config: DockerConfig = serde_json::from_str(json).unwrap();
        let entry = config.auths.get("https://index.docker.io/v1/").unwrap();
        let cred = entry.to_credential().unwrap();

        assert_eq!(cred.username(), Some("user"));
        assert_eq!(cred.password(), Some("pass"));
    }

    #[test]
    fn test_parse_auths_plaintext() {
        let json = r#"{
            "auths": {
                "gcr.io": {
                    "username": "myuser",
                    "password": "mypass"
                }
            }
        }"#;

        let config: DockerConfig = serde_json::from_str(json).unwrap();
        let entry = config.auths.get("gcr.io").unwrap();
        let cred = entry.to_credential().unwrap();

        assert_eq!(cred.username(), Some("myuser"));
        assert_eq!(cred.password(), Some("mypass"));
    }

    #[test]
    fn test_parse_creds_store() {
        let json = r#"{
            "credsStore": "osxkeychain"
        }"#;

        let config: DockerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.creds_store.as_deref(), Some("osxkeychain"));
    }

    #[test]
    fn test_parse_cred_helpers() {
        let json = r#"{
            "credHelpers": {
                "gcr.io": "gcloud",
                "123456789.dkr.ecr.us-east-1.amazonaws.com": "ecr-login"
            }
        }"#;

        let config: DockerConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.cred_helpers.get("gcr.io"), Some(&"gcloud".to_string()));
    }

    #[test]
    fn test_get_credential_helper() {
        let json = r#"{
            "credsStore": "osxkeychain",
            "credHelpers": {
                "gcr.io": "gcloud"
            }
        }"#;

        let config: DockerConfig = serde_json::from_str(json).unwrap();

        // Registry-specific helper takes precedence
        assert_eq!(config.get_credential_helper("gcr.io"), Some("gcloud"));

        // Falls back to credsStore for other registries
        assert_eq!(
            config.get_credential_helper("docker.io"),
            Some("osxkeychain")
        );
    }

    #[test]
    fn test_normalize_registry() {
        assert_eq!(normalize_registry("docker.io"), "index.docker.io");
        assert_eq!(
            normalize_registry("registry-1.docker.io"),
            "index.docker.io"
        );
        assert_eq!(normalize_registry("gcr.io"), "gcr.io");
        assert_eq!(normalize_registry("https://gcr.io/"), "gcr.io");
    }

    #[test]
    fn test_get_auth_normalized() {
        let json = r#"{
            "auths": {
                "https://index.docker.io/v1/": {
                    "auth": "dXNlcjpwYXNz"
                }
            }
        }"#;

        let config: DockerConfig = serde_json::from_str(json).unwrap();

        // Should find via various Docker Hub references
        assert!(config.get_auth("index.docker.io").is_some());
    }

    #[test]
    fn test_decode_auth_with_colon_in_password() {
        // "user:pass:with:colons" base64 encoded
        let auth = "dXNlcjpwYXNzOndpdGg6Y29sb25z";
        let cred = AuthEntry::decode_auth(auth).unwrap();
        assert_eq!(cred.username(), Some("user"));
        assert_eq!(cred.password(), Some("pass:with:colons"));
    }

    #[test]
    fn test_load_uses_docker_config_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.json");

        std::fs::write(
            &config_path,
            r#"{"auths":{"example.com":{"username":"user","password":"pass"}}}"#,
        )
        .unwrap();

        let prev = std::env::var("DOCKER_CONFIG").ok();
        unsafe {
            std::env::set_var("DOCKER_CONFIG", temp.path());
        }

        let config = DockerConfig::load().unwrap();
        let cred = config.get_auth("example.com").unwrap();
        assert_eq!(cred.username(), Some("user"));
        assert_eq!(cred.password(), Some("pass"));

        if let Some(value) = prev {
            unsafe {
                std::env::set_var("DOCKER_CONFIG", value);
            }
        } else {
            unsafe {
                std::env::remove_var("DOCKER_CONFIG");
            }
        }
    }
}
