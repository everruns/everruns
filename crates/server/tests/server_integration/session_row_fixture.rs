//! A base `CreateSessionRow` for the repository suites.
//!
//! `CreateSessionRow` has thirty fields and no `Default` — `owner_principal_id`
//! is a typed id with no meaningful zero value, so one cannot be derived. Every
//! seed in `repository_integration_test.rs` therefore spelled all thirty out,
//! which made that file one of the largest in the repo and meant every new
//! column touched seventeen call sites that did not care about it (EVE-1138
//! added `trigger_id` and hit exactly that).
//!
//! This gives the suites a base to spread over instead. A seed now names only
//! the fields it is actually testing:
//!
//! ```ignore
//! CreateSessionRow {
//!     agent_id: Some(agent.id),
//!     owner_principal_id,
//!     ..base_session_row(TEST_ORG_ID)
//! }
//! ```

use everruns_provider::typed_id::PrincipalId;
use everruns_server::storage::models::CreateSessionRow;

/// Every field at its inert value: no agent, no ingress, no workspace, no
/// blueprint, no capabilities. `owner_principal_id` is a fresh principal, which
/// callers that care about ownership override.
pub fn base_session_row(org_id: i64) -> CreateSessionRow {
    CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id,
        app_id: None,
        endpoint_id: None,
        trigger_id: None,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        virtual_user_id: None,
        owner_principal_id: PrincipalId::new(),
        resolved_owner_user_id: None,
        title: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    }
}
