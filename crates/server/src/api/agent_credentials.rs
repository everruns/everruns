use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::agents::AGENT_MANAGE;
use crate::domains::agents::credentials::{AgentCredentialBinding, CreateAgentCredentialBinding};
use crate::domains::common::{Command, Ctx};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{AgentId, Caller};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::{ApiResult, ErrorResponse, ListResponse, impl_auth_state};

#[derive(Clone)]
pub struct AppState {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);

#[derive(Deserialize, ToSchema)]
pub struct SetAgentCredentialValueRequest {
    /// Write-only credential value. It is encrypted and never returned.
    pub value: String,
}

pub fn routes(state: AppState) -> Router {
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
    state: &AppState,
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
    params(("agent_id" = String, Path)),
    responses((status = 200, body = ListResponse<AgentCredentialBinding>)),
    tag = "agents"
)]
pub async fn list_credentials(
    org: ResolvedOrg,
    State(state): State<AppState>,
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
    params(("agent_id" = String, Path)),
    request_body = CreateAgentCredentialBinding,
    responses((status = 200, body = AgentCredentialBinding)),
    tag = "agents"
)]
pub async fn create_credential_binding(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(mut body): Json<CreateAgentCredentialBinding>,
) -> ApiResult<AgentCredentialBinding> {
    body.agent_id = agent_id;
    Ok(Json(body.run(&state.ctx(&org)).await?))
}

#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/credentials/{binding_id}",
    params(("agent_id" = String, Path), ("binding_id" = Uuid, Path)),
    request_body = SetAgentCredentialValueRequest,
    responses((status = 200, body = AgentCredentialBinding)),
    tag = "agents"
)]
pub async fn set_credential_value(
    org: ResolvedOrg,
    State(state): State<AppState>,
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
    Ok(Json(AgentCredentialBinding::from_row(row, &public_id)))
}

#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/credentials/{binding_id}",
    params(("agent_id" = String, Path), ("binding_id" = Uuid, Path)),
    responses((status = 204), (status = 404, body = ErrorResponse)),
    tag = "agents"
)]
pub async fn delete_credential_binding(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, binding_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let (internal_id, _) = resolve_agent(&state, &org, &agent_id).await?;
    let deleted = state
        .db
        .delete_agent_mcp_secret_binding(org.org_id, internal_id, binding_id)
        .await
        .map_err(|_| ErrorResponse::internal_error())?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("Credential binding"))
    }
}
