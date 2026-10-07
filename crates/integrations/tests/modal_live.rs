#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live Modal API tests.
//!
//! Gated behind the `modal-live-tests` feature and the `MODAL_TOKEN_ID` /
//! `MODAL_TOKEN_SECRET` environment variables. Missing credentials panic
//! rather than skip, so a CI live job cannot report a false green (see
//! `knowledge/integrations/integrations.md`).
//!
//! Run locally:
//!   doppler run -- cargo test -p everruns-integrations \
//!       --features modal-live-tests --test modal_live -- --test-threads=1
//!
//! Every sandbox is created with a short Modal timeout and terminated by a
//! guard on drop, so a failing test still does not leave one running.

#![cfg(feature = "modal-live-tests")]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::{call, ok, tool_error};
use everruns_contracts::connector::Connector;
use everruns_contracts::session_sandbox::{
    SessionSandboxCredential, SessionSandboxCredentialSource,
};
use everruns_integrations::modal::client::{CreateSandboxParams, ModalClient, ModalCredentials};
use everruns_integrations::modal::{MODAL_APP_NAME, ModalConnector};
use serde_json::json;

fn credentials() -> ModalCredentials {
    let get = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| {
                panic!("{name} not set: live tests require real credentials (fail-closed policy)")
            })
    };
    ModalCredentials {
        token_id: get("MODAL_TOKEN_ID"),
        token_secret: get("MODAL_TOKEN_SECRET"),
    }
}

fn live_provider_credential() -> SessionSandboxCredential {
    SessionSandboxCredential {
        source: SessionSandboxCredentialSource::SessionUser,
        virtual_user_id: Some(uuid::Uuid::nil()),
        connection_id: None,
    }
}

/// Terminates the sandbox on drop, on success and panic paths alike.
struct SandboxGuard(String);

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        let sandbox_id = self.0.clone();
        let _ = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("cleanup runtime");
            runtime.block_on(async {
                let client = ModalClient::new(credentials()).expect("client");
                match client.terminate(&sandbox_id).await {
                    Ok(()) => eprintln!("[cleanup] terminated {sandbox_id}"),
                    Err(e) => eprintln!("[cleanup] failed to terminate {sandbox_id}: {e}"),
                }
            });
        })
        .join();
    }
}

async fn sandbox(client: &ModalClient, runtime: &str) -> (SandboxGuard, String) {
    let app_id = client.get_or_create_app(MODAL_APP_NAME).await.unwrap();
    let image_id = client
        .build_image(&app_id, "python:3.13-slim", &[])
        .await
        .unwrap();
    let (sandbox_id, task_id) = client
        .create_sandbox(
            &app_id,
            &CreateSandboxParams {
                image_id,
                runtime: Some(runtime.to_string()),
                timeout_secs: 300,
                tags: vec![("everruns.test".into(), "modal_live".into())],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (SandboxGuard(sandbox_id), task_id)
}

fn argv(command: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), command.into()]
}

#[tokio::test]
async fn connector_accepts_the_real_pair_and_rejects_a_wrong_secret() {
    let creds = credentials();
    ModalConnector
        .validate(&creds.to_connection_string())
        .await
        .unwrap();
    let wrong = format!("{}:as-wrongwrongwrong", creds.token_id);
    let err = ModalConnector.validate(&wrong).await.unwrap_err();
    assert!(err.contains("rejected the credentials"), "{err}");
}

#[tokio::test]
async fn vm_runtime_runs_its_own_kernel_and_streams_io() {
    let client = ModalClient::new(credentials()).unwrap();
    let (guard, task_id) = sandbox(&client, "vm").await;

    let uname = client
        .exec(
            &task_id,
            &argv("uname -r"),
            None,
            &HashMap::new(),
            Duration::from_secs(60),
            None,
        )
        .await
        .unwrap();
    assert_eq!(uname.exit_code, 0, "{uname:?}");
    // gVisor reports a fixed fake 4.4.0 kernel; a VM boots a real one.
    assert!(!uname.stdout.starts_with("4.4.0"), "{uname:?}");

    let piped = client
        .exec(
            &task_id,
            &argv("cat; echo to-stderr >&2; exit 7"),
            Some("/tmp"),
            &HashMap::from([("X".to_string(), "1".to_string())]),
            Duration::from_secs(60),
            Some(b"from stdin\n"),
        )
        .await
        .unwrap();
    assert_eq!(piped.stdout, "from stdin\n");
    assert_eq!(piped.stderr, "to-stderr\n");
    assert_eq!(piped.exit_code, 7);

    let env = client
        .exec(
            &task_id,
            &argv("pwd; echo $X"),
            Some("/tmp"),
            &HashMap::from([("X".to_string(), "42".to_string())]),
            Duration::from_secs(60),
            None,
        )
        .await
        .unwrap();
    assert_eq!(env.stdout, "/tmp\n42\n");

    // A command that outlives its timeout is killed, not waited on forever.
    let started = std::time::Instant::now();
    let slow = client
        .exec(
            &task_id,
            &argv("sleep 30"),
            None,
            &HashMap::new(),
            Duration::from_secs(2),
            None,
        )
        .await;
    assert!(started.elapsed() < Duration::from_secs(25), "{slow:?}");
    if let Ok(output) = slow {
        assert_ne!(output.exit_code, 0, "{output:?}");
    }

    let image_id = client.snapshot_filesystem(&guard.0).await.unwrap();
    assert!(image_id.starts_with("im-"), "{image_id}");
    drop(guard);
}

#[tokio::test]
async fn gvisor_runtime_still_works() {
    let client = ModalClient::new(credentials()).unwrap();
    let (_guard, task_id) = sandbox(&client, "gvisor").await;
    let output = client
        .exec(
            &task_id,
            &argv("echo ok"),
            None,
            &HashMap::new(),
            Duration::from_secs(60),
            None,
        )
        .await
        .unwrap();
    assert_eq!((output.stdout.as_str(), output.exit_code), ("ok\n", 0));
}

#[tokio::test]
async fn tools_drive_a_vm_sandbox_end_to_end() {
    let creds = credentials();
    let (context, storage, _leases) = common::context(Some(&creds.to_connection_string()));

    let created = ok(call(
        &context,
        "modal_create_sandbox",
        json!({"title": "everruns live test", "timeout_seconds": 600, "expose_ports": [8080]}),
    )
    .await);
    let sandbox_id = created["sandbox_id"].as_str().unwrap().to_string();
    let guard = SandboxGuard(sandbox_id.clone());
    assert_eq!(created["runtime"], "vm");
    let tunnel = created["tunnels"][0]["url"].as_str().unwrap();
    assert!(tunnel.starts_with("https://"), "{tunnel}");

    // The default image carries git and curl, and commands start in /workspace.
    let exec = ok(call(
        &context,
        "modal_exec",
        json!({"sandbox_id": sandbox_id, "command": "pwd && git --version && curl --version | head -1"}),
    )
    .await);
    assert_eq!(exec["exit_code"], 0, "{exec}");
    assert!(
        exec["stdout"].as_str().unwrap().starts_with("/workspace\n"),
        "{exec}"
    );

    ok(call(
        &context,
        "modal_write_file",
        json!({"sandbox_id": sandbox_id, "path": "nested/dir/hello.py", "content": "print('hello from modal')\n"}),
    )
    .await);
    let run = ok(call(
        &context,
        "modal_exec",
        json!({"sandbox_id": sandbox_id, "command": "python3 nested/dir/hello.py"}),
    )
    .await);
    assert_eq!(run["stdout"].as_str().unwrap().trim(), "hello from modal");
    let read = ok(call(
        &context,
        "modal_read_file",
        json!({"sandbox_id": sandbox_id, "path": "/workspace/nested/dir/hello.py"}),
    )
    .await);
    assert!(read.to_string().contains("hello from modal"), "{read}");

    let missing = tool_error(
        call(
            &context,
            "modal_read_file",
            json!({"sandbox_id": sandbox_id, "path": "/no/such/file"}),
        )
        .await,
    );
    assert!(missing.contains("No such file"), "{missing}");

    let snapshot = ok(call(
        &context,
        "modal_snapshot_sandbox",
        json!({"sandbox_id": sandbox_id}),
    )
    .await);
    let image_id = snapshot["snapshot_image_id"].as_str().unwrap().to_string();

    // A sandbox booted from the snapshot sees the file.
    let restored = ok(call(
        &context,
        "modal_create_sandbox",
        json!({"snapshot_image_id": image_id, "timeout_seconds": 300}),
    )
    .await);
    let restored_id = restored["sandbox_id"].as_str().unwrap().to_string();
    let restored_guard = SandboxGuard(restored_id.clone());
    let cat = ok(call(
        &context,
        "modal_exec",
        json!({"sandbox_id": restored_id, "command": "cat /workspace/nested/dir/hello.py"}),
    )
    .await);
    assert!(
        cat["stdout"].as_str().unwrap().contains("hello from modal"),
        "{cat}"
    );

    let listed = ok(call(&context, "modal_list_sandboxes", json!({})).await);
    assert_eq!(listed["count"], 2);

    for id in [&restored_id, &sandbox_id] {
        let terminated = ok(call(
            &context,
            "modal_manage_sandbox",
            json!({"sandbox_id": id, "action": "terminate"}),
        )
        .await);
        assert_eq!(terminated["success"], true);
    }
    assert!(storage.secrets.lock().await.is_empty());
    drop(restored_guard);
    drop(guard);
}

#[tokio::test]
async fn terminating_is_idempotent_and_unknown_ids_are_not_found() {
    let client = ModalClient::new(credentials()).unwrap();
    let (guard, _task_id) = sandbox(&client, "gvisor").await;
    client.terminate(&guard.0).await.unwrap();
    // Cleanup may run after the user already terminated it.
    client.terminate(&guard.0).await.unwrap();
    let err = client
        .terminate("sb-0000000000000000000000")
        .await
        .unwrap_err();
    // The worker's lease cleanup treats this wording as "already gone".
    assert!(err.contains("not found"), "{err}");
}

#[tokio::test]
async fn managed_provider_pauses_to_a_snapshot_and_resumes_with_files_intact() {
    use everruns_contracts::session_sandbox::{
        SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxProvider,
    };
    use everruns_integrations::modal::ModalSessionSandboxProvider;

    let creds = credentials();
    let token = format!("{}:{}", creds.token_id, creds.token_secret);
    let (ctx, _, leases) = common::context(Some(&token));
    let config = SessionSandboxConfig {
        provider: "modal".into(),
        credential: live_provider_credential(),
        provider_config: json!({"runtime": "vm"}),
        ..Default::default()
    };
    let provider = ModalSessionSandboxProvider;
    let exec = |command: &str| SessionSandboxExecRequest {
        command: command.into(),
        cwd: None,
        timeout_ms: Some(60_000),
        output_mode: "full".into(),
    };

    let created = provider.create(&ctx, &config).await.unwrap();
    let _first = SandboxGuard(created.external_id.clone());
    provider
        .write_file(&ctx, &config, &created, "keep.txt", b"survives pause")
        .await
        .unwrap();

    let paused = provider.pause(&ctx, &config, &created).await.unwrap();
    assert_eq!(paused.metadata["remote_state"], "paused");
    assert!(paused.provider_state["snapshot_image_id"].is_string());

    let resumed = provider.resume(&ctx, &config, &paused).await.unwrap();
    let _second = SandboxGuard(resumed.external_id.clone());
    assert_ne!(resumed.external_id, created.external_id);
    let read = provider
        .read_file(&ctx, &config, &resumed, "keep.txt")
        .await
        .unwrap();
    assert_eq!(read.content, "survives pause");
    let out = provider
        .exec(&ctx, &config, &resumed, &exec("pwd && uname -r"))
        .await
        .unwrap();
    assert!(out.success, "{}", out.stderr);
    assert!(out.stdout.starts_with("/workspace\n"), "{}", out.stdout);

    provider.delete(&ctx, &config, &resumed).await.unwrap();
    let active = leases
        .resources
        .lock()
        .await
        .iter()
        .filter(|r| {
            r.status == everruns_contracts::runtime::leased_resource::LeasedResourceStatus::Active
        })
        .count();
    assert_eq!(active, 0, "every lease is released");
}

/// Boots a managed sandbox with `provider_config` and runs `script` in it.
async fn managed_run(
    connections: &[(&str, &str)],
    provider_config: serde_json::Value,
    script: &str,
) -> everruns_contracts::session_sandbox::SessionSandboxExecResponse {
    use everruns_contracts::session_sandbox::{
        SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxProvider,
    };
    use everruns_integrations::modal::ModalSessionSandboxProvider;

    let creds = credentials();
    let token = format!("{}:{}", creds.token_id, creds.token_secret);
    let mut all = vec![("modal", token.as_str())];
    all.extend_from_slice(connections);
    let (ctx, _) = common::context_with_connections(&all);
    let config = SessionSandboxConfig {
        provider: "modal".into(),
        credential: live_provider_credential(),
        provider_config,
        ..Default::default()
    };
    let provider = ModalSessionSandboxProvider;
    let created = provider.create(&ctx, &config).await.unwrap();
    let _guard = SandboxGuard(created.external_id.clone());
    let out = provider
        .exec(
            &ctx,
            &config,
            &created,
            &SessionSandboxExecRequest {
                command: script.into(),
                cwd: None,
                timeout_ms: Some(90_000),
                output_mode: "full".into(),
            },
        )
        .await
        .unwrap();
    provider.delete(&ctx, &config, &created).await.unwrap();
    out
}

#[tokio::test]
async fn blocked_and_allowlisted_egress_is_enforced_by_modal() {
    let probe = "for u in https://example.com https://www.google.com; do \
                 if curl -s -o /dev/null -m 15 \"$u\"; then echo \"$u ok\"; else echo \"$u blocked\"; fi; done";

    let blocked = managed_run(&[], json!({"network": {"mode": "blocked"}}), probe).await;
    assert!(
        blocked.stdout.contains("https://example.com blocked"),
        "{}",
        blocked.stdout
    );

    let allowlisted = managed_run(
        &[],
        json!({"network": {"mode": "allowlist", "domains": ["example.com"]}}),
        probe,
    )
    .await;
    assert!(
        allowlisted.stdout.contains("https://example.com ok"),
        "{}",
        allowlisted.stdout
    );
    assert!(
        allowlisted
            .stdout
            .contains("https://www.google.com blocked"),
        "{}",
        allowlisted.stdout
    );
}

#[tokio::test]
async fn github_token_is_injected_but_never_visible_inside() {
    // A made-up token keeps this test free of a real GitHub credential:
    // GitHub answers "Requires authentication" when no Authorization header
    // arrives and "Bad credentials" when one does, so the second proves Modal
    // injected it. (The byte-exact header value is covered by a mock test.)
    let fake = "ghp_everrunsModalInjectionProbe000000000";
    let out = managed_run(
        &[("github", fake)],
        json!({"inject_connections": ["github"]}),
        &format!(
            "curl -s https://api.github.com/user; echo; \
             if env | grep -q -F -- '{fake}' || grep -rqs -F -- '{fake}' /etc /root /workspace; \
             then echo leaked; else echo clean; fi"
        ),
    )
    .await;
    assert!(
        out.stdout.contains("Bad credentials"),
        "{} {}",
        out.stdout,
        out.stderr
    );
    assert!(out.stdout.trim_end().ends_with("clean"), "{}", out.stdout);

    let without = managed_run(&[], json!({}), "curl -s https://api.github.com/user").await;
    assert!(
        without.stdout.contains("Requires authentication"),
        "{}",
        without.stdout
    );
}
