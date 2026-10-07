#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The Modal managed Sandboxes provider end to end against the in-process
//! gRPC mock (`mock_modal`): create, exec, files, pause, resume, delete.

use everruns_contracts::runtime::leased_resource::LeasedResourceStatus;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use everruns_contracts::session_sandbox::{
    SessionSandboxConfig, SessionSandboxCredential, SessionSandboxCredentialSource,
    SessionSandboxExecRequest, SessionSandboxInstance, SessionSandboxProvider, SessionSandboxState,
    SessionSandboxStatus, create_session_sandbox_provider,
};
use everruns_integrations::modal::ModalSessionSandboxProvider;
use serde_json::{Value, json};

mod common;
mod mock_modal;

use common::{MockLeasedResourceStore, context, context_with_connections};
use mock_modal::{MockModal, TOKEN_ID, TOKEN_SECRET, start_mock};

fn config(url: &str, extra: Value) -> SessionSandboxConfig {
    let mut provider_config = json!({ "_test_server_url": url });
    if let (Some(target), Some(extra)) = (provider_config.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
    SessionSandboxConfig {
        provider: "modal".into(),
        credential: SessionSandboxCredential {
            source: SessionSandboxCredentialSource::SessionUser,
            virtual_user_id: Some(uuid::Uuid::nil()),
            connection_id: None,
        },
        provider_config,
        ..Default::default()
    }
}

fn token() -> String {
    format!("{TOKEN_ID}:{TOKEN_SECRET}")
}

fn state(instance: &SessionSandboxInstance) -> SessionSandboxState {
    SessionSandboxState {
        sandbox: None,
        provider: "modal".into(),
        status: SessionSandboxStatus::Running,
        instance: instance.clone(),
        init_completed_at: None,
        last_init_error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn exec(command: &str, cwd: Option<&str>) -> SessionSandboxExecRequest {
    SessionSandboxExecRequest {
        command: command.into(),
        cwd: cwd.map(str::to_string),
        timeout_ms: None,
        output_mode: "full".into(),
    }
}

async fn lease_status(leases: &MockLeasedResourceStore, id: &str) -> Option<LeasedResourceStatus> {
    leases
        .resources
        .lock()
        .await
        .iter()
        .find(|r| r.external_id == id)
        .map(|r| r.status)
}

fn image_of(mock: &MockModal, id: &str) -> String {
    mock.lock().sandboxes[id].image_id.clone()
}

#[tokio::test]
async fn it_is_discoverable_by_provider_id() {
    let provider = create_session_sandbox_provider("modal").expect("registered");
    assert_eq!(provider.id(), "modal");
}

#[tokio::test]
async fn full_lifecycle_pauses_to_a_snapshot_and_resumes_from_it() {
    let (mock, url) = start_mock().await;
    let (ctx, _, leases) = context(Some(&token()));
    let cfg = config(
        &url,
        json!({"runtime": "gvisor", "cpu": 1.5, "memory_mb": 2048}),
    );
    let provider = ModalSessionSandboxProvider;

    // Create: a leased gVisor sandbox with the requested resources and session tags.
    let created = provider.create(&ctx, &cfg).await.unwrap();
    let first = created.external_id.clone();
    assert_eq!(created.workspace_path.as_deref(), Some("/workspace"));
    assert_eq!(
        created.display_name.as_deref(),
        Some(format!("Session Sandbox {}", ctx.session_id).as_str())
    );
    assert_eq!(created.provider_state["runtime"], "gvisor");
    {
        let state = mock.lock();
        let definition = &state.sandboxes[&first];
        assert_eq!(definition.runtime.as_deref(), Some("gvisor"));
        let resources = definition.resources.as_ref().unwrap();
        assert_eq!(resources.milli_cpu, 1500);
        assert_eq!(resources.memory_mb, 2048);
        // The default image gets git and curl.
        assert!(state.image_builds[0].iter().any(|l| l.contains("git curl")));
    }
    assert_eq!(
        lease_status(&leases, &first).await,
        Some(LeasedResourceStatus::Active)
    );

    // Exec runs in the workspace, or in a relative cwd resolved against it.
    let out = provider
        .exec(&ctx, &cfg, &created, &exec("echo hi", None))
        .await
        .unwrap();
    assert!(out.success);
    assert!(out.stdout.contains("cwd: /workspace\n"), "{}", out.stdout);
    let out = provider
        .exec(&ctx, &cfg, &created, &exec("ls", Some("src")))
        .await
        .unwrap();
    assert!(
        out.stdout.contains("cwd: /workspace/src\n"),
        "{}",
        out.stdout
    );
    let failed = provider
        .exec(&ctx, &cfg, &created, &exec("exit 3", None))
        .await
        .unwrap();
    assert!(!failed.success);
    assert_eq!(failed.exit_code, 3);

    // Files round-trip, binary included.
    let written = provider
        .write_file(&ctx, &cfg, &created, "notes/a.txt", b"hello")
        .await
        .unwrap();
    assert_eq!(written.path, "/workspace/notes/a.txt");
    assert_eq!(written.bytes_written, 5);
    let read = provider
        .read_file(&ctx, &cfg, &created, "notes/a.txt")
        .await
        .unwrap();
    assert_eq!(
        (read.content.as_str(), read.encoding.as_str()),
        ("hello", "text")
    );
    let binary = [0u8, 159, 146, 150, 255];
    provider
        .write_file(&ctx, &cfg, &created, "/tmp/blob.bin", &binary)
        .await
        .unwrap();
    let read = provider
        .read_file(&ctx, &cfg, &created, "/tmp/blob.bin")
        .await
        .unwrap();
    assert_eq!(read.encoding, "base64");
    use base64::Engine as _;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&read.content)
            .unwrap(),
        binary
    );
    let missing = provider
        .read_file(&ctx, &cfg, &created, "nope.txt")
        .await
        .unwrap_err();
    assert!(matches!(missing, ToolExecutionResult::ToolError(_)));

    let status = provider.status(&ctx, &cfg, &state(&created)).await.unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Running);

    // Pause snapshots, terminates, and releases the lease.
    let paused = provider.pause(&ctx, &cfg, &created).await.unwrap();
    assert_eq!(paused.external_id, first);
    assert_eq!(paused.provider_state["snapshot_image_id"], "im-snapshot");
    assert_eq!(paused.metadata["remote_state"], "paused");
    assert!(mock.lock().terminated.contains(&first));
    assert_eq!(
        lease_status(&leases, &first).await,
        Some(LeasedResourceStatus::Released)
    );
    let status = provider.status(&ctx, &cfg, &state(&paused)).await.unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Paused);
    // Pausing twice is a no-op.
    let again = provider.pause(&ctx, &cfg, &paused).await.unwrap();
    assert_eq!(again, paused);

    // Modal terminates asynchronously: the old sandbox can still report
    // running. Resume must trust the pause, not that report.
    mock.lock().terminated.retain(|id| id != &first);
    let status = provider.status(&ctx, &cfg, &state(&paused)).await.unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Paused);

    // Resume boots a new sandbox from the snapshot, with a fresh lease.
    let resumed = provider.resume(&ctx, &cfg, &paused).await.unwrap();
    let second = resumed.external_id.clone();
    assert_ne!(second, first);
    assert_eq!(image_of(&mock, &second), "im-snapshot");
    assert_eq!(mock.lock().image_builds.len(), 1, "no rebuild on resume");
    assert_eq!(resumed.display_name, created.display_name);
    assert_eq!(resumed.metadata["restored_from_snapshot"], true);
    assert_eq!(resumed.provider_state["paused"], false);
    assert_eq!(
        lease_status(&leases, &second).await,
        Some(LeasedResourceStatus::Active)
    );
    let out = provider
        .exec(&ctx, &cfg, &resumed, &exec("echo back", None))
        .await
        .unwrap();
    assert!(out.success);

    // Resuming a running sandbox keeps it.
    let again = provider.resume(&ctx, &cfg, &resumed).await.unwrap();
    assert_eq!(again.external_id, second);

    // Delete terminates and releases; deleting twice is fine.
    provider.delete(&ctx, &cfg, &resumed).await.unwrap();
    assert!(mock.lock().terminated.contains(&second));
    assert_eq!(
        lease_status(&leases, &second).await,
        Some(LeasedResourceStatus::Released)
    );
    provider.delete(&ctx, &cfg, &resumed).await.unwrap();
}

#[tokio::test]
async fn a_sandbox_that_ended_without_a_pause_is_lost_and_resume_boots_the_base_image() {
    let (mock, url) = start_mock().await;
    let (ctx, _, _) = context(Some(&token()));
    let cfg = config(&url, json!({}));
    let provider = ModalSessionSandboxProvider;

    let created = provider.create(&ctx, &cfg).await.unwrap();
    // Modal ended it (lifetime or idle timeout).
    mock.lock().terminated.push(created.external_id.clone());

    let status = provider.status(&ctx, &cfg, &state(&created)).await.unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Lost);

    let replacement = provider.resume(&ctx, &cfg, &created).await.unwrap();
    assert_ne!(replacement.external_id, created.external_id);
    let base = image_of(&mock, &created.external_id);
    assert_eq!(image_of(&mock, &replacement.external_id), base);
    assert_eq!(replacement.metadata["restored_from_snapshot"], false);
}

#[tokio::test]
async fn custom_image_and_workspace_are_used_as_is() {
    let (mock, url) = start_mock().await;
    let (ctx, _, _) = context(Some(&token()));
    let cfg = config(
        &url,
        json!({"image": "node:22", "workspace_path": "/srv/app", "title": "web"}),
    );
    let created = ModalSessionSandboxProvider
        .create(&ctx, &cfg)
        .await
        .unwrap();
    assert_eq!(created.workspace_path.as_deref(), Some("/srv/app"));
    assert_eq!(created.display_name.as_deref(), Some("web"));
    let state = mock.lock();
    assert_eq!(state.image_builds[0], vec!["FROM node:22".to_string()]);
    assert_eq!(
        state.sandboxes[&created.external_id].runtime.as_deref(),
        Some("vm")
    );
}

#[tokio::test]
async fn a_missing_connection_asks_for_one_and_creates_nothing() {
    let (mock, url) = start_mock().await;
    let (ctx, _, _) = context(None);
    let err = ModalSessionSandboxProvider
        .create(&ctx, &config(&url, json!({})))
        .await
        .unwrap_err();
    match err {
        ToolExecutionResult::ConnectionRequired { provider, .. } => assert_eq!(provider, "modal"),
        other => panic!("expected ConnectionRequired, got {other:?}"),
    }
    assert!(mock.lock().sandboxes.is_empty());
}

#[tokio::test]
async fn invalid_options_are_rejected_before_any_sandbox_exists() {
    let (mock, url) = start_mock().await;
    let (ctx, _, _) = context(Some(&token()));
    for bad in [
        json!({"runtime": "firecracker"}),
        json!({"image": "x; rm -rf /"}),
        json!({"workspace_path": "/srv/../etc"}),
        json!({"memory_mb": 1}),
    ] {
        let err = ModalSessionSandboxProvider
            .create(&ctx, &config(&url, bad.clone()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolExecutionResult::ToolError(_)), "{bad}");
    }
    assert!(mock.lock().sandboxes.is_empty());
}

#[tokio::test]
async fn egress_rules_reach_modal_and_the_token_stays_in_a_secret() {
    use everruns_integrations::modal::proto::client::network_access::NetworkAccessType;

    let (mock, url) = start_mock().await;
    let (ctx, leases) = context_with_connections(&[("modal", &token()), ("github", "ghp_secret")]);
    let cfg = config(
        &url,
        json!({
            "inject_connections": ["github"],
            "network": {"mode": "allowlist", "cidrs": ["140.82.112.0/20"]}
        }),
    );
    let provider = ModalSessionSandboxProvider;

    let created = provider.create(&ctx, &cfg).await.unwrap();
    let secret_id = created.provider_state["egress_secret_id"]
        .as_str()
        .unwrap()
        .to_string();
    {
        let state = mock.lock();
        let (_, values) = &state.secrets[&secret_id];
        assert_eq!(values["GITHUB_BEARER"], "ghp_secret");
        let definition = &state.sandboxes[&created.external_id];
        let network = definition.network_access.as_ref().unwrap();
        assert_eq!(
            network.network_access_type,
            NetworkAccessType::Allowlist as i32
        );
        assert_eq!(network.allowed_cidrs, vec!["140.82.112.0/20".to_string()]);
        let policy = definition.outbound_policy.as_ref().unwrap();
        let domains: Vec<_> = policy
            .header_replacements
            .iter()
            .map(|r| r.domain.as_str())
            .collect();
        assert_eq!(domains, ["api.github.com", "github.com"]);
        assert!(
            policy
                .header_replacements
                .iter()
                .all(|r| r.secret_id == secret_id)
        );
        // The sandbox definition carries only a reference, never the token.
        assert!(!format!("{definition:?}").contains("ghp_secret"));
    }
    // Cleanup can find the secret even if the session never pauses.
    let lease = leases.resources.lock().await[0].metadata.clone();
    assert_eq!(lease["egress_secret_id"], secret_id);
    assert!(!lease.to_string().contains("ghp_secret"));

    // Pause deletes the secret; resume makes a fresh one; delete removes it.
    let paused = provider.pause(&ctx, &cfg, &created).await.unwrap();
    assert!(mock.lock().deleted_secrets.contains(&secret_id));
    assert!(paused.provider_state.get("egress_secret_id").is_none());
    let resumed = provider.resume(&ctx, &cfg, &paused).await.unwrap();
    let second = resumed.provider_state["egress_secret_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(second, secret_id);
    assert!(
        mock.lock().sandboxes[&resumed.external_id]
            .outbound_policy
            .is_some()
    );
    provider.delete(&ctx, &cfg, &resumed).await.unwrap();
    assert!(
        mock.lock().secrets.is_empty(),
        "no secret outlives the sandbox"
    );
}

#[tokio::test]
async fn blocked_network_reaches_modal_without_a_secret() {
    use everruns_integrations::modal::proto::client::network_access::NetworkAccessType;

    let (mock, url) = start_mock().await;
    let (ctx, _, _) = context(Some(&token()));
    let cfg = config(&url, json!({"network": {"mode": "blocked"}}));
    let created = ModalSessionSandboxProvider
        .create(&ctx, &cfg)
        .await
        .unwrap();
    let state = mock.lock();
    let definition = &state.sandboxes[&created.external_id];
    assert_eq!(
        definition
            .network_access
            .as_ref()
            .unwrap()
            .network_access_type,
        NetworkAccessType::Blocked as i32
    );
    assert!(definition.outbound_policy.is_none());
    assert_eq!(state.secrets_created, 0);
}

#[tokio::test]
async fn a_missing_injected_connection_asks_for_it_and_creates_nothing() {
    let (mock, url) = start_mock().await;
    let (ctx, _) = context_with_connections(&[("modal", &token())]);
    let err = ModalSessionSandboxProvider
        .create(
            &ctx,
            &config(&url, json!({"inject_connections": ["github"]})),
        )
        .await
        .unwrap_err();
    match err {
        ToolExecutionResult::ConnectionRequired { provider, .. } => assert_eq!(provider, "github"),
        other => panic!("expected ConnectionRequired, got {other:?}"),
    }
    let state = mock.lock();
    assert!(state.sandboxes.is_empty() && state.secrets.is_empty());
}
