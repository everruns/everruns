//! Runtime-owned artifacts under a model-facing workspace policy.
//!
//! Delegation writes run records (`/.agent-runs`) and structured task results
//! (`/.tasks`) on the session's behalf. These tests pin that a restrictive
//! policy keeps applying to the model-facing store while those writes go
//! through the runtime's confined artifact store.

use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::typed_id::{HarnessId, SessionId};
use everruns_core::host::{
    HarnessBuilder, HostComposition, RuntimeHostAdapter, SessionBuilder, ToolContextRequest,
};
use everruns_core::session_files::RuntimeArtifactFileSystem;
use everruns_core::{CapabilityRegistry, ExecutionSession, ToolContext, WorkspacePolicy};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use std::sync::Arc;

fn platform() -> HostComposition {
    HostComposition::new(CapabilityRegistry::new(), DriverRegistry::new())
}

fn harness(harness_id: HarnessId) -> everruns_core::host::SeededHarness {
    HarnessBuilder::new("plain", "You are terse.")
        .id(harness_id)
        .build()
}

fn session(session_id: SessionId, harness_id: HarnessId) -> ExecutionSession {
    SessionBuilder::new(harness_id).id(session_id).build()
}

fn extensions(
    runtime: &everruns_core::host::InProcessRuntime,
    session_id: SessionId,
) -> everruns_core::tool_context::ToolContextExtensions {
    runtime.tool_context_extensions(ToolContextRequest {
        org_id: 0,
        session_id,
        resolved_capabilities: &[],
    })
}

/// Delegation records its runs under `/.agent-runs` and structured results
/// under `/.tasks`. Under the read-only default policy those writes must still
/// land, through the runtime's artifact store, while the model-facing store
/// keeps denying them and the artifact store reaches nothing else.
#[tokio::test]
async fn read_only_policy_keeps_runtime_artifact_writes_off_the_model_store() {
    let harness_id = "harness_00000000000000000000000000000055".parse().unwrap();
    let session_id = "session_00000000000000000000000000000055".parse().unwrap();
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform())
        .workspace_policy(WorkspacePolicy::read_only())
        .llm_sim_as_default(LlmSimConfig::fixed("ok"))
        .harness(harness(harness_id))
        .session(session(session_id, harness_id))
        .build()
        .await
        .unwrap();

    let mut context = ToolContext::new(session_id);
    context.file_store = Some(runtime.file_store(0));
    context.extensions = extensions(&runtime, session_id);
    let model_store = context.file_store.clone().unwrap();
    let artifacts = context.runtime_artifact_file_store().unwrap();

    for path in [
        "/.agent-runs/run_1/result.json",
        "/.tasks/task_1/result.json",
    ] {
        assert!(
            model_store
                .write_file(session_id, path, "{}", "utf-8")
                .await
                .is_err(),
            "the model-facing store must still deny `{path}`"
        );
        artifacts
            .write_file(session_id, path, "{}", "utf-8")
            .await
            .unwrap_or_else(|error| panic!("runtime artifact write to `{path}` failed: {error}"));
        assert!(
            artifacts
                .read_file(session_id, path)
                .await
                .unwrap()
                .is_some(),
            "runtime must read back `{path}`"
        );
    }
    for path in ["/notes.md", "/.agents/AGENTS.md", "/.env", "/.ssh/id_rsa"] {
        assert!(
            artifacts
                .write_file(session_id, path, "x", "utf-8")
                .await
                .is_err(),
            "the runtime artifact store must stay confined, but wrote `{path}`"
        );
        assert!(
            model_store
                .write_file(session_id, path, "x", "utf-8")
                .await
                .is_err(),
            "the read-only default must deny `{path}`"
        );
    }
}

/// Without a workspace policy there is nothing to route around: no artifact
/// store is installed and runtime artifacts use the ordinary store.
#[tokio::test]
async fn no_policy_installs_no_runtime_artifact_store() {
    let harness_id = "harness_00000000000000000000000000000056".parse().unwrap();
    let session_id = "session_00000000000000000000000000000056".parse().unwrap();
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform())
        .llm_sim_as_default(LlmSimConfig::fixed("ok"))
        .harness(harness(harness_id))
        .session(session(session_id, harness_id))
        .build()
        .await
        .unwrap();

    assert!(
        extensions(&runtime, session_id)
            .get::<RuntimeArtifactFileSystem>()
            .is_none()
    );
}

struct EmbedderMarker;

/// Installing the artifact store must not displace extensions the embedder
/// supplied through its own factory.
#[tokio::test]
async fn policy_keeps_the_embedders_tool_context_extensions() {
    let harness_id = "harness_00000000000000000000000000000057".parse().unwrap();
    let session_id = "session_00000000000000000000000000000057".parse().unwrap();
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(platform())
        .with_tool_context_extensions_factory(Arc::new(|_, _| {
            let mut extensions = everruns_core::tool_context::ToolContextExtensions::default();
            extensions.insert(Arc::new(EmbedderMarker));
            extensions
        }))
        .workspace_policy(WorkspacePolicy::read_only())
        .llm_sim_as_default(LlmSimConfig::fixed("ok"))
        .harness(harness(harness_id))
        .session(session(session_id, harness_id))
        .build()
        .await
        .unwrap();

    let extensions = extensions(&runtime, session_id);
    assert!(extensions.get::<EmbedderMarker>().is_some());
    assert!(extensions.get::<RuntimeArtifactFileSystem>().is_some());
}
