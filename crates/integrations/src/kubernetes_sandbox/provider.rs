//! The `kubernetes` managed Sandboxes provider.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::runtime::exec_tool_result::ExecToolResultPayload;
use everruns_contracts::runtime::sandbox_egress::{
    SandboxEgressMode, SandboxEgressRules, sandbox_egress_rules,
};
use everruns_contracts::sandbox_guest::GuestExecRequest;
use everruns_contracts::session_sandbox::{
    LABEL_ORG_ID, LABEL_SESSION_ID, OwnedSandbox, SandboxAdminError, SessionSandboxConfig,
    SessionSandboxContext, SessionSandboxExecRequest, SessionSandboxExecResponse,
    SessionSandboxInstance, SessionSandboxLease, SessionSandboxProvider,
    SessionSandboxReadFileResponse, SessionSandboxState, SessionSandboxStatus,
    SessionSandboxStatusResponse, SessionSandboxWriteFileResponse,
};
use everruns_contracts::tools::ToolExecutionResult;
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use k8s_openapi::api::networking::v1::NetworkPolicy;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use super::gate::{PlatformSandboxGate, SandboxRun, apply_grant, unsettled_run};
use super::settings::{GuestRoute, KubernetesSandboxSettings};
use super::spec::{
    ANNOTATION_SIZE, LABEL_SANDBOX, PodEgress, SandboxObjects, TOKEN_KEY, guest_secret_name,
    pod_egress, valid_label_value, workspace_claim_name,
};
use super::transport::ApiProxyTransport;
use super::{KUBERNETES_PROVIDER, LEASE_DURATION_SECONDS, Options, SandboxSize, parse_options};
use crate::sandbox_guest::client::{DirectTransport, GuestClient, GuestTransport};
use crate::x11_desktop::{DesktopExecOutput, DesktopHost, X11DesktopSession};

/// How often a long command refreshes the cleanup lease.
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// Lifetime of a sandbox's egress proxy credential. Each resume mints a
/// fresh one; a pod running longer than this without a pause loses egress,
/// it never falls back to direct.
const PROXY_CREDENTIAL_LIFETIME: Duration = Duration::from_secs(2 * 86_400);
/// How long the guest may take to answer once its pod reports ready.
const GUEST_READY_TIMEOUT: Duration = Duration::from_secs(60);
/// Field manager for server-side apply.
const FIELD_MANAGER: &str = "everruns";
/// Waiting reasons that will not fix themselves.
const FATAL_WAITING_REASONS: &[&str] = &[
    "ErrImagePull",
    "ImagePullBackOff",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "CrashLoopBackOff",
    "RunContainerError",
];

/// Settings from the environment, `None` when the target is off.
pub fn configured_settings() -> Result<Option<KubernetesSandboxSettings>, String> {
    KubernetesSandboxSettings::from_env()
}

#[derive(Clone)]
struct Backend {
    client: kube::Client,
    settings: Arc<KubernetesSandboxSettings>,
    gate: Option<Arc<dyn PlatformSandboxGate>>,
}

static ENV_BACKEND: tokio::sync::OnceCell<Backend> = tokio::sync::OnceCell::const_new();

/// The `kubernetes` provider. The registered instance reads its cluster and
/// settings from the environment on first use; embedders and tests pass
/// their own with [`KubernetesSandboxProvider::new`].
pub struct KubernetesSandboxProvider {
    backend: Option<Backend>,
}

impl KubernetesSandboxProvider {
    pub fn new(client: kube::Client, settings: KubernetesSandboxSettings) -> Self {
        Self {
            backend: Some(Backend {
                client,
                settings: Arc::new(settings),
                gate: None,
            }),
        }
    }

    /// Ask `gate` before every pod start and settle each run with it (see
    /// [`super::gate`]). The environment-built provider uses the gate
    /// installed with [`super::gate::install_gate`].
    pub fn with_gate(mut self, gate: Arc<dyn PlatformSandboxGate>) -> Self {
        if let Some(backend) = self.backend.as_mut() {
            backend.gate = Some(gate);
        }
        self
    }

    pub fn from_environment() -> Self {
        Self { backend: None }
    }

    async fn backend(&self) -> Result<Backend, String> {
        if let Some(backend) = &self.backend {
            return Ok(backend.clone());
        }
        ENV_BACKEND
            .get_or_try_init(|| async {
                let settings = configured_settings()?.ok_or(
                    "isolated sandboxes are not configured on this deployment (KUBERNETES_SANDBOX_NAMESPACE)",
                )?;
                let client = kube::Client::try_default()
                    .await
                    .map_err(|err| format!("cannot reach Kubernetes: {err}"))?;
                Ok::<_, String>(Backend {
                    client,
                    settings: Arc::new(settings),
                    gate: super::gate::installed_gate(),
                })
            })
            .await
            .cloned()
    }

    async fn tool_backend(&self) -> Result<Backend, ToolExecutionResult> {
        self.backend()
            .await
            .map_err(ToolExecutionResult::tool_error)
    }
}

impl Backend {
    fn pods(&self) -> Api<Pod> {
        Api::namespaced(self.client.clone(), &self.settings.namespace)
    }
    fn claims(&self) -> Api<PersistentVolumeClaim> {
        Api::namespaced(self.client.clone(), &self.settings.namespace)
    }
    fn secrets(&self) -> Api<Secret> {
        Api::namespaced(self.client.clone(), &self.settings.namespace)
    }
    fn policies(&self) -> Api<NetworkPolicy> {
        Api::namespaced(self.client.clone(), &self.settings.namespace)
    }

    /// Delete every object of `name`; what is already gone counts as deleted.
    async fn delete_all(&self, name: &str) -> Result<(), String> {
        let now = DeleteParams {
            grace_period_seconds: Some(0),
            ..DeleteParams::default()
        };
        self.settle(name).await;
        ignore_missing(self.pods().delete(name, &now).await.map(|_| ()))?;
        ignore_missing(
            self.claims()
                .delete(&workspace_claim_name(name), &DeleteParams::default())
                .await
                .map(|_| ()),
        )?;
        ignore_missing(
            self.secrets()
                .delete(&guest_secret_name(name), &DeleteParams::default())
                .await
                .map(|_| ()),
        )?;
        ignore_missing(
            self.policies()
                .delete(name, &DeleteParams::default())
                .await
                .map(|_| ()),
        )
    }

    /// Delete the pod only; the workspace and the rest stay.
    async fn stop(&self, name: &str) -> Result<(), String> {
        let graceful = DeleteParams {
            grace_period_seconds: Some(10),
            ..DeleteParams::default()
        };
        self.settle(name).await;
        ignore_missing(self.pods().delete(name, &graceful).await.map(|_| ()))
    }

    /// Start the sandbox's pod, after the gate (when installed) admits the
    /// run; the grant bounds the pod's lifetime.
    async fn start_pod(&self, mut pod: Pod) -> Result<(), String> {
        let grant = match &self.gate {
            Some(gate) => {
                let labels = pod.metadata.labels.clone().unwrap_or_default();
                let annotations = pod.metadata.annotations.clone().unwrap_or_default();
                let run = SandboxRun {
                    run_id: uuid::Uuid::new_v4().simple().to_string(),
                    sandbox: pod.metadata.name.clone().unwrap_or_default(),
                    org_id: labels.get(LABEL_ORG_ID).cloned(),
                    session_id: labels.get(LABEL_SESSION_ID).cloned(),
                    size: annotations
                        .get(ANNOTATION_SIZE)
                        .cloned()
                        .unwrap_or_default(),
                };
                let grant = gate.admit(&run).await?;
                apply_grant(&mut pod, &run, &grant)?;
                Some((gate, grant, run))
            }
            None => None,
        };
        if let Err(err) = self.pods().create(&PostParams::default(), &pod).await {
            if let Some((gate, grant, run)) = grant
                && let Err(settle) = gate.settle(&grant.id, &run, Duration::ZERO).await
            {
                warn!(sandbox = %run.sandbox, "settling a pod that never started failed: {settle}");
            }
            return Err(format!("cannot create the sandbox pod: {err}"));
        }
        Ok(())
    }

    /// Settle the pod's run with the gate before the pod goes. Failures are
    /// logged: the gate counts an unsettled grant as fully used.
    async fn settle(&self, name: &str) {
        let Some(gate) = &self.gate else {
            return;
        };
        let pod = match self.pods().get_opt(name).await {
            Ok(Some(pod)) => pod,
            Ok(None) => return,
            Err(err) => {
                warn!(sandbox = %name, "cannot read the pod to settle its run: {err}");
                return;
            }
        };
        if let Some((grant, run, ran)) = unsettled_run(&pod, chrono::Utc::now())
            && let Err(err) = gate.settle(&grant, &run, ran).await
        {
            warn!(sandbox = %name, "settling the sandbox run failed: {err}");
        }
    }

    async fn token(&self, name: &str) -> Result<String, String> {
        let secret = self
            .secrets()
            .get(&guest_secret_name(name))
            .await
            .map_err(|err| format!("sandbox {name} has no guest secret: {err}"))?;
        secret
            .data
            .as_ref()
            .and_then(|data| data.get(TOKEN_KEY))
            .and_then(|bytes| String::from_utf8(bytes.0.clone()).ok())
            .ok_or_else(|| format!("sandbox {name} guest secret has no token"))
    }

    fn transport(
        &self,
        name: &str,
        pod_ip: Option<&str>,
    ) -> Result<Arc<dyn GuestTransport>, String> {
        let port = self.settings.guest_port;
        Ok(match (self.settings.guest_route, pod_ip) {
            (GuestRoute::PodIp, Some(ip)) => {
                let host = if ip.contains(':') {
                    format!("[{ip}]")
                } else {
                    ip.to_string()
                };
                Arc::new(DirectTransport::new(format!("http://{host}:{port}"))?)
            }
            (GuestRoute::PodIp, None) => {
                return Err(format!("sandbox {name} has no pod address yet"));
            }
            (GuestRoute::ApiProxy, _) => Arc::new(ApiProxyTransport {
                client: self.client.clone(),
                namespace: self.settings.namespace.clone(),
                pod: name.to_string(),
                port,
            }),
        })
    }

    async fn guest(&self, state: &ProviderState) -> Result<GuestClient, String> {
        let pod_ip = match &state.pod_ip {
            Some(ip) => Some(ip.clone()),
            None if self.settings.guest_route == GuestRoute::PodIp => self
                .pods()
                .get_opt(&state.name)
                .await
                .map_err(|err| err.to_string())?
                .and_then(|pod| pod.status.and_then(|status| status.pod_ip)),
            None => None,
        };
        let transport = self.transport(&state.name, pod_ip.as_deref())?;
        Ok(GuestClient::new(transport, self.token(&state.name).await?))
    }

    /// Wait for the pod to run and its guest to answer; returns the pod IP.
    async fn wait_ready(&self, name: &str) -> Result<Option<String>, String> {
        let started = std::time::Instant::now();
        let mut last_reason = String::from("the pod was not scheduled yet");
        loop {
            if let Some(pod) = self.pods().get_opt(name).await.map_err(|e| e.to_string())? {
                let status = pod.status.unwrap_or_default();
                let waiting = status
                    .container_statuses
                    .iter()
                    .flatten()
                    .filter_map(|container| container.state.as_ref()?.waiting.as_ref())
                    .find(|waiting| {
                        waiting
                            .reason
                            .as_deref()
                            .is_some_and(|reason| FATAL_WAITING_REASONS.contains(&reason))
                    });
                if let Some(waiting) = waiting {
                    return Err(format!(
                        "the sandbox pod cannot start: {} {}",
                        waiting.reason.as_deref().unwrap_or_default(),
                        waiting.message.as_deref().unwrap_or_default()
                    )
                    .trim()
                    .to_string());
                }
                if let Some(unscheduled) = status.conditions.iter().flatten().find(|condition| {
                    condition.type_ == "PodScheduled" && condition.status == "False"
                }) {
                    last_reason = format!(
                        "the pod is not scheduled: {}",
                        unscheduled.message.as_deref().unwrap_or("no capacity")
                    );
                }
                let ready = status
                    .conditions
                    .iter()
                    .flatten()
                    .any(|condition| condition.type_ == "Ready" && condition.status == "True");
                if status.phase.as_deref() == Some("Running") && ready {
                    let guest = GuestClient::new(
                        self.transport(name, status.pod_ip.as_deref())?,
                        self.token(name).await?,
                    );
                    guest
                        .wait_ready(GUEST_READY_TIMEOUT)
                        .await
                        .map_err(|err| err.to_string())?;
                    return Ok(status.pod_ip);
                }
                if matches!(status.phase.as_deref(), Some("Failed" | "Succeeded")) {
                    return Err(format!(
                        "the sandbox pod stopped while starting: {}",
                        status.message.as_deref().unwrap_or("no message")
                    ));
                }
            }
            if started.elapsed() >= self.settings.ready_timeout {
                return Err(format!(
                    "the sandbox did not start within {} s: {last_reason}",
                    self.settings.ready_timeout.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(status) if status.is_not_found())
}

fn ignore_missing(result: Result<(), kube::Error>) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(err) if is_not_found(&err) => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

/// Non-secret state needed to reach the sandbox again.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct ProviderState {
    name: String,
    namespace: String,
    size: String,
    image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pod_ip: Option<String>,
}

impl ProviderState {
    fn from_instance(instance: &SessionSandboxInstance) -> Result<Self, ToolExecutionResult> {
        serde_json::from_value(instance.provider_state.clone()).map_err(|err| {
            ToolExecutionResult::internal_error_msg(format!(
                "Corrupt isolated sandbox state: {err}"
            ))
        })
    }

    fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

fn metadata(
    settings: &KubernetesSandboxSettings,
    state: &ProviderState,
    remote_state: &str,
) -> Value {
    json!({
        "remote_state": remote_state,
        "isolation": settings.runtime_class.as_deref().unwrap_or("container"),
        "vm_isolated": settings.vm_isolated(),
        "size": state.size,
        "image": state.image,
        "namespace": state.namespace,
        "pod": state.name,
    })
}

fn sandbox_name() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    format!("er-sbx-{}", &id[..16])
}

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Session labels Kubernetes can carry, plus the sandbox's own.
fn labels_for(
    session_labels: serde_json::Map<String, Value>,
    name: &str,
) -> BTreeMap<String, String> {
    let mut labels: BTreeMap<String, String> = session_labels
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key, value.to_string())))
        .filter(|(_, value)| valid_label_value(value))
        .collect();
    labels.insert(LABEL_SANDBOX.to_string(), name.to_string());
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "everruns".to_string(),
    );
    labels
}

fn session_rules(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
) -> Result<SandboxEgressRules, ToolExecutionResult> {
    let rules = sandbox_egress_rules(
        config.provider_config.get("network"),
        context.network_access().as_ref(),
    )
    .map_err(|error| {
        ToolExecutionResult::tool_error(format!("isolated sandbox network policy: {error}"))
    })?;
    if !rules.unenforced_blocks.is_empty() {
        warn!(
            blocks = ?rules.unenforced_blocks,
            "isolated sandboxes cannot enforce blocked hosts without an allowlist; tool egress still checks them"
        );
    }
    Ok(rules)
}

/// The egress for a sandbox, minting a proxy credential when it needs one.
fn egress_for(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    settings: &KubernetesSandboxSettings,
) -> Result<PodEgress, ToolExecutionResult> {
    let rules = session_rules(context, config)?;
    let credential = match (&rules.mode, &settings.egress) {
        (SandboxEgressMode::Open, super::settings::EgressRoute::Proxy { .. }) => {
            context.sandbox_proxy_credential(PROXY_CREDENTIAL_LIFETIME)?
        }
        _ => None,
    };
    pod_egress(&rules, settings, credential.as_ref()).map_err(ToolExecutionResult::tool_error)
}

fn exec_env(egress: &PodEgress) -> &str {
    match egress {
        PodEgress::Proxy { exec_env } => exec_env,
        _ => "",
    }
}

/// A desktop sandbox reached through its guest agent: xdotool and the
/// display scripts run as argv, never through a shell line.
struct GuestDesktopHost {
    guest: GuestClient,
}

#[async_trait::async_trait]
impl DesktopHost for GuestDesktopHost {
    async fn exec_argv(
        &self,
        program: &str,
        args: &[String],
        env: &[(&str, &str)],
        timeout_ms: u64,
    ) -> Result<DesktopExecOutput, String> {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(program.to_string());
        argv.extend(args.iter().cloned());
        let output = self
            .guest
            .exec(&GuestExecRequest {
                argv,
                env: env
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.to_string()))
                    .collect(),
                timeout_ms: Some(timeout_ms),
                ..Default::default()
            })
            .await
            .map_err(|err| err.to_string())?;
        Ok(DesktopExecOutput {
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        self.guest
            .read_file(path)
            .await
            .map_err(|err| err.to_string())
    }
}

/// Bring the desktop's display up as soon as the guest answers, not on the
/// first `computer` action: a browser the agent starts from the shell needs
/// the X server already there. A new pod (create, resume) has none.
async fn start_desktop(
    backend: &Backend,
    config: &SessionSandboxConfig,
    state: &ProviderState,
) -> Result<(), String> {
    let Some(settings) = config.desktop() else {
        return Ok(());
    };
    let display =
        everruns_contracts::runtime::computer_use::ComputerUseConfig::from_value_or_default(
            &settings,
        )
        .display();
    let guest = backend.guest(state).await?;
    crate::x11_desktop::start_display(&GuestDesktopHost { guest }, display).await
}

fn image_for(
    options: &Options,
    config: &SessionSandboxConfig,
    settings: &KubernetesSandboxSettings,
) -> String {
    match &options.image {
        Some(image) => image.clone(),
        None if config.desktop().is_some() => settings.desktop_image.clone(),
        None => settings.image.clone(),
    }
}

async fn refresh_lease(
    context: &dyn SessionSandboxContext,
    config: &SessionSandboxConfig,
    instance: &SessionSandboxInstance,
    state: &ProviderState,
) -> Result<(), ToolExecutionResult> {
    context
        .refresh_lease(SessionSandboxLease {
            provider: KUBERNETES_PROVIDER.to_string(),
            external_id: instance.external_id.clone(),
            display_name: instance.display_name.clone(),
            duration_seconds: LEASE_DURATION_SECONDS,
            credential: config.credential.clone(),
            // THREAT[TM-API-015]: lease metadata is API-visible; non-secret only.
            metadata: json!({
                "namespace": state.namespace,
                "size": state.size,
                "image": state.image,
                "workspace_path": instance.workspace_path,
                "managed": true,
            }),
        })
        .await
}

fn resolve_path(instance: &SessionSandboxInstance, path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    let base = instance
        .workspace_path
        .as_deref()
        .unwrap_or(super::DEFAULT_WORKSPACE_PATH);
    let joined = format!("{}/{}", base.trim_end_matches('/'), path);
    joined.trim_end_matches("/.").to_string()
}

fn exit_code_hint(exit_code: i32) -> Option<&'static str> {
    match exit_code {
        0 => None,
        124 => Some("Command timed out."),
        126 => Some("Command found but not executable. Check file permissions."),
        127 => Some("Command not found. Check that the tool is installed and in PATH."),
        137 => Some(
            "Process was killed (SIGKILL). This often means the sandbox ran out of memory; a larger size may help.",
        ),
        _ if exit_code > 128 && exit_code <= 192 => Some("Process was killed by a signal."),
        _ => None,
    }
}

fn tool_error(err: impl std::fmt::Display) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(err.to_string())
}

impl KubernetesSandboxProvider {
    /// Create the objects and start the pod. Everything created is removed
    /// again when a later step fails, so a failed create leaves nothing.
    async fn create_sandbox(
        &self,
        backend: &Backend,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let settings = &backend.settings;
        let options = parse_options(&config.provider_config).map_err(tool_error)?;
        if !settings.sizes.contains(&options.size) {
            return Err(tool_error(format!(
                "size '{}' is not offered on this deployment (allowed: {})",
                options.size.as_str(),
                settings
                    .sizes
                    .iter()
                    .map(|size| size.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let image = image_for(&options, config, settings);
        if !settings.image_allowed(&image) {
            return Err(tool_error(format!(
                "image '{image}' is not allowed on this deployment"
            )));
        }
        let egress = egress_for(context, config, settings)?;
        let session_labels = context.managed_resource_labels().await;
        let org = session_labels
            .get(LABEL_ORG_ID)
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(org) = &org
            && valid_label_value(org)
        {
            let running = backend
                .pods()
                .list(&ListParams::default().labels(&format!(
                    "{LABEL_ORG_ID}={org},{}=true",
                    everruns_contracts::session_sandbox::LABEL_MANAGED
                )))
                .await
                .map_err(tool_error)?
                .items
                .len();
            if running >= settings.max_per_org as usize {
                return Err(tool_error(format!(
                    "this organization already runs {running} isolated sandboxes, the most this deployment allows; stop one, or wait until an idle one pauses"
                )));
            }
        }

        let name = sandbox_name();
        let objects = SandboxObjects {
            settings,
            name: &name,
            labels: labels_for(session_labels, &name),
            size: options.size,
            image: &image,
            workspace_path: &options.workspace_path,
            desktop: config.desktop().is_some(),
        };
        let token = new_token();
        let created = async {
            let post = PostParams::default();
            backend
                .secrets()
                .create(&post, &objects.guest_secret(&token, exec_env(&egress))?)
                .await
                .map_err(|err| format!("cannot create the guest secret: {err}"))?;
            backend
                .claims()
                .create(&post, &objects.workspace_claim()?)
                .await
                .map_err(|err| format!("cannot create the workspace volume: {err}"))?;
            backend
                .policies()
                .create(&post, &objects.network_policy(&egress)?)
                .await
                .map_err(|err| format!("cannot create the network policy: {err}"))?;
            backend.start_pod(objects.pod()?).await?;
            Ok::<_, String>(())
        }
        .await;
        if let Err(err) = created {
            if let Err(cleanup) = backend.delete_all(&name).await {
                warn!(sandbox = %name, "cleanup after a failed create failed: {cleanup}");
            }
            return Err(tool_error(err));
        }

        let mut state = ProviderState {
            name: name.clone(),
            namespace: settings.namespace.clone(),
            size: options.size.as_str().to_string(),
            image: image.clone(),
            pod_ip: None,
        };
        let display_name = config
            .provider_config
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .map(str::to_string)
            .or_else(|| Some(format!("Session Sandbox {}", context.session_id())));
        let mut instance = SessionSandboxInstance {
            external_id: name.clone(),
            display_name,
            workspace_path: Some(options.workspace_path.clone()),
            provider_state: state.to_value(),
            metadata: metadata(settings, &state, "starting"),
        };
        // A sandbox nothing can clean up must not outlive this call.
        if let Err(err) = refresh_lease(context, config, &instance, &state).await {
            let _ = backend.delete_all(&name).await;
            return Err(err);
        }
        let ready = async {
            state.pod_ip = backend.wait_ready(&name).await?;
            start_desktop(backend, config, &state).await
        };
        match ready.await {
            Ok(()) => {}
            Err(err) => {
                let _ = backend.delete_all(&name).await;
                let _ = context.release_lease(KUBERNETES_PROVIDER, &name).await;
                return Err(tool_error(err));
            }
        }
        instance.provider_state = state.to_value();
        instance.metadata = metadata(settings, &state, "running");
        Ok(instance)
    }

    /// Start a fresh sandbox for one whose workspace is gone, and clean up
    /// what is left of the old one.
    async fn replace(
        &self,
        backend: &Backend,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let mut replacement = self.create_sandbox(backend, context, config).await?;
        if let Err(err) = backend.delete_all(&instance.external_id).await {
            warn!(sandbox = %instance.external_id, "cleanup of a replaced sandbox failed: {err}");
        }
        if let Err(err) = context
            .release_lease(KUBERNETES_PROVIDER, &instance.external_id)
            .await
        {
            warn!(sandbox = %instance.external_id, error = ?err, "lease release for a replaced sandbox failed");
        }
        replacement.display_name = instance.display_name.clone();
        if let Some(metadata) = replacement.metadata.as_object_mut() {
            metadata.insert("replaced".to_string(), json!(instance.external_id));
        }
        Ok(replacement)
    }
}

#[async_trait::async_trait]
impl SessionSandboxProvider for KubernetesSandboxProvider {
    fn id(&self) -> &str {
        KUBERNETES_PROVIDER
    }

    async fn list_owned(
        &self,
        _token: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
        let backend = self.backend().await.map_err(SandboxAdminError::Provider)?;
        if labels.values().any(|value| !valid_label_value(value)) {
            return Err(SandboxAdminError::Provider(
                "label values Kubernetes cannot hold".to_string(),
            ));
        }
        let selector = labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(",");
        let params = ListParams::default().labels(&selector);
        let claims = backend
            .claims()
            .list(&params)
            .await
            .map_err(|err| SandboxAdminError::Provider(err.to_string()))?;
        let pods = backend
            .pods()
            .list(&params)
            .await
            .map_err(|err| SandboxAdminError::Provider(err.to_string()))?;
        // Deletion is asynchronous: a deleted volume claim stays listed, with
        // a deletion timestamp, until its pod is gone. A sandbox being torn
        // down is no longer owned, so the orphan scan neither reports nor
        // deletes it twice.
        let running: std::collections::HashSet<String> = pods
            .items
            .iter()
            .filter(|pod| pod.metadata.deletion_timestamp.is_none())
            .filter_map(|pod| pod.metadata.name.clone())
            .collect();
        Ok(claims
            .items
            .into_iter()
            .filter(|claim| claim.metadata.deletion_timestamp.is_none())
            .filter_map(|claim| {
                let labels = claim.metadata.labels.clone().unwrap_or_default();
                let name = labels.get(LABEL_SANDBOX)?.clone();
                let active = running.contains(&name);
                Some(OwnedSandbox {
                    external_id: name,
                    labels,
                    state: if active { "running" } else { "paused" }.to_string(),
                    active,
                    created_at: claim.metadata.creation_timestamp.map(|time| {
                        chrono::DateTime::from_timestamp(time.0.as_second(), 0).unwrap_or_default()
                    }),
                })
            })
            .collect())
    }

    async fn describe_by_id(
        &self,
        token: &str,
        external_id: &str,
    ) -> Result<everruns_contracts::session_sandbox::ProviderSandboxView, SandboxAdminError> {
        use everruns_contracts::session_sandbox::ProviderSandboxView;
        let selector = BTreeMap::from([(LABEL_SANDBOX.to_string(), external_id.to_string())]);
        let Some(found) = self
            .list_owned(token, &selector)
            .await?
            .into_iter()
            .find(|sandbox| sandbox.external_id == external_id)
        else {
            return Ok(ProviderSandboxView::missing(external_id));
        };
        let backend = self.backend().await.map_err(SandboxAdminError::Provider)?;
        // The pod's own trouble (image pull, crash loop, eviction), if any.
        let error = match backend.pods().get_opt(external_id).await {
            Ok(Some(pod)) => pod.status.and_then(|status| {
                let waiting = status
                    .container_statuses
                    .unwrap_or_default()
                    .into_iter()
                    .find_map(|container| {
                        let waiting = container.state?.waiting?;
                        Some(match waiting.message {
                            Some(message) => {
                                format!("{}: {message}", waiting.reason.unwrap_or_default())
                            }
                            None => waiting.reason?,
                        })
                    });
                waiting.or(status.reason)
            }),
            _ => None,
        };
        Ok(ProviderSandboxView {
            external_id: found.external_id,
            exists: true,
            state: found.state,
            active: Some(found.active),
            error,
            created_at: found.created_at,
            labels: ProviderSandboxView::everruns_labels(found.labels),
            ..ProviderSandboxView::default()
        })
    }

    async fn stop_by_id(&self, _token: &str, external_id: &str) -> Result<(), SandboxAdminError> {
        let backend = self.backend().await.map_err(SandboxAdminError::Provider)?;
        backend
            .stop(external_id)
            .await
            .map_err(SandboxAdminError::Provider)
    }

    async fn delete_by_id(&self, _token: &str, external_id: &str) -> Result<(), SandboxAdminError> {
        let backend = self.backend().await.map_err(SandboxAdminError::Provider)?;
        backend
            .delete_all(external_id)
            .await
            .map_err(SandboxAdminError::Provider)
    }

    async fn create(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        self.create_sandbox(&backend, context, config).await
    }

    async fn resume(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        let mut state = ProviderState::from_instance(instance)?;
        let name = state.name.clone();
        let claim = backend
            .claims()
            .get_opt(&workspace_claim_name(&name))
            .await
            .map_err(tool_error)?;
        if claim.is_none() {
            // The workspace is gone: nothing to resume, start over.
            return self.replace(&backend, context, config, instance).await;
        }
        // The session policy may have changed since the sandbox started, and
        // a proxy credential expires: rewrite both before the pod starts.
        let egress = egress_for(context, config, &backend.settings)?;
        let objects = SandboxObjects {
            settings: &backend.settings,
            name: &name,
            labels: labels_for(context.managed_resource_labels().await, &name),
            size: SandboxSize::parse(&state.size).unwrap_or(SandboxSize::Small),
            image: &state.image,
            workspace_path: instance
                .workspace_path
                .as_deref()
                .unwrap_or(super::DEFAULT_WORKSPACE_PATH),
            desktop: config.desktop().is_some(),
        };
        let apply = PatchParams::apply(FIELD_MANAGER).force();
        let token = backend.token(&name).await.map_err(tool_error)?;
        backend
            .secrets()
            .patch(
                &guest_secret_name(&name),
                &apply,
                &Patch::Apply(
                    objects
                        .guest_secret(&token, exec_env(&egress))
                        .map_err(tool_error)?,
                ),
            )
            .await
            .map_err(tool_error)?;
        backend
            .policies()
            .patch(
                &name,
                &apply,
                &Patch::Apply(objects.network_policy(&egress).map_err(tool_error)?),
            )
            .await
            .map_err(tool_error)?;

        let pod = backend.pods().get_opt(&name).await.map_err(tool_error)?;
        let phase = pod
            .as_ref()
            .and_then(|pod| pod.status.as_ref())
            .and_then(|status| status.phase.clone());
        let terminating = pod
            .as_ref()
            .is_some_and(|pod| pod.metadata.deletion_timestamp.is_some());
        if pod.is_some()
            && (terminating || matches!(phase.as_deref(), Some("Failed" | "Succeeded")))
        {
            // Evicted or still stopping: wait for it to go, then start anew.
            backend.stop(&name).await.map_err(tool_error)?;
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            while backend
                .pods()
                .get_opt(&name)
                .await
                .map_err(tool_error)?
                .is_some()
            {
                if std::time::Instant::now() > deadline {
                    return Err(tool_error("the old sandbox pod did not stop"));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        if pod.is_none() || terminating || matches!(phase.as_deref(), Some("Failed" | "Succeeded"))
        {
            backend
                .start_pod(objects.pod().map_err(tool_error)?)
                .await
                .map_err(tool_error)?;
        }
        state.pod_ip = backend.wait_ready(&name).await.map_err(tool_error)?;
        start_desktop(&backend, config, &state)
            .await
            .map_err(tool_error)?;
        refresh_lease(context, config, instance, &state).await?;
        Ok(SessionSandboxInstance {
            provider_state: state.to_value(),
            metadata: metadata(&backend.settings, &state, "running"),
            ..instance.clone()
        })
    }

    async fn pause(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<SessionSandboxInstance, ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        let mut state = ProviderState::from_instance(instance)?;
        backend.stop(&state.name).await.map_err(tool_error)?;
        state.pod_ip = None;
        // A paused sandbox holds only its volume; the lease keeps it owned.
        refresh_lease(context, config, instance, &state).await?;
        Ok(SessionSandboxInstance {
            provider_state: state.to_value(),
            metadata: metadata(&backend.settings, &state, "paused"),
            ..instance.clone()
        })
    }

    async fn delete(
        &self,
        context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
    ) -> Result<(), ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        backend
            .delete_all(&instance.external_id)
            .await
            .map_err(tool_error)?;
        context
            .release_lease(KUBERNETES_PROVIDER, &instance.external_id)
            .await
    }

    async fn exec(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        request: &SessionSandboxExecRequest,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        let state = ProviderState::from_instance(instance)?;
        let guest = backend.guest(&state).await.map_err(tool_error)?;

        // Long commands keep the lease alive so cleanup does not reap a busy
        // sandbox.
        let heartbeat_context = context.clone_context();
        let heartbeat_config = config.clone();
        let heartbeat_instance = instance.clone();
        let heartbeat_state = state.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::time::sleep(LEASE_HEARTBEAT_INTERVAL).await;
                if let Err(err) = refresh_lease(
                    heartbeat_context.as_ref(),
                    &heartbeat_config,
                    &heartbeat_instance,
                    &heartbeat_state,
                )
                .await
                {
                    warn!(error = ?err, "isolated sandbox heartbeat failed");
                }
            }
        });
        let result = guest
            .exec(&GuestExecRequest {
                command: request.command.clone(),
                cwd: Some(resolve_path(
                    instance,
                    request.cwd.as_deref().unwrap_or("."),
                )),
                timeout_ms: request.timeout_ms,
                ..Default::default()
            })
            .await;
        heartbeat.abort();
        let output = result.map_err(tool_error)?;
        refresh_lease(context, config, instance, &state).await?;
        let payload = ExecToolResultPayload::new(
            &output.stdout,
            &output.stderr,
            output.exit_code,
            &request.output_mode,
        );
        Ok(SessionSandboxExecResponse {
            exit_code: payload.exit_code,
            stdout: payload.stdout,
            stderr: payload.stderr,
            success: payload.success,
            truncated: payload.truncated || output.truncated,
            total_lines: payload.total_lines,
            raw_output: Some(payload.raw_output),
            hint: exit_code_hint(payload.exit_code).map(str::to_string),
        })
    }

    async fn read_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
    ) -> Result<SessionSandboxReadFileResponse, ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        let state = ProviderState::from_instance(instance)?;
        let full_path = resolve_path(instance, path);
        let bytes = backend
            .guest(&state)
            .await
            .map_err(tool_error)?
            .read_file(&full_path)
            .await
            .map_err(|err| tool_error(format!("Failed to read {full_path}: {err}")))?;
        refresh_lease(context, config, instance, &state).await?;
        let (content, encoding) = everruns_contracts::runtime::SessionFile::encode_content(&bytes);
        Ok(SessionSandboxReadFileResponse {
            path: full_path,
            content,
            encoding: encoding.to_string(),
        })
    }

    async fn write_file(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        path: &str,
        content: &[u8],
    ) -> Result<SessionSandboxWriteFileResponse, ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        let state = ProviderState::from_instance(instance)?;
        let full_path = resolve_path(instance, path);
        backend
            .guest(&state)
            .await
            .map_err(tool_error)?
            .write_file(&full_path, content)
            .await
            .map_err(|err| tool_error(format!("Failed to write {full_path}: {err}")))?;
        refresh_lease(context, config, instance, &state).await?;
        Ok(SessionSandboxWriteFileResponse {
            path: full_path,
            bytes_written: content.len(),
        })
    }

    async fn open_desktop(
        &self,
        context: &dyn SessionSandboxContext,
        config: &SessionSandboxConfig,
        instance: &SessionSandboxInstance,
        display: everruns_contracts::runtime::computer_use::DisplaySize,
    ) -> Result<
        Box<dyn everruns_contracts::runtime::computer_use::ComputerSession>,
        ToolExecutionResult,
    > {
        if config.desktop().is_none() {
            return Err(tool_error(
                "this isolated sandbox was created without a desktop",
            ));
        }
        let backend = self.tool_backend().await?;
        let state = ProviderState::from_instance(instance)?;
        let guest = backend.guest(&state).await.map_err(tool_error)?;
        let session = X11DesktopSession::open(GuestDesktopHost { guest }, display)
            .await
            .map_err(tool_error)?;
        refresh_lease(context, config, instance, &state).await?;
        Ok(Box::new(session))
    }

    async fn status(
        &self,
        _context: &dyn SessionSandboxContext,
        _config: &SessionSandboxConfig,
        state: &SessionSandboxState,
    ) -> Result<SessionSandboxStatusResponse, ToolExecutionResult> {
        let backend = self.tool_backend().await?;
        let provider_state = ProviderState::from_instance(&state.instance)?;
        let name = &provider_state.name;
        let claim = backend
            .claims()
            .get_opt(&workspace_claim_name(name))
            .await
            .map_err(tool_error)?;
        let pod = backend.pods().get_opt(name).await.map_err(tool_error)?;
        let (session_status, remote_state) = match (claim, pod) {
            (None, _) => (SessionSandboxStatus::Lost, "workspace gone".to_string()),
            (Some(_), None) => (SessionSandboxStatus::Paused, "paused".to_string()),
            (Some(_), Some(pod)) => {
                let phase = pod
                    .status
                    .and_then(|status| status.phase)
                    .unwrap_or_else(|| "Pending".to_string());
                let status = match phase.as_str() {
                    // Evicted or stopped: resume starts a new pod.
                    "Failed" | "Succeeded" => SessionSandboxStatus::Paused,
                    _ => SessionSandboxStatus::Running,
                };
                (status, phase.to_lowercase())
            }
        };
        Ok(SessionSandboxStatusResponse {
            provider: state.provider.clone(),
            session_status,
            external_id: state.instance.external_id.clone(),
            display_name: state.instance.display_name.clone(),
            workspace_path: state.instance.workspace_path.clone(),
            metadata: metadata(&backend.settings, &provider_state, &remote_state),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_tokens_are_fresh_and_label_safe() {
        let (a, b) = (sandbox_name(), sandbox_name());
        assert_ne!(a, b);
        assert!(valid_label_value(&a) && a.len() == 23, "{a}");
        let token = new_token();
        assert_eq!(token.len(), 64);
        assert_ne!(token, new_token());
    }

    #[test]
    fn labels_keep_only_what_kubernetes_accepts() {
        let mut session = serde_json::Map::new();
        session.insert("everruns.org_id".into(), json!("org_1"));
        session.insert("everruns.managed".into(), json!("true"));
        session.insert("bad".into(), json!("has space"));
        session.insert("number".into(), json!(3));
        let labels = labels_for(session, "er-sbx-1");
        assert_eq!(labels["everruns.org_id"], "org_1");
        assert_eq!(labels[LABEL_SANDBOX], "er-sbx-1");
        assert!(!labels.contains_key("bad") && !labels.contains_key("number"));
    }

    #[test]
    fn desktops_use_the_deployment_desktop_image_unless_the_template_names_one() {
        let mut settings = KubernetesSandboxSettings::new("sandboxes");
        settings.image = "ghcr.io/everruns/sandbox-dev:edge".into();
        settings.desktop_image = "ghcr.io/everruns/sandbox-desktop:edge".into();
        let config = |desktop: bool| -> SessionSandboxConfig {
            serde_json::from_value(json!({
                "provider": KUBERNETES_PROVIDER,
                "provider_config": { "desktop": desktop },
            }))
            .unwrap()
        };
        let mut options = Options {
            size: SandboxSize::Small,
            image: None,
            workspace_path: "/workspace".into(),
        };
        assert_eq!(
            image_for(&options, &config(true), &settings),
            settings.desktop_image
        );
        assert_eq!(
            image_for(&options, &config(false), &settings),
            settings.image
        );
        options.image = Some("example.com/own:1".into());
        assert_eq!(
            image_for(&options, &config(true), &settings),
            "example.com/own:1"
        );
    }

    #[test]
    fn relative_paths_resolve_against_the_workspace() {
        let instance = SessionSandboxInstance {
            workspace_path: Some("/workspace".into()),
            ..Default::default()
        };
        assert_eq!(resolve_path(&instance, "a/b"), "/workspace/a/b");
        assert_eq!(resolve_path(&instance, "."), "/workspace");
        assert_eq!(resolve_path(&instance, "/etc/hosts"), "/etc/hosts");
    }
}
