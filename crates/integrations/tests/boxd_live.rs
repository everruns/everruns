#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live boxd API tests.
//!
//! Gated behind the `boxd-live-tests` feature and the `BOXD_API_KEY`
//! environment variable. A missing key panics rather than skips, so a CI live
//! job cannot report a false green (see `knowledge/integrations/integrations.md`).
//!
//! Run locally:
//!   doppler run -- cargo test -p everruns-integrations \
//!       --features boxd-live-tests --test boxd_live -- --test-threads=1
//!
//! Every machine is created with boxd's own delete timer as a backstop and
//! destroyed by a guard on drop, so a failing test does not leave one running.

#![cfg(feature = "boxd-live-tests")]

mod common;

use std::time::Duration;

use common::context_with_connections;
use everruns_contracts::connector::Connector;
use everruns_contracts::session_sandbox::{
    SessionSandboxConfig, SessionSandboxCredential, SessionSandboxCredentialSource,
    SessionSandboxExecRequest, SessionSandboxProvider,
};
use everruns_integrations::boxd::client::{BoxdClient, CreateMachineParams};
use everruns_integrations::boxd::{
    BoxdConnector, BoxdSessionSandboxProvider, MachineRef, machine_name,
};
use serde_json::json;

fn api_key() -> String {
    std::env::var("BOXD_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .expect("BOXD_API_KEY not set: live tests require a real key (fail-closed policy)")
}

/// Destroys the machine on drop, on success and panic paths alike.
struct MachineGuard(MachineRef);

impl Drop for MachineGuard {
    fn drop(&mut self) {
        let machine = self.0.clone();
        let _ = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("cleanup runtime");
            runtime.block_on(async {
                let client = BoxdClient::new(api_key()).expect("client");
                match client.destroy(&machine.region, &machine.id).await {
                    Ok(()) => eprintln!("[cleanup] destroyed {machine}"),
                    Err(e) => eprintln!("[cleanup] {machine}: {e}"),
                }
            });
        })
        .join();
    }
}

const BACKSTOP_SECS: u64 = 30 * 60;

async fn machine(client: &BoxdClient, egress: Vec<String>) -> MachineGuard {
    let region = client.resolve_region(None).await.unwrap();
    let created = client
        .create_machine(
            &region.id,
            &CreateMachineParams {
                name: machine_name(),
                size: Some("1x4".into()),
                snapshot: None,
                labels: [("everruns.live_test".to_string(), "true".to_string())].into(),
                egress,
                delete_after_secs: Some(BACKSTOP_SECS),
            },
        )
        .await
        .unwrap();
    assert_eq!(created.status, "running");
    MachineGuard(MachineRef {
        region: region.id,
        id: created.id,
    })
}

#[tokio::test]
async fn the_connection_validates_a_real_key_and_refuses_a_bad_one() {
    BoxdConnector.validate(&api_key()).await.unwrap();
    let invalid = BoxdConnector
        .validate("bxd_not_a_real_key")
        .await
        .unwrap_err();
    assert!(invalid.contains("rejected"), "{invalid}");
}

#[tokio::test]
async fn regions_list_the_enabled_ones_and_the_default_resolves() {
    let client = BoxdClient::new(api_key()).unwrap();
    let regions = client.regions().await.unwrap();
    assert!(regions.iter().any(|r| r.home), "{regions:?}");
    let default = client.resolve_region(None).await.unwrap();
    assert!(!default.api_endpoint.is_empty(), "{default:?}");
    eprintln!("regions: {regions:?}, default: {}", default.id);
}

#[tokio::test]
async fn a_machine_runs_commands_keeps_files_and_processes_across_hibernation() {
    let client = BoxdClient::new(api_key()).unwrap();
    let guard = machine(&client, Vec::new()).await;
    let MachineRef { region, id } = guard.0.clone();
    let exec = |command: &'static str| {
        let client = client.clone();
        let (region, id) = (region.clone(), id.clone());
        async move {
            client
                .exec(&region, &id, command, None, Duration::from_secs(120))
                .await
                .unwrap()
        }
    };

    let out = exec("echo hi && uname -s && docker --version").await;
    assert_eq!(out.exit_code, 0, "{out:?}");
    assert!(out.stdout.starts_with("hi\nLinux\n"), "{out:?}");
    let out = exec("echo err >&2; exit 7").await;
    assert_eq!(out.exit_code, 7);
    assert!(out.stderr.contains("err"), "{out:?}");

    // stdin reaches the command.
    let out = client
        .exec(
            &region,
            &id,
            "cat",
            Some(b"from stdin"),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert_eq!(out.stdout, "from stdin");

    // A timeout kills the command and reports 124.
    let out = client
        .exec(&region, &id, "sleep 600", None, Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(out.exit_code, 124, "{out:?}");

    // Files round-trip through the filesystem API, parents created.
    let path = "/home/boxd/workspace/live/a.bin";
    let data: Vec<u8> = (0..=255u8).cycle().take(3 * 1024 * 1024 + 7).collect();
    client.write_file(&region, &id, path, &data).await.unwrap();
    let back = client
        .read_file(&region, &id, path, 8 * 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(back, data);
    let missing = client
        .read_file(&region, &id, "/nope/missing", 1024)
        .await
        .unwrap_err();
    assert!(
        everruns_integrations::boxd::client::is_not_found(&missing),
        "{missing}"
    );

    // Hibernate keeps a running process; wake brings it back.
    exec("nohup sleep 3000 >/dev/null 2>&1 & echo started").await;
    client.hibernate(&region, &id).await.unwrap();
    let hibernated = client.get_machine(&region, &id).await.unwrap();
    assert!(
        matches!(hibernated.status.as_str(), "hibernated" | "suspended"),
        "{}",
        hibernated.status
    );
    client.wake(&region, &id).await.unwrap();
    client.wait_until_running(&region, &id).await.unwrap();
    let out = exec("pgrep -x sleep >/dev/null && echo alive").await;
    assert_eq!(out.stdout.trim(), "alive", "{out:?}");
    assert_eq!(
        client
            .read_file(&region, &id, path, 8 * 1024 * 1024)
            .await
            .unwrap()
            .len(),
        data.len()
    );

    // Labels come back, and a selector finds the machine.
    let listed = client
        .list_machines(&region, &[("everruns.live_test".into(), "true".into())])
        .await
        .unwrap();
    assert!(listed.iter().any(|m| m.id == id), "{listed:?}");
}

#[tokio::test]
async fn the_egress_allowlist_is_enforced_from_birth_and_on_change() {
    let client = BoxdClient::new(api_key()).unwrap();
    let guard = machine(&client, vec!["none".into()]).await;
    let MachineRef { region, id } = guard.0.clone();
    let curl = |url: &'static str| {
        let client = client.clone();
        let (region, id) = (region.clone(), id.clone());
        async move {
            client
                .exec(
                    &region,
                    &id,
                    &format!("curl -sS -o /dev/null -m 10 -w '%{{http_code}}' {url}"),
                    None,
                    Duration::from_secs(60),
                )
                .await
                .unwrap()
        }
    };

    let blocked = curl("https://example.com").await;
    assert_ne!(
        blocked.exit_code, 0,
        "blocked egress reached out: {blocked:?}"
    );

    client
        .set_egress(&region, &id, &["example.com".to_string()])
        .await
        .unwrap();
    let allowed = curl("https://example.com").await;
    assert_eq!(allowed.exit_code, 0, "{allowed:?}");
    let other = curl("https://www.wikipedia.org").await;
    assert_ne!(other.exit_code, 0, "unlisted host reached: {other:?}");
}

#[tokio::test]
async fn the_session_sandbox_provider_runs_a_full_lifecycle() {
    let (ctx, _) = context_with_connections(&[("boxd", &api_key())]);
    let cfg = SessionSandboxConfig {
        provider: "boxd".into(),
        credential: SessionSandboxCredential {
            source: SessionSandboxCredentialSource::SessionUser,
            virtual_user_id: Some(uuid::Uuid::nil()),
            connection_id: None,
        },
        provider_config: json!({"size": "1x4"}),
        ..Default::default()
    };
    let provider = BoxdSessionSandboxProvider;
    let created = provider.create(&ctx, &cfg).await.unwrap();
    let _guard = MachineGuard(MachineRef::parse(&created.external_id).unwrap());

    let request = SessionSandboxExecRequest {
        command: "pwd && echo ok > marker".into(),
        cwd: None,
        timeout_ms: Some(60_000),
        output_mode: "full".into(),
    };
    let out = provider.exec(&ctx, &cfg, &created, &request).await.unwrap();
    assert!(out.success, "{out:?}");
    assert!(out.stdout.starts_with("/home/boxd/workspace"), "{out:?}");

    let paused = provider.pause(&ctx, &cfg, &created).await.unwrap();
    let resumed = provider.resume(&ctx, &cfg, &paused).await.unwrap();
    assert_eq!(resumed.external_id, created.external_id);
    let read = provider
        .read_file(&ctx, &cfg, &resumed, "marker")
        .await
        .unwrap();
    assert_eq!(read.content.trim(), "ok");

    provider.delete(&ctx, &cfg, &resumed).await.unwrap();
}
