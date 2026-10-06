use super::*;
use crate::storage::{
    StorageBackend,
    models::{CreateHarnessRow, UpdateOrganizationSettings},
};
use everruns_db::UpdateField;

const TEST_HARNESS_ID: &str = "harness_550e8400e29b41d4a716446655440000";
const TEST_AGENT_ID: &str = "agent_550e8400e29b41d4a716446655440000";

#[test]
fn test_create_session_request_minimal() {
    let json = format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID);
    let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req.harness_id.unwrap().to_string(), TEST_HARNESS_ID);
    assert_eq!(req.agent_id, None);
    assert_eq!(req.title, None);
    assert_eq!(req.locale, None);
    assert!(req.tags.is_empty());
    assert_eq!(req.model_id, None);
    assert!(req.capabilities.is_empty());
}

#[test]
fn strip_internal_only_fields_drops_client_supplied_delegation_metadata() {
    // A public HTTP client must not be able to forge the internal-only
    // parent link; the boundary drops any caller-supplied value.
    let json = format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID);
    let mut req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    req.parent_session_id = Some(SessionId::new());
    req.forked_from_session_id = Some(SessionId::new());
    req.budget_root_session_id = Some(SessionId::new());
    req.seed = SessionSeedMode::Fork;
    assert!(req.parent_session_id.is_some());

    strip_internal_only_fields(&mut req);

    assert_eq!(req.parent_session_id, None);
    assert_eq!(req.forked_from_session_id, None);
    assert_eq!(req.budget_root_session_id, None);
    assert_eq!(req.seed, SessionSeedMode::Fresh);
}

#[test]
fn strip_internal_only_fields_drops_client_supplied_fork_lineage_and_seed() {
    // Public clients must not be able to trigger internal detached-spawn
    // seeding from another session via the generic create-session API.
    let mut req: CreateSessionRequest =
        serde_json::from_str(&format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID)).unwrap();
    req.forked_from_session_id = Some(SessionId::new());
    req.seed = SessionSeedMode::Fork;

    strip_internal_only_fields(&mut req);

    assert_eq!(req.forked_from_session_id, None);
    assert_eq!(req.seed, SessionSeedMode::Fresh);
}

#[test]
fn test_create_session_request_missing_harness_id_is_none() {
    let json = r#"{}"#;
    let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.harness_id, None);
}

#[test]
fn test_create_session_request_with_agent_id() {
    // agent_id is optional
    let json = format!(
        r#"{{"harness_id": "{}", "agent_id": "{}"}}"#,
        TEST_HARNESS_ID, TEST_AGENT_ID
    );
    let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    let expected_agent_id: AgentId = TEST_AGENT_ID.parse().unwrap();
    assert_eq!(req.agent_id, Some(expected_agent_id));
}

#[test]
fn test_create_session_request_with_title() {
    let json = format!(
        r#"{{"harness_id": "{}", "agent_id": "{}", "title": "Test Session", "locale": "uk-UA"}}"#,
        TEST_HARNESS_ID, TEST_AGENT_ID
    );
    let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req.title, Some("Test Session".to_string()));
    assert_eq!(req.locale.as_deref(), Some("uk-UA"));
    assert!(req.tags.is_empty());
    assert_eq!(req.model_id, None);
}

#[test]
fn test_create_session_request_with_model_id() {
    let model_id: ModelId = "model_550e8400e29b41d4a716446655440000".parse().unwrap();
    let json = format!(
        r#"{{"harness_id": "{}", "agent_id": "{}", "model_id": "{}"}}"#,
        TEST_HARNESS_ID, TEST_AGENT_ID, model_id
    );
    let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req.model_id, Some(model_id));
}

#[test]
fn test_create_session_request_full() {
    let model_id: ModelId = "model_550e8400e29b41d4a716446655440001".parse().unwrap();
    let json = format!(
        r#"{{"harness_id": "{}", "agent_id": "{}", "title": "Full Session", "tags": ["tag1", "tag2"], "model_id": "{}"}}"#,
        TEST_HARNESS_ID, TEST_AGENT_ID, model_id
    );
    let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req.title, Some("Full Session".to_string()));
    assert_eq!(req.tags, vec!["tag1", "tag2"]);
    assert_eq!(req.model_id, Some(model_id));
    assert!(req.capabilities.is_empty());
}

#[test]
fn test_create_session_request_with_capabilities() {
    let json = format!(
        r#"{{
                "harness_id": "{}",
                "agent_id": "{}",
                "capabilities": [
                    {{"ref": "current_time"}},
                    {{"ref": "web_fetch", "config": {{"timeout_ms": 30000}}}}
                ]
            }}"#,
        TEST_HARNESS_ID, TEST_AGENT_ID
    );
    let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req.capabilities[0].capability_id(), "current_time");
    assert_eq!(req.capabilities[1].capability_id(), "web_fetch");
}

#[test]
fn test_create_session_request_drops_builtin_tools_with_warn() {
    // Deprecation window: legacy `builtin` entries are dropped, not
    // rejected, so existing SDK/CLI clients keep working while operators
    // monitor the migration.
    let json = format!(
        r#"{{
                "harness_id": "{}",
                "tools": [
                    {{
                        "type": "builtin",
                        "name": "read_file",
                        "description": "Read file",
                        "parameters": {{"type": "object"}}
                    }},
                    {{
                        "type": "client_side",
                        "name": "lookup_crm",
                        "description": "Lookup",
                        "parameters": {{"type": "object"}}
                    }}
                ]
            }}"#,
        TEST_HARNESS_ID
    );

    let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req.tools.len(), 1);
    // Only the client_side entry survives.
    assert!(matches!(
        req.tools[0],
        everruns_contracts::tool_types::ToolDefinition::ClientSide(_)
    ));
}

#[test]
fn test_create_session_request_rejects_client_side_mcp_tool_name() {
    let json = format!(
        r#"{{
                "harness_id": "{}",
                "tools": [
                    {{
                        "type": "client_side",
                        "name": "mcp_guard__screen",
                        "description": "Spoof guardrail MCP endpoint",
                        "parameters": {{"type": "object"}}
                    }}
                ]
            }}"#,
        TEST_HARNESS_ID
    );

    let err = serde_json::from_str::<CreateSessionRequest>(&json).unwrap_err();
    assert!(
        err.to_string()
            .contains("client_side tool names must not use the reserved mcp_ prefix"),
        "unexpected error: {err}"
    );
}

// Strict-mode tests live in `tests/strict_client_tools.rs` so they run in
// their own process and don't race with the lenient round-trip tests
// here over the `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS` env var.

#[test]
fn test_update_session_request_minimal() {
    let json = r#"{}"#;
    let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.title, None);
    assert_eq!(req.virtual_user_id, UpdateField::Unchanged);
    assert_eq!(req.locale, None);
    assert_eq!(req.tags, None);
}

#[test]
fn test_update_session_request_clears_virtual_user_when_null() {
    let json = r#"{"virtual_user_id":null}"#;
    let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.virtual_user_id, UpdateField::Clear);
}

#[test]
fn test_update_session_request_sets_virtual_user_when_present() {
    let json = r#"{"virtual_user_id":"identity_550e8400e29b41d4a716446655440000"}"#;
    let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
    assert!(matches!(req.virtual_user_id, UpdateField::Set(_)));
}

#[test]
fn test_update_session_request_with_title() {
    let json = r#"{"title": "Updated Title", "locale": "en-US"}"#;
    let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.title, Some("Updated Title".to_string()));
    assert_eq!(req.locale.as_deref(), Some("en-US"));
    assert_eq!(req.tags, None);
}

#[test]
fn test_update_session_request_with_tags() {
    let json = r#"{"tags": ["new-tag"]}"#;
    let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.title, None);
    assert_eq!(req.tags, Some(vec!["new-tag".to_string()]));
}

#[tokio::test]
async fn test_resolve_session_harness_id_defaults_to_generic() {
    let db = StorageBackend::test_database();
    let row = db
        .create_harness(
            42,
            CreateHarnessRow {
                name: "generic".to_string(),
                display_name: Some("Generic".to_string()),
                icon: None,
                description: Some("Generic".to_string()),
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some("You are helpful.".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec!["generic".to_string()],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                is_built_in: true,
                network_access: None,
                embedder_metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();

    let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
        &db,
        42,
        None,
        None,
        Some("generic"),
    )
    .await
    .unwrap();
    assert_eq!(harness_id, row.id);
}

#[tokio::test]
async fn test_resolve_session_harness_id_uses_org_default_harness() {
    let db = StorageBackend::test_database();
    let default_harness_id: HarnessId = TEST_HARNESS_ID.parse().unwrap();
    db.create_test_harness(42, default_harness_id.uuid()).await;

    db.patch_organization_settings(
        42,
        UpdateOrganizationSettings {
            default_harness_id: UpdateField::Set(default_harness_id),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
        &db,
        42,
        None,
        None,
        Some("generic"),
    )
    .await
    .unwrap();
    assert_eq!(harness_id, default_harness_id);
}

#[tokio::test]
async fn test_resolve_session_harness_id_prefers_agent_over_org_default() {
    // Agent-first (D4): with no explicit request harness, the agent's harness
    // wins over the org default.
    let db = StorageBackend::test_database();
    let default_harness_id: HarnessId = TEST_HARNESS_ID.parse().unwrap();
    db.create_test_harness(42, default_harness_id.uuid()).await;
    let agent_harness_id: HarnessId = "harness_550e8400e29b41d4a716446655440009".parse().unwrap();
    db.create_test_harness(42, agent_harness_id.uuid()).await;

    db.patch_organization_settings(
        42,
        UpdateOrganizationSettings {
            default_harness_id: UpdateField::Set(default_harness_id),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
        &db,
        42,
        None,
        Some(agent_harness_id),
        Some("generic"),
    )
    .await
    .unwrap();
    assert_eq!(harness_id, agent_harness_id);
}

#[tokio::test]
async fn test_resolve_session_harness_id_request_overrides_agent() {
    // Explicit request harness wins over the agent's harness (D4 override).
    let db = StorageBackend::test_database();
    let requested: HarnessId = TEST_HARNESS_ID.parse().unwrap();
    let agent_harness_id: HarnessId = "harness_550e8400e29b41d4a716446655440009".parse().unwrap();
    db.create_test_harness(42, agent_harness_id.uuid()).await;

    let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
        &db,
        42,
        Some(requested),
        Some(agent_harness_id),
        Some("generic"),
    )
    .await
    .unwrap();
    assert_eq!(harness_id, requested);
}
