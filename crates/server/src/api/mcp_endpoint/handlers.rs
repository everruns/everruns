// Direct service call handlers for MCP catalog operations.
//
// Each handler takes parsed JSON params and a CatalogContext, calls the
// appropriate service method, and returns a JSON string result.
// No HTTP, no Router — pure Rust service calls.

use super::catalog::CatalogContext;
use crate::api::common::Pagination;
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use serde_json::{Value, json};

// ============================================================================
// Param helpers
// ============================================================================

fn param_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn param_bool(params: &Value, key: &str) -> bool {
    params.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn param_u32(params: &Value, key: &str) -> Option<u32> {
    params.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

fn param_i64(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.as_i64())
}

fn require_str(params: &Value, key: &str) -> Result<String, String> {
    param_str(params, key).ok_or_else(|| format!("Missing required parameter: {key}"))
}

fn pagination(params: &Value) -> Pagination {
    let offset = param_u32(params, "offset").unwrap_or(0);
    let limit = param_u32(params, "limit").unwrap_or(20).min(100);
    Pagination::new(offset, limit)
}

fn to_json(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn to_list(items: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(&json!({ "data": items })).map_err(|e| e.to_string())
}

fn to_paginated(
    items: &impl serde::Serialize,
    total: u32,
    offset: u32,
    limit: u32,
) -> Result<String, String> {
    serde_json::to_string(&json!({
        "data": items,
        "total": total,
        "offset": offset,
        "limit": limit,
    }))
    .map_err(|e| e.to_string())
}

fn parse_session_id(s: &str) -> Result<uuid::Uuid, String> {
    s.parse::<SessionId>()
        .map(|id| id.uuid())
        .map_err(|e| format!("Invalid session ID: {e}"))
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ============================================================================
// System
// ============================================================================

pub async fn health_check(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    to_json(&json!({"status": "ok"}))
}

// ============================================================================
// Agents
// ============================================================================

pub async fn create_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let agent = ctx
        .agent_service
        .create(&ctx.caller, None, req)
        .await
        .map_err(err)?;
    to_json(&agent)
}

pub async fn list_agents(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let search = param_str(params, "search");
    let include_archived = param_bool(params, "include_archived");
    let pg = pagination(params);
    let (agents, total) = ctx
        .agent_service
        .list(&ctx.caller, search.as_deref(), include_archived, pg)
        .await
        .map_err(err)?;
    to_paginated(&agents, total, pg.offset, pg.limit)
}

pub async fn get_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = require_str(params, "id")?;
    // Try as typed ID first, fall back to name lookup
    let agent = if let Ok(agent_id) = id.parse::<AgentId>() {
        ctx.agent_service
            .get_by_public_id(&ctx.caller, &agent_id.to_string())
            .await
            .map_err(err)?
    } else {
        ctx.agent_service
            .get_by_name(&ctx.caller, &id)
            .await
            .map_err(err)?
    };
    to_json(&agent.ok_or("Agent not found")?)
}

pub async fn update_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = require_str(params, "id")?;
    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let agent = ctx
        .agent_service
        .update(&ctx.caller, &id, req)
        .await
        .map_err(err)?
        .ok_or("Agent not found")?;
    to_json(&agent)
}

pub async fn delete_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = require_str(params, "id")?;
    ctx.agent_service
        .delete(&ctx.caller, &id)
        .await
        .map_err(err)?;
    to_json(&json!({"deleted": true}))
}

pub async fn upsert_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = require_str(params, "id")?;
    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let (agent, _created) = ctx
        .agent_service
        .upsert(&ctx.caller, &id, req)
        .await
        .map_err(err)?;
    to_json(&agent)
}

pub async fn copy_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = require_str(params, "id")?;
    let agent = ctx
        .agent_service
        .copy(&ctx.caller, &id)
        .await
        .map_err(err)?
        .ok_or("Agent not found")?;
    to_json(&agent)
}

pub async fn export_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = require_str(params, "id")?;
    let agent = ctx
        .agent_service
        .get_by_public_id(&ctx.caller, &id)
        .await
        .map_err(err)?
        .ok_or("Agent not found")?;
    // Return the agent as JSON (Markdown export is an HTTP-layer concern)
    to_json(&agent)
}

pub async fn import_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    // import_agent expects a full agent definition as JSON
    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let agent = ctx
        .agent_service
        .create(&ctx.caller, None, req)
        .await
        .map_err(err)?;
    to_json(&agent)
}

pub async fn preview_agent(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let system_prompt = param_str(params, "system_prompt").unwrap_or_default();
    let capabilities: Vec<everruns_core::AgentCapabilityConfig> = params
        .get("capabilities")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let (prompt, tools) = ctx
        .capability_service
        .preview(ctx.org_id, &system_prompt, &capabilities)
        .await
        .map_err(err)?;
    to_json(&json!({"system_prompt": prompt, "tools": tools}))
}

// ============================================================================
// Sessions
// ============================================================================

pub async fn create_session(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let req = serde_json::from_value(params.clone()).map_err(err)?;

    // Resolve harness — use provided ID or fall back to org default
    let harness_uuid = if let Some(id_str) = param_str(params, "harness_id") {
        id_str
            .parse::<HarnessId>()
            .map_err(|e| format!("Invalid harness ID: {e}"))?
            .uuid()
    } else if let Some(ref name) = ctx.fallback_base_harness_name {
        let row = ctx
            .db
            .get_harness_by_name(ctx.org_id, name)
            .await
            .map_err(err)?
            .ok_or("Default harness not found")?;
        row.id.uuid()
    } else {
        return Err("harness_id is required".into());
    };

    // Resolve agent if provided
    let agent_public_id = param_str(params, "agent_id")
        .map(|s| s.parse::<AgentId>())
        .transpose()
        .map_err(|e| format!("Invalid agent ID: {e}"))?;

    let session = ctx
        .session_service
        .create(&ctx.caller, harness_uuid, None, agent_public_id, req)
        .await
        .map_err(err)?;
    to_json(&session)
}

pub async fn list_sessions(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let search = param_str(params, "search");
    let agent_id = param_str(params, "agent_id")
        .map(|s| s.parse::<AgentId>())
        .transpose()
        .map_err(|e| format!("Invalid agent ID: {e}"))?;
    let pg = pagination(params);
    let (sessions, total) = ctx
        .session_service
        .list(
            &ctx.caller,
            agent_id.as_ref().map(|id| id.uuid()),
            ctx.user_id,
            search.as_deref(),
            pg,
        )
        .await
        .map_err(err)?;
    to_paginated(&sessions, total, pg.offset, pg.limit)
}

pub async fn get_session(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = parse_session_id(&require_str(params, "session_id")?)?;
    let session = ctx
        .session_service
        .get(&ctx.caller, id, ctx.user_id)
        .await
        .map_err(err)?
        .ok_or("Session not found")?;
    to_json(&session)
}

pub async fn update_session(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = parse_session_id(&require_str(params, "session_id")?)?;
    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let session = ctx
        .session_service
        .update(&ctx.caller, id, req)
        .await
        .map_err(err)?
        .ok_or("Session not found")?;
    to_json(&session)
}

pub async fn delete_session(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = parse_session_id(&require_str(params, "session_id")?)?;
    ctx.session_service
        .delete(&ctx.caller, id)
        .await
        .map_err(err)?;
    to_json(&json!({"deleted": true}))
}

pub async fn cancel_session(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id: SessionId = require_str(params, "session_id")?
        .parse()
        .map_err(|e| format!("Invalid session ID: {e}"))?;
    ctx.runner.cancel_run(id).await.map_err(err)?;
    to_json(&json!({"cancelled": true}))
}

// ============================================================================
// Budgets
// TODO: Storage Row types need Serialize derive to support direct serialization.
// For now, budget check (which uses BudgetService) works; CRUD operations
// delegate to not_implemented until Row types are updated.
// ============================================================================

pub async fn create_budget(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn list_budgets(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn get_budget(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn update_budget(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn delete_budget(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn top_up_budget(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn check_budget(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn list_session_budgets(
    _params: &Value,
    _ctx: &CatalogContext,
) -> Result<String, String> {
    not_implemented(_params, _ctx).await
}
pub async fn check_session_budgets(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let session_id = require_str(params, "session_id")?;
    let result = ctx
        .budget_service
        .check_budgets_for_session(ctx.org_id, &session_id, None)
        .await;
    to_json(&result)
}

// ============================================================================
// Messages
// ============================================================================

pub async fn create_message(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let session_id = parse_session_id(&require_str(params, "session_id")?)?;

    // Get session to resolve harness/agent context
    let session = ctx
        .session_service
        .get(&ctx.caller, session_id, ctx.user_id)
        .await
        .map_err(err)?
        .ok_or("Session not found")?;

    let msg_ctx = crate::services::CreateMessageContext {
        org_id: ctx.org_id,
        user_id: ctx.user_id,
        harness_id: session.harness_id.uuid(),
        agent_id: session.agent_id.map(|id| id.uuid()),
        session_id,
        event_metadata: None,
    };

    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let message = ctx
        .message_service
        .create(msg_ctx, req)
        .await
        .map_err(err)?;
    to_json(&message)
}

pub async fn list_messages(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let session_id = parse_session_id(&require_str(params, "session_id")?)?;
    let messages = ctx.message_service.list(session_id).await.map_err(err)?;
    to_list(&messages)
}

// ============================================================================
// Events
// ============================================================================

pub async fn list_events(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let session_id = parse_session_id(&require_str(params, "session_id")?)?;
    let since_id = param_str(params, "since_id")
        .map(|s| s.parse::<uuid::Uuid>())
        .transpose()
        .map_err(|e| format!("Invalid since_id: {e}"))?;
    let types: Vec<String> = param_str(params, "types")
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();
    let exclude: Vec<String> = param_str(params, "exclude")
        .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();
    let limit = param_i64(params, "limit").map(|n| n as i32);

    let events = ctx
        .event_service
        .list(session_id, None, since_id, &types, &exclude, None, limit)
        .await
        .map_err(err)?;
    to_list(&events)
}

pub async fn stream_sse(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    Err("SSE streaming is not available in bash mode. Use list_events instead.".into())
}

// ============================================================================
// Harnesses, Models, Providers
// TODO: Storage Row types need Serialize derive. Stubbed for now.
// ============================================================================

pub async fn list_harnesses(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn get_harness(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_models(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn get_model(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_providers(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn create_provider(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}

// ============================================================================
// MCP Servers
// ============================================================================

pub async fn list_mcp_servers(_params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let servers = ctx
        .mcp_server_service
        .list_active(&ctx.caller)
        .await
        .map_err(err)?;
    to_list(&servers)
}

pub async fn create_mcp_server(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let server = ctx
        .mcp_server_service
        .create(&ctx.caller, req)
        .await
        .map_err(err)?;
    to_json(&server)
}

pub async fn get_mcp_server(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id: uuid::Uuid = require_str(params, "id")?
        .parse()
        .map_err(|e| format!("Invalid MCP server ID: {e}"))?;
    let server = ctx
        .mcp_server_service
        .get(&ctx.caller, id)
        .await
        .map_err(err)?
        .ok_or("MCP server not found")?;
    to_json(&server)
}

// ============================================================================
// Capabilities
// ============================================================================

pub async fn list_capabilities(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let mut all = ctx
        .capability_service
        .list_all(ctx.org_id)
        .await
        .map_err(err)?;

    // Client-side search/pagination (capability service returns all)
    if let Some(search) = param_str(params, "search") {
        let search = search.to_lowercase();
        all.retain(|c| {
            serde_json::to_string(c)
                .unwrap_or_default()
                .to_lowercase()
                .contains(&search)
        });
    }
    let total = all.len() as u32;
    let offset = param_u32(params, "offset").unwrap_or(0) as usize;
    let limit = param_u32(params, "limit").unwrap_or(100).min(200) as usize;
    let page: Vec<_> = all.into_iter().skip(offset).take(limit).collect();
    to_paginated(&page, total, offset as u32, limit as u32)
}

pub async fn get_capability(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let id = require_str(params, "id")?;
    let cap_id = id
        .parse()
        .map_err(|e| format!("Invalid capability ID: {e}"))?;
    let cap = ctx
        .capability_service
        .get(ctx.org_id, &cap_id)
        .await
        .map_err(err)?
        .ok_or("Capability not found")?;
    to_json(&cap)
}

// ============================================================================
// Skills
// ============================================================================

pub async fn list_skills(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let include_archived = param_bool(params, "include_archived");
    let skills = ctx
        .skill_service
        .list(&ctx.caller, None, include_archived)
        .await
        .map_err(err)?;
    to_list(&skills)
}

pub async fn create_skill(params: &Value, ctx: &CatalogContext) -> Result<String, String> {
    let req = serde_json::from_value(params.clone()).map_err(err)?;
    let skill = ctx
        .skill_service
        .create(&ctx.caller, req)
        .await
        .map_err(err)?;
    to_json(&skill)
}

// ============================================================================
// Images, Schedules, Organizations, Users, Files, Databases, Storage,
// Tool Results
// TODO: These use StorageBackend directly with non-serializable Row types.
// Need to add Serialize to Row types or create service layer methods.
// ============================================================================

pub async fn list_images(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn get_image(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_schedules(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn create_schedule(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_orgs(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn get_org(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_users(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_session_files(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn get_session_file(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_session_databases(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn list_session_storage(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}
pub async fn submit_tool_results(_p: &Value, _c: &CatalogContext) -> Result<String, String> {
    not_implemented(_p, _c).await
}

// ============================================================================
// Fallback
// ============================================================================

pub async fn not_implemented(_params: &Value, _ctx: &CatalogContext) -> Result<String, String> {
    Err("This operation is not yet implemented for direct service calls.".into())
}
