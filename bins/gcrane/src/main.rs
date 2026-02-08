//! gcrane CLI - A tool for GCR-specific container registry operations.
//!
//! gcrane provides GCR-optimized commands, including recursive repository
//! copying and cross-repository blob mounting.

use std::io;

use anyhow::{Context, Result};
use clap::Parser;

use containerregistry_image::{ImageIndex, Manifest, Platform};
use containerregistry_registry::{Client, ClientConfig, ManifestOrIndex, Reference};

#[derive(Parser)]
#[command(name = "gcrane")]
#[command(about = "A tool for GCR-specific container registry operations")]
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
    /// Copy images with GCR optimizations
    Cp {
        /// Source image or repository reference
        source: String,
        /// Destination image or repository reference
        destination: String,
        /// Copy all tags recursively (repository mode)
        #[arg(short, long)]
        recursive: bool,
        /// Number of concurrent copy operations
        #[arg(short = 'j', long, default_value = "4")]
        jobs: usize,
    },
    /// List all tags in a repository
    Ls {
        /// Repository reference
        repository: String,
    },
    /// Garbage collect unreferenced blobs (not yet implemented)
    Gc {
        /// Repository reference
        repository: String,
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
        Commands::Cp {
            source,
            destination,
            recursive,
            jobs,
        } => {
            cmd_cp(
                &client,
                &source,
                &destination,
                recursive,
                jobs,
                platform.as_ref(),
            )
            .await
        }
        Commands::Ls { repository } => cmd_ls(&client, &repository).await,
        Commands::Gc { repository } => cmd_gc(&repository).await,
    }
}

/// Parse a platform string like "linux/amd64" into a Platform.
fn parse_platform(s: &str) -> Result<Platform> {
    let parts: Vec<&str> = s.split('/').collect();
    match parts.as_slice() {
        [os, arch] => Ok(Platform::new(arch.to_string(), os.to_string())),
        [os, arch, variant] => {
            let mut p = Platform::new(arch.to_string(), os.to_string());
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

/// Copy images with GCR optimizations.
async fn cmd_cp(
    client: &Client,
    source: &str,
    destination: &str,
    recursive: bool,
    jobs: usize,
    platform: Option<&Platform>,
) -> Result<()> {
    let src_ref: Reference = source.parse().context("invalid source reference")?;
    let dst_ref: Reference = destination
        .parse()
        .context("invalid destination reference")?;

    if recursive {
        // Copy all tags from source repository to destination
        copy_repository(client, &src_ref, &dst_ref, jobs, platform).await
    } else {
        // Copy single image
        copy_single_image(client, &src_ref, &dst_ref, platform).await
    }
}

/// Copy all tags from a repository.
async fn copy_repository(
    client: &Client,
    src_ref: &Reference,
    dst_ref: &Reference,
    jobs: usize,
    platform: Option<&Platform>,
) -> Result<()> {
    // List all tags in the source repository
    let tags = client
        .list_tags(src_ref)
        .await
        .context("failed to list source tags")?;

    if tags.is_empty() {
        eprintln!("No tags found in {}", src_ref);
        return Ok(());
    }

    eprintln!("Found {} tags to copy", tags.len());

    // Copy tags with concurrency limit
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(jobs));
    let mut handles = Vec::new();

    for tag in tags {
        let client = client.clone();
        let src_ref = src_ref.clone().with_new_tag(tag.clone());
        let dst_ref = dst_ref.clone().with_new_tag(tag.clone());
        let semaphore = semaphore.clone();
        let platform = platform.cloned();

        let handle = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let result = copy_single_image(&client, &src_ref, &dst_ref, platform.as_ref()).await;
            (tag, result)
        });

        handles.push(handle);
    }

    // Collect results
    let mut success_count = 0;
    let mut error_count = 0;

    for handle in handles {
        match handle.await {
            Ok((tag, Ok(()))) => {
                eprintln!("  {} copied", tag);
                success_count += 1;
            }
            Ok((tag, Err(e))) => {
                eprintln!("  {} failed: {}", tag, e);
                error_count += 1;
            }
            Err(e) => {
                eprintln!("  task failed: {}", e);
                error_count += 1;
            }
        }
    }

    eprintln!(
        "Copied {}/{} tags ({} errors)",
        success_count,
        success_count + error_count,
        error_count
    );

    if error_count > 0 {
        anyhow::bail!("{} tags failed to copy", error_count);
    }

    Ok(())
}

/// Copy a single image.
async fn copy_single_image(
    client: &Client,
    src_ref: &Reference,
    dst_ref: &Reference,
    platform: Option<&Platform>,
) -> Result<()> {
    let (manifest_or_index, _) = client
        .get_manifest(src_ref)
        .await
        .context("failed to get source manifest")?;

    match manifest_or_index {
        ManifestOrIndex::Manifest(manifest) => {
            copy_manifest(client, src_ref, dst_ref, &manifest).await?;
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
                copy_manifest(client, src_ref, dst_ref, &manifest).await?;
            } else {
                // Copy entire index with all manifests
                copy_index(client, src_ref, dst_ref, &index).await?;
            }
        }
    }

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
        .put_blob(dst_ref, &config_bytes)
        .await
        .context("failed to push config")?;

    // Copy layer blobs
    for layer in manifest.layers() {
        let layer_bytes = client
            .get_blob(src_ref, &layer.digest)
            .await
            .with_context(|| format!("failed to get layer {}", layer.digest))?;
        client
            .put_blob(dst_ref, &layer_bytes)
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
            let (inner, _) = client
                .get_manifest(&manifest_ref)
                .await
                .with_context(|| format!("failed to get manifest {}", manifest_desc.digest))?;

            match inner {
                ManifestOrIndex::Manifest(m) => {
                    copy_manifest(client, src_ref, dst_ref, &m).await?;
                }
                ManifestOrIndex::Index(nested_index) => {
                    copy_index(client, src_ref, dst_ref, &nested_index).await?;
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

/// List all tags in a repository.
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

/// Garbage collect unreferenced blobs.
async fn cmd_gc(repository: &str) -> Result<()> {
    anyhow::bail!("gc not yet implemented for repository {}", repository);
}
