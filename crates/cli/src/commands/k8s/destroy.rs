//! `everruns k8s destroy`: delete an install and everything it made.
//!
//! Decisions:
//! - The namespace goes too, but only when nothing but Kubernetes' own
//!   defaults is left in it once the install is gone. A namespace someone
//!   shares with other workloads, or made by hand for something else, is
//!   kept and the command says what is still in it. `default` and `kube-*`
//!   are never deleted.
//! - Objects still labeled with this release (pods that are terminating) do
//!   not count as something else.

use std::io::IsTerminal;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::cluster::Target;
use super::{DestroyArgs, existing};
use crate::commands::docker;
use crate::ui;

/// Namespaced kinds checked before the namespace is deleted.
const LEFTOVER_KINDS: &str = "all,pvc,configmaps,secrets,ingresses,serviceaccounts,roles,\
                              rolebindings,networkpolicies,jobs,cronjobs";

pub(super) fn destroy(args: &DestroyArgs) -> Result<()> {
    let Some((inst, record)) = existing(&args.instance)? else {
        return Ok(());
    };
    let target = record.target();
    let namespace_too = !args.keep_namespace && deletable_namespace(&target.namespace);
    if !args.yes {
        if !std::io::stdin().is_terminal() {
            bail!(
                "no terminal to ask on; pass --yes to delete '{}'",
                inst.name
            );
        }
        ui::warn(format!(
            "This deletes Everruns '{}' in {}/{}: its workloads, the generated secrets with the \
             encryption key, the bundled database and its backups on this machine, and CLI profile '{}'.",
            inst.name,
            target.context,
            target.namespace,
            inst.profile()
        ));
        if namespace_too {
            ui::detail(format!(
                "Namespace {} goes too if nothing else is in it.",
                target.namespace
            ));
        }
        let typed: String = dialoguer::Input::with_theme(ui::theme())
            .with_prompt(format!("Type '{}' to delete it", inst.name))
            .interact_text()
            .context("Failed to read the answer")?;
        if typed.trim() != inst.name {
            bail!("not deleted");
        }
    }

    if target.release_values()?.is_some() {
        let spinner = ui::Spinner::start(format!("Uninstalling release {}", target.release));
        let out = target
            .helm()
            .args(["uninstall", &target.release, "--wait"])
            .output()
            .context("Failed to run helm")?;
        if !out.status.success() {
            spinner.failed("`helm uninstall` failed");
            eprint!("{}", String::from_utf8_lossy(&out.stderr));
            bail!("`helm uninstall` failed");
        }
        spinner.done("Removed the workloads");
    }
    let full = target.fullname();
    let secrets = [format!("{full}-generated"), format!("{full}-internal")];
    let mut cmd = target.kubectl();
    cmd.args(["delete", "secret", "--ignore-not-found"]);
    cmd.args(&secrets);
    if !cmd
        .stdout(std::process::Stdio::null())
        .status()
        .context("Failed to run kubectl")?
        .success()
    {
        bail!("deleting the generated secrets failed");
    }
    ui::ok("Deleted the generated secrets");
    let volumes = target
        .kubectl()
        .args([
            "delete",
            "pvc",
            "--ignore-not-found",
            "--selector",
            &target.selector(None),
        ])
        .stdout(std::process::Stdio::null())
        .status()
        .context("Failed to run kubectl")?;
    if !volumes.success() {
        bail!("deleting the volumes failed");
    }
    ui::ok("Deleted the volumes");

    if namespace_too {
        delete_namespace_if_empty(&target)?;
    } else if args.keep_namespace {
        ui::note(format!(
            "Kept namespace {} (--keep-namespace).",
            target.namespace
        ));
    } else {
        ui::note(format!("Kept namespace {}.", target.namespace));
    }

    if docker::forget_profile(&inst.profile())? {
        ui::ok(format!("Removed CLI profile '{}'", inst.profile()));
    }
    if inst.dir.exists() {
        std::fs::remove_dir_all(&inst.dir)
            .with_context(|| format!("Failed to remove {}", inst.dir.display()))?;
    }
    ui::ok(format!("Destroyed '{}'.", inst.name));
    Ok(())
}

/// Namespaces that are never deleted, whatever is in them.
fn deletable_namespace(ns: &str) -> bool {
    ns != "default" && !ns.starts_with("kube-")
}

fn delete_namespace_if_empty(target: &Target) -> Result<()> {
    let out = target
        .kubectl()
        .args(["get", LEFTOVER_KINDS, "--ignore-not-found", "-o", "json"])
        .output()
        .context("Failed to run kubectl")?;
    if !out.status.success() {
        // The namespace may already be gone, or listing is not allowed; either
        // way it is not ours to delete blind.
        ui::note(format!(
            "Kept namespace {}: could not list what is in it.",
            target.namespace
        ));
        return Ok(());
    }
    let list: Value = if out.stdout.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&out.stdout).context("kubectl printed something unexpected")?
    };
    let others = leftovers(&list, &target.release);
    if !others.is_empty() {
        let shown = others
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = if others.len() > 5 {
            format!(" and {} more", others.len() - 5)
        } else {
            String::new()
        };
        ui::note(format!(
            "Kept namespace {}: it still has {shown}{more}.",
            target.namespace
        ));
        return Ok(());
    }
    let spinner = ui::Spinner::start(format!("Deleting namespace {}", target.namespace));
    let out = std::process::Command::new(super::cluster::kubectl_program())
        .args([
            "--context",
            &target.context,
            "delete",
            "namespace",
            &target.namespace,
            "--ignore-not-found",
        ])
        .output()
        .context("Failed to run kubectl")?;
    if !out.status.success() {
        spinner.failed(format!("Could not delete namespace {}", target.namespace));
        ui::detail(String::from_utf8_lossy(&out.stderr).trim().to_string());
        return Ok(());
    }
    spinner.done(format!("Deleted namespace {}", target.namespace));
    Ok(())
}

/// What is in the namespace besides this release and Kubernetes' defaults,
/// as `kind/name`.
fn leftovers(list: &Value, release: &str) -> Vec<String> {
    let Some(items) = list.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let kind = item.get("kind")?.as_str()?;
            let meta = item.get("metadata")?;
            let name = meta.get("name")?.as_str()?;
            let ours = meta
                .pointer("/labels/app.kubernetes.io~1instance")
                .and_then(Value::as_str)
                == Some(release);
            let default = matches!(
                (kind, name),
                ("ConfigMap", "kube-root-ca.crt") | ("ServiceAccount", "default")
            ) || (kind == "Secret"
                && item.get("type").and_then(Value::as_str)
                    == Some("kubernetes.io/service-account-token")
                && meta
                    .pointer("/annotations/kubernetes.io~1service-account.name")
                    .and_then(Value::as_str)
                    == Some("default"));
            (!ours && !default).then(|| format!("{}/{name}", kind.to_lowercase()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kubernetes_defaults_and_this_release_are_not_leftovers() {
        let list = json!({"items": [
            {"kind": "ConfigMap", "metadata": {"name": "kube-root-ca.crt"}},
            {"kind": "ServiceAccount", "metadata": {"name": "default"}},
            {"kind": "Secret", "type": "kubernetes.io/service-account-token",
             "metadata": {"name": "default-token-x",
                          "annotations": {"kubernetes.io/service-account.name": "default"}}},
            {"kind": "Pod", "metadata": {"name": "everruns-server-0",
                                         "labels": {"app.kubernetes.io/instance": "everruns"}}},
        ]});
        assert!(leftovers(&list, "everruns").is_empty());
    }

    #[test]
    fn anything_else_keeps_the_namespace() {
        let list = json!({"items": [
            {"kind": "ConfigMap", "metadata": {"name": "kube-root-ca.crt"}},
            {"kind": "Deployment", "metadata": {"name": "other-app"}},
            {"kind": "Pod", "metadata": {"name": "staging-server-0",
                                         "labels": {"app.kubernetes.io/instance": "staging"}}},
        ]});
        assert_eq!(
            leftovers(&list, "everruns"),
            ["deployment/other-app", "pod/staging-server-0"]
        );
        assert!(leftovers(&Value::Null, "everruns").is_empty());
    }

    #[test]
    fn system_namespaces_are_never_deleted() {
        assert!(deletable_namespace("everruns"));
        assert!(!deletable_namespace("default"));
        assert!(!deletable_namespace("kube-system"));
    }
}
