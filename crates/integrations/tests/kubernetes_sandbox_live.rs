#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The `kubernetes` provider against a real cluster: a kind cluster in CI
//! (`kubernetes-sandbox-live.yml`), any cluster locally.
//!
//! Needs a kubeconfig, `KUBERNETES_SANDBOX_NAMESPACE` (an existing namespace
//! the kubeconfig may manage pods, volume claims, secrets and network
//! policies in) and `KUBERNETES_SANDBOX_IMAGE` (an image with the sandbox
//! guest the cluster can start, such as a `sandbox-dev` build loaded into
//! kind). The test runs outside the cluster, so it reaches the guest through
//! the API server's pod proxy. Fails closed: a missing setting is a failure,
//! never a skip, because this target only runs when it is asked for.

use std::sync::Arc;

use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::session_sandbox::{
    SessionSandboxConfig, SessionSandboxCredential, SessionSandboxCredentialSource,
    SessionSandboxExecRequest, SessionSandboxProvider,
};
use everruns_contracts::typed_id::{OrgId, SessionId};
use everruns_integrations::kubernetes_sandbox::{
    EgressRoute, GuestRoute, KubernetesSandboxProvider, KubernetesSandboxSettings,
};
use serde_json::json;

mod common;

use common::{MockLeasedResourceStore, MockStorageStore};

fn exec(command: &str) -> SessionSandboxExecRequest {
    SessionSandboxExecRequest {
        command: command.into(),
        cwd: None,
        timeout_ms: Some(60_000),
        output_mode: "full".into(),
    }
}

#[tokio::test]
async fn a_real_pod_runs_commands_keeps_files_across_pause_and_cleans_up() {
    let mut settings = KubernetesSandboxSettings::from_env()
        .expect("valid KUBERNETES_SANDBOX_* settings")
        .expect("KUBERNETES_SANDBOX_NAMESPACE must be set for the live test");
    std::env::var("KUBERNETES_SANDBOX_IMAGE")
        .expect("KUBERNETES_SANDBOX_IMAGE must name an image with the sandbox guest");
    settings.guest_route = GuestRoute::ApiProxy;
    settings.egress = EgressRoute::None;
    let client = kube::Client::try_default()
        .await
        .expect("a kubeconfig for the live cluster");
    let provider = KubernetesSandboxProvider::new(client, settings);

    let leases = Arc::new(MockLeasedResourceStore::default());
    let ctx =
        ToolContext::with_storage_store(SessionId::new(), Arc::new(MockStorageStore::default()))
            .with_leased_resource_store(leases.clone())
            .with_org_id(OrgId::new());
    let config = SessionSandboxConfig {
        provider: "kubernetes".into(),
        credential: SessionSandboxCredential {
            source: SessionSandboxCredentialSource::Platform,
            virtual_user_id: None,
            connection_id: None,
        },
        provider_config: json!({"network": {"mode": "blocked"}}),
        ..Default::default()
    };

    let instance = provider.create(&ctx, &config).await.unwrap();
    let outcome = async {
        let output = provider
            .exec(&ctx, &config, &instance, &exec("pwd && uname -s && id -u"))
            .await
            .map_err(|err| format!("{err:?}"))?;
        assert_eq!(output.exit_code, 0, "{output:?}");
        assert!(
            output.stdout.starts_with("/workspace\nLinux\n"),
            "{}",
            output.stdout
        );

        // Blocked egress is enforced by the cluster, not by the command.
        let blocked = provider
            .exec(
                &ctx,
                &config,
                &instance,
                &exec(
                    "timeout 5 bash -c 'echo > /dev/tcp/1.1.1.1/443' && echo open || echo closed",
                ),
            )
            .await
            .map_err(|err| format!("{err:?}"))?;
        assert!(blocked.stdout.contains("closed"), "{blocked:?}");

        provider
            .write_file(&ctx, &config, &instance, "notes/a.txt", b"kept")
            .await
            .map_err(|err| format!("{err:?}"))?;
        let paused = provider
            .pause(&ctx, &config, &instance)
            .await
            .map_err(|err| format!("{err:?}"))?;
        let resumed = provider
            .resume(&ctx, &config, &paused)
            .await
            .map_err(|err| format!("{err:?}"))?;
        let read = provider
            .read_file(&ctx, &config, &resumed, "notes/a.txt")
            .await
            .map_err(|err| format!("{err:?}"))?;
        assert_eq!(read.content, "kept");
        Ok::<_, String>(resumed)
    }
    .await;
    // Delete whatever happened above, so a failed run leaves no pod behind.
    let last = outcome.as_ref().map_or(&instance, |resumed| resumed);
    provider.delete(&ctx, &config, last).await.unwrap();
    outcome.unwrap();
    let owned = provider
        .list_owned("", &std::collections::BTreeMap::new())
        .await
        .expect("list sandboxes after delete");
    assert!(
        owned
            .iter()
            .all(|sandbox| sandbox.external_id != instance.external_id),
        "the sandbox is still listed after delete"
    );
}
