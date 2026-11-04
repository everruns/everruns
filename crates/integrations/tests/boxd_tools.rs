#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The `boxd_*` agent tools end to end against the in-process mock
//! (`mock_boxd`): create, exec, files, list, hibernate, wake, delete, and the
//! ownership check that keeps a session to its own machines.

use std::sync::Arc;

use everruns_contracts::runtime::leased_resource::LeasedResourceStatus;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use everruns_integrations::boxd::BoxdCapability;
use everruns_integrations::boxd::state::BoxdServerUrlOverride;
use serde_json::{Value, json};

mod common;
mod mock_boxd;

use common::{MockLeasedResourceStore, context_with_connections, ok, tool_error, tool_of};
use mock_boxd::{API_KEY, MockBoxd, start_mock};

async fn setup() -> (MockBoxd, ToolContext, Arc<MockLeasedResourceStore>) {
    let (mock, url) = start_mock().await;
    let (ctx, leases) = context_with_connections(&[("boxd", API_KEY)]);
    let ctx = ctx.with_extension(Arc::new(BoxdServerUrlOverride(url)));
    (mock, ctx, leases)
}

async fn call(context: &ToolContext, name: &str, args: Value) -> ToolExecutionResult {
    tool_of(&BoxdCapability, name)
        .execute_with_context(args, context)
        .await
}

#[tokio::test]
async fn create_exec_files_hibernate_wake_and_delete() {
    let (mock, ctx, leases) = setup().await;

    let created = ok(call(
        &ctx,
        "boxd_create_sandbox",
        json!({"title": "build box", "region": "nyc", "size": "2x8"}),
    )
    .await);
    let sandbox_id = created["sandbox_id"].as_str().unwrap().to_string();
    assert!(sandbox_id.starts_with("nyc/vm_"), "{sandbox_id}");
    assert_eq!(created["workspace_path"], "/home/boxd/workspace");
    assert!(created["url"].as_str().unwrap().starts_with("https://er-"));
    let id = sandbox_id.split_once('/').unwrap().1.to_string();
    assert_eq!(
        leases.resources.lock().await[0].status,
        LeasedResourceStatus::Active
    );

    let out = ok(call(
        &ctx,
        "boxd_exec",
        json!({"sandbox_id": sandbox_id, "command": "exit 2", "cwd": "app"}),
    )
    .await);
    assert_eq!(out["exit_code"], 2);
    assert_eq!(out["cwd"], "/home/boxd/workspace/app");
    assert!(out["hint"].is_string() || out["hint"].is_null());

    ok(call(
        &ctx,
        "boxd_write_file",
        json!({"sandbox_id": sandbox_id, "path": "hello.txt", "content": "hi there"}),
    )
    .await);
    assert_eq!(
        mock.file(&id, "/home/boxd/workspace/hello.txt").unwrap(),
        b"hi there"
    );
    let read = ok(call(
        &ctx,
        "boxd_read_file",
        json!({"sandbox_id": sandbox_id, "path": "hello.txt"}),
    )
    .await);
    assert!(read.to_string().contains("hi there"), "{read}");

    let listed = ok(call(&ctx, "boxd_list_sandboxes", json!({})).await);
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["sandboxes"][0]["title"], "build box");

    let hibernated = ok(call(
        &ctx,
        "boxd_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "hibernate"}),
    )
    .await);
    assert_eq!(hibernated["status"], "hibernated");
    let woken = ok(call(
        &ctx,
        "boxd_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "wake"}),
    )
    .await);
    assert_eq!(woken["status"], "running");
    // A machine boxd stopped is started instead of woken.
    mock.force_status(&id, "stopped");
    let started = ok(call(
        &ctx,
        "boxd_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "wake"}),
    )
    .await);
    assert_eq!(started["status"], "running");

    ok(call(
        &ctx,
        "boxd_manage_sandbox",
        json!({"sandbox_id": sandbox_id, "action": "delete"}),
    )
    .await);
    assert!(mock.lock().destroyed.contains(&id));
    assert_eq!(
        leases.resources.lock().await[0].status,
        LeasedResourceStatus::Released
    );
    let listed = ok(call(&ctx, "boxd_list_sandboxes", json!({})).await);
    assert_eq!(listed["count"], 0);
    let gone = tool_error(
        call(
            &ctx,
            "boxd_exec",
            json!({"sandbox_id": sandbox_id, "command": "ls"}),
        )
        .await,
    );
    assert!(gone.contains("not created by this session"), "{gone}");
}

#[tokio::test]
async fn a_machine_boxd_lost_is_explained() {
    let (mock, ctx, _) = setup().await;
    let created = ok(call(&ctx, "boxd_create_sandbox", json!({})).await);
    let sandbox_id = created["sandbox_id"].as_str().unwrap().to_string();
    // No region asked for: the org's default.
    assert!(sandbox_id.starts_with("fra/"), "{sandbox_id}");
    mock.lock().machines.clear();
    let err = tool_error(
        call(
            &ctx,
            "boxd_exec",
            json!({"sandbox_id": sandbox_id, "command": "ls"}),
        )
        .await,
    );
    assert!(err.contains("no longer exists"), "{err}");
}

#[tokio::test]
async fn a_session_cannot_reach_another_sessions_machine() {
    let (mock, ctx, _) = setup().await;
    let created = ok(call(&ctx, "boxd_create_sandbox", json!({})).await);
    let sandbox_id = created["sandbox_id"].as_str().unwrap().to_string();

    // Another session with its own stores, same boxd account.
    let (other, _) = context_with_connections(&[("boxd", API_KEY)]);
    let other = other.with_extension(Arc::new(BoxdServerUrlOverride(mock.lock().url.clone())));
    let err = tool_error(
        call(
            &other,
            "boxd_exec",
            json!({"sandbox_id": sandbox_id, "command": "ls"}),
        )
        .await,
    );
    assert!(err.contains("not created by this session"), "{err}");
    assert!(
        mock.lock()
            .commands
            .values()
            .all(|commands| commands.is_empty())
    );
}

#[tokio::test]
async fn bad_arguments_are_refused_before_boxd_is_called() {
    let (mock, ctx, _) = setup().await;
    for (args, needle) in [
        (json!({"region": "lon"}), "region"),
        (json!({"size": "3x12"}), "size"),
        (
            json!({"snapshot": "base", "size": "2x8"}),
            "cannot be combined",
        ),
    ] {
        let err = tool_error(call(&ctx, "boxd_create_sandbox", args.clone()).await);
        assert!(err.contains(needle), "{args}: {err}");
    }
    for sandbox_id in ["../x", "vm_1", "fra/"] {
        let err = tool_error(
            call(
                &ctx,
                "boxd_exec",
                json!({"sandbox_id": sandbox_id, "command": "ls"}),
            )
            .await,
        );
        assert!(err.contains("boxd_create_sandbox"), "{sandbox_id}: {err}");
    }
    let err = tool_error(
        call(
            &ctx,
            "boxd_manage_sandbox",
            json!({"sandbox_id": "fra/vm_1", "action": "reboot"}),
        )
        .await,
    );
    assert!(err.contains("Invalid action"), "{err}");
    assert!(mock.lock().machines.is_empty());
}

#[tokio::test]
async fn without_a_connection_the_tools_ask_for_one() {
    let (_mock, url) = start_mock().await;
    let (ctx, _) = context_with_connections(&[]);
    let ctx = ctx.with_extension(Arc::new(BoxdServerUrlOverride(url)));
    let result = call(&ctx, "boxd_create_sandbox", json!({})).await;
    assert!(
        matches!(&result, ToolExecutionResult::ConnectionRequired { provider, .. } if provider == "boxd"),
        "{result:?}"
    );
}
