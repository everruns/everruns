#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The boxd managed Sandboxes provider end to end against the in-process
//! mock (`mock_boxd`): create, exec, files, pause, resume, replace, delete,
//! and the network policy that reaches boxd.

use everruns_contracts::runtime::leased_resource::LeasedResourceStatus;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use everruns_contracts::session_sandbox::{
    SessionSandboxConfig, SessionSandboxCredential, SessionSandboxCredentialSource,
    SessionSandboxExecRequest, SessionSandboxInstance, SessionSandboxProvider, SessionSandboxState,
    SessionSandboxStatus,
};
use everruns_integrations::boxd::BoxdSessionSandboxProvider;
use serde_json::{Value, json};

mod common;
mod mock_boxd;

use common::{MockLeasedResourceStore, context_with_connections};
use mock_boxd::{API_KEY, MockBoxd, start_mock};

fn config(url: &str, extra: Value) -> SessionSandboxConfig {
    let mut provider_config = json!({ "_test_server_url": url });
    if let (Some(target), Some(extra)) = (provider_config.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
    SessionSandboxConfig {
        provider: "boxd".into(),
        credential: SessionSandboxCredential {
            source: SessionSandboxCredentialSource::SessionUser,
            virtual_user_id: Some(uuid::Uuid::nil()),
            connection_id: None,
        },
        provider_config,
        ..Default::default()
    }
}

fn state(instance: &SessionSandboxInstance) -> SessionSandboxState {
    SessionSandboxState {
        sandbox: None,
        provider: "boxd".into(),
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

fn machine_id(instance: &SessionSandboxInstance) -> String {
    instance.external_id.split_once('/').unwrap().1.to_string()
}

fn status_of(mock: &MockBoxd, id: &str) -> String {
    mock.lock().machines[id].status.clone()
}

#[tokio::test]
async fn full_lifecycle_hibernates_and_wakes_the_same_machine() {
    let (mock, url) = start_mock().await;
    let (ctx, leases) = context_with_connections(&[("boxd", API_KEY)]);
    let cfg = config(&url, json!({"region": "nyc", "size": "4x16"}));
    let provider = BoxdSessionSandboxProvider;

    // Create: an isolated machine of the requested size, in the requested
    // region, with boxd's own idle timers off and the workspace made.
    let created = provider.create(&ctx, &cfg).await.unwrap();
    assert!(
        created.external_id.starts_with("nyc/vm_"),
        "{}",
        created.external_id
    );
    let id = machine_id(&created);
    assert_eq!(
        created.workspace_path.as_deref(),
        Some("/home/boxd/workspace")
    );
    assert_eq!(
        created.display_name.as_deref(),
        Some(format!("Session Sandbox {}", ctx.session_id).as_str())
    );
    {
        let state = mock.lock();
        let request = &state.create_requests[0];
        let spec = request.spec.as_ref().unwrap();
        assert!(spec.isolated);
        assert_eq!(spec.size, "4x16");
        assert!(request.name.starts_with("er-"), "{}", request.name);
        assert_eq!(
            request
                .config
                .as_ref()
                .and_then(|c| c.srf.as_ref())
                .and_then(|srf| srf.auto_suspend_timeout_secs),
            Some(0),
            "boxd must not suspend on its own"
        );
        assert!(state.auto_hibernate_calls.contains(&(id.clone(), 0)));
        assert!(state.dirs[&id].contains("/home/boxd/workspace"));
        // The first JWT was revoked, so a refresh happened.
        assert!(state.jwts_issued >= 2);
    }
    assert_eq!(status_of(&mock, &id), "running");
    assert_eq!(
        lease_status(&leases, &created.external_id).await,
        Some(LeasedResourceStatus::Active)
    );

    // Exec runs in the workspace, or a relative cwd below it.
    let out = provider
        .exec(&ctx, &cfg, &created, &exec("echo hi", None))
        .await
        .unwrap();
    assert!(out.success);
    assert!(
        out.stdout
            .contains("cd -- '/home/boxd/workspace' && exec sh -c 'echo hi'"),
        "{}",
        out.stdout
    );
    let out = provider
        .exec(&ctx, &cfg, &created, &exec("ls", Some("src")))
        .await
        .unwrap();
    assert!(
        out.stdout.contains("'/home/boxd/workspace/src'"),
        "{}",
        out.stdout
    );
    let failed = provider
        .exec(&ctx, &cfg, &created, &exec("echo err >&2; exit 3", None))
        .await
        .unwrap();
    assert!(!failed.success);
    assert_eq!(failed.exit_code, 3);
    assert!(failed.stderr.contains("oops"), "{}", failed.stderr);

    // Files round-trip, binary included, relative paths under the workspace.
    let written = provider
        .write_file(&ctx, &cfg, &created, "notes/a.txt", b"hello")
        .await
        .unwrap();
    assert_eq!(written.path, "/home/boxd/workspace/notes/a.txt");
    assert_eq!(written.bytes_written, 5);
    let read = provider
        .read_file(&ctx, &cfg, &created, "notes/a.txt")
        .await
        .unwrap();
    assert_eq!(read.content, "hello");
    let binary = [0u8, 159, 146, 150];
    provider
        .write_file(&ctx, &cfg, &created, "/tmp/b.bin", &binary)
        .await
        .unwrap();
    let read = provider
        .read_file(&ctx, &cfg, &created, "/tmp/b.bin")
        .await
        .unwrap();
    assert_eq!(read.encoding, "base64");
    let missing = provider
        .read_file(&ctx, &cfg, &created, "nope.txt")
        .await
        .unwrap_err();
    assert!(
        matches!(&missing, ToolExecutionResult::ToolError(m) if m.contains("nope.txt")),
        "{missing:?}"
    );

    // Pause hibernates; status reads paused; resume wakes the same machine.
    let paused = provider.pause(&ctx, &cfg, &created).await.unwrap();
    assert_eq!(paused.metadata["remote_state"], "hibernated");
    assert_eq!(status_of(&mock, &id), "hibernated");
    let status = provider.status(&ctx, &cfg, &state(&paused)).await.unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Paused);
    let resumed = provider.resume(&ctx, &cfg, &paused).await.unwrap();
    assert_eq!(resumed.external_id, created.external_id);
    assert_eq!(status_of(&mock, &id), "running");
    // The file written before the pause is still there.
    assert_eq!(
        mock.file(&id, "/home/boxd/workspace/notes/a.txt").unwrap(),
        b"hello"
    );

    // A machine stopped by boxd is started, not woken.
    mock.force_status(&id, "stopped");
    provider.resume(&ctx, &cfg, &resumed).await.unwrap();
    assert_eq!(status_of(&mock, &id), "running");

    // Delete destroys it and releases the lease; a second delete is fine.
    provider.delete(&ctx, &cfg, &resumed).await.unwrap();
    assert!(mock.lock().destroyed.contains(&id));
    assert_eq!(
        lease_status(&leases, &created.external_id).await,
        Some(LeasedResourceStatus::Released)
    );
    provider.delete(&ctx, &cfg, &resumed).await.unwrap();
}

#[tokio::test]
async fn a_lost_or_failed_machine_is_replaced_on_resume() {
    let (mock, url) = start_mock().await;
    let (ctx, leases) = context_with_connections(&[("boxd", API_KEY)]);
    let cfg = config(&url, json!({}));
    let provider = BoxdSessionSandboxProvider;

    let first = provider.create(&ctx, &cfg).await.unwrap();
    // No region asked for: the org's default region.
    assert!(
        first.external_id.starts_with("fra/"),
        "{}",
        first.external_id
    );
    let first_id = machine_id(&first);

    mock.force_status(&first_id, "failed");
    let status = provider.status(&ctx, &cfg, &state(&first)).await.unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Lost);
    let second = provider.resume(&ctx, &cfg, &first).await.unwrap();
    assert_ne!(second.external_id, first.external_id);
    assert_eq!(second.metadata["replaced"], first.external_id);
    assert!(mock.lock().destroyed.contains(&first_id));
    assert_eq!(
        lease_status(&leases, &first.external_id).await,
        Some(LeasedResourceStatus::Released)
    );

    // Gone altogether: replaced too.
    mock.lock().machines.remove(&machine_id(&second));
    let status = provider.status(&ctx, &cfg, &state(&second)).await.unwrap();
    assert_eq!(status.session_status, SessionSandboxStatus::Lost);
    let third = provider.resume(&ctx, &cfg, &second).await.unwrap();
    assert_ne!(third.external_id, second.external_id);
    assert_eq!(mock.lock().machines.len(), 1);
}

#[tokio::test]
async fn a_snapshot_template_boots_from_the_snapshot() {
    let (mock, url) = start_mock().await;
    mock.lock().snapshots.insert("base".into(), "vm_0".into());
    let (ctx, _) = context_with_connections(&[("boxd", API_KEY)]);
    let cfg = config(
        &url,
        json!({"snapshot": "base", "workspace_path": "/srv/app"}),
    );
    let provider = BoxdSessionSandboxProvider;

    let created = provider.create(&ctx, &cfg).await.unwrap();
    assert_eq!(created.workspace_path.as_deref(), Some("/srv/app"));
    assert_eq!(created.metadata["restored_from_snapshot"], true);
    let state = mock.lock();
    assert!(state.create_requests.is_empty());
    assert_eq!(state.snapshot_creates[0].snapshot, "base");
    assert!(state.snapshot_creates[0].isolated);
}

#[tokio::test]
async fn exec_timeouts_and_large_output_come_back_bounded() {
    let (_mock, url) = start_mock().await;
    let (ctx, _) = context_with_connections(&[("boxd", API_KEY)]);
    let cfg = config(&url, json!({}));
    let provider = BoxdSessionSandboxProvider;
    let created = provider.create(&ctx, &cfg).await.unwrap();

    let mut slow = exec("sleep 600", None);
    slow.timeout_ms = Some(300);
    let out = provider.exec(&ctx, &cfg, &created, &slow).await.unwrap();
    assert_eq!(out.exit_code, 124);
    assert!(out.stderr.contains("timed out"), "{}", out.stderr);

    let out = provider
        .exec(&ctx, &cfg, &created, &exec("yes", None))
        .await
        .unwrap();
    assert!(out.truncated);
}

#[tokio::test]
async fn the_network_policy_reaches_boxd_and_is_reapplied_on_resume() {
    let (mock, url) = start_mock().await;
    let (ctx, _) = context_with_connections(&[("boxd", API_KEY)]);
    let provider = BoxdSessionSandboxProvider;

    let blocked = config(&url, json!({"network": {"mode": "blocked"}}));
    let created = provider.create(&ctx, &blocked).await.unwrap();
    let id = machine_id(&created);
    assert_eq!(mock.lock().machines[&id].egress_allow, vec!["none"]);

    // The template now allows two hosts: resume pushes the new list.
    let allowlist = config(
        &url,
        json!({"network": {"mode": "allowlist", "domains": ["github.com", "*.github.com"]}}),
    );
    provider.pause(&ctx, &allowlist, &created).await.unwrap();
    provider.resume(&ctx, &allowlist, &created).await.unwrap();
    assert_eq!(
        mock.lock().egress_calls.last().unwrap(),
        &(
            id.clone(),
            vec!["github.com".to_string(), "*.github.com".to_string()]
        )
    );

    // Open egress is an empty list; an unchanged policy sends nothing.
    let open = config(&url, json!({}));
    let calls = mock.lock().egress_calls.len();
    provider.resume(&ctx, &open, &created).await.unwrap();
    assert!(mock.lock().machines[&id].egress_allow.is_empty());
    provider.resume(&ctx, &open, &created).await.unwrap();
    assert_eq!(mock.lock().egress_calls.len(), calls + 1);

    // What boxd cannot express fails before any machine exists.
    let v6 = config(
        &url,
        json!({"network": {"mode": "allowlist", "domains": ["2001:db8::/32"]}}),
    );
    let before = mock.lock().machines.len();
    let err = provider.create(&ctx, &v6).await.unwrap_err();
    assert!(
        matches!(&err, ToolExecutionResult::ToolError(m) if m.contains("IPv6")),
        "{err:?}"
    );
    assert_eq!(mock.lock().machines.len(), before);
}

#[tokio::test]
async fn invalid_options_and_regions_are_refused_before_any_machine_exists() {
    let (mock, url) = start_mock().await;
    let (ctx, _) = context_with_connections(&[("boxd", API_KEY)]);
    let provider = BoxdSessionSandboxProvider;
    for (options, needle) in [
        (json!({"region": "lon"}), "region"),
        (json!({"size": "3x12"}), "size"),
        (
            json!({"snapshot": "base", "size": "2x8"}),
            "cannot be combined",
        ),
        (json!({"workspace_path": "/srv/../etc"}), "workspace_path"),
    ] {
        let err = provider
            .create(&ctx, &config(&url, options.clone()))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ToolExecutionResult::ToolError(m) if m.contains(needle)),
            "{options}: {err:?}"
        );
    }
    assert!(mock.lock().machines.is_empty());
}

#[tokio::test]
async fn a_missing_or_rejected_key_asks_for_a_connection() {
    let (mock, url) = start_mock().await;
    let provider = BoxdSessionSandboxProvider;

    let (ctx, _) = context_with_connections(&[]);
    let err = provider
        .create(&ctx, &config(&url, json!({})))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ToolExecutionResult::ConnectionRequired { .. }),
        "{err:?}"
    );

    let (ctx, _) = context_with_connections(&[("boxd", "bxd_wrong")]);
    let err = provider
        .create(&ctx, &config(&url, json!({})))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ToolExecutionResult::ToolError(m) if m.contains("rejected the API key")),
        "{err:?}"
    );
    assert!(mock.lock().machines.is_empty());
}

#[tokio::test]
async fn managed_machines_carry_the_session_labels() {
    let (mock, url) = start_mock().await;
    let (ctx, _) = context_with_connections(&[("boxd", API_KEY)]);
    let provider = BoxdSessionSandboxProvider;
    let created = provider
        .create(&ctx, &config(&url, json!({})))
        .await
        .unwrap();
    let labels = mock.lock().machines[&machine_id(&created)].labels.clone();
    assert_eq!(
        labels.get("everruns.session_id").map(String::as_str),
        Some(ctx.session_id.to_string().as_str()),
        "{labels:?}"
    );
}
