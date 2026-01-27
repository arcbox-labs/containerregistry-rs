//! crane CLI - A tool for interacting with container registries.

use std::io;

use anyhow::{Context, Result};
use clap::Parser;

use containerregistry_image::{Digest, ImageIndex, Manifest, Platform};
use containerregistry_layout::Layout;
use containerregistry_registry::{Client, ClientConfig, ManifestOrIndex, Reference};

#[derive(Parser)]
#[command(name = "crane")]
#[command(about = "A tool for interacting with container registries")]
#[command(version)]
struct Cli {
    /// Allow insecure connections to registries
    #[arg(long, global = true)]
    insecure: bool,

    /// Platform to use for multi-arch images (e.g., linux/amd64)
    #[arg(long, global = true)]
    platform: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Get the digest of an image
    Digest {
        /// Image reference
        image: String,
        /// Use full digest output (sha256:...)
        #[arg(long)]
        full_ref: bool,
    },
    /// Pull an image to a tarball or OCI layout
    Pull {
        /// Image reference
        image: String,
        /// Output path (tarball or directory)
        #[arg(short, long)]
        output: Option<String>,
        /// Output format: tarball or oci
        #[arg(long, default_value = "oci")]
        format: String,
    },
    /// Push an image from a tarball or OCI layout
    Push {
        /// Image reference
        image: String,
        /// Input path (tarball or directory)
        input: String,
    },
    /// Copy an image from one registry to another
    Copy {
        /// Source image reference
        source: String,
        /// Destination image reference
        destination: String,
    },
    /// Get the manifest of an image
    Manifest {
        /// Image reference
        image: String,
    },
    /// List tags for a repository
    Ls {
        /// Repository reference (e.g., gcr.io/library/alpine)
        repository: String,
    },
    /// Get the config of an image
    Config {
        /// Image reference
        image: String,
    },
    /// List the repositories in a registry (catalog)
    Catalog {
        /// Registry hostname
        registry: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_writer(io::stderr)
        .init();

    let cli = Cli::parse();

    // Create client with configuration
    let config = ClientConfig::new()
        .with_https(!cli.insecure)
        .with_insecure(cli.insecure);
    let client = Client::with_config(config)?;

    // Parse platform if specified
    let platform = cli
        .platform
        .as_ref()
        .map(|p| parse_platform(p))
        .transpose()?;

    match cli.command {
        Commands::Digest { image, full_ref } => {
            cmd_digest(&client, &image, full_ref, platform.as_ref()).await
        }
        Commands::Pull {
            image,
            output,
            format,
        } => {
            cmd_pull(
                &client,
                &image,
                output.as_deref(),
                &format,
                platform.as_ref(),
            )
            .await
        }
        Commands::Push { image, input } => cmd_push(&client, &image, &input).await,
        Commands::Copy {
            source,
            destination,
        } => cmd_copy(&client, &source, &destination, platform.as_ref()).await,
        Commands::Manifest { image } => cmd_manifest(&client, &image, platform.as_ref()).await,
        Commands::Ls { repository } => cmd_ls(&client, &repository).await,
        Commands::Config { image } => cmd_config(&client, &image, platform.as_ref()).await,
        Commands::Catalog { registry } => cmd_catalog(&client, &registry).await,
    }
}

/// Parse a platform string like "linux/amd64" into a Platform.
fn parse_platform(s: &str) -> Result<Platform> {
    let parts: Vec<&str> = s.split('/').collect();
    match parts.as_slice() {
        [os, arch] => Ok(Platform::new(os.to_string(), arch.to_string())),
        [os, arch, variant] => {
            let mut p = Platform::new(os.to_string(), arch.to_string());
            p.variant = Some(variant.to_string());
            Ok(p)
        }
        _ => anyhow::bail!(
            "invalid platform format: {}, expected os/arch or os/arch/variant",
            s
        ),
    }
}

/// Find a matching platform in an index.
fn find_platform_in_index<'a>(
    index: &'a ImageIndex,
    platform: &Platform,
) -> Option<&'a containerregistry_image::Descriptor> {
    index.manifests().iter().find(|m| {
        m.platform.as_ref().is_some_and(|p| {
            p.os == platform.os
                && p.architecture == platform.architecture
                && (platform.variant.is_none() || p.variant == platform.variant)
        })
    })
}

/// Get the digest of an image.
async fn cmd_digest(
    client: &Client,
    image: &str,
    full_ref: bool,
    platform: Option<&Platform>,
) -> Result<()> {
    let reference: Reference = image.parse().context("invalid image reference")?;

    let (manifest_or_index, digest) = client
        .get_manifest(&reference)
        .await
        .context("failed to get manifest")?;

    // If it's an index and platform is specified, find the matching manifest
    let final_digest = if let Some(index) = manifest_or_index.as_index() {
        if let Some(platform) = platform {
            let matching =
                find_platform_in_index(index, platform).context("no matching platform in index")?;
            matching.digest.clone()
        } else {
            // Return the index digest
            digest
        }
    } else {
        digest
    };

    if full_ref {
        println!(
            "{}/{}@{}",
            reference.registry(),
            reference.repository(),
            final_digest
        );
    } else {
        println!("{}", final_digest);
    }

    Ok(())
}

/// Get the manifest of an image.
async fn cmd_manifest(client: &Client, image: &str, platform: Option<&Platform>) -> Result<()> {
    let reference: Reference = image.parse().context("invalid image reference")?;

    let (manifest_or_index, _digest) = client
        .get_manifest(&reference)
        .await
        .context("failed to get manifest")?;

    // If it's an index and platform is specified, get the specific manifest
    let output = if let Some(index) = manifest_or_index.as_index() {
        if let Some(platform) = platform {
            let matching =
                find_platform_in_index(index, platform).context("no matching platform in index")?;

            // Fetch the actual manifest
            let manifest_ref = reference.clone().with_new_digest(matching.digest.clone());
            let (inner_manifest, _) = client
                .get_manifest(&manifest_ref)
                .await
                .context("failed to get platform manifest")?;

            match inner_manifest {
                ManifestOrIndex::Manifest(m) => {
                    String::from_utf8(m.to_bytes()?).context("manifest is not valid UTF-8")?
                }
                ManifestOrIndex::Index(i) => {
                    String::from_utf8(i.to_bytes()?).context("index is not valid UTF-8")?
                }
            }
        } else {
            String::from_utf8(index.to_bytes()?).context("index is not valid UTF-8")?
        }
    } else if let Some(manifest) = manifest_or_index.as_manifest() {
        String::from_utf8(manifest.to_bytes()?).context("manifest is not valid UTF-8")?
    } else {
        anyhow::bail!("unexpected manifest type");
    };

    println!("{}", output);
    Ok(())
}

/// List tags for a repository.
async fn cmd_ls(client: &Client, repository: &str) -> Result<()> {
    let reference: Reference = repository.parse().context("invalid repository reference")?;

    let tags = client
        .list_tags(&reference)
        .await
        .context("failed to list tags")?;

    for tag in tags {
        println!("{}", tag);
    }

    Ok(())
}

/// Get the config of an image.
async fn cmd_config(client: &Client, image: &str, platform: Option<&Platform>) -> Result<()> {
    let reference: Reference = image.parse().context("invalid image reference")?;

    let (manifest_or_index, _) = client
        .get_manifest(&reference)
        .await
        .context("failed to get manifest")?;

    // Resolve to a manifest (handle multi-arch)
    let manifest = if let Some(index) = manifest_or_index.as_index() {
        if let Some(platform) = platform {
            let matching =
                find_platform_in_index(index, platform).context("no matching platform in index")?;

            // Fetch the actual manifest
            let manifest_ref = reference.clone().with_new_digest(matching.digest.clone());
            let (inner, _) = client
                .get_manifest(&manifest_ref)
                .await
                .context("failed to get platform manifest")?;
            inner
                .into_manifest()
                .context("expected manifest, got index")?
        } else {
            anyhow::bail!("image is a multi-arch index, use --platform to select one");
        }
    } else {
        manifest_or_index
            .into_manifest()
            .context("expected manifest")?
    };

    // Get the config blob
    let config_digest = &manifest.config().digest;
    let config_bytes = client
        .get_blob(&reference, config_digest)
        .await
        .context("failed to get config blob")?;

    // Output as JSON
    let config_str = String::from_utf8(config_bytes).context("config is not valid UTF-8")?;
    println!("{}", config_str);

    Ok(())
}

/// Pull an image to a tarball or OCI layout.
async fn cmd_pull(
    client: &Client,
    image: &str,
    output: Option<&str>,
    format: &str,
    platform: Option<&Platform>,
) -> Result<()> {
    let reference: Reference = image.parse().context("invalid image reference")?;

    let (manifest_or_index, digest) = client
        .get_manifest(&reference)
        .await
        .context("failed to get manifest")?;

    // Resolve to a manifest if needed, tracking the digest to return.
    let (manifest, manifest_digest) = match manifest_or_index {
        ManifestOrIndex::Index(index) => {
            if let Some(platform) = platform {
                let matching = find_platform_in_index(&index, platform)
                    .context("no matching platform in index")?;

                let manifest_ref = reference.clone().with_new_digest(matching.digest.clone());
                let (inner, inner_digest) = client
                    .get_manifest(&manifest_ref)
                    .await
                    .context("failed to get platform manifest")?;
                let manifest = inner
                    .into_manifest()
                    .context("expected manifest, got index")?;
                (manifest, inner_digest)
            } else {
                anyhow::bail!("image is a multi-arch index, use --platform to select one");
            }
        }
        ManifestOrIndex::Manifest(manifest) => (*manifest, digest),
    };

    // Determine output path
    let output_path = output.unwrap_or("image");

    match format {
        "oci" => {
            // Write to OCI layout directory
            let layout = Layout::create(output_path).context("failed to create OCI layout")?;

            // Write config blob
            let config_bytes = client
                .get_blob(&reference, &manifest.config().digest)
                .await
                .context("failed to get config")?;
            layout
                .write_blob_with_digest(&config_bytes, &manifest.config().digest)
                .context("failed to write config")?;

            // Write layer blobs
            for layer in manifest.layers() {
                let layer_bytes = client
                    .get_blob(&reference, &layer.digest)
                    .await
                    .with_context(|| format!("failed to get layer {}", layer.digest))?;
                layout
                    .write_blob_with_digest(&layer_bytes, &layer.digest)
                    .with_context(|| format!("failed to write layer {}", layer.digest))?;
            }

            // Add manifest to index (this also writes the manifest blob)
            layout
                .add_manifest(&manifest)
                .context("failed to add manifest to index")?;

            eprintln!("Pulled {} to {}", image, output_path);
            println!("{}", manifest_digest);
        }
        "tarball" => {
            // For tarball format, we'd need to implement Docker tarball format
            anyhow::bail!("tarball format not yet implemented, use --format=oci");
        }
        _ => {
            anyhow::bail!("unknown format: {}, use 'oci' or 'tarball'", format);
        }
    }

    Ok(())
}

/// Push an image from a tarball or OCI layout.
async fn cmd_push(client: &Client, image: &str, input: &str) -> Result<()> {
    let reference: Reference = image.parse().context("invalid image reference")?;

    // Load from OCI layout
    let layout = Layout::open(input).context("failed to open OCI layout")?;

    let index = layout.index().context("failed to read index")?;
    if index.manifests().is_empty() {
        anyhow::bail!("no manifests in layout index");
    }

    let digest = push_index_from_layout(client, &layout, &reference, &index).await?;

    eprintln!("Pushed {} ({})", image, digest);
    println!("{}", digest);

    Ok(())
}

/// Copy an image from one registry to another.
async fn cmd_copy(
    client: &Client,
    source: &str,
    destination: &str,
    platform: Option<&Platform>,
) -> Result<()> {
    let src_ref: Reference = source.parse().context("invalid source reference")?;
    let dst_ref: Reference = destination
        .parse()
        .context("invalid destination reference")?;

    let (manifest_or_index, _) = client
        .get_manifest(&src_ref)
        .await
        .context("failed to get source manifest")?;

    match manifest_or_index {
        ManifestOrIndex::Manifest(manifest) => {
            // Copy single manifest
            copy_manifest(client, &src_ref, &dst_ref, &manifest).await?;
        }
        ManifestOrIndex::Index(index) => {
            if let Some(platform) = platform {
                // Copy only the specified platform
                let matching = find_platform_in_index(&index, platform)
                    .context("no matching platform in index")?;

                let manifest_ref = src_ref.clone().with_new_digest(matching.digest.clone());
                let (inner, _) = client
                    .get_manifest(&manifest_ref)
                    .await
                    .context("failed to get platform manifest")?;

                let manifest = inner
                    .into_manifest()
                    .context("expected manifest, got index")?;
                copy_manifest(client, &src_ref, &dst_ref, &manifest).await?;
            } else {
                // Copy entire index with all manifests
                copy_index(client, &src_ref, &dst_ref, &index).await?;
            }
        }
    }

    eprintln!("Copied {} -> {}", source, destination);
    Ok(())
}

/// Copy a single manifest and its blobs.
async fn copy_manifest(
    client: &Client,
    src_ref: &Reference,
    dst_ref: &Reference,
    manifest: &Manifest,
) -> Result<()> {
    // Copy config blob
    let config_bytes = client
        .get_blob(src_ref, &manifest.config().digest)
        .await
        .context("failed to get config")?;
    client
        .put_blob_with_digest(dst_ref, &config_bytes, &manifest.config().digest)
        .await
        .context("failed to push config")?;

    // Copy layer blobs
    for layer in manifest.layers() {
        let layer_bytes = client
            .get_blob(src_ref, &layer.digest)
            .await
            .with_context(|| format!("failed to get layer {}", layer.digest))?;
        client
            .put_blob_with_digest(dst_ref, &layer_bytes, &layer.digest)
            .await
            .with_context(|| format!("failed to push layer {}", layer.digest))?;
    }

    // Push manifest
    client
        .put_manifest(dst_ref, manifest)
        .await
        .context("failed to push manifest")?;

    Ok(())
}

/// Push a manifest and its blobs from an OCI layout to a registry.
async fn push_manifest_from_layout(
    client: &Client,
    layout: &Layout,
    dst_ref: &Reference,
    manifest: &Manifest,
) -> Result<()> {
    let config_bytes = layout
        .read_blob(&manifest.config().digest)
        .context("failed to read config")?;
    client
        .put_blob_with_digest(dst_ref, &config_bytes, &manifest.config().digest)
        .await
        .context("failed to push config")?;

    for layer in manifest.layers() {
        let layer_bytes = layout
            .read_blob(&layer.digest)
            .with_context(|| format!("failed to read layer {}", layer.digest))?;
        client
            .put_blob_with_digest(dst_ref, &layer_bytes, &layer.digest)
            .await
            .with_context(|| format!("failed to push layer {}", layer.digest))?;
    }

    client
        .put_manifest(dst_ref, manifest)
        .await
        .context("failed to push manifest")?;

    Ok(())
}

/// Push an image index and all referenced manifests from an OCI layout.
fn push_index_from_layout<'a>(
    client: &'a Client,
    layout: &'a Layout,
    dst_ref: &'a Reference,
    index: &'a ImageIndex,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Digest>> + Send + 'a>> {
    Box::pin(async move {
        for manifest_desc in index.manifests() {
            let manifest_ref = dst_ref
                .clone()
                .with_new_digest(manifest_desc.digest.clone());
            let manifest_bytes = layout
                .read_blob(&manifest_desc.digest)
                .with_context(|| format!("failed to read manifest {}", manifest_desc.digest))?;

            if manifest_desc.media_type.is_index() {
                let nested_index =
                    ImageIndex::from_bytes(&manifest_bytes).context("failed to parse index")?;
                push_index_from_layout(client, layout, &manifest_ref, &nested_index).await?;
            } else {
                let manifest =
                    Manifest::from_bytes(&manifest_bytes).context("failed to parse manifest")?;
                push_manifest_from_layout(client, layout, &manifest_ref, &manifest).await?;
            }
        }

        let digest = client
            .put_index(dst_ref, index)
            .await
            .context("failed to push index")?;

        Ok(digest)
    })
}

/// Copy an entire index and all its manifests.
fn copy_index<'a>(
    client: &'a Client,
    src_ref: &'a Reference,
    dst_ref: &'a Reference,
    index: &'a ImageIndex,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        // Copy each manifest in the index
        for manifest_desc in index.manifests() {
            let manifest_ref = src_ref
                .clone()
                .with_new_digest(manifest_desc.digest.clone());
            let target_ref = dst_ref
                .clone()
                .with_new_digest(manifest_desc.digest.clone());
            let (inner, _) = client
                .get_manifest(&manifest_ref)
                .await
                .with_context(|| format!("failed to get manifest {}", manifest_desc.digest))?;

            match inner {
                ManifestOrIndex::Manifest(m) => {
                    copy_manifest(client, src_ref, &target_ref, &m).await?;
                }
                ManifestOrIndex::Index(nested_index) => {
                    // Recursively copy nested index
                    copy_index(client, src_ref, &target_ref, &nested_index).await?;
                }
            }
        }

        // Push the index itself
        client
            .put_index(dst_ref, index)
            .await
            .context("failed to push index")?;

        Ok(())
    })
}

/// List repositories in a registry (catalog).
async fn cmd_catalog(_client: &Client, registry: &str) -> Result<()> {
    // The catalog API is not implemented in the registry client yet
    anyhow::bail!("catalog not yet implemented for registry {}", registry);
}
