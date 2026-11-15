//! The Kubernetes objects of one sandbox, built as pure functions so tests
//! can check every field without a cluster.
//!
//! One sandbox is four objects sharing the label `everruns.sandbox=<name>`:
//! - Pod `<name>`: the sandbox itself. Deleted on pause, recreated on resume.
//! - PersistentVolumeClaim `<name>-workspace`: the workspace. Lives until the
//!   sandbox is deleted, so it carries files across pause.
//! - Secret `<name>-guest`: the guest token and the exec environment (egress
//!   proxy settings). Mounted only into this pod.
//! - NetworkPolicy `<name>`: who may reach the guest, and where the sandbox
//!   may connect.
//!
//! Decisions:
//! - No owner references: the pod comes and goes while the rest stays, and
//!   garbage collection keyed on the pod would take the workspace with it.
//!   Cleanup is by label.
//! - Network isolation is a NetworkPolicy, not Nitro's
//!   `sandbox.amazonaws.com/network-isolated` annotation: that annotation
//!   leaves only loopback, which would also cut the server off from the guest.
//! - `automountServiceAccountToken: false` and `enableServiceLinks: false`:
//!   the sandbox learns nothing about the cluster it runs in.
//! - Plain containers (no runtime class) also get
//!   `allowPrivilegeEscalation: false`; inside a VM the sandbox user may use
//!   sudo, since the VM is the boundary.

use std::collections::BTreeMap;

use everruns_contracts::runtime::SandboxProxyCredential;
use everruns_contracts::runtime::sandbox_egress::{SandboxEgressMode, SandboxEgressRules};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use serde_json::{Value, json};

use super::settings::{EgressRoute, KubernetesSandboxSettings};
use super::{GUEST_BINARY, SandboxSize};

/// Label every object of one sandbox carries; its value is the sandbox name.
pub const LABEL_SANDBOX: &str = "everruns.sandbox";
/// Where the guest secret is mounted.
pub const GUEST_SECRET_DIR: &str = "/var/run/everruns-guest";
/// Key of the guest token in the Secret.
pub const TOKEN_KEY: &str = "token";
/// Key of the exec environment file in the Secret.
pub const EXEC_ENV_KEY: &str = "exec.env";
/// Container name inside the pod.
pub const CONTAINER: &str = "sandbox";

/// Ranges an open direct route never reaches: private networks (the
/// cluster, the VPC), link-local (instance metadata), CGNAT.
const DIRECT_EXCLUDED: &[&str] = &[
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "100.64.0.0/10",
    "127.0.0.0/8",
];

pub fn workspace_claim_name(name: &str) -> String {
    format!("{name}-workspace")
}

pub fn guest_secret_name(name: &str) -> String {
    format!("{name}-guest")
}

/// Where a sandbox may connect, decided from its rules and the deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodEgress {
    /// Nothing leaves.
    Blocked,
    /// Only the egress proxy, with this exec environment.
    Proxy { exec_env: String },
    /// The internet, minus [`DIRECT_EXCLUDED`].
    Direct,
    /// Only these IP ranges.
    Cidrs(Vec<String>),
}

/// Decide a sandbox's egress. Fails rather than widening: a domain
/// allowlist cannot be enforced by a NetworkPolicy, and an open template on
/// a deployment with no route cannot start.
pub fn pod_egress(
    rules: &SandboxEgressRules,
    settings: &KubernetesSandboxSettings,
    proxy: Option<&SandboxProxyCredential>,
) -> Result<PodEgress, String> {
    match &rules.mode {
        SandboxEgressMode::Blocked => Ok(PodEgress::Blocked),
        SandboxEgressMode::Allowlist { domains, .. } if !domains.is_empty() => Err(format!(
            "isolated sandboxes cannot enforce a domain allowlist yet ({}); use network open or deny, or allow IP ranges only",
            domains.join(", ")
        )),
        SandboxEgressMode::Allowlist { cidrs, .. } if cidrs.is_empty() => Ok(PodEgress::Blocked),
        SandboxEgressMode::Allowlist { cidrs, .. } => Ok(PodEgress::Cidrs(cidrs.clone())),
        SandboxEgressMode::Open => match &settings.egress {
            EgressRoute::Proxy { address, .. } => {
                let credential = proxy.ok_or(
                    "the egress proxy is configured for isolated sandboxes, but this server cannot mint sandbox proxy credentials (EVERRUNS_EGRESS_PROXY_SOCKS_ADDRESS and the proxy secret)",
                )?;
                Ok(PodEgress::Proxy {
                    exec_env: proxy_exec_env(address, credential),
                })
            }
            EgressRoute::Direct => Ok(PodEgress::Direct),
            EgressRoute::None => Err(
                "this deployment gives isolated sandboxes no internet route; use network deny, or configure the egress proxy".to_string(),
            ),
        },
    }
}

fn url_encode(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

/// The `KEY=VALUE` lines that point every command at the proxy.
pub fn proxy_exec_env(address: &str, credential: &SandboxProxyCredential) -> String {
    let url = format!(
        "socks5h://{}:{}@{address}",
        url_encode(&credential.username),
        url_encode(&credential.password)
    );
    let mut lines = String::new();
    for key in [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ] {
        lines.push_str(&format!("{key}={url}\n"));
    }
    lines.push_str("NO_PROXY=localhost,127.0.0.1,::1\nno_proxy=localhost,127.0.0.1,::1\n");
    lines
}

fn labels_value(labels: &BTreeMap<String, String>) -> Value {
    json!(labels)
}

/// Inputs shared by every object of one sandbox.
#[derive(Debug, Clone)]
pub struct SandboxObjects<'a> {
    pub settings: &'a KubernetesSandboxSettings,
    pub name: &'a str,
    /// Managed labels from the session plus [`LABEL_SANDBOX`].
    pub labels: BTreeMap<String, String>,
    pub size: SandboxSize,
    pub image: &'a str,
    pub workspace_path: &'a str,
    pub desktop: bool,
}

impl SandboxObjects<'_> {
    pub fn workspace_claim(&self) -> Result<PersistentVolumeClaim, String> {
        let mut spec = json!({
            "accessModes": ["ReadWriteOnce"],
            "resources": {"requests": {"storage": self.settings.workspace_size}},
        });
        if let Some(class) = &self.settings.storage_class {
            spec["storageClassName"] = json!(class);
        }
        serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": {
                "name": workspace_claim_name(self.name),
                "labels": labels_value(&self.labels),
            },
            "spec": spec,
        }))
        .map_err(|err| format!("workspace claim: {err}"))
    }

    pub fn guest_secret(&self, token: &str, exec_env: &str) -> Result<Secret, String> {
        serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": guest_secret_name(self.name),
                "labels": labels_value(&self.labels),
            },
            "type": "Opaque",
            "stringData": {TOKEN_KEY: token, EXEC_ENV_KEY: exec_env},
        }))
        .map_err(|err| format!("guest secret: {err}"))
    }

    pub fn pod(&self) -> Result<Pod, String> {
        let (cpu, memory) = self.size.resources();
        let isolated = self.settings.vm_isolated();
        let mut env = vec![
            json!({"name": "EVERRUNS_GUEST_TOKEN_FILE", "value": format!("{GUEST_SECRET_DIR}/{TOKEN_KEY}")}),
            json!({"name": "EVERRUNS_GUEST_EXEC_ENV_FILE", "value": format!("{GUEST_SECRET_DIR}/{EXEC_ENV_KEY}")}),
            json!({"name": "EVERRUNS_GUEST_WORKSPACE", "value": self.workspace_path}),
            json!({"name": "EVERRUNS_GUEST_LISTEN", "value": format!("0.0.0.0:{}", self.settings.guest_port)}),
        ];
        if self.desktop {
            env.push(json!({"name": "EVERRUNS_GUEST_DESKTOP", "value": "1"}));
            // The display the desktop session starts; programs the agent
            // launches from the shell open their windows on it.
            env.push(json!({"name": "DISPLAY", "value": crate::x11_desktop::DESKTOP_DISPLAY}));
        }
        let mut container_security = json!({"privileged": false});
        if !isolated {
            container_security["allowPrivilegeEscalation"] = json!(false);
            container_security["capabilities"] = json!({"drop": ["NET_RAW"]});
        }
        let mut spec = json!({
            "automountServiceAccountToken": false,
            "enableServiceLinks": false,
            "restartPolicy": "Always",
            "terminationGracePeriodSeconds": 10,
            "hostname": "sandbox",
            "securityContext": {
                "fsGroup": 1000,
                "seccompProfile": {"type": "RuntimeDefault"},
            },
            "containers": [{
                "name": CONTAINER,
                "image": self.image,
                "command": [GUEST_BINARY],
                "env": env,
                "ports": [{"name": "guest", "containerPort": self.settings.guest_port, "protocol": "TCP"}],
                "resources": {
                    "requests": {"cpu": cpu, "memory": memory},
                    "limits": {"cpu": cpu, "memory": memory},
                },
                "readinessProbe": {
                    "httpGet": {"path": everruns_contracts::sandbox_guest::paths::HEALTH, "port": self.settings.guest_port},
                    "periodSeconds": 2,
                    "failureThreshold": 3,
                },
                "securityContext": container_security,
                "volumeMounts": [
                    {"name": "workspace", "mountPath": self.workspace_path},
                    {"name": "guest", "mountPath": GUEST_SECRET_DIR, "readOnly": true},
                ],
            }],
            "volumes": [
                {"name": "workspace", "persistentVolumeClaim": {"claimName": workspace_claim_name(self.name)}},
                {"name": "guest", "secret": {"secretName": guest_secret_name(self.name), "defaultMode": 0o444}},
            ],
        });
        if let Some(class) = &self.settings.runtime_class {
            spec["runtimeClassName"] = json!(class);
        }
        if !self.settings.node_selector.is_empty() {
            spec["nodeSelector"] = json!(self.settings.node_selector);
        }
        if !self.settings.tolerations.is_empty() {
            spec["tolerations"] = json!(self.settings.tolerations);
        }
        serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": self.name,
                "labels": labels_value(&self.labels),
                "annotations": {
                    "everruns.size": self.size.as_str(),
                    "everruns.workspace_path": self.workspace_path,
                },
            },
            "spec": spec,
        }))
        .map_err(|err| format!("sandbox pod: {err}"))
    }

    pub fn network_policy(&self, egress: &PodEgress) -> Result<NetworkPolicy, String> {
        let settings = self.settings;
        // Ingress: only the control plane, only the guest port.
        let mut from = json!({});
        if !settings.control_pod_labels.is_empty() {
            from["podSelector"] = json!({"matchLabels": settings.control_pod_labels});
        }
        match &settings.control_namespace {
            Some(namespace) => {
                from["namespaceSelector"] =
                    json!({"matchLabels": {"kubernetes.io/metadata.name": namespace}});
            }
            None if settings.control_pod_labels.is_empty() => {
                // No selector configured: any pod in the cluster may knock,
                // and still needs the per-sandbox token.
                from["namespaceSelector"] = json!({});
            }
            None => {}
        }
        let ingress = json!([{
            "from": [from],
            "ports": [{"protocol": "TCP", "port": settings.guest_port}],
        }]);
        let dns = json!({
            "to": [{"namespaceSelector": {}, "podSelector": {"matchLabels": {"k8s-app": "kube-dns"}}}],
            "ports": [{"protocol": "UDP", "port": 53}, {"protocol": "TCP", "port": 53}],
        });
        let egress_rules = match egress {
            PodEgress::Blocked => json!([]),
            PodEgress::Proxy { .. } => {
                let EgressRoute::Proxy {
                    address,
                    pod_labels,
                    namespace,
                } = &settings.egress
                else {
                    return Err("proxy egress without a proxy route".to_string());
                };
                let port: u16 = address
                    .rsplit_once(':')
                    .and_then(|(_, port)| port.parse().ok())
                    .ok_or_else(|| format!("egress proxy address '{address}' has no port"))?;
                let mut to = json!({"podSelector": {"matchLabels": pod_labels}});
                if let Some(namespace) = namespace {
                    to["namespaceSelector"] =
                        json!({"matchLabels": {"kubernetes.io/metadata.name": namespace}});
                }
                // DNS resolves the proxy's Service name; the tunnel itself
                // resolves destinations at the proxy (socks5h).
                json!([dns, {"to": [to], "ports": [{"protocol": "TCP", "port": port}]}])
            }
            PodEgress::Direct => json!([
                dns,
                {"to": [{"ipBlock": {"cidr": "0.0.0.0/0", "except": DIRECT_EXCLUDED}}]},
            ]),
            PodEgress::Cidrs(cidrs) => json!([{
                "to": cidrs.iter().map(|cidr| json!({"ipBlock": {"cidr": cidr}})).collect::<Vec<_>>(),
            }]),
        };
        serde_json::from_value(json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "NetworkPolicy",
            "metadata": {
                "name": self.name,
                "labels": labels_value(&self.labels),
            },
            "spec": {
                "podSelector": {"matchLabels": {LABEL_SANDBOX: self.name}},
                "policyTypes": ["Ingress", "Egress"],
                "ingress": ingress,
                "egress": egress_rules,
            },
        }))
        .map_err(|err| format!("network policy: {err}"))
    }
}

/// Whether `value` is a valid label value (≤ 63, alphanumerics at both ends,
/// `-`, `_`, `.` inside).
pub fn valid_label_value(value: &str) -> bool {
    value.len() <= 63
        && (value.is_empty()
            || (value.starts_with(|c: char| c.is_ascii_alphanumeric())
                && value.ends_with(|c: char| c.is_ascii_alphanumeric())
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> KubernetesSandboxSettings {
        let mut settings = KubernetesSandboxSettings::new("sandboxes");
        settings.runtime_class = Some("isolated-sandbox".into());
        settings.egress = EgressRoute::Proxy {
            address: "egress-proxy.everruns.svc:1080".into(),
            pod_labels: BTreeMap::from([("app".into(), "egress-proxy".into())]),
            namespace: Some("everruns".into()),
        };
        settings.control_pod_labels = BTreeMap::from([("app".into(), "everruns".into())]);
        settings.control_namespace = Some("everruns".into());
        settings
    }

    fn objects(settings: &KubernetesSandboxSettings) -> SandboxObjects<'_> {
        SandboxObjects {
            settings,
            name: "er-sbx-0123456789abcdef",
            labels: BTreeMap::from([
                (LABEL_SANDBOX.into(), "er-sbx-0123456789abcdef".into()),
                ("everruns.org_id".into(), "org_1".into()),
            ]),
            size: SandboxSize::Small,
            image: "ghcr.io/everruns/sandbox-dev:1",
            workspace_path: "/workspace",
            desktop: false,
        }
    }

    fn credential() -> SandboxProxyCredential {
        SandboxProxyCredential {
            address: "proxy.example.com:1080".into(),
            tls: true,
            username: "s1".into(),
            password: "s1.org.session.9.sig/+".into(),
            expires_at: 9,
        }
    }

    fn rules(mode: SandboxEgressMode) -> SandboxEgressRules {
        SandboxEgressRules {
            mode,
            unenforced_blocks: Vec::new(),
        }
    }

    #[test]
    fn desktop_pods_point_programs_at_the_desktop_display() {
        let settings = settings();
        let plain = serde_json::to_value(objects(&settings).pod().unwrap()).unwrap();
        let names = |pod: &Value| -> Vec<String> {
            pod["spec"]["containers"][0]["env"]
                .as_array()
                .unwrap()
                .iter()
                .map(|var| var["name"].as_str().unwrap().to_string())
                .collect()
        };
        assert!(!names(&plain).contains(&"DISPLAY".to_string()));
        let mut desktop = objects(&settings);
        desktop.desktop = true;
        let pod = serde_json::to_value(desktop.pod().unwrap()).unwrap();
        let env = pod["spec"]["containers"][0]["env"].as_array().unwrap();
        let display = env.iter().find(|var| var["name"] == "DISPLAY").unwrap();
        assert_eq!(display["value"], crate::x11_desktop::DESKTOP_DISPLAY);
        assert!(names(&pod).contains(&"EVERRUNS_GUEST_DESKTOP".to_string()));
    }

    #[test]
    fn pods_are_locked_down_and_sized() {
        let settings = settings();
        let pod = serde_json::to_value(objects(&settings).pod().unwrap()).unwrap();
        let spec = &pod["spec"];
        assert_eq!(spec["runtimeClassName"], "isolated-sandbox");
        assert_eq!(spec["automountServiceAccountToken"], false);
        assert_eq!(spec["enableServiceLinks"], false);
        let container = &spec["containers"][0];
        assert_eq!(container["command"][0], GUEST_BINARY);
        assert_eq!(container["resources"]["limits"]["cpu"], "1");
        assert_eq!(container["resources"]["limits"]["memory"], "2Gi");
        assert_eq!(
            container["resources"]["requests"],
            container["resources"]["limits"]
        );
        assert_eq!(container["securityContext"]["privileged"], false);
        // Inside a VM sudo is the user's business.
        assert!(container["securityContext"]["allowPrivilegeEscalation"].is_null());
        assert_eq!(
            spec["volumes"][0]["persistentVolumeClaim"]["claimName"],
            "er-sbx-0123456789abcdef-workspace"
        );
        assert!(spec["hostNetwork"].is_null());
    }

    #[test]
    fn plain_containers_cannot_escalate() {
        let mut settings = settings();
        settings.runtime_class = None;
        let pod = serde_json::to_value(objects(&settings).pod().unwrap()).unwrap();
        assert!(pod["spec"]["runtimeClassName"].is_null());
        assert_eq!(
            pod["spec"]["containers"][0]["securityContext"]["allowPrivilegeEscalation"],
            false
        );
    }

    #[test]
    fn open_egress_goes_only_to_the_proxy() {
        let settings = settings();
        let egress =
            pod_egress(&SandboxEgressRules::open(), &settings, Some(&credential())).unwrap();
        let PodEgress::Proxy { exec_env } = &egress else {
            panic!("{egress:?}");
        };
        // The password is URL-encoded and the in-cluster address replaces
        // the public one.
        assert!(exec_env.contains(
            "ALL_PROXY=socks5h://s1:s1.org.session.9.sig%2F%2B@egress-proxy.everruns.svc:1080\n"
        ));
        let policy =
            serde_json::to_value(objects(&settings).network_policy(&egress).unwrap()).unwrap();
        let rules = policy["spec"]["egress"].as_array().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1]["ports"][0]["port"], 1080);
        assert_eq!(
            rules[1]["to"][0]["podSelector"]["matchLabels"]["app"],
            "egress-proxy"
        );
        assert_eq!(
            policy["spec"]["ingress"][0]["from"][0]["podSelector"]["matchLabels"]["app"],
            "everruns"
        );
        assert_eq!(policy["spec"]["ingress"][0]["ports"][0]["port"], 8080);
    }

    #[test]
    fn blocked_egress_allows_nothing_out() {
        let settings = settings();
        let egress = pod_egress(&rules(SandboxEgressMode::Blocked), &settings, None).unwrap();
        assert_eq!(egress, PodEgress::Blocked);
        let policy =
            serde_json::to_value(objects(&settings).network_policy(&egress).unwrap()).unwrap();
        assert_eq!(policy["spec"]["egress"], json!([]));
        assert_eq!(policy["spec"]["policyTypes"], json!(["Ingress", "Egress"]));
    }

    #[test]
    fn what_a_network_policy_cannot_enforce_fails_instead_of_widening() {
        let settings = settings();
        let domains = pod_egress(
            &rules(SandboxEgressMode::Allowlist {
                domains: vec!["github.com".into()],
                cidrs: vec![],
            }),
            &settings,
            Some(&credential()),
        )
        .unwrap_err();
        assert!(domains.contains("domain allowlist"), "{domains}");

        let mut no_route = settings.clone();
        no_route.egress = EgressRoute::None;
        assert!(pod_egress(&SandboxEgressRules::open(), &no_route, None).is_err());
        // A proxy route without a credential never falls back to direct.
        assert!(pod_egress(&SandboxEgressRules::open(), &settings, None).is_err());
    }

    #[test]
    fn ip_allowlists_and_direct_routes_map_to_ip_blocks() {
        let settings = settings();
        let cidrs = pod_egress(
            &rules(SandboxEgressMode::Allowlist {
                domains: vec![],
                cidrs: vec!["203.0.113.0/24".into()],
            }),
            &settings,
            None,
        )
        .unwrap();
        let policy =
            serde_json::to_value(objects(&settings).network_policy(&cidrs).unwrap()).unwrap();
        assert_eq!(
            policy["spec"]["egress"][0]["to"][0]["ipBlock"]["cidr"],
            "203.0.113.0/24"
        );

        let mut direct = settings.clone();
        direct.egress = EgressRoute::Direct;
        let egress = pod_egress(&SandboxEgressRules::open(), &direct, None).unwrap();
        let policy =
            serde_json::to_value(objects(&direct).network_policy(&egress).unwrap()).unwrap();
        let except = &policy["spec"]["egress"][1]["to"][0]["ipBlock"]["except"];
        assert!(
            except
                .as_array()
                .unwrap()
                .contains(&json!("169.254.0.0/16")),
            "the metadata service stays unreachable"
        );
    }

    #[test]
    fn label_values_follow_kubernetes_rules() {
        assert!(valid_label_value("org_0123"));
        assert!(valid_label_value("true"));
        assert!(!valid_label_value("-x"));
        assert!(!valid_label_value("a b"));
        assert!(!valid_label_value(&"a".repeat(64)));
    }
}
