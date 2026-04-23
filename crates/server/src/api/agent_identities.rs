// Agent identity CRUD HTTP routes.
// Routes use ResolvedOrg: org derived from auth context (API key or cookie).

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::agent_identities::types::{
    CreateAgentIdentityRequest, ListAgentIdentitiesQuery, UpdateAgentIdentityRequest,
};
use crate::domains::agent_identities::{
    AGENT_IDENTITY_DANGEROUS, AGENT_IDENTITY_MANAGE, AGENT_IDENTITY_VIEW,
};
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{AgentIdentity, Caller, ResourceConfigResponse, evaluate_policies_with};
use std::sync::Arc;

use super::common::{ErrorResponse, ListResponse, impl_auth_state};
use crate::domains::common::Command;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            capability_service,
            auth,
        }
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agent-identities/config", get(agent_identity_config))
        .route(
            "/v1/agent-identities",
            post(create_agent_identity).get(list_agent_identities),
        )
        .route(
            "/v1/agent-identities/{identity_id}",
            get(get_agent_identity)
                .patch(update_agent_identity)
                .delete(delete_agent_identity),
        )
        .route(
            "/v1/agent-identities/{identity_id}/delete",
            post(destroy_agent_identity),
        )
        .with_state(state)
}

pub async fn agent_identity_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[
            &AGENT_IDENTITY_VIEW,
            &AGENT_IDENTITY_MANAGE,
            &AGENT_IDENTITY_DANGEROUS,
        ],
    );
    Json(ResourceConfigResponse { policies })
}

pub async fn create_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateAgentIdentityRequest>,
) -> Result<(StatusCode, Json<AgentIdentity>), (StatusCode, Json<ErrorResponse>)> {
    let identity = crate::domains::agent_identities::CreateAgentIdentity(req)
        .run(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(identity)))
}

pub async fn list_agent_identities(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAgentIdentitiesQuery>,
) -> Result<Json<ListResponse<AgentIdentity>>, (StatusCode, Json<ErrorResponse>)> {
    let identities = crate::domains::agent_identities::ListAgentIdentities {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(ListResponse::new(identities)))
}

pub async fn get_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<Json<AgentIdentity>, (StatusCode, Json<ErrorResponse>)> {
    let identity = crate::domains::agent_identities::GetAgentIdentity { id: identity_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(identity))
}

pub async fn update_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
    Json(req): Json<UpdateAgentIdentityRequest>,
) -> Result<Json<AgentIdentity>, (StatusCode, Json<ErrorResponse>)> {
    let identity = crate::domains::agent_identities::UpdateAgentIdentityCmd {
        id: identity_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(identity))
}

pub async fn delete_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::agent_identities::DeleteAgentIdentity { id: identity_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn destroy_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::agent_identities::DestroyAgentIdentity { id: identity_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
