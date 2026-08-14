// Commands API
//
// Provides endpoints for discovering and executing slash commands.
// Commands come from two sources:
// 1. System commands — from capabilities, executed without persisting a chat
//    message
// 2. Skill commands — from skills with user-invocable: true (prompt expansion)

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::session_commands::SessionCommandService;
use crate::domains::sessions::SESSION_VIEW;
use crate::services::CapabilityService;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};

use super::common::{ApiPolicyResultExt, ApiResult, impl_auth_state};
use everruns_core::Caller;
use everruns_core::command::{CommandDescriptor, CommandResult, ExecuteCommandRequest};
use everruns_provider::typed_id::SessionId;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

/// App state for commands routes
#[derive(Clone)]
pub struct AppState {
    pub capability_service: Arc<CapabilityService>,
    pub command_service: Arc<SessionCommandService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        capability_service: Arc<CapabilityService>,
        command_service: Arc<SessionCommandService>,
        auth: AuthState,
    ) -> Self {
        Self {
            capability_service,
            command_service,
            auth,
        }
    }
}

impl_auth_state!(AppState);

/// Response for listing available commands
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommandsResponse {
    /// Available commands
    pub commands: Vec<CommandDescriptor>,
}

/// Create command routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/commands",
            get(list_session_commands),
        )
        .route(
            "/v1/sessions/{session_id}/commands/execute",
            post(execute_session_command),
        )
        .with_state(state)
}

/// GET /v1/sessions/{session_id}/commands
///
/// List all available commands for a session. Returns system commands from
/// capabilities and invocable skills from the session's org.
async fn list_session_commands(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(session_id): Path<SessionId>,
) -> ApiResult<CommandsResponse> {
    let caller = Caller::from(&org);
    SESSION_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(anyhow::Error::from)
        .map_policy_or_internal("authorize list session commands")?;

    let mut commands = state
        .command_service
        .list_system_commands(&caller, session_id)
        .await
        .map_policy_or_internal("list session system commands")?;

    // Collect invocable skill commands from the org's skills
    let skill_commands = state
        .capability_service
        .list_skill_commands(org.org_id)
        .await
        .map_policy_or_internal("list skill commands")?;
    commands.extend(skill_commands);

    Ok(Json(CommandsResponse { commands }))
}

/// POST /v1/sessions/{session_id}/commands/execute
///
/// Execute a system command against the current session without persisting a
/// chat message. Skill commands are not handled here.
async fn execute_session_command(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(session_id): Path<SessionId>,
    Json(req): Json<ExecuteCommandRequest>,
) -> ApiResult<CommandResult> {
    let caller = Caller::from(&org);
    SESSION_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(anyhow::Error::from)
        .map_policy_or_internal("authorize execute session command")?;

    let result = state
        .command_service
        .execute(&caller, session_id, req)
        .await
        .map_policy_or_internal("execute session command")?;
    Ok(Json(result))
}
