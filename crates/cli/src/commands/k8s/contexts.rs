//! Kubeconfig selection and proof that a context belongs to a local runtime.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::apple;
use super::cluster::{self, Target, kubectl_program};
use crate::commands::docker::Asker;

pub(super) fn list() -> Result<Vec<String>> {
    let output = Command::new(kubectl_program())
        .args(["config", "get-contexts", "--output", "name"])
        .output()
        .context("Failed to list Kubernetes contexts")?;
    if !output.status.success() {
        bail!(
            "Could not list Kubernetes contexts: {}",
            cluster::output_text(&output)
        );
    }
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect())
}

pub(super) fn select(asker: Asker) -> Result<String> {
    let contexts = list()?;
    if contexts.is_empty() {
        if cfg!(target_os = "macos") {
            if asker.can_ask() && apple::available() {
                let agreed = dialoguer::Confirm::with_theme(crate::ui::theme())
                    .with_prompt("Create a local Apple Container cluster with access to your shared image store?")
                    .default(false)
                    .interact()
                    .context("Failed to read the cluster choice")?;
                if agreed {
                    let name = apple::DEFAULT_NAME;
                    apple::create(name)?;
                    return Ok(name.to_string());
                }
            }
            bail!(
                "No Kubernetes contexts. Run `everruns k8s cluster create --provider apple` on this Mac, or add an existing cluster to kubeconfig."
            );
        }
        bail!(
            "No Kubernetes contexts. Create a local cluster or add an existing cluster to kubeconfig."
        );
    }

    let current = cluster::current_context()
        .ok()
        .filter(|name| contexts.contains(name));
    if contexts.len() > 1 && asker.can_ask() {
        let default = current
            .as_ref()
            .and_then(|name| contexts.iter().position(|ctx| ctx == name))
            .unwrap_or(0);
        let index = dialoguer::Select::with_theme(crate::ui::theme())
            .with_prompt("Kubernetes context")
            .items(&contexts)
            .default(default)
            .interact()
            .context("Failed to read the cluster choice")?;
        return Ok(contexts[index].clone());
    }
    current
        .or_else(|| (contexts.len() == 1).then(|| contexts[0].clone()))
        .context("No current Kubernetes context. Select one with --context <name>.")
}

/// Match both the API address and the CA reported by a runtime on this machine.
/// Context names and loopback addresses alone can identify a forwarded remote API.
pub(super) fn is_local(target: &Target) -> bool {
    let Some(actual) = kubeconfig(target.kubectl().args([
        "config",
        "view",
        "--minify",
        "--flatten",
        "--raw",
        "--output",
        "json",
    ])) else {
        return false;
    };
    let local = if let Some(name) = target.context.strip_prefix("kind-") {
        kubeconfig(Command::new("kind").args(["get", "kubeconfig", "--name", name]))
    } else if let Some(name) = target.context.strip_prefix("k3d-") {
        kubeconfig(Command::new("k3d").args(["kubeconfig", "get", name]))
    } else {
        None
    };
    local.is_some_and(|expected| same_cluster(&actual, &expected))
        || apple::kubeconfig(&target.context)
            .is_some_and(|expected| same_cluster(&actual, &expected))
}

fn kubeconfig(command: &mut Command) -> Option<Value> {
    let output = command.output().ok().filter(|o| o.status.success())?;
    serde_yaml::from_slice(&output.stdout).ok()
}

fn identity(config: &Value) -> Option<(&str, &str)> {
    let cluster = config
        .get("clusters")?
        .as_array()?
        .first()?
        .get("cluster")?;
    if cluster.get("proxy-url").and_then(Value::as_str).is_some()
        || cluster
            .get("insecure-skip-tls-verify")
            .and_then(Value::as_bool)
            == Some(true)
    {
        return None;
    }
    let server = cluster.get("server")?.as_str()?;
    let ca = cluster.get("certificate-authority-data")?.as_str()?;
    (!server.is_empty() && !ca.is_empty()).then_some((server, ca))
}

fn same_cluster(actual: &Value, expected: &Value) -> bool {
    identity(actual).is_some_and(|actual| Some(actual) == identity(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_api_address_and_ca_must_match_the_local_runtime() {
        let config = |server, ca, proxy| {
            json!({"clusters": [{"cluster": {
                "server": server, "certificate-authority-data": ca, "proxy-url": proxy
            }}]})
        };
        let local = config("https://127.0.0.1:6443", "local-ca", None);
        assert!(same_cluster(&local, &local));
        assert!(!same_cluster(
            &local,
            &config("https://127.0.0.1:6443", "remote-ca", None)
        ));
        assert!(!same_cluster(
            &local,
            &config("https://remote.example:6443", "local-ca", None)
        ));
        assert!(!same_cluster(
            &local,
            &config("https://127.0.0.1:6443", "local-ca", Some("http://proxy"))
        ));
    }
}
