use super::common::ApiResult;
use crate::{
    auth::ResolvedOrg,
    domains::{common::Command, external_agents::*},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
pub type AppState = super::agent_triggers::AppState;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/external-worker",
            get(get_external_agent)
                .post(register_external_agent)
                .delete(disable_external_agent),
        )
        .with_state(state)
}

#[utoipa::path(get, path = "/v1/agents/{agent_id}/external-worker", params(("agent_id" = String, Path)), responses((status = 200, body = Option<ExternalAgent>)), tag = "agents")]
pub async fn get_external_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<Option<ExternalAgent>> {
    Ok(Json(
        GetExternalAgent { agent_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(post, path = "/v1/agents/{agent_id}/external-worker", params(("agent_id" = String, Path)), request_body = RegisterExternalAgent, responses((status = 200, body = ExternalAgent)), tag = "agents")]
pub async fn register_external_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(mut request): Json<RegisterExternalAgent>,
) -> ApiResult<ExternalAgent> {
    request.agent_id = agent_id;
    Ok(Json(request.run(&state.ctx(&org)).await?))
}

#[utoipa::path(delete, path = "/v1/agents/{agent_id}/external-worker", params(("agent_id" = String, Path)), responses((status = 200)), tag = "agents")]
pub async fn disable_external_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<()> {
    Ok(Json(
        DisableExternalAgent { agent_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}
