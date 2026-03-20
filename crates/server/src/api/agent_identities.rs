// Agent identity CRUD HTTP routes.
// Routes use ResolvedOrg: org derived from auth context (API key or cookie).

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::AgentIdentityService;
use crate::services::agent_identity::{
    AGENT_IDENTITY_DANGEROUS, AGENT_IDENTITY_MANAGE, AGENT_IDENTITY_VIEW,
};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{
    AgentIdentity, AgentIdentityId, AgentIdentityStatus, Caller, ResourceConfigResponse,
    evaluate_policies_with,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use super::common::{ApiOptionExt, ApiPolicyResultExt, ErrorResponse, ListResponse};

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAgentIdentityRequest {
    #[schema(example = "Ops Bot")]
    pub name: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    #[schema(example = "en-US")]
    pub locale: Option<String>,
    #[schema(example = "America/Los_Angeles")]
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentIdentityRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub status: Option<AgentIdentityStatus>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAgentIdentitiesQuery {
    pub search: Option<String>,
    pub include_archived: Option<bool>,
}

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AgentIdentityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            service: Arc::new(AgentIdentityService::new(db)),
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

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
    let caller = Caller::from(&org);
    let identity = state
        .service
        .create(&caller, req)
        .await
        .map_policy_or_internal("create agent identity")?;
    Ok((StatusCode::CREATED, Json(identity)))
}

pub async fn list_agent_identities(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAgentIdentitiesQuery>,
) -> Result<Json<ListResponse<AgentIdentity>>, (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(&org);
    let identities = state
        .service
        .list(
            &caller,
            query.search.as_deref(),
            query.include_archived.unwrap_or(false),
        )
        .await
        .map_policy_or_internal("list agent identities")?;
    Ok(Json(ListResponse::new(identities)))
}

pub async fn get_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<Json<AgentIdentity>, (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let identity = state
        .service
        .get(&caller, identity_id)
        .await
        .map_policy_or_internal("get agent identity")?
        .ok_or_not_found_json("Agent identity")?;
    Ok(Json(identity))
}

pub async fn update_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
    Json(req): Json<UpdateAgentIdentityRequest>,
) -> Result<Json<AgentIdentity>, (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let identity = state
        .service
        .update(&caller, identity_id, req)
        .await
        .map_policy_or_internal("update agent identity")?
        .ok_or_not_found_json("Agent identity")?;
    Ok(Json(identity))
}

pub async fn delete_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, identity_id)
        .await
        .map_policy_or_internal("archive agent identity")?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Agent identity not found".to_string())
            .into_response(StatusCode::NOT_FOUND))
    }
}

pub async fn destroy_agent_identity(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let deleted = state
        .service
        .destroy(&caller, identity_id)
        .await
        .map_policy_or_internal("delete agent identity")?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Agent identity not found".to_string())
            .into_response(StatusCode::NOT_FOUND))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_agent_identity_request_roundtrip() {
        let json = r#"{"name":"Ops Bot","locale":"en-US","timezone":"America/Los_Angeles"}"#;
        let req: CreateAgentIdentityRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name, "Ops Bot");
        assert_eq!(req.locale.as_deref(), Some("en-US"));
        assert_eq!(req.timezone.as_deref(), Some("America/Los_Angeles"));
    }

    #[test]
    fn test_update_agent_identity_request_status() {
        let json = r#"{"status":"archived"}"#;
        let req: UpdateAgentIdentityRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.status, Some(AgentIdentityStatus::Archived));
    }
}
