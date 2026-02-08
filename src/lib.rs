//! Rust client libraries for OCI container registries.
//!
//! This crate re-exports the core library crates for convenience:
//!
//! - [`image`] — Manifest, index, descriptor, digest, and platform types
//! - [`layout`] — OCI image layout read/write
//! - [`registry`] — HTTP registry client, reference parsing, retry logic
//! - [`auth`] — Credential resolution and Docker config parsing

pub use containerregistry_auth as auth;
pub use containerregistry_image as image;
pub use containerregistry_layout as layout;
pub use containerregistry_registry as registry;
