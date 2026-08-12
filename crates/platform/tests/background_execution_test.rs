// The hosted product registry supplies both `background_execution` (EVE-885)
// and the real background-capable `bashkit_shell` tool, so this file needs the
// product's environment integrations compiled in.
#![cfg(feature = "environment-capabilities")]

// EVE-501: integration tests for the `background_execution` capability.
//
// These tests assemble the runtime tool set the same way the worker and the
// in-process runtime do (`collect_capabilities_with_configs` -> merge into a
// `ToolRegistry`), and assert the lockstep invariant between the model-visible
// tool list and the worker's executor: when any collected tool declares
// `supports_background = true`, both the executor registry and the model-facing
// tool definitions must include `spawn_background`.
//
// LLM-driver-level scripted-session coverage (driving an agent through a full
// `spawn_background` invocation using llmsim 0.4.0 scripted mode) is the
// suggested next layer; this test pins the contract that path depends on.

use everruns_core::capabilities::{
    AgentCapabilityConfig, CapabilityId, SystemPromptContext, collect_capabilities_with_configs,
};
use everruns_core::tools::ToolRegistry;
use everruns_core::typed_id::SessionId;
use everruns_platform::capabilities::BACKGROUND_EXECUTION_CAPABILITY_ID;

fn ctx() -> SystemPromptContext {
    SystemPromptContext::without_file_store(SessionId::new())
}

fn cap(id: &str) -> AgentCapabilityConfig {
    AgentCapabilityConfig::with_config(
        CapabilityId::new(id),
        serde_json::Value::Object(serde_json::Map::new()),
    )
}

/// Assemble the executor registry the same way the worker does:
/// `with_defaults()` + collected capability tools. The model-visible tool list
/// comes from `tool_definitions`.
async fn assemble(
    cap_ids: &[&str],
) -> (
    ToolRegistry,
    Vec<everruns_core::tool_types::ToolDefinition>,
    Vec<String>,
) {
    let registry = everruns_platform::capabilities::hosted_capability_registry();
    let configs: Vec<AgentCapabilityConfig> = cap_ids.iter().map(|id| cap(id)).collect();
    let collected = collect_capabilities_with_configs(&configs, &registry, &ctx()).await;

    let mut tool_registry = ToolRegistry::with_defaults();
    for tool in collected.tools {
        tool_registry.register_boxed(tool);
    }
    (
        tool_registry,
        collected.tool_definitions,
        collected.applied_ids,
    )
}

/// Auto-activation: a session that includes `bashkit_shell` (which advertises
/// `supports_background=true` on its `bash` tool) must end up with
/// `spawn_background` in BOTH the worker tool registry AND the model-visible
/// tool definitions, via the `background_execution` capability.
#[tokio::test]
async fn bashkit_shell_session_auto_activates_spawn_background_in_lockstep() {
    let (registry, defs, applied) = assemble(&["bashkit_shell"]).await;

    assert!(
        registry.has("bash"),
        "bashkit_shell must contribute the bash tool to the executor"
    );
    assert!(
        registry.has("spawn_background"),
        "spawn_background must be in the worker executor registry when a \
         background-capable tool is present (lockstep)"
    );

    let def_names: Vec<&str> = defs.iter().map(|d| d.name()).collect();
    assert!(
        def_names.contains(&"bash"),
        "model-visible tool list must include bash; got: {:?}",
        def_names
    );
    assert!(
        def_names.contains(&"spawn_background"),
        "model-visible tool list must include spawn_background when a \
         background-capable tool is present; got: {:?}",
        def_names
    );

    assert!(
        applied
            .iter()
            .any(|id| id == BACKGROUND_EXECUTION_CAPABILITY_ID),
        "background_execution must appear in applied_ids when auto-activated; \
         got: {:?}",
        applied
    );
}

/// Negative path: a session with only `current_time` (no background-capable
/// tools) must NOT advertise `spawn_background` to the model, and the worker
/// registry must not silently include it either — `with_defaults()` is no
/// longer a hidden source.
#[tokio::test]
async fn non_background_session_does_not_expose_spawn_background() {
    let (registry, defs, applied) = assemble(&["current_time"]).await;

    assert!(
        !registry.has("spawn_background"),
        "spawn_background must NOT be in the executor when no \
         background-capable tool is collected (EVE-501 lockstep contract)"
    );

    let def_names: Vec<&str> = defs.iter().map(|d| d.name()).collect();
    assert!(
        !def_names.contains(&"spawn_background"),
        "spawn_background must NOT be in tool_definitions without a \
         background-capable tool; got: {:?}",
        def_names
    );

    assert!(
        !applied
            .iter()
            .any(|id| id == BACKGROUND_EXECUTION_CAPABILITY_ID),
        "background_execution must NOT appear in applied_ids without a \
         background-capable tool; got: {:?}",
        applied
    );
}

/// Explicit selection of `background_execution` alongside a background-capable
/// tool must not duplicate `spawn_background` in either layer.
#[tokio::test]
async fn explicit_background_execution_plus_bashkit_shell_is_idempotent() {
    let (registry, defs, applied) =
        assemble(&["bashkit_shell", BACKGROUND_EXECUTION_CAPABILITY_ID]).await;

    assert!(registry.has("spawn_background"));

    let spawn_count = defs
        .iter()
        .filter(|d| d.name() == "spawn_background")
        .count();
    assert_eq!(
        spawn_count, 1,
        "spawn_background must appear exactly once in tool_definitions even \
         under explicit + auto-activation overlap"
    );

    let applied_count = applied
        .iter()
        .filter(|id| id.as_str() == BACKGROUND_EXECUTION_CAPABILITY_ID)
        .count();
    assert_eq!(
        applied_count, 1,
        "background_execution must appear exactly once in applied_ids"
    );
}

/// Hint discovery: `spawn_background`'s tool definition carries the
/// `background_execution` capability attribution so reporting/usage tracking
/// attributes the dispatch correctly.
#[tokio::test]
async fn spawn_background_definition_carries_capability_attribution() {
    let (_registry, defs, _applied) = assemble(&["bashkit_shell"]).await;
    let spawn_def = defs
        .iter()
        .find(|d| d.name() == "spawn_background")
        .expect("spawn_background must be in tool_definitions");
    let (cap_id, cap_name) = spawn_def
        .capability_attribution()
        .expect("spawn_background must carry capability attribution");
    assert_eq!(cap_id, BACKGROUND_EXECUTION_CAPABILITY_ID);
    assert_eq!(cap_name, Some("Background Execution"));
}
