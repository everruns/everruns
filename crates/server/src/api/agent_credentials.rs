use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::domains::agents::AGENT_MANAGE;
use crate::domains::agents::credentials::{AgentCredentialBinding, CreateAgentCredentialBinding};
use crate::domains::change_history::{ChangeAction, EntityKind, rest::RestChange};
use crate::domains::common::Command;
use crate::kernel_imports::{Caller, contracts::typed_id::AgentId};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::{ApiResult, ErrorResponse, ListResponse};

#[derive(Deserialize, ToSchema)]
/// Request to replace a credential binding's encrypted value.
pub struct SetAgentCredentialValueRequest {
    /// Write-only credential value. It is encrypted and never returned.
    #[schema(example = "vsk_disposable_example")]
    pub value: String,
}

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/credentials",
            get(list_credentials).post(create_credential_binding),
        )
        .route(
            "/v1/agents/{agent_id}/credentials/{binding_id}",
            axum::routing::put(set_credential_value).delete(delete_credential_binding),
        )
        .with_state(state)
}

async fn resolve_agent(
    state: &ApiState,
    org: &ResolvedOrg,
    raw_agent_id: &str,
) -> Result<(AgentId, String), (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(org);
    AGENT_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(|_| {
            ErrorResponse::new("Permission denied").into_response(StatusCode::FORBIDDEN)
        })?;
    let agent_id: AgentId = raw_agent_id.parse().map_err(|_| {
        ErrorResponse::new("Invalid agent ID").into_response(StatusCode::BAD_REQUEST)
    })?;
    let row = state
        .db
        .get_agent(org.org_id, agent_id)
        .await
        .map_err(|_| ErrorResponse::internal_error())?
        .ok_or_else(|| ErrorResponse::not_found("Agent"))?;
    Ok((row.id, raw_agent_id.to_string()))
}

#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/credentials",
    description = "List write-only MCP credential bindings for an agent. Responses contain metadata and configuration status, never credential values.",
    params(("agent_id" = String, Path, description = "Agent ID")),
    responses((status = 200, description = "Credential binding metadata", body = ListResponse<AgentCredentialBinding>)),
    tag = "agents"
)]
pub async fn list_credentials(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(agent_id): Path<String>,
) -> ApiResult<ListResponse<AgentCredentialBinding>> {
    let (internal_id, public_id) = resolve_agent(&state, &org, &agent_id).await?;
    let rows = state
        .db
        .list_agent_mcp_secret_bindings(org.org_id, internal_id)
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    Ok(Json(ListResponse::new(
        rows.into_iter()
            .map(|row| AgentCredentialBinding::from_row(row, &public_id))
            .collect(),
    )))
}

#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/credentials",
    description = "Declare a write-only credential binding for one MCP tool parameter. The secret value is provisioned separately.",
    params(("agent_id" = String, Path, description = "Agent ID")),
    request_body = CreateAgentCredentialBinding,
    responses((status = 200, description = "Credential binding created or updated", body = AgentCredentialBinding)),
    tag = "agents"
)]
pub async fn create_credential_binding(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(agent_id): Path<String>,
    Json(mut body): Json<CreateAgentCredentialBinding>,
) -> ApiResult<AgentCredentialBinding> {
    body.agent_id = agent_id;
    Ok(Json(body.run(&state.ctx(&org)).await?))
}

#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/credentials/{binding_id}",
    description = "Encrypt and replace the write-only value for an agent credential binding.",
    params(
        ("agent_id" = String, Path, description = "Agent ID"),
        ("binding_id" = Uuid, Path, description = "Credential binding ID")
    ),
    request_body = SetAgentCredentialValueRequest,
    responses((status = 200, description = "Credential value replaced", body = AgentCredentialBinding)),
    tag = "agents"
)]
pub async fn set_credential_value(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path((agent_id, binding_id)): Path<(String, Uuid)>,
    Json(body): Json<SetAgentCredentialValueRequest>,
) -> ApiResult<AgentCredentialBinding> {
    let (internal_id, public_id) = resolve_agent(&state, &org, &agent_id).await?;
    if body.value.is_empty() || body.value.len() > 64 * 1024 {
        return Err(
            ErrorResponse::new("Credential value must be between 1 byte and 64 KiB")
                .into_response(StatusCode::BAD_REQUEST),
        );
    }
    let change = credential_change(
        &state,
        &org,
        &public_id,
        "set_agent_credential_value",
        ChangeAction::Updated,
    )
    .await?;
    let encryption = state
        .encryption
        .as_ref()
        .ok_or_else(ErrorResponse::internal_error)?;
    let encrypted = encryption
        .encrypt_string(&body.value)
        .map_err(|_| ErrorResponse::internal_error())?;
    let row = state
        .db
        .set_agent_mcp_secret_binding_value(org.org_id, internal_id, binding_id, encrypted)
        .await
        .map_err(|_| ErrorResponse::internal_error())?
        .ok_or_else(|| ErrorResponse::not_found("Credential binding"))?;
    change.finish(&public_id).await;
    Ok(Json(AgentCredentialBinding::from_row(row, &public_id)))
}

#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/credentials/{binding_id}",
    description = "Revoke an agent credential binding and permanently remove its encrypted value.",
    params(
        ("agent_id" = String, Path, description = "Agent ID"),
        ("binding_id" = Uuid, Path, description = "Credential binding ID")
    ),
    responses(
        (status = 204, description = "Credential binding revoked"),
        (status = 404, description = "Credential binding not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn delete_credential_binding(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path((agent_id, binding_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let (internal_id, public_id) = resolve_agent(&state, &org, &agent_id).await?;
    let change = credential_change(
        &state,
        &org,
        &public_id,
        "delete_agent_credential_binding",
        ChangeAction::Detached,
    )
    .await?;
    let deleted = state
        .db
        .delete_agent_mcp_secret_binding(org.org_id, internal_id, binding_id)
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    if deleted {
        change.finish(&public_id).await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("Credential binding"))
    }
}

/// Credential values are written here, not through a command, so the change
/// is recorded on the parent agent's history (`change_history::rest`). The
/// value itself never reaches history.
async fn credential_change(
    state: &ApiState,
    org: &ResolvedOrg,
    agent_ref: &str,
    operation: &'static str,
    action: ChangeAction,
) -> Result<RestChange, (StatusCode, Json<ErrorResponse>)> {
    let (db, caller, kind) = (state.db.clone(), Caller::from(org), EntityKind::Agent);
    let fields = &["credentials"];
    Ok(RestChange::begin(db, caller, operation, kind, action, Some(agent_ref), fields).await?)
}
