//! Choose classes for the chart's persistent volumes before applying it.

use std::io::Write;
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use super::cluster::{Target, output_text};
use super::{ask_yes, contexts};
use crate::commands::docker::Asker;
use crate::ui;

const LOCAL_CLASS: &str = "local-path";
const PROVISIONER: &str = include_str!("manifests/local-path-storage.yaml");

#[derive(Deserialize)]
struct StorageClasses {
    items: Vec<StorageClass>,
}

#[derive(Deserialize)]
struct StorageClass {
    metadata: ClassMetadata,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClassMetadata {
    name: String,
    creation_timestamp: Option<String>,
    #[serde(default)]
    annotations: std::collections::BTreeMap<String, String>,
}

impl StorageClasses {
    fn get(target: &Target) -> Result<Self> {
        let output = target
            .kubectl()
            .args([
                "get",
                "storageclass",
                "--output",
                "json",
                "--request-timeout",
                "15s",
            ])
            .output()
            .context("Failed to list StorageClasses")?;
        if !output.status.success() {
            bail!(
                "Cannot list StorageClasses in '{}'. Ask a cluster admin for access: {}",
                target.context,
                output_text(&output)
            );
        }
        serde_json::from_slice(&output.stdout).context("Invalid StorageClass list from kubectl")
    }

    fn contains(&self, name: &str) -> bool {
        self.items.iter().any(|class| class.metadata.name == name)
    }

    fn default(&self) -> Option<&str> {
        self.items
            .iter()
            .map(|class| &class.metadata)
            .filter(|m| {
                [
                    "storageclass.kubernetes.io/is-default-class",
                    "storageclass.beta.kubernetes.io/is-default-class",
                ]
                .iter()
                .any(|key| m.annotations.get(*key).is_some_and(|v| v == "true"))
            })
            // Kubernetes chooses the latest-created default if several exist.
            .max_by_key(|m| (&m.creation_timestamp, &m.name))
            .map(|m| m.name.as_str())
    }
}

pub(super) fn default_class(target: &Target) -> Result<Option<String>> {
    Ok(StorageClasses::get(target)?.default().map(str::to_string))
}

#[derive(Clone, Copy)]
enum Volume {
    Postgres,
    Nats,
    ShardData,
    ShardStore,
}

impl Volume {
    fn path(self) -> &'static [&'static str] {
        match self {
            Self::Postgres => &["postgresql", "storage", "storageClass"],
            Self::Nats => &["nats", "storage", "storageClass"],
            Self::ShardData => &["dataPlane", "shard", "storage", "storageClass"],
            Self::ShardStore => &["dataPlane", "storeVolume", "storageClass"],
        }
    }
}

struct Claim {
    volume: Volume,
    class: Option<String>,
    shared: bool,
}

fn chart_claims(manifest: &[u8]) -> Result<Vec<Claim>> {
    let mut result = Vec::new();
    for doc in serde_yaml::Deserializer::from_slice(manifest) {
        let resource = Value::deserialize(doc).context("Invalid Kubernetes manifest from Helm")?;
        let component = resource["metadata"]["labels"]["app.kubernetes.io/component"].as_str();
        let (volume, spec) = match (resource["kind"].as_str(), component) {
            (Some("StatefulSet"), Some("postgresql")) => (
                Volume::Postgres,
                &resource["spec"]["volumeClaimTemplates"][0]["spec"],
            ),
            (Some("StatefulSet"), Some("nats")) => (
                Volume::Nats,
                &resource["spec"]["volumeClaimTemplates"][0]["spec"],
            ),
            (Some("StatefulSet"), Some("shard")) => (
                Volume::ShardData,
                &resource["spec"]["volumeClaimTemplates"][0]["spec"],
            ),
            (Some("PersistentVolumeClaim"), Some("shard")) => (Volume::ShardStore, &resource["spec"]),
            _ => continue,
        };
        if spec.is_null() {
            continue;
        }
        result.push(Claim {
            volume,
            class: spec["storageClassName"].as_str().map(str::to_string),
            shared: spec["accessModes"]
                .as_array()
                .is_some_and(|modes| modes.iter().any(|mode| mode == "ReadWriteMany")),
        });
    }
    Ok(result)
}

pub(super) fn prepare(
    target: &Target,
    choice: &str,
    manifest: &[u8],
    first_install: bool,
    asker: Asker,
) -> Result<Vec<String>> {
    let claims = chart_claims(manifest)?;
    if claims.is_empty() {
        return Ok(Vec::new());
    }
    let classes = StorageClasses::get(target)?;
    for name in claims
        .iter()
        .filter_map(|claim| claim.class.as_deref())
        .filter(|name| !name.is_empty())
    {
        if !classes.contains(name) {
            bail!(
                "The chart requests StorageClass '{name}', which does not exist in '{}'. Create it or change the corresponding storageClass value.",
                target.context
            );
        }
    }
    let unassigned: Vec<_> = claims
        .iter()
        .filter(|claim| claim.class.is_none())
        .collect();
    if unassigned.is_empty() {
        return Ok(Vec::new());
    }

    let class = if choice != "auto" {
        if !classes.contains(choice) {
            bail!(
                "StorageClass '{choice}' does not exist in '{}'. Create it or choose an existing class with --storage-class <name>.",
                target.context
            );
        }
        choice.to_string()
    } else if let Some(name) = classes.default() {
        name.to_string()
    } else {
        if unassigned.iter().any(|claim| claim.shared) {
            bail!(
                "Cluster '{}' has no default StorageClass for a shared volume. Set a class that supports ReadWriteMany in the chart values or pass it with --storage-class.",
                target.context
            );
        }
        if !contexts::is_local(target) {
            bail!(
                "Cluster '{}' has no default StorageClass for the chart's volumes. Ask a cluster admin to configure one, or pass --storage-class <name> for an existing class.",
                target.context
            );
        }
        if !ask_yes(
            asker,
            &format!(
                "Install Local Path Provisioner v0.0.37 for local cluster '{}'?",
                target.context
            ),
            "--yes",
        )? {
            bail!(
                "Storage provisioner setup declined. Specify an existing class with --storage-class <name>."
            );
        }
        ensure_default(target)?
    };

    if !first_install && choice != "auto" && classes.default() != Some(class.as_str()) {
        bail!(
            "The installed release has claim templates that use the cluster default. Use its default StorageClass or migrate those volumes before selecting '{class}'."
        );
    }
    ui::ok(format!("StorageClass {class}"));
    Ok(if first_install {
        storage_values(&unassigned, &class)
    } else {
        Vec::new()
    })
}

fn storage_values(claims: &[&Claim], class: &str) -> Vec<String> {
    claims
        .iter()
        .map(|claim| format!("{}={class}", claim.volume.path().join(".")))
        .collect()
}

/// For a context independently proved local. Preserve an operator-installed
/// default and retry the readiness wait if a previous setup was interrupted.
pub(super) fn ensure_default(target: &Target) -> Result<String> {
    let classes = StorageClasses::get(target)?;
    if let Some(name) = classes.default() {
        return Ok(name.to_string());
    }
    if !contexts::is_local(target) {
        bail!(
            "Context '{}' could not be verified as a local cluster. Configure a default StorageClass or choose an existing one with --storage-class.",
            target.context
        );
    }
    if classes.contains(LOCAL_CLASS) {
        // A previous attempt may have applied the resources but timed out.
        // Refuse to take over an unrelated local-path class.
        let output = target
            .cluster_kubectl()
            .args([
                "--namespace",
                "local-path-storage",
                "get",
                "deployment/local-path-provisioner",
                "--output",
                "jsonpath={.spec.template.spec.containers[0].image}",
                "--request-timeout",
                "15s",
            ])
            .output()
            .context("Failed to check the existing local-path provisioner")?;
        if !output.status.success()
            || output.stdout != b"docker.io/rancher/local-path-provisioner:v0.0.37"
        {
            bail!(
                "StorageClass 'local-path' already exists with a separately managed provisioner. Verify it is ready, then pass --storage-class local-path or configure a cluster default."
            );
        }
    } else {
        let mut child = target
            .cluster_kubectl()
            .args(["apply", "--filename", "-", "--request-timeout", "30s"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to apply Local Path Provisioner")?;
        child
            .stdin
            .take()
            .context("kubectl has no stdin")?
            .write_all(PROVISIONER.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!(
                "Could not install Local Path Provisioner in '{}': {}",
                target.context,
                output_text(&output)
            );
        }
    }

    let output = target
        .cluster_kubectl()
        .args([
            "--namespace",
            "local-path-storage",
            "rollout",
            "status",
            "deployment/local-path-provisioner",
            "--timeout",
            "180s",
        ])
        .output()
        .context("Failed to wait for Local Path Provisioner")?;
    if !output.status.success() {
        bail!(
            "Local Path Provisioner did not become ready. Check `kubectl --context {} -n local-path-storage get pods`: {}",
            target.context,
            output_text(&output)
        );
    }
    if let Some(name) = default_class(target)? {
        return Ok(name);
    }
    let output = target
        .cluster_kubectl()
        .args([
            "annotate",
            "storageclass",
            LOCAL_CLASS,
            "storageclass.kubernetes.io/is-default-class=true",
            "--overwrite",
            "--request-timeout",
            "15s",
        ])
        .output()
        .context("Failed to set the local default StorageClass")?;
    if !output.status.success() {
        bail!(
            "Could not set the default StorageClass in '{}': {}",
            target.context,
            output_text(&output)
        );
    }
    Ok(LOCAL_CLASS.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_follow_kubernetes_precedence_and_recognize_the_beta_annotation() {
        let classes: StorageClasses = serde_json::from_value(json!({"items": [
            {"metadata": {"name": "standard", "creationTimestamp": "2025-01-01T00:00:00Z",
              "annotations": {"storageclass.kubernetes.io/is-default-class": "true"}}},
            {"metadata": {"name": "fast", "creationTimestamp": "2026-01-01T00:00:00Z",
              "annotations": {"storageclass.beta.kubernetes.io/is-default-class": "true"}}},
            {"metadata": {"name": "later-non-default", "creationTimestamp": "2026-11-01T00:00:00Z"}}
        ]}))
        .unwrap();
        assert_eq!(classes.default(), Some("fast"));
        assert!(classes.contains("standard"));
        assert!(!classes.contains("unknown"));
    }

    #[test]
    fn only_unassigned_rendered_claims_get_the_selected_class() {
        let manifest = br#"
kind: StatefulSet
metadata: {labels: {app.kubernetes.io/component: postgresql}}
spec: {volumeClaimTemplates: [{spec: {accessModes: [ReadWriteOnce]}}]}
---
kind: StatefulSet
metadata: {labels: {app.kubernetes.io/component: nats}}
spec: {volumeClaimTemplates: [{spec: {accessModes: [ReadWriteOnce]}}]}
---
kind: StatefulSet
metadata: {labels: {app.kubernetes.io/component: shard}}
spec: {volumeClaimTemplates: [{spec: {accessModes: [ReadWriteOnce]}}]}
---
kind: PersistentVolumeClaim
metadata: {labels: {app.kubernetes.io/component: shard}}
spec: {accessModes: [ReadWriteMany], storageClassName: shared-nfs}
"#;
        let claims = chart_claims(manifest).unwrap();
        assert_eq!(claims.len(), 4);
        let unresolved: Vec<_> = claims
            .iter()
            .filter(|claim| claim.class.is_none())
            .collect();
        assert_eq!(
            storage_values(&unresolved, "local-path"),
            [
                "postgresql.storage.storageClass=local-path",
                "nats.storage.storageClass=local-path",
                "dataPlane.shard.storage.storageClass=local-path",
            ]
        );
        let shared = claims.last().unwrap();
        assert!(shared.shared);
        assert_eq!(
            storage_values(&[shared], "shared-nfs"),
            ["dataPlane.storeVolume.storageClass=shared-nfs"]
        );
        assert!(chart_claims(b"kind: Deployment").unwrap().is_empty());
    }
}
