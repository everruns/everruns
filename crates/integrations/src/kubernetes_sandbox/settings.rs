//! Deployment settings for the `kubernetes` target, read from the
//! environment the Helm chart sets on server and worker.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use super::{SandboxSize, default_image};

/// Where an open sandbox's traffic goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EgressRoute {
    /// Only to the tenant egress proxy's SOCKS5 listener, with a per-sandbox
    /// credential. The NetworkPolicy allows nothing else.
    Proxy {
        /// In-cluster `host:port` of the SOCKS5 listener (plain, not TLS).
        address: String,
        /// Pods of the proxy, for the NetworkPolicy peer.
        pod_labels: BTreeMap<String, String>,
        /// Their namespace; the sandbox namespace when `None`.
        namespace: Option<String>,
    },
    /// Straight to the internet, private and link-local ranges excluded.
    /// Opt-in for deployments without an egress proxy; never on hosted demo.
    Direct,
    /// No route: templates with open egress fail to start.
    None,
}

/// How the server reaches a sandbox's guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestRoute {
    /// The pod IP on the guest port; the server runs in the same cluster.
    PodIp,
    /// The API server's pod proxy (`pods/proxy`); the server runs outside.
    ApiProxy,
}

/// Everything the provider needs from the deployment.
#[derive(Debug, Clone, PartialEq)]
pub struct KubernetesSandboxSettings {
    /// Namespace every sandbox lives in. Required: its presence is what
    /// turns the target on.
    pub namespace: String,
    /// `runtimeClassName` for sandbox pods: `isolated-sandbox` on EKS Nitro,
    /// `gvisor` or `kata` elsewhere. Empty means plain containers, which the
    /// UI flags as no isolation boundary.
    pub runtime_class: Option<String>,
    pub image: String,
    /// Image prefixes templates may name; empty allows any image.
    pub allowed_image_prefixes: Vec<String>,
    pub storage_class: Option<String>,
    /// Size of each sandbox's workspace volume (Kubernetes quantity).
    pub workspace_size: String,
    pub node_selector: BTreeMap<String, String>,
    /// Tolerations, as Kubernetes JSON.
    pub tolerations: Vec<Value>,
    /// Sizes templates may pick.
    pub sizes: Vec<SandboxSize>,
    /// Most running sandboxes one organization may hold (paused ones do not
    /// count: they hold no compute).
    pub max_per_org: u32,
    pub egress: EgressRoute,
    pub guest_route: GuestRoute,
    pub guest_port: u16,
    /// Pods allowed to reach the guest port; anything in the sandbox
    /// namespace's own default-deny otherwise.
    pub control_pod_labels: BTreeMap<String, String>,
    pub control_namespace: Option<String>,
    /// How long a new pod may take to pull its image and start.
    pub ready_timeout: Duration,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `a=b,c=d` into a map.
pub fn parse_labels(value: &str) -> Result<BTreeMap<String, String>, String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            pair.split_once('=')
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .filter(|(key, _)| !key.is_empty())
                .ok_or_else(|| format!("'{pair}' is not key=value"))
        })
        .collect()
}

impl KubernetesSandboxSettings {
    /// Settings for tests and embedders: one namespace, defaults otherwise.
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            runtime_class: None,
            image: default_image(),
            allowed_image_prefixes: Vec::new(),
            storage_class: None,
            workspace_size: "10Gi".to_string(),
            node_selector: BTreeMap::new(),
            tolerations: Vec::new(),
            sizes: SandboxSize::ALL.to_vec(),
            max_per_org: 5,
            egress: EgressRoute::None,
            guest_route: GuestRoute::PodIp,
            guest_port: everruns_contracts::sandbox_guest::GUEST_PORT,
            control_pod_labels: BTreeMap::new(),
            control_namespace: None,
            ready_timeout: Duration::from_secs(300),
        }
    }

    /// Read from the environment. `Ok(None)` when the target is off
    /// (`KUBERNETES_SANDBOX_NAMESPACE` unset); an error when it is on but
    /// misconfigured, so a typo never silently widens egress.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(namespace) = var("KUBERNETES_SANDBOX_NAMESPACE") else {
            return Ok(None);
        };
        let mut settings = Self::new(namespace);
        settings.runtime_class = var("KUBERNETES_SANDBOX_RUNTIME_CLASS");
        if let Some(image) = var("KUBERNETES_SANDBOX_IMAGE") {
            settings.image = image;
        }
        if let Some(prefixes) = var("KUBERNETES_SANDBOX_ALLOWED_IMAGES") {
            settings.allowed_image_prefixes = prefixes
                .split(',')
                .map(str::trim)
                .filter(|prefix| !prefix.is_empty())
                .map(str::to_string)
                .collect();
        }
        settings.storage_class = var("KUBERNETES_SANDBOX_STORAGE_CLASS");
        if let Some(size) = var("KUBERNETES_SANDBOX_WORKSPACE_SIZE") {
            settings.workspace_size = size;
        }
        if let Some(selector) = var("KUBERNETES_SANDBOX_NODE_SELECTOR") {
            settings.node_selector = serde_json::from_str(&selector)
                .map_err(|err| format!("KUBERNETES_SANDBOX_NODE_SELECTOR: {err}"))?;
        }
        if let Some(tolerations) = var("KUBERNETES_SANDBOX_TOLERATIONS") {
            settings.tolerations = serde_json::from_str(&tolerations)
                .map_err(|err| format!("KUBERNETES_SANDBOX_TOLERATIONS: {err}"))?;
        }
        if let Some(sizes) = var("KUBERNETES_SANDBOX_SIZES") {
            settings.sizes = sizes
                .split(',')
                .map(str::trim)
                .filter(|size| !size.is_empty())
                .map(|size| {
                    SandboxSize::parse(size)
                        .ok_or_else(|| format!("KUBERNETES_SANDBOX_SIZES: unknown size '{size}'"))
                })
                .collect::<Result<_, _>>()?;
            if settings.sizes.is_empty() {
                return Err("KUBERNETES_SANDBOX_SIZES names no size".to_string());
            }
        }
        if let Some(max) = var("KUBERNETES_SANDBOX_MAX_PER_ORG") {
            settings.max_per_org = max
                .parse()
                .map_err(|err| format!("KUBERNETES_SANDBOX_MAX_PER_ORG: {err}"))?;
        }
        let proxy_address = var("KUBERNETES_SANDBOX_EGRESS_PROXY_ADDRESS");
        settings.egress = match var("KUBERNETES_SANDBOX_EGRESS").as_deref() {
            Some("direct") => EgressRoute::Direct,
            Some("none") => EgressRoute::None,
            Some("proxy") | None if proxy_address.is_some() => EgressRoute::Proxy {
                address: proxy_address.unwrap_or_default(),
                pod_labels: parse_labels(
                    &var("KUBERNETES_SANDBOX_EGRESS_PROXY_POD_LABELS").unwrap_or_default(),
                )
                .map_err(|err| format!("KUBERNETES_SANDBOX_EGRESS_PROXY_POD_LABELS: {err}"))?,
                namespace: var("KUBERNETES_SANDBOX_EGRESS_PROXY_NAMESPACE"),
            },
            Some("proxy") => {
                return Err(
                    "KUBERNETES_SANDBOX_EGRESS=proxy needs KUBERNETES_SANDBOX_EGRESS_PROXY_ADDRESS"
                        .to_string(),
                );
            }
            None => EgressRoute::None,
            Some(other) => {
                return Err(format!(
                    "KUBERNETES_SANDBOX_EGRESS must be proxy, direct or none, not '{other}'"
                ));
            }
        };
        if let EgressRoute::Proxy { pod_labels, .. } = &settings.egress
            && pod_labels.is_empty()
        {
            return Err(
                "KUBERNETES_SANDBOX_EGRESS_PROXY_POD_LABELS must name the egress proxy pods"
                    .to_string(),
            );
        }
        settings.guest_route = match var("KUBERNETES_SANDBOX_GUEST_ROUTE").as_deref() {
            Some("pod_ip") => GuestRoute::PodIp,
            Some("api_proxy") => GuestRoute::ApiProxy,
            None if std::env::var_os("KUBERNETES_SERVICE_HOST").is_some() => GuestRoute::PodIp,
            None => GuestRoute::ApiProxy,
            Some(other) => {
                return Err(format!(
                    "KUBERNETES_SANDBOX_GUEST_ROUTE must be pod_ip or api_proxy, not '{other}'"
                ));
            }
        };
        if let Some(labels) = var("KUBERNETES_SANDBOX_CONTROL_POD_LABELS") {
            settings.control_pod_labels = parse_labels(&labels)
                .map_err(|err| format!("KUBERNETES_SANDBOX_CONTROL_POD_LABELS: {err}"))?;
        }
        settings.control_namespace = var("KUBERNETES_SANDBOX_CONTROL_NAMESPACE");
        if let Some(seconds) = var("KUBERNETES_SANDBOX_READY_TIMEOUT_SECONDS") {
            settings.ready_timeout = Duration::from_secs(
                seconds
                    .parse()
                    .map_err(|err| format!("KUBERNETES_SANDBOX_READY_TIMEOUT_SECONDS: {err}"))?,
            );
        }
        Ok(Some(settings))
    }

    /// Whether pods run in their own VM or kernel sandbox (a runtime class
    /// is set), rather than as plain containers sharing the node's kernel.
    pub fn vm_isolated(&self) -> bool {
        self.runtime_class.is_some()
    }

    /// Whether `image` may be used by a template.
    pub fn image_allowed(&self, image: &str) -> bool {
        image == self.image
            || self.allowed_image_prefixes.is_empty()
            || self
                .allowed_image_prefixes
                .iter()
                .any(|prefix| image.starts_with(prefix.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_parse_and_reject_garbage() {
        let labels = parse_labels("app=egress, tier = proxy").unwrap();
        assert_eq!(labels["app"], "egress");
        assert_eq!(labels["tier"], "proxy");
        assert!(parse_labels("app").is_err());
        assert!(parse_labels("=x").is_err());
        assert!(parse_labels("").unwrap().is_empty());
    }

    #[test]
    fn images_follow_the_allowed_prefixes() {
        let mut settings = KubernetesSandboxSettings::new("ns");
        assert!(settings.image_allowed("docker.io/anything"));
        settings.allowed_image_prefixes = vec!["ghcr.io/everruns/".into()];
        assert!(settings.image_allowed("ghcr.io/everruns/sandbox-desktop:1"));
        assert!(!settings.image_allowed("docker.io/evil"));
        assert!(settings.image_allowed(&settings.image.clone()));
    }
}
