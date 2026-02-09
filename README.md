# containerregistry-rs

A Rust implementation of [go-containerregistry](https://github.com/google/go-containerregistry), providing OCI container registry client libraries and `crane`/`gcrane`-compatible CLI tools.

## Features

- **OCI & Docker support** — Handles both OCI Image/Distribution specs and Docker v2 schema 2 manifests, with automatic format detection
- **Multi-arch images** — Full support for image indexes / manifest lists with platform filtering
- **OCI image layouts** — Read and write [OCI Image Layout](https://github.com/opencontainers/image-spec/blob/main/image-layout.md) directories
- **Auth** — Docker config (`~/.docker/config.json`), credential helpers, bearer token negotiation, and basic auth
- **Digest verification** — SHA-256/384/512 content verification on all downloads
- **Retry & resilience** — Exponential backoff with `Retry-After` support, automatic bearer token refresh on 401

## Crates

| Package | Path | Description |
|---------|------|-------------|
| `containerregistry` | `.` | Facade crate re-exporting all library crates |
| `containerregistry-image` | `crates/image` | Core types: `Manifest`, `ImageIndex`, `Descriptor`, `Digest`, `MediaType`, `Platform` |
| `containerregistry-registry` | `crates/registry` | HTTP registry client, reference parsing, retry logic, metrics |
| `containerregistry-auth` | `crates/auth` | Credential resolution: Docker config, credential helpers, bearer/basic auth |
| `containerregistry-layout` | `crates/layout` | OCI image layout read/write |
| `containerregistry-crane` | `bins/crane` | crane-compatible CLI (digest, manifest, pull, push, copy, ls) |
| `containerregistry-gcrane` | `bins/gcrane` | gcrane-compatible CLI (cp, ls) |
| `containerregistry-testing` | `crates/testing` | Test harness: fixtures, golden comparators, parity runner, fault injection |

## Quick Start

### As a library

```rust
use containerregistry::registry::{Client, Reference};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::new()?;
    let reference: Reference = "ghcr.io/my-org/my-image:latest".parse()?;

    // Get manifest (auto-detects manifest vs index)
    let (manifest_or_index, digest) = client.get_manifest(&reference).await?;
    println!("digest: {}", digest);

    // Download a blob
    let blob = client.get_blob(&reference, &digest).await?;

    Ok(())
}
```

### CLI

```bash
# Build the CLI tools
cargo build --release

# Get image digest
crane digest ghcr.io/my-org/my-image:latest

# Pull to OCI layout
crane pull --format oci --output ./image ghcr.io/my-org/my-image:latest

# Push from OCI layout
crane push ghcr.io/my-org/my-image:v2 ./image

# List tags
crane ls ghcr.io/my-org/my-image

# Copy between registries
crane copy source-registry/image:tag dest-registry/image:tag
```

## Building

```bash
cargo build            # Debug build
cargo build --release  # Release build
cargo clippy           # Lint (CI enforces -D warnings)
cargo fmt              # Format
```

## Testing

```bash
# Unit and mock-server tests
cargo test

# Integration tests (requires local Docker registries)
cd tests/compose/registry && docker compose up -d
REGISTRY_INTEGRATION=1 cargo test
cd tests/compose/registry && docker compose down -v

# Parity tests (compare Rust crane vs Go crane)
CRANE_RUST_PATH=./target/debug/crane cargo test -p containerregistry-testing
```

## Specifications

- [OCI Distribution Spec](https://github.com/opencontainers/distribution-spec/blob/main/spec.md)
- [OCI Image Spec](https://github.com/opencontainers/image-spec/blob/main/spec.md)
- [OCI Image Layout Spec](https://github.com/opencontainers/image-spec/blob/main/image-layout.md)

When specs are ambiguous, go-containerregistry behavior is followed.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
