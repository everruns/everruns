//! Field-by-field precedence for runtime reconstruction (EVE-1299).

use super::*;
use everruns_contracts::tool_types::{ClientSideTool, ToolDefinition};
use everruns_contracts::typed_id::{AgentId, HarnessId, ModelId, SessionId, WorkspaceId};
use everruns_core::host::SessionBuilder;
use serde_json::json;

fn client_tool(name: &str) -> ToolDefinition {
    let tool: ClientSideTool = serde_json::from_value(json!({
        "name": name,
        "description": format!("{name} tool"),
        "parameters": {"type": "object"},
    }))
    .unwrap();
    ToolDefinition::ClientSide(tool)
}

fn tool_names(session: &ExecutionSession) -> Vec<String> {
    session
        .tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect()
}

fn capability_ids(session: &ExecutionSession) -> Vec<&str> {
    session
        .capabilities
        .iter()
        .map(CapabilityRef::capability_id)
        .collect()
}

/// A record as a previous build plus later live changes left it.
fn persisted(id: SessionId) -> ExecutionSession {
    let mut record = SessionBuilder::new(HarnessId::new())
        .id(id)
        .agent(AgentId::new())
        .workspace(WorkspaceId::new())
        .title("kept title")
        .goal("kept goal")
        .locale("uk-UA")
        .tags(["kept"])
        .model_id(ModelId::new())
        .max_iterations(7)
        .tool(client_tool("stale_client_tool"))
        .system_prompt("stale run-scoped instructions")
        .parallel_tool_calls(true)
        .build();
    record.organization_id = "org_stale".into();
    record.parent_session_id = Some(SessionId::new());
    record.forked_from_session_id = Some(SessionId::new());
    record.blueprint_id = Some("blueprint".into());
    record.blueprint_config = Some(json!({"kept": true}));
    record.usage = Some(Default::default());
    record.status = everruns_core::session::SessionExecutionState::WaitingForToolResults;
    record.mcp_servers.insert(
        "stale".into(),
        serde_json::from_value(json!({"url": "https://stale.invalid/mcp"})).unwrap(),
    );
    record.hints = Some(
        [
            ("setup_connection".to_string(), json!(true)),
            ("ask_user".to_string(), json!(true)),
            ("client_kind".to_string(), json!("tui")),
        ]
        .into_iter()
        .collect(),
    );
    // Legacy seeding copied the Agent default (`agent_default`, old config)
    // into the session layer; `hot_enabled` was activated on the session.
    record.capabilities = vec![
        CapabilityRef::with_config("agent_default", json!({"version": 1})),
        CapabilityRef::new("hot_enabled"),
        CapabilityRef::new(crate::hooks::LIFECYCLE_HOOK_CAPABILITY_ID),
    ];
    record
}

#[test]
fn session_owned_fields_survive_and_agent_owned_fields_are_rederived() {
    let id = SessionId::new();
    let existing = persisted(id);
    let fresh = SessionBuilder::new(HarnessId::new())
        .id(id)
        .agent(AgentId::new())
        .workspace(WorkspaceId::new())
        .tool(client_tool("agent_client_tool"))
        .build();
    let expected_identity = (
        fresh.organization_id.clone(),
        fresh.workspace_id,
        fresh.harness_id,
        fresh.agent_id,
    );
    let agent = [CapabilityRef::with_config(
        "agent_default",
        json!({"version": 2}),
    )];

    let merged = reconcile(fresh, existing.clone(), &agent);

    // Session-owned: the persisted value wins.
    assert_eq!(merged.title, existing.title);
    assert_eq!(merged.goal, existing.goal);
    assert_eq!(merged.locale, existing.locale);
    assert_eq!(merged.tags, existing.tags);
    assert_eq!(merged.model_id, existing.model_id);
    assert_eq!(merged.max_iterations, existing.max_iterations);
    assert_eq!(merged.parent_session_id, existing.parent_session_id);
    assert_eq!(
        merged.forked_from_session_id,
        existing.forked_from_session_id
    );
    assert_eq!(merged.blueprint_id, existing.blueprint_id);
    assert_eq!(merged.blueprint_config, existing.blueprint_config);
    assert!(merged.usage.is_some());

    // Agent/Engine-owned: association ids and Agent configuration come from
    // this build, never from the older record.
    assert_eq!(
        (
            merged.organization_id.clone(),
            merged.workspace_id,
            merged.harness_id,
            merged.agent_id,
        ),
        expected_identity
    );
    assert_eq!(tool_names(&merged), ["agent_client_tool"]);
    assert!(merged.mcp_servers.is_empty(), "stale MCP server copied");
    assert_eq!(merged.system_prompt, None);
    assert_eq!(merged.parallel_tool_calls, None);
    assert_eq!(
        merged.status,
        everruns_core::session::SessionExecutionState::Started
    );

    // Capabilities: the hot-enabled overlay stays; the legacy copy of an Agent
    // default and the Engine-owned hook capability leave the session layer.
    assert_eq!(capability_ids(&merged), ["hot_enabled"]);

    // Hints: Agent-derived keys are re-derived, client keys are kept.
    let hints = merged.hints.expect("client hint kept");
    assert_eq!(hints.len(), 1);
    assert_eq!(hints["client_kind"], json!("tui"));
}

#[test]
fn agent_derived_hints_come_from_the_fresh_build() {
    let id = SessionId::new();
    let mut fresh = SessionBuilder::new(HarnessId::new()).id(id).build();
    fresh.hints = Some(
        [("ask_user".to_string(), json!(true))]
            .into_iter()
            .collect(),
    );
    let merged = reconcile(fresh, persisted(id), &[]);
    let hints = merged.hints.expect("hints");
    assert_eq!(hints.get("ask_user"), Some(&json!(true)));
    assert_eq!(hints.get("setup_connection"), None);
    assert_eq!(hints.get("client_kind"), Some(&json!("tui")));
}

#[test]
fn a_disabled_overlay_stays_disabled_and_reconciling_is_idempotent() {
    let id = SessionId::new();
    let mut existing = persisted(id);
    // Deactivation removes the overlay entry from the record.
    existing
        .capabilities
        .retain(|capability| capability.capability_id() != "hot_enabled");
    let agent = [CapabilityRef::new("agent_default")];
    let build = || SessionBuilder::new(HarnessId::new()).id(id).build();

    let once = reconcile(build(), existing, &agent);
    assert!(capability_ids(&once).is_empty());
    let twice = reconcile(build(), once.clone(), &agent);
    assert_eq!(
        serde_json::to_value(&twice.capabilities).unwrap(),
        serde_json::to_value(&once.capabilities).unwrap()
    );
    assert_eq!(twice.title, once.title);
    assert_eq!(twice.parent_session_id, once.parent_session_id);
}

#[tokio::test]
async fn a_new_record_is_the_fresh_build() {
    let store = everruns_core::host::HostBackends::in_memory().session_store;
    let fresh = SessionBuilder::new(HarnessId::new())
        .title("fresh")
        .capability(CapabilityRef::new("overlay"))
        .build();
    let merged = reconcile_session_record(store.as_ref(), fresh.clone(), &[]).await;
    assert_eq!(
        serde_json::to_value(&merged).unwrap(),
        serde_json::to_value(&fresh).unwrap()
    );
}
