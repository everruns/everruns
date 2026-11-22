//! Native Kubernetes clusters from Apple Container.

use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::Value;

use super::cluster::{Target, kubectl_program};
use super::{contexts, storage};
use crate::ui;

pub(super) const DEFAULT_NAME: &str = "everruns-dev";

#[derive(Subcommand)]
pub enum ClusterCommand {
    /// Create a local Kubernetes cluster with a shared image store and storage provisioner
    Create(CreateArgs),
}

#[derive(Args)]
pub struct CreateArgs {
    /// Local Kubernetes provider
    #[arg(long, value_enum)]
    provider: ClusterProvider,
    /// Name for the new cluster and its kubeconfig context
    #[arg(long, default_value = DEFAULT_NAME)]
    name: String,
}

#[derive(Clone, ValueEnum)]
enum ClusterProvider {
    Apple,
}

pub(super) fn run(command: ClusterCommand) -> Result<()> {
    match command {
        ClusterCommand::Create(CreateArgs {
            provider: ClusterProvider::Apple,
            name,
        }) => create(&name),
    }
}

pub(super) fn available() -> bool {
    cfg!(target_os = "macos")
        && Command::new("container")
            .args(["cluster", "create", "--help"])
            .output()
            .is_ok_and(|out| {
                out.status.success()
                    && String::from_utf8_lossy(&out.stdout).contains("--shared-image-store")
            })
}

pub(super) fn create(name: &str) -> Result<()> {
    if !available() {
        bail!(
            "Apple clusters require Apple Container with `container cluster create --shared-image-store` on macOS. Install or update it from https://github.com/apple/container/releases."
        );
    }
    super::Instance::new(name).context("Invalid cluster name")?;
    if contexts::list()?.iter().any(|context| context == name) {
        bail!(
            "A Kubernetes context named '{name}' already exists. Use --name for a new cluster, or install into the existing one with --context."
        );
    }

    ui::note("This cluster can read images in your local Apple Container image store.");
    run_container(&["system", "start"])?;
    run_container(&[
        "cluster",
        "create",
        "--name",
        name,
        "--shared-image-store",
        "--memory",
        "4g",
    ])?;

    let target = Target {
        context: name.to_string(),
        namespace: "default".to_string(),
        release: name.to_string(),
    };
    if !contexts::is_local(&target) {
        bail!(
            "Cluster '{name}' was created, but kubectl does not resolve it to the local Apple cluster. Check KUBECONFIG, then run `everruns k8s up --context {name}`."
        );
    }
    let class = storage::ensure_default(&target)?;
    ui::ok(format!(
        "Local cluster '{name}' is ready with StorageClass {class}"
    ));
    ui::note(format!(
        "Install Everruns with `everruns k8s up --context {name} --bundled-postgres`."
    ));
    Ok(())
}

fn run_container(args: &[&str]) -> Result<()> {
    let status = Command::new("container")
        .args(args)
        .status()
        .context("Failed to run Apple Container")?;
    if !status.success() {
        bail!("`container {}` failed with {status}", args.join(" "));
    }
    Ok(())
}

/// Apple writes a fresh kubeconfig from its own cluster, including its CA.
/// Keep it in a private temporary directory and remove it after comparison.
pub(super) fn kubeconfig(name: &str) -> Option<Value> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let path = dir.path().join("kubeconfig");
    let status = Command::new("container")
        .args(["cluster", "write-config", "--name", name, "--kubeconfig"])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    // Flatten exactly as kubectl does for the active config.
    let output = Command::new(kubectl_program())
        .arg("--kubeconfig")
        .arg(&path)
        .args(["config", "view", "--flatten", "--raw", "--output", "json"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}
