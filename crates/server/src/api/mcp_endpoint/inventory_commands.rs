// Decision: inventory is now the only MCP catalog source of truth.
// Decision: keep the remaining direct-service MCP operations registered here
// until their request/response types finish moving under feature domains.

use crate::api::common::Pagination;
use crate::domains::common::{CommandDescriptor, CommandError, CommandMeta, Ctx, classify_anyhow};
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use serde::Serialize;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

type DispatchResult<'a> = Pin<Box<dyn Future<Output = Result<String, CommandError>> + Send + 'a>>;

#[derive(Clone, Copy)]
struct SchemaParam {
    name: &'static str,
    typ: &'static str,
    description: &'static str,
}

impl SchemaParam {
    const fn new(name: &'static str, typ: &'static str, description: &'static str) -> Self {
        Self {
            name,
            typ,
            description,
        }
    }
}

macro_rules! mcp_command {
    (
        meta_fn: $meta_fn:ident,
        schema_fn: $schema_fn:ident,
        dispatch: $dispatch_fn:ident,
        positional: $positional_fn:ident,
        name: $name:literal,
        category: $category:literal,
        description: $description:literal,
        method: $method:literal,
        path: $path:literal,
        params: [$(($param_name:literal, $param_type:literal, $param_description:literal)),* $(,)?]
    ) => {
        fn $meta_fn() -> CommandMeta {
            CommandMeta {
                name: $name,
                category: $category,
                description: $description,
                method: $method,
                path: $path,
            }
        }

        fn $schema_fn() -> Value {
            schema_from_params(&[$(SchemaParam::new($param_name, $param_type, $param_description)),*])
        }

        inventory::submit! {
            CommandDescriptor {
                meta: $meta_fn,
                positional_arg: $positional_fn,
                param_schema: $schema_fn,
                dispatch: $dispatch_fn,
            }
        }
    };
}

fn no_positional_arg() -> Option<&'static str> {
    None
}

fn budget_id_positional_arg() -> Option<&'static str> {
    Some("budget_id")
}

fn id_positional_arg() -> Option<&'static str> {
    Some("id")
}

fn org_positional_arg() -> Option<&'static str> {
    Some("org")
}

fn session_id_positional_arg() -> Option<&'static str> {
    Some("session_id")
}

fn schema_from_params(params: &[SchemaParam]) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for param in params {
        let schema_type = match param.typ {
            "array" => "array",
            "boolean" => "boolean",
            "integer" => "integer",
            "number" => "number",
            "object" => "object",
            _ => "string",
        };

        properties.insert(
            param.name.to_string(),
            json!({
                "type": schema_type,
                "description": param.description,
            }),
        );

        if param.typ == "path" {
            required.push(param.name.to_string());
        }
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn bad_request(error: impl std::fmt::Display) -> CommandError {
    CommandError::bad_request(error.to_string())
}

fn param_i64(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.as_i64())
}

fn param_str(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn param_u32(params: &Value, key: &str) -> Option<u32> {
    params.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

fn pagination(params: &Value) -> Pagination {
    let offset = param_u32(params, "offset").unwrap_or(0);
    let limit = param_u32(params, "limit").unwrap_or(20).min(100);
    Pagination::new(offset, limit)
}

fn parse_session_id(raw: &str) -> Result<uuid::Uuid, CommandError> {
    raw.parse::<SessionId>()
        .map(|id| id.uuid())
        .map_err(|e| bad_request(format!("Invalid session ID: {e}")))
}

fn require_str(params: &Value, key: &str) -> Result<String, CommandError> {
    param_str(params, key).ok_or_else(|| bad_request(format!("Missing required parameter: {key}")))
}

fn to_json(value: &impl Serialize) -> Result<String, CommandError> {
    serde_json::to_string(value).map_err(|e| CommandError::Internal(e.into()))
}

fn to_list(items: &impl Serialize) -> Result<String, CommandError> {
    serde_json::to_string(&json!({ "data": items })).map_err(|e| CommandError::Internal(e.into()))
}

fn to_paginated(
    items: &impl Serialize,
    total: u32,
    offset: u32,
    limit: u32,
) -> Result<String, CommandError> {
    serde_json::to_string(&json!({
        "data": items,
        "total": total,
        "offset": offset,
        "limit": limit,
    }))
    .map_err(|e| CommandError::Internal(e.into()))
}

mcp_command! {
    meta_fn: create_session_meta,
    schema_fn: create_session_schema,
    dispatch: dispatch_create_session,
    positional: no_positional_arg,
    name: "create_session",
    category: "sessions",
    description: "Create a new session. Optionally assign an agent and harness.",
    method: "POST",
    path: "/v1/sessions",
    params: [
        ("agent_id", "string", "Agent ID (optional)"),
        ("harness_id", "string", "Harness ID (optional, defaults to org base)"),
        ("title", "string", "Session title"),
        ("model_id", "string", "Model override"),
    ]
}

mcp_command! {
    meta_fn: list_sessions_meta,
    schema_fn: list_sessions_schema,
    dispatch: dispatch_list_sessions,
    positional: no_positional_arg,
    name: "list_sessions",
    category: "sessions",
    description: "List sessions. Filter by agent_id, search by title. Supports pagination (limit/offset).",
    method: "GET",
    path: "/v1/sessions",
    params: [
        ("agent_id", "query", "Filter by agent (optional)"),
        ("search", "query", "Search by title (optional)"),
        ("offset", "query", "Pagination offset (default: 0)"),
        ("limit", "query", "Page size (default: 20, max: 100)"),
    ]
}

mcp_command! {
    meta_fn: get_session_meta,
    schema_fn: get_session_schema,
    dispatch: dispatch_get_session,
    positional: session_id_positional_arg,
    name: "get_session",
    category: "sessions",
    description: "Get session details including status, agent, harness, and model.",
    method: "GET",
    path: "/v1/sessions/{session_id}",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: update_session_meta,
    schema_fn: update_session_schema,
    dispatch: dispatch_update_session,
    positional: session_id_positional_arg,
    name: "update_session",
    category: "sessions",
    description: "Update session title, tags, or locale.",
    method: "PATCH",
    path: "/v1/sessions/{session_id}",
    params: [
        ("session_id", "path", "Session ID"),
        ("title", "string", "New title"),
        ("tags", "array", "New tags"),
    ]
}

mcp_command! {
    meta_fn: delete_session_meta,
    schema_fn: delete_session_schema,
    dispatch: dispatch_delete_session,
    positional: session_id_positional_arg,
    name: "delete_session",
    category: "sessions",
    description: "Delete a session.",
    method: "DELETE",
    path: "/v1/sessions/{session_id}",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: cancel_session_meta,
    schema_fn: cancel_session_schema,
    dispatch: dispatch_cancel_session,
    positional: session_id_positional_arg,
    name: "cancel_session",
    category: "sessions",
    description: "Cancel the currently executing turn in a session.",
    method: "POST",
    path: "/v1/sessions/{session_id}/cancel",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: create_budget_meta,
    schema_fn: create_budget_schema,
    dispatch: dispatch_create_budget,
    positional: no_positional_arg,
    name: "create_budget",
    category: "budgets",
    description: "Create a budget for a subject (session, agent, user, org). Sets a spending cap in the given currency.",
    method: "POST",
    path: "/v1/budgets",
    params: [
        ("subject_type", "string", "Subject type: session, agent, user, or org"),
        ("subject_id", "string", "Subject ID (e.g. session ID, agent ID)"),
        ("currency", "string", "Budget currency: usd, tokens, credits, or custom"),
        ("limit", "number", "Hard spending limit"),
        ("soft_limit", "number", "Optional soft limit (pauses before hard stop)"),
        ("period", "string", "Optional period as JSON (e.g. {\"type\":\"calendar\",\"unit\":\"month\"})"),
        ("metadata", "string", "Optional metadata as JSON"),
    ]
}

mcp_command! {
    meta_fn: list_budgets_meta,
    schema_fn: list_budgets_schema,
    dispatch: dispatch_list_budgets,
    positional: no_positional_arg,
    name: "list_budgets",
    category: "budgets",
    description: "List budgets. Filter by subject_type and subject_id.",
    method: "GET",
    path: "/v1/budgets",
    params: [
        ("subject_type", "query", "Filter by subject type (optional)"),
        ("subject_id", "query", "Filter by subject ID (optional)"),
    ]
}

mcp_command! {
    meta_fn: get_budget_meta,
    schema_fn: get_budget_schema,
    dispatch: dispatch_get_budget,
    positional: budget_id_positional_arg,
    name: "get_budget",
    category: "budgets",
    description: "Get a budget with current balance.",
    method: "GET",
    path: "/v1/budgets/{budget_id}",
    params: [("budget_id", "path", "Budget ID")]
}

mcp_command! {
    meta_fn: update_budget_meta,
    schema_fn: update_budget_schema,
    dispatch: dispatch_update_budget,
    positional: budget_id_positional_arg,
    name: "update_budget",
    category: "budgets",
    description: "Update a budget's limit, soft_limit, or status.",
    method: "PATCH",
    path: "/v1/budgets/{budget_id}",
    params: [
        ("budget_id", "path", "Budget ID"),
        ("limit", "number", "New hard limit (optional)"),
        ("soft_limit", "number", "New soft limit (optional)"),
        ("status", "string", "New status: active, disabled (optional)"),
        ("metadata", "string", "Optional metadata as JSON"),
    ]
}

mcp_command! {
    meta_fn: delete_budget_meta,
    schema_fn: delete_budget_schema,
    dispatch: dispatch_delete_budget,
    positional: budget_id_positional_arg,
    name: "delete_budget",
    category: "budgets",
    description: "Soft-delete a budget (sets status to disabled).",
    method: "DELETE",
    path: "/v1/budgets/{budget_id}",
    params: [("budget_id", "path", "Budget ID")]
}

mcp_command! {
    meta_fn: top_up_budget_meta,
    schema_fn: top_up_budget_schema,
    dispatch: dispatch_top_up_budget,
    positional: budget_id_positional_arg,
    name: "top_up_budget",
    category: "budgets",
    description: "Add credits to a budget. Reactivates exhausted/paused budgets if balance becomes positive.",
    method: "POST",
    path: "/v1/budgets/{budget_id}/top_up",
    params: [
        ("budget_id", "path", "Budget ID"),
        ("amount", "number", "Amount to add"),
        ("description", "string", "Optional description for the top-up"),
    ]
}

mcp_command! {
    meta_fn: check_budget_meta,
    schema_fn: check_budget_schema,
    dispatch: dispatch_check_budget,
    positional: budget_id_positional_arg,
    name: "check_budget",
    category: "budgets",
    description: "Check budget status and remaining balance.",
    method: "GET",
    path: "/v1/budgets/{budget_id}/check",
    params: [("budget_id", "path", "Budget ID")]
}

mcp_command! {
    meta_fn: list_session_budgets_meta,
    schema_fn: list_session_budgets_schema,
    dispatch: dispatch_list_session_budgets,
    positional: session_id_positional_arg,
    name: "list_session_budgets",
    category: "budgets",
    description: "List all budgets for a session.",
    method: "GET",
    path: "/v1/sessions/{session_id}/budgets",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: check_session_budgets_meta,
    schema_fn: check_session_budgets_schema,
    dispatch: dispatch_check_session_budgets,
    positional: session_id_positional_arg,
    name: "check_session_budgets",
    category: "budgets",
    description: "Check all budgets for a session (currently includes session and agent scopes).",
    method: "GET",
    path: "/v1/sessions/{session_id}/budgets/check",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: create_message_meta,
    schema_fn: create_message_schema,
    dispatch: dispatch_create_message,
    positional: session_id_positional_arg,
    name: "create_message",
    category: "messages",
    description: "Create a user message in a session. Triggers the agent workflow.",
    method: "POST",
    path: "/v1/sessions/{session_id}/messages",
    params: [
        ("session_id", "path", "Session ID"),
        ("message", "object", "{\"role\": \"user\", \"content\": [{\"type\": \"text\", \"text\": \"...\"}]}"),
    ]
}

mcp_command! {
    meta_fn: list_messages_meta,
    schema_fn: list_messages_schema,
    dispatch: dispatch_list_messages,
    positional: session_id_positional_arg,
    name: "list_messages",
    category: "messages",
    description: "List all messages in a session (user and agent messages).",
    method: "GET",
    path: "/v1/sessions/{session_id}/messages",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: list_events_meta,
    schema_fn: list_events_schema,
    dispatch: dispatch_list_events,
    positional: session_id_positional_arg,
    name: "list_events",
    category: "events",
    description: "List events for a session (JSON). Supports filtering by type and pagination.",
    method: "GET",
    path: "/v1/sessions/{session_id}/events",
    params: [
        ("session_id", "path", "Session ID"),
        ("since_id", "query", "Return events after this event ID"),
        ("types", "query", "Filter by event types (repeatable)"),
        ("exclude", "query", "Exclude event types (repeatable)"),
        ("limit", "query", "Max events (1-1000)"),
    ]
}

mcp_command! {
    meta_fn: stream_sse_meta,
    schema_fn: stream_sse_schema,
    dispatch: dispatch_stream_sse,
    positional: session_id_positional_arg,
    name: "stream_sse",
    category: "events",
    description: "Stream events via SSE (Server-Sent Events). Real-time event streaming.",
    method: "GET",
    path: "/v1/sessions/{session_id}/sse",
    params: [
        ("session_id", "path", "Session ID"),
        ("since_id", "query", "Resume from event ID"),
        ("types", "query", "Filter by event types"),
    ]
}

mcp_command! {
    meta_fn: list_models_meta,
    schema_fn: list_models_schema,
    dispatch: dispatch_list_models,
    positional: no_positional_arg,
    name: "list_models",
    category: "models",
    description: "List all LLM models across all providers.",
    method: "GET",
    path: "/v1/models",
    params: []
}

mcp_command! {
    meta_fn: get_model_meta,
    schema_fn: get_model_schema,
    dispatch: dispatch_get_model,
    positional: id_positional_arg,
    name: "get_model",
    category: "models",
    description: "Get a single LLM model by ID.",
    method: "GET",
    path: "/v1/models/{id}",
    params: [("id", "path", "Model ID")]
}

mcp_command! {
    meta_fn: list_providers_meta,
    schema_fn: list_providers_schema,
    dispatch: dispatch_list_providers,
    positional: no_positional_arg,
    name: "list_providers",
    category: "providers",
    description: "List configured LLM providers (OpenAI, Anthropic, etc.).",
    method: "GET",
    path: "/v1/providers",
    params: []
}

mcp_command! {
    meta_fn: create_provider_meta,
    schema_fn: create_provider_schema,
    dispatch: dispatch_create_provider,
    positional: no_positional_arg,
    name: "create_provider",
    category: "providers",
    description: "Create a new LLM provider configuration.",
    method: "POST",
    path: "/v1/providers",
    params: [
        ("name", "string", "Provider name"),
        ("provider_type", "string", "openai, anthropic, google, azure_openai, custom"),
        ("api_key", "string", "API key for the provider"),
    ]
}

mcp_command! {
    meta_fn: list_images_meta,
    schema_fn: list_images_schema,
    dispatch: dispatch_list_images,
    positional: no_positional_arg,
    name: "list_images",
    category: "images",
    description: "List uploaded images. Supports pagination (limit/offset).",
    method: "GET",
    path: "/v1/images",
    params: [
        ("offset", "query", "Pagination offset (default: 0)"),
        ("limit", "query", "Page size (default: 50, max: 100)"),
    ]
}

mcp_command! {
    meta_fn: get_image_meta,
    schema_fn: get_image_schema,
    dispatch: dispatch_get_image,
    positional: id_positional_arg,
    name: "get_image",
    category: "images",
    description: "Get image data by ID.",
    method: "GET",
    path: "/v1/images/{id}",
    params: [("id", "path", "Image ID")]
}

mcp_command! {
    meta_fn: list_schedules_meta,
    schema_fn: list_schedules_schema,
    dispatch: dispatch_list_schedules,
    positional: no_positional_arg,
    name: "list_schedules",
    category: "schedules",
    description: "List durable scheduled tasks.",
    method: "GET",
    path: "/v1/schedules",
    params: []
}

mcp_command! {
    meta_fn: create_schedule_meta,
    schema_fn: create_schedule_schema,
    dispatch: dispatch_create_schedule,
    positional: no_positional_arg,
    name: "create_schedule",
    category: "schedules",
    description: "Create a new durable scheduled task with a cron expression.",
    method: "POST",
    path: "/v1/schedules",
    params: [
        ("name", "string", "Schedule name"),
        ("cron", "string", "Cron expression"),
        ("target", "object", "Schedule target configuration"),
    ]
}

mcp_command! {
    meta_fn: list_orgs_meta,
    schema_fn: list_orgs_schema,
    dispatch: dispatch_list_orgs,
    positional: no_positional_arg,
    name: "list_orgs",
    category: "organizations",
    description: "List organizations for the current user.",
    method: "GET",
    path: "/v1/orgs",
    params: []
}

mcp_command! {
    meta_fn: get_org_meta,
    schema_fn: get_org_schema,
    dispatch: dispatch_get_org,
    positional: org_positional_arg,
    name: "get_org",
    category: "organizations",
    description: "Get organization details.",
    method: "GET",
    path: "/v1/orgs/{org}",
    params: [("org", "path", "Organization ID or slug")]
}

mcp_command! {
    meta_fn: list_users_meta,
    schema_fn: list_users_schema,
    dispatch: dispatch_list_users,
    positional: no_positional_arg,
    name: "list_users",
    category: "users",
    description: "List users in the current organization. Supports search filtering.",
    method: "GET",
    path: "/v1/users",
    params: [("search", "query", "Filter users by search term")]
}

mcp_command! {
    meta_fn: list_session_files_meta,
    schema_fn: list_session_files_schema,
    dispatch: dispatch_list_session_files,
    positional: session_id_positional_arg,
    name: "list_session_files",
    category: "files",
    description: "Get the root directory listing of session files.",
    method: "GET",
    path: "/v1/sessions/{session_id}/files",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: get_session_file_meta,
    schema_fn: get_session_file_schema,
    dispatch: dispatch_get_session_file,
    positional: no_positional_arg,
    name: "get_session_file",
    category: "files",
    description: "Get a file or directory at a path in the session filesystem.",
    method: "GET",
    path: "/v1/sessions/{session_id}/files/{path}",
    params: [
        ("session_id", "path", "Session ID"),
        ("path", "path", "File path"),
    ]
}

mcp_command! {
    meta_fn: list_session_databases_meta,
    schema_fn: list_session_databases_schema,
    dispatch: dispatch_list_session_databases,
    positional: session_id_positional_arg,
    name: "list_session_databases",
    category: "databases",
    description: "List session-scoped SQL databases.",
    method: "GET",
    path: "/v1/sessions/{session_id}/databases",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: list_session_storage_meta,
    schema_fn: list_session_storage_schema,
    dispatch: dispatch_list_session_storage,
    positional: session_id_positional_arg,
    name: "list_session_storage",
    category: "storage",
    description: "List key-value pairs in session storage.",
    method: "GET",
    path: "/v1/sessions/{session_id}/storage",
    params: [("session_id", "path", "Session ID")]
}

mcp_command! {
    meta_fn: submit_tool_results_meta,
    schema_fn: submit_tool_results_schema,
    dispatch: dispatch_submit_tool_results,
    positional: session_id_positional_arg,
    name: "submit_tool_results",
    category: "tool_results",
    description: "Submit client-side tool results back to a waiting session.",
    method: "POST",
    path: "/v1/sessions/{session_id}/tool_results",
    params: [
        ("session_id", "path", "Session ID"),
        ("results", "array", "Array of {tool_call_id, output} objects"),
    ]
}

mcp_command! {
    meta_fn: health_check_meta,
    schema_fn: health_check_schema,
    dispatch: dispatch_health_check,
    positional: no_positional_arg,
    name: "health_check",
    category: "system",
    description: "Health check endpoint. Returns server version and runner mode.",
    method: "GET",
    path: "/health",
    params: []
}

fn dispatch_create_session(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let req = serde_json::from_value(params.clone()).map_err(bad_request)?;

        let harness_uuid = if let Some(id_str) = param_str(&params, "harness_id") {
            id_str
                .parse::<HarnessId>()
                .map(|id| id.uuid())
                .map_err(|e| bad_request(format!("Invalid harness ID: {e}")))?
        } else if let Some(ref name) = runtime.fallback_base_harness_name {
            let row = ctx
                .db
                .get_harness_by_name(ctx.org_id(), name)
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::not_found("Default harness"))?;
            row.id.uuid()
        } else {
            return Err(CommandError::bad_request("harness_id is required"));
        };

        let agent_public_id = param_str(&params, "agent_id")
            .map(|s| s.parse::<AgentId>())
            .transpose()
            .map_err(|e| bad_request(format!("Invalid agent ID: {e}")))?;

        let session = runtime
            .session_service
            .create(&ctx.caller, harness_uuid, None, agent_public_id, req)
            .await
            .map_err(classify_anyhow)?;

        to_json(&session)
    })
}

fn dispatch_list_sessions(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let search = param_str(&params, "search");
        let agent_id = param_str(&params, "agent_id")
            .map(|s| s.parse::<AgentId>())
            .transpose()
            .map_err(|e| bad_request(format!("Invalid agent ID: {e}")))?;
        let pg = pagination(&params);
        let (sessions, total) = runtime
            .session_service
            .list(
                &ctx.caller,
                agent_id.as_ref().map(|id| id.uuid()),
                ctx.caller.user_id,
                search.as_deref(),
                pg,
            )
            .await
            .map_err(classify_anyhow)?;

        to_paginated(&sessions, total, pg.offset, pg.limit)
    })
}

fn dispatch_get_session(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let session = runtime
            .session_service
            .get(&ctx.caller, session_id, ctx.caller.user_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        to_json(&session)
    })
}

fn dispatch_update_session(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let req = serde_json::from_value(params).map_err(bad_request)?;
        let session = runtime
            .session_service
            .update(&ctx.caller, session_id, req)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        to_json(&session)
    })
}

fn dispatch_delete_session(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        runtime
            .session_service
            .delete(&ctx.caller, session_id)
            .await
            .map_err(classify_anyhow)?;
        to_json(&json!({ "deleted": true }))
    })
}

fn dispatch_cancel_session(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id: SessionId = require_str(&params, "session_id")?
            .parse()
            .map_err(|e| bad_request(format!("Invalid session ID: {e}")))?;
        runtime
            .runner
            .cancel_run(session_id)
            .await
            .map_err(classify_anyhow)?;
        to_json(&json!({ "cancelled": true }))
    })
}

fn dispatch_create_budget(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let mut params = params;
        if let Some(obj) = params.as_object_mut() {
            obj.insert("org_id".to_string(), json!(ctx.org_id()));
        }
        let input = serde_json::from_value(params).map_err(bad_request)?;
        let budget = ctx.db.create_budget(input).await.map_err(classify_anyhow)?;
        to_json(&budget)
    })
}

fn dispatch_list_budgets(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let subject_type = param_str(&params, "subject_type");
        let subject_id = param_str(&params, "subject_id");
        let budgets = ctx
            .db
            .list_budgets(ctx.org_id(), subject_type.as_deref(), subject_id.as_deref())
            .await
            .map_err(classify_anyhow)?;
        to_list(&budgets)
    })
}

fn dispatch_get_budget(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let budget_id: uuid::Uuid = require_str(&params, "budget_id")?
            .parse()
            .map_err(|e| bad_request(format!("Invalid budget ID: {e}")))?;
        let budget = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        to_json(&budget)
    })
}

fn dispatch_update_budget(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let budget_id: uuid::Uuid = require_str(&params, "budget_id")?
            .parse()
            .map_err(|e| bad_request(format!("Invalid budget ID: {e}")))?;
        let input = serde_json::from_value(params).map_err(bad_request)?;
        let budget = ctx
            .db
            .update_budget(ctx.org_id(), budget_id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        to_json(&budget)
    })
}

fn dispatch_delete_budget(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let budget_id: uuid::Uuid = require_str(&params, "budget_id")?
            .parse()
            .map_err(|e| bad_request(format!("Invalid budget ID: {e}")))?;
        ctx.db
            .delete_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?;
        to_json(&json!({ "deleted": true }))
    })
}

fn dispatch_top_up_budget(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let input = serde_json::from_value(params).map_err(bad_request)?;
        let (entry, budget) = ctx
            .db
            .create_budget_ledger_entry(input)
            .await
            .map_err(classify_anyhow)?;
        to_json(&json!({ "entry": entry, "budget": budget }))
    })
}

fn dispatch_check_budget(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let budget_id: uuid::Uuid = require_str(&params, "budget_id")?
            .parse()
            .map_err(|e| bad_request(format!("Invalid budget ID: {e}")))?;
        let budget = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        to_json(&budget)
    })
}

fn dispatch_list_session_budgets(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let session_id = require_str(&params, "session_id")?;
        let budgets = ctx
            .db
            .list_budgets(ctx.org_id(), Some("session"), Some(&session_id))
            .await
            .map_err(classify_anyhow)?;
        to_list(&budgets)
    })
}

fn dispatch_check_session_budgets(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id = require_str(&params, "session_id")?;
        let result = runtime
            .budget_service
            .check_budgets_for_session(ctx.org_id(), &session_id, None)
            .await;
        to_json(&result)
    })
}

fn dispatch_create_message(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let session = runtime
            .session_service
            .get(&ctx.caller, session_id, ctx.caller.user_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Session"))?;

        let msg_ctx = crate::services::CreateMessageContext {
            org_id: ctx.org_id(),
            user_id: ctx.caller.user_id,
            harness_id: session.harness_id.uuid(),
            agent_id: session.agent_id.map(|id| id.uuid()),
            session_id,
            event_metadata: None,
            request_id: None,
        };

        let req = serde_json::from_value(params).map_err(bad_request)?;
        let message = runtime
            .message_service
            .create(msg_ctx, req)
            .await
            .map_err(classify_anyhow)?;
        to_json(&message)
    })
}

fn dispatch_list_messages(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let messages = runtime
            .message_service
            .list(session_id)
            .await
            .map_err(classify_anyhow)?;
        to_list(&messages)
    })
}

fn dispatch_list_events(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let runtime = ctx.mcp_runtime()?;
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let since_id = param_str(&params, "since_id")
            .map(|s| {
                s.parse::<everruns_core::typed_id::EventId>()
                    .map(|id| id.uuid())
                    .or_else(|_| s.parse::<uuid::Uuid>())
                    .map_err(|e| bad_request(format!("Invalid since_id: {e}")))
            })
            .transpose()?;
        let types: Vec<String> = param_str(&params, "types")
            .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
            .unwrap_or_default();
        let exclude: Vec<String> = param_str(&params, "exclude")
            .map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
            .unwrap_or_default();
        let limit = param_i64(&params, "limit").map(|n| n as i32);

        let events = runtime
            .event_service
            .list(session_id, None, since_id, &types, &exclude, None, limit)
            .await
            .map_err(classify_anyhow)?;
        to_list(&events)
    })
}

fn dispatch_stream_sse(_params: Value, _ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        Err(CommandError::bad_request(
            "SSE streaming is not available in bash mode. Use list_events instead.",
        ))
    })
}

fn dispatch_list_models(_params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let models = ctx
            .db
            .list_all_llm_models(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        to_list(&models)
    })
}

fn dispatch_get_model(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let model_id: uuid::Uuid = require_str(&params, "id")?
            .parse()
            .map_err(|e| bad_request(format!("Invalid model ID: {e}")))?;
        let model = ctx
            .db
            .get_llm_model(ctx.org_id(), model_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Model"))?;
        to_json(&model)
    })
}

fn dispatch_list_providers(_params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let providers = ctx
            .db
            .list_llm_providers(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        to_list(&providers)
    })
}

fn dispatch_create_provider(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let row = serde_json::from_value(params).map_err(bad_request)?;
        let provider = ctx
            .db
            .create_llm_provider(ctx.org_id(), row)
            .await
            .map_err(classify_anyhow)?;
        to_json(&provider)
    })
}

fn dispatch_list_images(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let offset = param_u32(&params, "offset").unwrap_or(0) as i64;
        let limit = param_u32(&params, "limit").unwrap_or(50).min(100) as i64;
        let images = ctx
            .db
            .list_images(ctx.org_id(), limit, offset)
            .await
            .map_err(classify_anyhow)?;
        to_list(&images)
    })
}

fn dispatch_get_image(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let image_id: uuid::Uuid = require_str(&params, "id")?
            .parse()
            .map_err(|e| bad_request(format!("Invalid image ID: {e}")))?;
        let image = ctx
            .db
            .get_image(ctx.org_id(), image_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Image"))?;
        to_json(&image)
    })
}

fn dispatch_list_schedules(_params: Value, _ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move { not_implemented() })
}

fn dispatch_create_schedule(_params: Value, _ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move { not_implemented() })
}

fn dispatch_list_orgs(_params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let org = ctx
            .db
            .get_organization(ctx.org_id())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Organization"))?;
        to_list(&[org])
    })
}

fn dispatch_get_org(_params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let org = ctx
            .db
            .get_organization(ctx.org_id())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Organization"))?;
        to_json(&org)
    })
}

fn dispatch_list_users(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let search = param_str(&params, "search");
        let users = ctx
            .db
            .list_users(search.as_deref())
            .await
            .map_err(classify_anyhow)?;
        to_list(&users)
    })
}

fn dispatch_list_session_files(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let files = ctx
            .db
            .list_session_files(session_id, "/")
            .await
            .map_err(classify_anyhow)?;
        to_list(&files)
    })
}

fn dispatch_get_session_file(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let path = require_str(&params, "path")?;
        let file = ctx
            .db
            .get_session_file(session_id, &path)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("File"))?;
        to_json(&file)
    })
}

fn dispatch_list_session_databases(_params: Value, _ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move { not_implemented() })
}

fn dispatch_list_session_storage(params: Value, ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move {
        let session_id = parse_session_id(&require_str(&params, "session_id")?)?;
        let keys = ctx
            .db
            .list_session_keys(session_id)
            .await
            .map_err(classify_anyhow)?;
        to_list(&keys)
    })
}

fn dispatch_submit_tool_results(_params: Value, _ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move { not_implemented() })
}

fn dispatch_health_check(_params: Value, _ctx: &Ctx) -> DispatchResult<'_> {
    Box::pin(async move { to_json(&json!({ "status": "ok" })) })
}

fn not_implemented() -> Result<String, CommandError> {
    Err(CommandError::bad_request(
        "This operation is not yet implemented for direct service calls.",
    ))
}
