//! Integration tests for the auth crate.
//!
//! These tests verify the complete auth resolution flow and edge cases.

use containerregistry_auth::{AuthResolver, Credential, DockerConfig};

// ============================================================================
// Credential Creation and Authorization Header Tests
// ============================================================================

#[test]
fn test_basic_auth_header_encoding() {
    let cred = Credential::basic("user", "pass");
    let header = cred.authorization_header().unwrap();

    // "user:pass" in base64 is "dXNlcjpwYXNz"
    assert_eq!(header, "Basic dXNlcjpwYXNz");
}

#[test]
fn test_basic_auth_with_colon_in_password() {
    // Password contains colons - should be handled correctly
    let cred = Credential::basic("user", "pass:with:colons");
    let header = cred.authorization_header().unwrap();

    // Verify we can decode it back
    let encoded = header.strip_prefix("Basic ").unwrap();
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        encoded,
    )
    .unwrap();
    let decoded_str = String::from_utf8(decoded).unwrap();
    assert_eq!(decoded_str, "user:pass:with:colons");
}

#[test]
fn test_basic_auth_with_empty_password() {
    let cred = Credential::basic("user", "");
    assert_eq!(cred.username(), Some("user"));
    assert_eq!(cred.password(), Some(""));

    // Should still produce a valid header
    let header = cred.authorization_header().unwrap();
    assert!(header.starts_with("Basic "));
}

#[test]
fn test_basic_auth_with_empty_username() {
    let cred = Credential::basic("", "token");
    assert_eq!(cred.username(), Some(""));
    assert_eq!(cred.password(), Some("token"));

    // This is valid for some registries (e.g., OAuth token as password)
    let header = cred.authorization_header().unwrap();
    assert!(header.starts_with("Basic "));
}

#[test]
fn test_bearer_token_header() {
    let cred = Credential::bearer("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test");
    let header = cred.authorization_header().unwrap();
    assert_eq!(
        header,
        "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test"
    );
}

#[test]
fn test_identity_token_as_bearer() {
    let cred = Credential::identity_token("identity-token-value");
    let header = cred.authorization_header().unwrap();
    assert_eq!(header, "Bearer identity-token-value");
}

// ============================================================================
// DockerConfig Parsing Tests
// ============================================================================

#[test]
fn test_config_parse_mixed_auth_formats() {
    let json = r#"{
        "auths": {
            "gcr.io": {
                "auth": "dXNlcjE6cGFzczE="
            },
            "docker.io": {
                "username": "user2",
                "password": "pass2"
            },
            "quay.io": {
                "auth": "dXNlcjM6cGFzczM=",
                "username": "ignored",
                "password": "ignored"
            }
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();

    // Base64 auth
    let cred1 = config.get_auth("gcr.io").unwrap();
    assert_eq!(cred1.username(), Some("user1"));
    assert_eq!(cred1.password(), Some("pass1"));

    // Plaintext
    let cred2 = config.get_auth("docker.io").unwrap();
    assert_eq!(cred2.username(), Some("user2"));
    assert_eq!(cred2.password(), Some("pass2"));

    // Auth field takes precedence over username/password
    let cred3 = config.get_auth("quay.io").unwrap();
    assert_eq!(cred3.username(), Some("user3"));
    assert_eq!(cred3.password(), Some("pass3"));
}

#[test]
fn test_config_parse_password_with_colon() {
    // Base64 of "user:pass:with:colons" = "dXNlcjpwYXNzOndpdGg6Y29sb25z"
    let json = r#"{
        "auths": {
            "example.com": {
                "auth": "dXNlcjpwYXNzOndpdGg6Y29sb25z"
            }
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();
    let cred = config.get_auth("example.com").unwrap();

    // Only split on first colon
    assert_eq!(cred.username(), Some("user"));
    assert_eq!(cred.password(), Some("pass:with:colons"));
}

#[test]
fn test_config_parse_empty_auth_entry() {
    let json = r#"{
        "auths": {
            "example.com": {}
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();
    // Empty entry should return None (anonymous)
    assert!(config.get_auth("example.com").is_none());
}

#[test]
fn test_config_parse_auth_with_email() {
    // Some older Docker configs include email
    let json = r#"{
        "auths": {
            "example.com": {
                "auth": "dXNlcjpwYXNz",
                "email": "user@example.com"
            }
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();
    let cred = config.get_auth("example.com").unwrap();
    assert_eq!(cred.username(), Some("user"));
}

// ============================================================================
// Registry Normalization Tests
// ============================================================================

#[test]
fn test_docker_hub_normalization() {
    let json = r#"{
        "auths": {
            "https://index.docker.io/v1/": {
                "auth": "dXNlcjpwYXNz"
            }
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();

    // All these should resolve to the same credential
    assert!(config.get_auth("docker.io").is_some());
    assert!(config.get_auth("index.docker.io").is_some());
    assert!(config.get_auth("registry-1.docker.io").is_some());
}

#[test]
fn test_registry_with_port() {
    let json = r#"{
        "auths": {
            "localhost:5000": {
                "username": "user",
                "password": "pass"
            }
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();
    let cred = config.get_auth("localhost:5000").unwrap();
    assert_eq!(cred.username(), Some("user"));
}

#[test]
fn test_registry_with_https_prefix() {
    let json = r#"{
        "auths": {
            "https://gcr.io": {
                "username": "user",
                "password": "pass"
            }
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();

    // Should match with or without scheme
    assert!(config.get_auth("gcr.io").is_some());
    assert!(config.get_auth("https://gcr.io").is_some());
}

// ============================================================================
// Resolver Priority Tests
// ============================================================================

#[test]
fn test_explicit_credential_always_wins() {
    let config = DockerConfig::from_json(
        r#"{
        "auths": {
            "example.com": { "username": "config-user", "password": "config-pass" }
        },
        "credHelpers": {
            "example.com": "fake-helper"
        }
    }"#,
    )
    .unwrap();

    let resolver = AuthResolver::with_config(config)
        .with_explicit(Credential::basic("explicit-user", "explicit-pass"));

    let cred = resolver.resolve("example.com").unwrap();
    assert_eq!(cred.username(), Some("explicit-user"));
}

#[test]
fn test_auths_take_precedence_over_helpers() {
    let config = DockerConfig::from_json(
        r#"{
        "auths": {
            "example.com": { "username": "auths-user", "password": "auths-pass" }
        },
        "credHelpers": {
            "example.com": "nonexistent-helper"
        }
    }"#,
    )
    .unwrap();

    let resolver = AuthResolver::with_config(config);
    let cred = resolver.resolve("example.com").unwrap();

    // Should use auths, not try the helper
    assert_eq!(cred.username(), Some("auths-user"));
}

#[test]
fn test_cred_helpers_used_when_no_auths() {
    let config = DockerConfig::from_json(
        r#"{
        "credHelpers": {
            "example.com": "nonexistent-helper-12345"
        }
    }"#,
    )
    .unwrap();

    let resolver = AuthResolver::with_config(config);
    let cred = resolver.resolve("example.com").unwrap();

    // Helper not found, should fall back to anonymous
    assert!(cred.is_anonymous());
}

#[test]
fn test_creds_store_used_as_fallback() {
    let config = DockerConfig::from_json(
        r#"{
        "credsStore": "nonexistent-store-12345"
    }"#,
    )
    .unwrap();

    let resolver = AuthResolver::with_config(config);
    let cred = resolver.resolve("any-registry.io").unwrap();

    // Store helper not found, should fall back to anonymous
    assert!(cred.is_anonymous());
}

#[test]
fn test_resolve_or_anonymous_never_errors() {
    let config = DockerConfig::from_json(
        r#"{
        "auths": {
            "example.com": { "auth": "invalid-base64!!!" }
        }
    }"#,
    )
    .unwrap();

    let resolver = AuthResolver::with_config(config);

    // resolve() might error on invalid base64
    // resolve_or_anonymous() should return Anonymous instead
    let cred = resolver.resolve_or_anonymous("example.com");
    assert!(cred.is_anonymous());
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_empty_config_returns_anonymous() {
    let resolver = AuthResolver::with_config(DockerConfig::from_json("{}").unwrap());
    let cred = resolver.resolve("any-registry.io").unwrap();
    assert!(cred.is_anonymous());
}

#[test]
fn test_anonymous_resolver() {
    let resolver = AuthResolver::anonymous();
    let cred = resolver.resolve("any-registry.io").unwrap();
    assert!(cred.is_anonymous());
    assert_eq!(cred.authorization_header(), None);
}

#[test]
fn test_credential_equality() {
    let cred1 = Credential::basic("user", "pass");
    let cred2 = Credential::basic("user", "pass");
    let cred3 = Credential::basic("user", "different");

    assert_eq!(cred1, cred2);
    assert_ne!(cred1, cred3);
}

#[test]
fn test_credential_default_is_anonymous() {
    let cred: Credential = Default::default();
    assert!(cred.is_anonymous());
}

// ============================================================================
// Helper Credential Conversion Tests
// ============================================================================

#[test]
fn test_helper_credential_empty_username_with_secret() {
    use containerregistry_auth::HelperCredential;

    let helper = HelperCredential {
        username: Some("".to_string()), // Empty username
        secret: Some("token".to_string()),
        server_url: None,
    };

    let cred = helper.into_credential();
    // Empty username with secret should become identity token
    assert!(matches!(cred, Credential::IdentityToken(_)));
}

#[test]
fn test_helper_credential_no_secret() {
    use containerregistry_auth::HelperCredential;

    let helper = HelperCredential {
        username: Some("user".to_string()),
        secret: None,
        server_url: None,
    };

    let cred = helper.into_credential();
    // No secret means anonymous
    assert!(cred.is_anonymous());
}

#[test]
fn test_helper_credential_empty_both() {
    use containerregistry_auth::HelperCredential;

    let helper = HelperCredential {
        username: None,
        secret: Some("".to_string()), // Empty secret
        server_url: None,
    };

    let cred = helper.into_credential();
    assert!(cred.is_anonymous());
}

// ============================================================================
// JSON Roundtrip Tests
// ============================================================================

#[test]
fn test_config_json_roundtrip() {
    let json = r#"{
        "auths": {
            "gcr.io": {
                "auth": "dXNlcjpwYXNz"
            }
        },
        "credsStore": "osxkeychain",
        "credHelpers": {
            "ecr.aws": "ecr-login"
        }
    }"#;

    let config = DockerConfig::from_json(json).unwrap();

    // Verify all fields parsed correctly
    assert!(config.get_auth("gcr.io").is_some());
    assert_eq!(config.get_credential_helper("ecr.aws"), Some("ecr-login"));
    assert_eq!(
        config.get_credential_helper("other.io"),
        Some("osxkeychain")
    );
}
