//! Running `kubectl` and `helm` against one install, and the checks `up` and
//! `doctor` make before touching a cluster.
//!
//! Decision: shell out to the two tools people already have configured. They
//! know the kubeconfig, its contexts and cloud sign-in plugins (EKS, GKE, AKS),
//! which a Kubernetes client crate would have to reimplement.
//! `EVERRUNS_KUBECTL` and `EVERRUNS_HELM` name other binaries.

use crate::ui;
use std::process::{Child, Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;

pub fn kubectl_program() -> String {
    std::env::var("EVERRUNS_KUBECTL").unwrap_or_else(|_| "kubectl".to_string())
}

pub fn helm_program() -> String {
    std::env::var("EVERRUNS_HELM").unwrap_or_else(|_| "helm".to_string())
}

/// Where one install lives.
#[derive(Clone, Debug)]
pub struct Target {
    pub context: String,
    pub namespace: String,
    pub release: String,
}

impl Target {
    pub fn kubectl(&self) -> Command {
        let mut cmd = Command::new(kubectl_program());
        cmd.args(["--context", &self.context, "--namespace", &self.namespace]);
        cmd
    }

    pub fn helm(&self) -> Command {
        let mut cmd = Command::new(helm_program());
        cmd.args([
            "--kube-context",
            &self.context,
            "--namespace",
            &self.namespace,
        ]);
        cmd
    }

    /// The chart's `everruns.fullname`: the release name when it already
    /// says everruns, else `<release>-everruns`.
    pub fn fullname(&self) -> String {
        let name = if self.release.contains("everruns") {
            self.release.clone()
        } else {
            format!("{}-everruns", self.release)
        };
        name.chars()
            .take(50)
            .collect::<String>()
            .trim_end_matches('-')
            .to_string()
    }

    pub fn selector(&self, component: Option<&str>) -> String {
        let mut s = format!(
            "app.kubernetes.io/name=everruns,app.kubernetes.io/instance={}",
            self.release
        );
        if let Some(c) = component {
            s.push_str(",app.kubernetes.io/component=");
            s.push_str(c);
        }
        s
    }

    /// The values the release was installed with, or `None` when there is
    /// no release yet.
    pub fn release_values(&self) -> Result<Option<Value>> {
        let out = self
            .helm()
            .args(["get", "values", &self.release, "--output", "json"])
            .output()
            .context("Failed to run helm")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("not found") {
                return Ok(None);
            }
            bail!("`helm get values` failed: {}", err.trim());
        }
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        Ok(Some(v))
    }

    /// The deployed chart version (`everruns-0.51.0` → `0.51.0`).
    pub fn deployed_chart_version(&self) -> Option<String> {
        let out = self
            .helm()
            .args([
                "list",
                "--filter",
                &format!("^{}$", self.release),
                "--output",
                "json",
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        let list: Value = serde_json::from_slice(&out.stdout).ok()?;
        list.as_array()?
            .first()?
            .get("chart")?
            .as_str()?
            .strip_prefix("everruns-")
            .map(str::to_string)
    }

    /// A key of a chart Secret, decoded.
    pub fn secret_value(&self, secret: &str, key: &str) -> Result<String> {
        let out = self
            .kubectl()
            .args([
                "get",
                "secret",
                secret,
                "--output",
                &format!("jsonpath={{.data.{key}}}"),
            ])
            .output()
            .context("Failed to run kubectl")?;
        if !out.status.success() {
            bail!(
                "could not read {key} from Secret {secret}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(String::from_utf8_lossy(&out.stdout).trim())
            .context("Secret value is not base64")?;
        Ok(String::from_utf8_lossy(&raw).to_string())
    }

    /// `kubectl port-forward` to the proxy Service in the background.
    pub fn port_forward(&self, local_port: u16) -> Result<PortForward> {
        let child = self
            .kubectl()
            .args([
                "port-forward",
                &format!("svc/{}", self.fullname()),
                &format!("{local_port}:9300"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to start kubectl port-forward")?;
        Ok(PortForward(child))
    }
}

/// A background port-forward, stopped when dropped.
pub struct PortForward(Child);

impl Drop for PortForward {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn current_context() -> Result<String> {
    let out = Command::new(kubectl_program())
        .args(["config", "current-context"])
        .output()
        .context("Failed to run kubectl")?;
    let ctx = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || ctx.is_empty() {
        bail!(
            "kubectl has no current context. Point it at a cluster (kubectl config use-context <name>) \
             or pass --context"
        );
    }
    Ok(ctx)
}

pub fn output_text(out: &Output) -> String {
    let mut s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(err.trim());
    }
    s
}

/// One line of a check list, like `everruns docker doctor` prints.
pub struct Check {
    pub name: &'static str,
    pub outcome: Outcome,
}

pub enum Outcome {
    Ok(String),
    Warn(String),
    Fail(String),
}

impl Check {
    pub fn failed(&self) -> bool {
        matches!(self.outcome, Outcome::Fail(_))
    }

    pub fn print(&self) {
        match &self.outcome {
            Outcome::Ok(msg) => ui::ok(ui::check_line(self.name, msg)),
            Outcome::Warn(msg) => ui::warn(format!("{}: {msg}", self.name)),
            Outcome::Fail(msg) => ui::fail(format!("{}: {msg}", self.name)),
        }
    }
}

/// kubectl and helm are installed and new enough (Helm 3.8 reads OCI charts).
pub fn tool_checks() -> Vec<Check> {
    let kubectl = Command::new(kubectl_program())
        .args(["version", "--client", "--output", "json"])
        .output();
    let kubectl = match kubectl {
        Ok(out) if out.status.success() => {
            let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
            let ver = v["clientVersion"]["gitVersion"]
                .as_str()
                .unwrap_or("installed")
                .to_string();
            Outcome::Ok(ver)
        }
        _ => Outcome::Fail(
            "not found; install it: https://kubernetes.io/docs/tasks/tools/".to_string(),
        ),
    };
    let helm = Command::new(helm_program())
        .args(["version", "--template", "{{.Version}}"])
        .output();
    let helm = match helm {
        Ok(out) if out.status.success() => {
            let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
            match parse_version(&ver) {
                Some(v) if v < (3, 8) => {
                    Outcome::Fail(format!("{ver}; 3.8 or newer is needed for OCI charts"))
                }
                _ => Outcome::Ok(ver),
            }
        }
        _ => {
            Outcome::Fail("not found; install it: https://helm.sh/docs/intro/install/".to_string())
        }
    };
    vec![
        Check {
            name: "kubectl",
            outcome: kubectl,
        },
        Check {
            name: "helm",
            outcome: helm,
        },
    ]
}

/// The cluster answers, is new enough, lets this user install into the
/// namespace, and has what the chosen values need.
pub fn cluster_checks(target: &Target, needs_volumes: bool, needs_keda: bool) -> Vec<Check> {
    let mut checks = Vec::new();
    let version = target
        .kubectl()
        .args(["version", "--output", "json", "--request-timeout", "15s"])
        .output();
    let reachable = match &version {
        Ok(out) if out.status.success() => {
            let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
            let server = &v["serverVersion"];
            let label = server["gitVersion"].as_str().unwrap_or("?").to_string();
            let minor: u64 = server["minor"]
                .as_str()
                .unwrap_or("0")
                .trim_end_matches('+')
                .parse()
                .unwrap_or(0);
            let outcome = if minor != 0 && minor < 27 {
                Outcome::Fail(format!("{label}; Kubernetes 1.27 or newer is needed"))
            } else {
                Outcome::Ok(format!("{} ({label})", target.context))
            };
            checks.push(Check {
                name: "Cluster",
                outcome,
            });
            true
        }
        Ok(out) => {
            checks.push(Check {
                name: "Cluster",
                outcome: Outcome::Fail(format!(
                    "context {} does not answer: {}",
                    target.context,
                    output_text(out)
                )),
            });
            false
        }
        Err(e) => {
            checks.push(Check {
                name: "Cluster",
                outcome: Outcome::Fail(format!("could not run kubectl: {e}")),
            });
            false
        }
    };
    if !reachable {
        return checks;
    }

    let can = |verb: &str, resource: &str| {
        target
            .kubectl()
            .args(["auth", "can-i", verb, resource])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes")
    };
    let missing: Vec<&str> = [
        ("create", "deployments"),
        ("create", "secrets"),
        ("create", "services"),
    ]
    .into_iter()
    .filter(|(v, r)| !can(v, r))
    .map(|(_, r)| r)
    .collect();
    checks.push(Check {
        name: "Permissions",
        outcome: if missing.is_empty() {
            Outcome::Ok(format!("can install into namespace {}", target.namespace))
        } else {
            Outcome::Fail(format!(
                "cannot create {} in namespace {}; ask a cluster admin for access",
                missing.join(", "),
                target.namespace
            ))
        },
    });

    if needs_volumes {
        let out = target
            .kubectl()
            .args(["get", "storageclass", "--output", "json"])
            .output();
        let default = out.ok().filter(|o| o.status.success()).and_then(|o| {
            let v: Value = serde_json::from_slice(&o.stdout).ok()?;
            v["items"].as_array()?.iter().find_map(|sc| {
                let ann = &sc["metadata"]["annotations"];
                let is_default = ann["storageclass.kubernetes.io/is-default-class"] == "true"
                    || ann["storageclass.beta.kubernetes.io/is-default-class"] == "true";
                is_default.then(|| sc["metadata"]["name"].as_str().unwrap_or("").to_string())
            })
        });
        checks.push(Check {
            name: "Storage",
            outcome: match default {
                Some(name) => Outcome::Ok(format!("default StorageClass {name}")),
                None => Outcome::Warn(
                    "no default StorageClass; the bundled PostgreSQL and NATS volumes may stay \
                     Pending (set postgresql.storage.storageClass and nats.storage.storageClass)"
                        .to_string(),
                ),
            },
        });
    }

    if needs_keda {
        let has = target
            .kubectl()
            .args(["get", "crd", "scaledobjects.keda.sh"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        checks.push(Check {
            name: "KEDA",
            outcome: if has {
                Outcome::Ok("installed".to_string())
            } else {
                Outcome::Fail(
                    "worker autoscaling is on but KEDA is not installed: https://keda.sh/docs/deploy/"
                        .to_string(),
                )
            },
        });
    }
    checks
}

/// `v3.19.0` → (3, 19).
pub fn parse_version(v: &str) -> Option<(u64, u64)> {
    let mut parts = v.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(release: &str) -> Target {
        Target {
            context: "kind".into(),
            namespace: "everruns".into(),
            release: release.into(),
        }
    }

    #[test]
    fn fullname_matches_the_chart() {
        assert_eq!(target("everruns").fullname(), "everruns");
        assert_eq!(target("my-everruns").fullname(), "my-everruns");
        assert_eq!(target("smoke").fullname(), "smoke-everruns");
    }

    #[test]
    fn selector_names_the_release() {
        assert_eq!(
            target("smoke").selector(Some("server")),
            "app.kubernetes.io/name=everruns,app.kubernetes.io/instance=smoke,app.kubernetes.io/component=server"
        );
    }

    #[test]
    fn parses_helm_versions() {
        assert_eq!(parse_version("v3.19.0"), Some((3, 19)));
        assert_eq!(parse_version("v3.7.2"), Some((3, 7)));
        assert!(parse_version("v3.7.2").unwrap() < (3, 8));
        assert_eq!(parse_version("nonsense"), None);
    }
}
