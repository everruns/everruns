// Commands API
//
// Provides endpoints for discovering and executing slash commands.
// Commands come from two sources:
// 1. System commands — from capabilities (no LLM, direct action)
// 2. Skill commands — from skills with user-invocable: true (prompt expansion)

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::CapabilityService;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::command::{CommandDescriptor, CommandResult};
use everruns_core::typed_id::SessionId;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

/// App state for commands routes
#[derive(Clone)]
pub struct AppState {
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(capability_service: Arc<CapabilityService>, auth: AuthState) -> Self {
        Self {
            capability_service,
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

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
        .with_state(state)
}

/// GET /v1/sessions/{session_id}/commands
///
/// List all available commands for a session. Returns system commands from
/// capabilities and invocable skills from the session's VFS.
async fn list_session_commands(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(session_id): Path<SessionId>,
    // session_id is used to scope the response; skills discovery
    // will be added when VFS scanning is integrated
) -> Result<Json<CommandsResponse>, (StatusCode, Json<super::ErrorResponse>)> {
    let _ = (org.org_id, session_id); // Will be used for skill discovery

    // Collect system commands from all capabilities
    let mut commands = Vec::new();
    let system_commands = state.capability_service.list_system_commands();
    commands.extend(system_commands);

    // Collect invocable skill commands from the org's skills
    let skill_commands = state
        .capability_service
        .list_skill_commands(org.org_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(super::ErrorResponse {
                    error: format!("Failed to list skill commands: {e}"),
                }),
            )
        })?;
    commands.extend(skill_commands);

    Ok(Json(CommandsResponse { commands }))
}

/// POST /v1/sessions/{session_id}/commands/{command_name}
///
/// Execute a system command. Skill commands should be invoked by sending
/// a message with the expanded skill instructions instead.
async fn _execute_command(
    State(_state): State<AppState>,
    _org: ResolvedOrg,
    Path((_session_id, _command_name)): Path<(SessionId, String)>,
) -> Result<Json<CommandResult>, (StatusCode, Json<super::ErrorResponse>)> {
    // System command execution will be implemented when specific
    // commands (clear, status, compact) have their handlers
    Err((
        StatusCode::NOT_IMPLEMENTED,
        Json(super::ErrorResponse {
            error: "Command execution not yet implemented".to_string(),
        }),
    ))
}
