#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The `kubernetes` managed Sandboxes provider end to end against an
//! in-process Kubernetes API (`mock_kubernetes`) whose pods run the real
//! sandbox guest: create, exec, files, pause, resume, eviction, replace,
//! delete, caps, egress and the orphan-scan admin calls.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::runtime::egress::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressScope, EgressService,
    EgressStreamResponse,
};
use everruns_contracts::runtime::leased_resource::LeasedResourceStatus;
use everruns_contracts::runtime::network_access::NetworkAccessList;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use everruns_contracts::runtime::{SandboxProxyCredential, tool_context::ToolContext};
use everruns_contracts::session_sandbox::{
    SessionSandboxConfig, SessionSandboxContext, SessionSandboxCredential,
    SessionSandboxCredentialSource, SessionSandboxExecRequest, SessionSandboxInstance,
    SessionSandboxLease, SessionSandboxProvider, SessionSandboxState, SessionSandboxStatus,
};
use everruns_contracts::typed_id::{OrgId, SessionId};
use everruns_integrations::kubernetes_sandbox::{
    EgressRoute, GuestRoute, KubernetesSandboxProvider, KubernetesSandboxSettings,
};
use serde_json::{Value, json};

mod common;
mod mock_kubernetes;

use common::{MockLeasedResourceStore, MockStorageStore};
use mock_kubernetes::MockCluster;

struct Fixture {
    cluster: MockCluster,
    provider: KubernetesSandboxProvider,
    workspace: tempfile::TempDir,
}

fn settings(cluster: &MockCluster) -> KubernetesSandboxSettings {
    let mut settings = KubernetesSandboxSettings::new("sandboxes");
    settings.runtime_class = Some("isolated-sandbox".into());
    settings.guest_port = cluster.state.guest_port;
    settings.ready_timeout = std::time::Duration::from_secs(5);
    settings.egress = EgressRoute::Proxy {
        address: "egress-proxy.everruns.svc:1080".into(),
        pod_labels: BTreeMap::from([("app".into(), "egress-proxy".into())]),
        namespace: None,
    };
    settings
}

async fn fixture_with(edit: impl FnOnce(&mut KubernetesSandboxSettings)) -> Fixture {
    let cluster = mock_kubernetes::start().await;
    let mut settings = settings(&cluster);
    edit(&mut settings);
    let provider = KubernetesSandboxProvider::new(cluster.client(), settings);
    Fixture {
        cluster,
        provider,
        workspace: tempfile::tempdir().unwrap(),
    }
}

async fn fixture() -> Fixture {
    fixture_with(|_| {}).await
}

/// Hands out sandbox proxy credentials, as the host's egress service does
/// when it routes sandboxes.
struct ProxyEgress;

#[async_trait]
impl EgressService for ProxyEgress {
    async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
        Err(EgressError::InvalidRequest("not used".into()))
    }
    async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::InvalidRequest("not used".into()))
    }
    fn sandbox_proxy_credential(
        &self,
        _: Option<&EgressScope>,
        _: std::time::Duration,
    ) -> EgressResult<Option<SandboxProxyCredential>> {
        Ok(Some(SandboxProxyCredential {
            address: "proxy.example.com:1080".into(),
            tls: true,
            username: "s1".into(),
            password: "signed-claims".into(),
            expires_at: 4_102_444_800,
        }))
    }
}

/// The tool context, with the organization label the host's session store
/// would add (the cap and the orphan scan key on it).
#[derive(Clone)]
struct OrgContext {
    inner: ToolContext,
    org: OrgId,
}

#[async_trait]
impl SessionSandboxContext for OrgContext {
    fn session_id(&self) -> SessionId {
        SessionSandboxContext::session_id(&self.inner)
    }
    fn clone_context(&self) -> Arc<dyn SessionSandboxContext> {
        Arc::new(self.clone())
    }
    async fn connection_token(
        &self,
        provider: &str,
    ) -> Result<Option<String>, ToolExecutionResult> {
        self.inner.connection_token(provider).await
    }
    async fn sandbox_connection_token(
        &self,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> Result<Option<String>, ToolExecutionResult> {
        self.inner
            .sandbox_connection_token(provider, credential)
            .await
    }
    async fn resource_labels(&self) -> serde_json::Map<String, Value> {
        let mut labels = self.inner.resource_labels().await;
        labels.insert("everruns.org_id".into(), json!(self.org.to_string()));
        labels
    }
    fn network_access(&self) -> Option<NetworkAccessList> {
        SessionSandboxContext::network_access(&self.inner)
    }
    fn sandbox_proxy_credential(
        &self,
        valid_for: std::time::Duration,
    ) -> Result<Option<SandboxProxyCredential>, ToolExecutionResult> {
        SessionSandboxContext::sandbox_proxy_credential(&self.inner, valid_for)
    }
    async fn refresh_lease(&self, lease: SessionSandboxLease) -> Result<(), ToolExecutionResult> {
        self.inner.refresh_lease(lease).await
    }
    async fn release_lease(
        &self,
        provider: &str,
        external_id: &str,
    ) -> Result<(), ToolExecutionResult> {
        self.inner.release_lease(provider, external_id).await
    }
}

fn context_with(
    org: OrgId,
    access: Option<NetworkAccessList>,
) -> (OrgContext, Arc<MockLeasedResourceStore>) {
    let leases = Arc::new(MockLeasedResourceStore::default());
    let inner =
        ToolContext::with_storage_store(SessionId::new(), Arc::new(MockStorageStore::default()))
            .with_leased_resource_store(leases.clone())
            .with_org_id(org)
            .with_network_access(access)
            .with_egress_service(Arc::new(ProxyEgress));
    (OrgContext { inner, org }, leases)
}

fn context(org: OrgId) -> (OrgContext, Arc<MockLeasedResourceStore>) {
    context_with(org, None)
}

impl Fixture {
    fn config(&self, extra: Value) -> SessionSandboxConfig {
        let mut provider_config = json!({
            "workspace_path": self.workspace.path().join("ws").display().to_string(),
        });
        if let (Some(target), Some(extra)) = (provider_config.as_object_mut(), extra.as_object()) {
            target.extend(extra.clone());
        }
        SessionSandboxConfig {
            provider: "kubernetes".into(),
            credential: SessionSandboxCredential {
                source: SessionSandboxCredentialSource::Platform,
                virtual_user_id: None,
                connection_id: None,
            },
            provider_config,
            ..Default::default()
        }
    }
}

fn state(instance: &SessionSandboxInstance) -> SessionSandboxState {
    SessionSandboxState {
        sandbox: None,
        provider: "kubernetes".into(),
        status: SessionSandboxStatus::Running,
        instance: instance.clone(),
        init_completed_at: None,
        last_init_error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn exec(command: &str) -> SessionSandboxExecRequest {
    SessionSandboxExecRequest {
        command: command.into(),
        cwd: None,
        timeout_ms: Some(10_000),
        output_mode: "full".into(),
    }
}

fn error_text(result: ToolExecutionResult) -> String {
    format!("{result:?}")
}

#[tokio::test]
async fn a_sandbox_lives_through_create_exec_files_pause_resume_and_delete() {
    let f = fixture().await;
    let (ctx, leases) = context(OrgId::new());
    let config = f.config(json!({"size": "medium"}));

    let instance = f.provider.create(&ctx, &config).await.unwrap();
    let name = instance.external_id.clone();
    assert!(name.starts_with("er-sbx-"), "{name}");
    assert_eq!(instance.metadata["isolation"], "isolated-sandbox");
    assert_eq!(instance.metadata["size"], "medium");
    assert_eq!(instance.metadata["remote_state"], "running");
    for kind in [
        "pods",
        "secrets",
        "persistentvolumeclaims",
        "networkpolicies",
    ] {
        assert_eq!(f.cluster.state.names(kind).len(), 1, "{kind}");
    }
    let pod = f.cluster.state.object("pods", &name).unwrap();
    assert_eq!(pod["spec"]["runtimeClassName"], "isolated-sandbox");
    assert_eq!(
        pod["spec"]["containers"][0]["resources"]["limits"],
        json!({"cpu": "2", "memory": "4Gi"})
    );
    assert_eq!(pod["metadata"]["labels"]["everruns.managed"], "true");
    assert_eq!(pod["metadata"]["labels"]["everruns.sandbox"], name);
    assert_eq!(leases.resources.lock().await.len(), 1);

    // Exec runs in the workspace, with the proxy pointed at the in-cluster
    // listener and the minted credential.
    let output = f
        .provider
        .exec(&ctx, &config, &instance, &exec("pwd && echo $ALL_PROXY"))
        .await
        .unwrap();
    assert_eq!(output.exit_code, 0, "{output:?}");
    assert!(output.stdout.contains("/ws\n"), "{}", output.stdout);
    assert!(
        output
            .stdout
            .contains("socks5h://s1:signed-claims@egress-proxy.everruns.svc:1080"),
        "{}",
        output.stdout
    );

    let failed = f
        .provider
        .exec(&ctx, &config, &instance, &exec("exit 127"))
        .await
        .unwrap();
    assert_eq!(failed.exit_code, 127);
    assert!(failed.hint.unwrap().contains("not found"));

    let written = f
        .provider
        .write_file(&ctx, &config, &instance, "notes/a.txt", b"hello")
        .await
        .unwrap();
    assert!(written.path.ends_with("/ws/notes/a.txt"));
    let read = f
        .provider
        .read_file(&ctx, &config, &instance, "notes/a.txt")
        .await
        .unwrap();
    assert_eq!(read.content, "hello");

    // Pause stops compute and keeps the workspace.
    let paused = f.provider.pause(&ctx, &config, &instance).await.unwrap();
    assert_eq!(paused.metadata["remote_state"], "paused");
    assert!(f.cluster.state.object("pods", &name).is_none());
    assert!(
        f.cluster
            .state
            .object("persistentvolumeclaims", &format!("{name}-workspace"))
            .is_some()
    );
    let status = f
        .provider
        .status(&ctx, &config, &state(&paused))
        .await
        .unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Paused);

    // Resume starts a new pod on the same volume: files are back.
    let resumed = f.provider.resume(&ctx, &config, &paused).await.unwrap();
    assert_eq!(resumed.external_id, name);
    let read = f
        .provider
        .read_file(&ctx, &config, &resumed, "notes/a.txt")
        .await
        .unwrap();
    assert_eq!(read.content, "hello");
    let status = f
        .provider
        .status(&ctx, &config, &state(&resumed))
        .await
        .unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Running);

    f.provider.delete(&ctx, &config, &resumed).await.unwrap();
    for kind in [
        "pods",
        "secrets",
        "persistentvolumeclaims",
        "networkpolicies",
    ] {
        assert!(f.cluster.state.names(kind).is_empty(), "{kind} left behind");
    }
    let leases = leases.resources.lock().await;
    assert_eq!(leases[0].status, LeasedResourceStatus::Released);
}

#[tokio::test]
async fn an_evicted_pod_is_replaced_on_resume_keeping_files() {
    let f = fixture().await;
    let (ctx, _) = context(OrgId::new());
    let config = f.config(json!({}));
    let instance = f.provider.create(&ctx, &config).await.unwrap();
    f.provider
        .write_file(&ctx, &config, &instance, "kept.txt", b"still here")
        .await
        .unwrap();
    f.cluster.state.evict(&instance.external_id);
    let status = f
        .provider
        .status(&ctx, &config, &state(&instance))
        .await
        .unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Paused);

    let resumed = f.provider.resume(&ctx, &config, &instance).await.unwrap();
    let pod = f
        .cluster
        .state
        .object("pods", &resumed.external_id)
        .unwrap();
    assert_eq!(pod["status"]["phase"], "Running");
    let read = f
        .provider
        .read_file(&ctx, &config, &resumed, "kept.txt")
        .await
        .unwrap();
    assert_eq!(read.content, "still here");
}

#[tokio::test]
async fn a_lost_workspace_is_replaced_by_a_fresh_sandbox() {
    let f = fixture().await;
    let (ctx, _) = context(OrgId::new());
    let config = f.config(json!({}));
    let instance = f.provider.create(&ctx, &config).await.unwrap();
    let paused = f.provider.pause(&ctx, &config, &instance).await.unwrap();
    f.cluster.state.objects.lock().unwrap().remove(&(
        "persistentvolumeclaims".to_string(),
        format!("{}-workspace", instance.external_id),
    ));
    let status = f
        .provider
        .status(&ctx, &config, &state(&paused))
        .await
        .unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Lost);

    let replaced = f.provider.resume(&ctx, &config, &paused).await.unwrap();
    assert_ne!(replaced.external_id, instance.external_id);
    assert_eq!(replaced.metadata["replaced"], json!(instance.external_id));
    // Nothing of the old sandbox is left.
    assert!(
        f.cluster
            .state
            .object("secrets", &format!("{}-guest", instance.external_id))
            .is_none()
    );
}

#[tokio::test]
async fn blocked_egress_writes_a_deny_all_policy_and_no_proxy() {
    let f = fixture().await;
    let (ctx, _) = context(OrgId::new());
    let config = f.config(json!({"network": {"mode": "blocked"}}));
    let instance = f.provider.create(&ctx, &config).await.unwrap();
    let policy = f
        .cluster
        .state
        .object("networkpolicies", &instance.external_id)
        .unwrap();
    assert_eq!(policy["spec"]["egress"], json!([]));
    let output = f
        .provider
        .exec(&ctx, &config, &instance, &exec("echo \"[$ALL_PROXY]\""))
        .await
        .unwrap();
    assert_eq!(output.stdout.trim(), "[]");
}

#[tokio::test]
async fn resume_applies_the_current_session_policy() {
    let f = fixture().await;
    let (ctx, _) = context(OrgId::new());
    let open = f.config(json!({}));
    let instance = f.provider.create(&ctx, &open).await.unwrap();
    let paused = f.provider.pause(&ctx, &open, &instance).await.unwrap();
    let blocked = f.config(json!({"network": {"mode": "blocked"}}));
    f.provider.resume(&ctx, &blocked, &paused).await.unwrap();
    let policy = f
        .cluster
        .state
        .object("networkpolicies", &instance.external_id)
        .unwrap();
    assert_eq!(policy["spec"]["egress"], json!([]));
}

#[tokio::test]
async fn domain_allowlists_are_refused_rather_than_widened() {
    let f = fixture().await;
    let (ctx, _) = context_with(
        OrgId::new(),
        Some(NetworkAccessList::allow_only(["github.com"])),
    );
    let err = f
        .provider
        .create(&ctx, &f.config(json!({})))
        .await
        .unwrap_err();
    assert!(error_text(err).contains("domain allowlist"));
    assert!(f.cluster.state.names("pods").is_empty());
}

#[tokio::test]
async fn open_egress_without_a_route_does_not_start() {
    let f = fixture_with(|settings| settings.egress = EgressRoute::None).await;
    let (ctx, _) = context(OrgId::new());
    let err = f
        .provider
        .create(&ctx, &f.config(json!({})))
        .await
        .unwrap_err();
    assert!(error_text(err).contains("no internet route"));
    // Deny still works on such a deployment.
    f.provider
        .create(&ctx, &f.config(json!({"network": {"mode": "blocked"}})))
        .await
        .unwrap();
}

#[tokio::test]
async fn the_organization_cap_counts_running_sandboxes_only() {
    let f = fixture_with(|settings| settings.max_per_org = 1).await;
    let org = OrgId::new();
    let (ctx, _) = context(org);
    let config = f.config(json!({}));
    let first = f.provider.create(&ctx, &config).await.unwrap();
    let (other, _) = context(org);
    let err = f.provider.create(&other, &config).await.unwrap_err();
    assert!(error_text(err).contains("already runs 1"));
    // Another organization is not affected.
    let (stranger, _) = context(OrgId::new());
    f.provider.create(&stranger, &config).await.unwrap();
    // A paused sandbox frees the slot.
    f.provider.pause(&ctx, &config, &first).await.unwrap();
    f.provider.create(&other, &config).await.unwrap();
}

#[tokio::test]
async fn deployment_limits_on_size_and_image_hold_at_creation() {
    let f = fixture_with(|settings| {
        settings.sizes = vec![everruns_integrations::kubernetes_sandbox::SandboxSize::Small];
        settings.allowed_image_prefixes = vec!["ghcr.io/everruns/".into()];
    })
    .await;
    let (ctx, _) = context(OrgId::new());
    let large = f
        .provider
        .create(&ctx, &f.config(json!({"size": "large"})))
        .await
        .unwrap_err();
    assert!(error_text(large).contains("not offered"));
    let foreign = f
        .provider
        .create(&ctx, &f.config(json!({"image": "docker.io/someone/miner"})))
        .await
        .unwrap_err();
    assert!(error_text(foreign).contains("not allowed"));
    assert!(f.cluster.state.names("pods").is_empty());
}

#[tokio::test]
async fn an_image_that_cannot_be_pulled_fails_fast_and_leaves_nothing() {
    let f = fixture().await;
    f.cluster
        .state
        .broken_images
        .lock()
        .unwrap()
        .push("ghcr.io/acme/missing:1".into());
    let (ctx, leases) = context(OrgId::new());
    let started = std::time::Instant::now();
    let err = f
        .provider
        .create(&ctx, &f.config(json!({"image": "ghcr.io/acme/missing:1"})))
        .await
        .unwrap_err();
    assert!(error_text(err).contains("ErrImagePull"));
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    for kind in [
        "pods",
        "secrets",
        "persistentvolumeclaims",
        "networkpolicies",
    ] {
        assert!(f.cluster.state.names(kind).is_empty(), "{kind} left behind");
    }
    assert_eq!(
        leases.resources.lock().await[0].status,
        LeasedResourceStatus::Released
    );
}

#[tokio::test]
async fn servers_outside_the_cluster_reach_the_guest_through_the_api_proxy() {
    let f = fixture_with(|settings| settings.guest_route = GuestRoute::ApiProxy).await;
    let (ctx, _) = context(OrgId::new());
    let config = f.config(json!({}));
    let instance = f.provider.create(&ctx, &config).await.unwrap();
    let output = f
        .provider
        .exec(&ctx, &config, &instance, &exec("echo through-the-proxy"))
        .await
        .unwrap();
    assert_eq!(output.stdout.trim(), "through-the-proxy");
    assert!(
        f.cluster
            .state
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.starts_with("PROXY "))
    );
}

#[tokio::test]
async fn the_orphan_scan_lists_stops_and_deletes_by_label() {
    let f = fixture().await;
    let org = OrgId::new();
    let (ctx, _) = context(org);
    let config = f.config(json!({}));
    let running = f.provider.create(&ctx, &config).await.unwrap();
    let paused = f.provider.create(&ctx, &config).await.unwrap();
    f.provider.pause(&ctx, &config, &paused).await.unwrap();

    let labels = BTreeMap::from([("everruns.managed".to_string(), "true".to_string())]);
    let mut owned = f.provider.list_owned("", &labels).await.unwrap();
    owned.sort_by_key(|sandbox| !sandbox.active);
    assert_eq!(owned.len(), 2);
    assert_eq!(owned[0].external_id, running.external_id);
    assert!(owned[0].active && !owned[1].active);
    assert_eq!(owned[0].labels["everruns.org_id"], org.to_string());

    f.provider
        .stop_by_id("", &running.external_id)
        .await
        .unwrap();
    assert!(
        f.cluster
            .state
            .object("pods", &running.external_id)
            .is_none()
    );
    f.provider
        .delete_by_id("", &running.external_id)
        .await
        .unwrap();
    // Gone twice is still deleted.
    f.provider
        .delete_by_id("", &running.external_id)
        .await
        .unwrap();
    assert_eq!(f.provider.list_owned("", &labels).await.unwrap().len(), 1);
}
