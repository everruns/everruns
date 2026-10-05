#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Modal tools end to end against an in-process mock of Modal's gRPC API
//! (`mock_modal`).

use std::sync::Arc;

use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::leased_resource::LeasedResourceStatus;
use everruns_contracts::runtime::session_services::SessionStorageStore;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use everruns_integrations::modal::ModalCapability;
use everruns_integrations::modal::state::ModalServerUrlOverride;
use serde_json::{Value, json};

mod common;
mod mock_modal;

use common::{MockLeasedResourceStore, MockStorageStore, call as call_tool, ok, tool_error};
use mock_modal::{MockModal, TOKEN_ID, TOKEN_SECRET, start_mock};

async fn call(h: &Fixture, name: &str, args: Value) -> ToolExecutionResult {
    call_tool(&h.context, name, args).await
}

struct Fixture {
    mock: MockModal,
    storage: Arc<MockStorageStore>,
    leases: Arc<MockLeasedResourceStore>,
    context: ToolContext,
}

async fn harness_with_token(token: Option<&str>) -> Fixture {
    let (mock, url) = start_mock().await;
    let (context, storage, leases) = common::context(token);
    let context = context.with_extension(Arc::new(ModalServerUrlOverride(url)));
    Fixture {
        mock,
        storage,
        leases,
        context,
    }
}

async fn harness() -> Fixture {
    harness_with_token(Some(&format!("{TOKEN_ID}:{TOKEN_SECRET}"))).await
}

async fn create(h: &Fixture, args: Value) -> String {
    let created = ok(call(h, "modal_create_sandbox", args).await);
    created["sandbox_id"].as_str().unwrap().to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn full_lifecycle_runs_through_the_wire() {
    let h = harness().await;

    let created = ok(call(
        &h,
        "modal_create_sandbox",
        json!({"title": "demo", "expose_ports": [8080, 8080, 3000], "cpu": 2, "memory_mb": 4096}),
    )
    .await);
    let sandbox_id = created["sandbox_id"].as_str().unwrap().to_string();
    assert_eq!(created["runtime"], "vm");
    assert_eq!(created["workspace_path"], "/workspace");
    assert_eq!(created["tunnels"].as_array().unwrap().len(), 2);

    {
        let state = h.mock.lock();
        // The default image adds git and curl on top of the base image.
        assert_eq!(state.image_builds.len(), 1);
        assert_eq!(state.image_builds[0][0], "FROM python:3.13-slim");
        assert!(state.image_builds[0][1].contains("git curl"));
        let definition = &state.sandboxes[&sandbox_id];
        assert_eq!(definition.runtime.as_deref(), Some("vm"));
        assert_eq!(definition.image_id, "im-built");
        assert_eq!(definition.timeout_secs, 3600);
        assert_eq!(definition.entrypoint_args, ["sleep", "infinity"]);
        let resources = definition.resources.unwrap();
        assert_eq!((resources.milli_cpu, resources.memory_mb), (2000, 4096));
    }
    let lease = h.leases.resources.lock().await[0].clone();
    assert_eq!(
        (lease.provider.as_str(), lease.resource_type.as_str()),
        ("modal", "sandbox")
    );
    assert_eq!(lease.external_id, sandbox_id);
    assert_eq!(lease.display_name.as_deref(), Some("demo"));

    let exec = ok(call(
        &h,
        "modal_exec",
        json!({"sandbox_id": sandbox_id, "command": "echo hi", "cwd": "src", "env": {"GREETING": "hello"}}),
    )
    .await);
    let stdout = exec["stdout"].as_str().unwrap();
    assert!(stdout.contains("ran: echo hi"), "{stdout}");
    assert!(stdout.contains("cwd: /workspace/src"), "{stdout}");
    assert!(stdout.contains("GREETING=hello"), "{stdout}");
    assert_eq!(exec["exit_code"], 0);

    let failed = ok(call(
        &h,
        "modal_exec",
        json!({"sandbox_id": sandbox_id, "command": "exit 3"}),
    )
    .await);
    assert_eq!(failed["exit_code"], 3);
    assert_eq!(failed["success"], false);
    assert!(failed["stderr"].as_str().unwrap().contains("boom"));

    // 3 MiB crosses the 1 MiB stdin chunk size, so offsets must line up.
    let big = "x".repeat(3 * 1024 * 1024 + 7);
    let written = ok(call(
        &h,
        "modal_write_file",
        json!({"sandbox_id": sandbox_id, "path": "data/big.txt", "content": big}),
    )
    .await);
    assert_eq!(written["path"], "/workspace/data/big.txt");
    assert_eq!(
        h.mock.lock().files["/workspace/data/big.txt"].len(),
        big.len()
    );

    ok(call(
        &h,
        "modal_write_file",
        json!({"sandbox_id": sandbox_id, "path": "/tmp/notes.txt", "content": "one\ntwo\nthree\n"}),
    )
    .await);
    let read = ok(call(
        &h,
        "modal_read_file",
        json!({"sandbox_id": sandbox_id, "path": "/tmp/notes.txt", "offset": 1, "limit": 1}),
    )
    .await);
    let rendered = read.to_string();
    assert!(rendered.contains("two"), "{rendered}");
    assert!(!rendered.contains("three"), "{rendered}");
    assert_eq!(read["sandbox_id"], sandbox_id);

    let missing = tool_error(
        call(
            &h,
            "modal_read_file",
            json!({"sandbox_id": sandbox_id, "path": "/nope"}),
        )
        .await,
    );
    assert!(missing.contains("No such file"), "{missing}");

    let listed = ok(call(&h, "modal_list_sandboxes", json!({})).await);
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["sandboxes"][0]["exposed_ports"], json!([3000, 8080]));

    let snapshot = ok(call(
        &h,
        "modal_snapshot_sandbox",
        json!({"sandbox_id": sandbox_id}),
    )
    .await);
    assert_eq!(snapshot["snapshot_image_id"], "im-snapshot");

    let tunnels = ok(call(&h, "modal_tunnel_urls", json!({"sandbox_id": sandbox_id})).await);
    assert_eq!(
        tunnels["tunnels"][1]["url"],
        format!("https://{sandbox_id}-8080.modal.host")
    );

    let status = ok(call(
        &h,
        "modal_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "status"}),
    )
    .await);
    assert_eq!(status["status"], "running");

    ok(call(
        &h,
        "modal_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "terminate"}),
    )
    .await);
    assert_eq!(h.mock.lock().terminated, std::slice::from_ref(&sandbox_id));
    assert!(h.storage.secrets.lock().await.is_empty());
    assert_eq!(
        h.leases.resources.lock().await[0].status,
        LeasedResourceStatus::Released
    );
    let gone = tool_error(
        call(
            &h,
            "modal_exec",
            json!({"sandbox_id": sandbox_id, "command": "ls"}),
        )
        .await,
    );
    // The released lease is what refuses it, before any call reaches Modal.
    assert!(gone.contains("not created by this session"), "{gone}");
}

#[tokio::test]
async fn custom_images_and_snapshots_boot_what_was_asked() {
    let h = harness().await;
    create(
        &h,
        json!({"image": "node:22", "setup_commands": ["RUN npm i -g pnpm"], "runtime": "gvisor"}),
    )
    .await;
    create(
        &h,
        json!({"snapshot_image_id": "im-snapshot", "timeout_seconds": 600}),
    )
    .await;

    let state = h.mock.lock();
    assert_eq!(
        state.image_builds,
        [vec![
            "FROM node:22".to_string(),
            "RUN npm i -g pnpm".to_string()
        ]]
    );
    let gvisor = &state.sandboxes["sb-mock1"];
    assert_eq!(gvisor.runtime.as_deref(), Some("gvisor"));
    let restored = &state.sandboxes["sb-mock2"];
    assert_eq!(restored.image_id, "im-snapshot");
    assert_eq!(restored.timeout_secs, 600);
}

#[tokio::test]
async fn a_finished_sandbox_is_reported_as_such() {
    let h = harness().await;
    let sandbox_id = create(&h, json!({})).await;
    // Modal ended it (timeout, idle timeout, or outside termination).
    h.mock.lock().terminated.push(sandbox_id.clone());
    let message = tool_error(
        call(
            &h,
            "modal_exec",
            json!({"sandbox_id": sandbox_id, "command": "ls"}),
        )
        .await,
    );
    assert!(message.contains("no longer running"), "{message}");
    assert!(message.contains("terminated"), "{message}");
}

#[tokio::test]
async fn a_missing_connection_asks_for_one() {
    let h = harness_with_token(None).await;
    match call(&h, "modal_create_sandbox", json!({})).await {
        ToolExecutionResult::ConnectionRequired { provider, .. } => assert_eq!(provider, "modal"),
        other => panic!("expected ConnectionRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn rejected_credentials_say_so() {
    let h = harness_with_token(Some("ak-test:as-wrong")).await;
    let message = tool_error(call(&h, "modal_create_sandbox", json!({})).await);
    assert!(message.contains("rejected the credentials"), "{message}");
    let malformed = harness_with_token(Some("only-one-part")).await;
    let message = tool_error(call(&malformed, "modal_create_sandbox", json!({})).await);
    assert!(message.contains("not a valid token pair"), "{message}");
}

#[tokio::test]
async fn forged_state_without_a_lease_is_refused() {
    let h = harness().await;
    // A real sandbox gives the session lease tracking...
    create(&h, json!({})).await;
    // ...then a state record for someone else's sandbox is planted directly.
    let forged = json!({
        "sandbox_id": "sb-victim", "task_id": "ta-victim", "app_id": "ap-x",
        "runtime": "vm", "image": "x", "workspace_path": "/workspace",
        "started_at": "2026-10-05T00:00:00Z", "timeout_seconds": 60
    });
    h.storage
        .set_secret(
            h.context.session_id,
            "modal_sandbox:sb-victim",
            &forged.to_string(),
        )
        .await
        .unwrap();
    let result = call(
        &h,
        "modal_exec",
        json!({"sandbox_id": "sb-victim", "command": "id"}),
    )
    .await;
    assert!(
        !matches!(result, ToolExecutionResult::Success(_)),
        "forged sandbox was used: {result:?}"
    );
    assert!(
        h.mock
            .lock()
            .execs
            .values()
            .all(|(argv, _, _)| argv != &["sh", "-c", "id"])
    );
}

#[tokio::test]
async fn invalid_arguments_are_rejected_before_any_call() {
    let h = harness().await;
    for (args, needle) in [
        (json!({"runtime": "firecracker"}), "runtime"),
        (json!({"timeout_seconds": 10}), "timeout_seconds"),
        (json!({"cpu": 1000}), "cpu"),
        (json!({"expose_ports": [0]}), "expose_ports"),
        (json!({"image": "bad image"}), "image"),
        (
            json!({"snapshot_image_id": "im-x", "image": "node:22"}),
            "cannot be combined",
        ),
        (json!({"snapshot_image_id": "sb-x"}), "snapshot_image_id"),
        (
            json!({"setup_commands": ["RUN a\nRUN b"]}),
            "setup_commands",
        ),
    ] {
        let message = tool_error(call(&h, "modal_create_sandbox", args.clone()).await);
        assert!(message.contains(needle), "{args}: {message}");
    }
    let message = tool_error(
        call(
            &h,
            "modal_exec",
            json!({"sandbox_id": "../etc", "command": "ls"}),
        )
        .await,
    );
    assert!(message.contains("Invalid Modal sandbox ID"), "{message}");
    let message = tool_error(call(&h, "modal_exec", json!({"sandbox_id": "sb-1"})).await);
    assert!(
        message.contains("Missing required parameter: command"),
        "{message}"
    );
    let message = tool_error(
        call(
            &h,
            "modal_manage_sandbox",
            json!({"sandbox_id": "sb-1", "action": "pause"}),
        )
        .await,
    );
    assert!(message.contains("Invalid action"), "{message}");
    assert!(h.mock.lock().sandboxes.is_empty());
}

#[tokio::test]
async fn tools_without_context_refuse() {
    for tool in ModalCapability.tools() {
        match tool.execute(json!({})).await {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("requires context"), "{message}")
            }
            other => panic!("{}: expected ToolError, got {other:?}", tool.name()),
        }
    }
}
