use crate::auth::ResolvedOrg;
use crate::domains::agent_endpoints::types::{
    CreateAgentEndpointRequest, UpdateAgentEndpointRequest,
};
use crate::domains::agent_endpoints::{
    CreateAgentEndpoint, DeleteAgentEndpoint, GetAgentEndpoint, ListAgentEndpoints,
    PublishAgentEndpoint, TriggerAgentEndpoint, TriggerAgentEndpointOutput, UnpublishAgentEndpoint,
    UpdateAgentEndpointCmd,
};
use crate::domains::common::Command;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_platform::AppChannel;
use serde_json::Value;

use super::common::{ApiResult, ErrorResponse};

pub type AppState = super::agent_triggers::AppState;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/endpoints",
            get(list_agent_endpoints).post(create_agent_endpoint),
        )
        .route(
            "/v1/agents/{agent_id}/endpoints/{endpoint_id}",
            get(get_agent_endpoint)
                .patch(update_agent_endpoint)
                .delete(delete_agent_endpoint),
        )
        .route(
            "/v1/agents/{agent_id}/endpoints/{endpoint_id}/publish",
            post(publish_agent_endpoint),
        )
        .route(
            "/v1/agents/{agent_id}/endpoints/{endpoint_id}/unpublish",
            post(unpublish_agent_endpoint),
        )
        .route(
            "/v1/agents/{agent_id}/endpoints/{endpoint_id}/trigger",
            post(trigger_agent_endpoint),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "List ingress endpoints owned by an Agent.",
    get,
    path = "/v1/agents/{agent_id}/endpoints",
    params(("agent_id" = String, Path, description = "Agent ID or name")),
    responses(
        (status = 200, description = "Agent endpoints", body = Vec<AppChannel>),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn list_agent_endpoints(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<Vec<AppChannel>> {
    let endpoints = ListAgentEndpoints { agent_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(endpoints))
}

#[utoipa::path(
    description = "Create an ingress endpoint owned by an Agent.",
    post,
    path = "/v1/agents/{agent_id}/endpoints",
    params(("agent_id" = String, Path, description = "Agent ID or name")),
    request_body = CreateAgentEndpointRequest,
    responses(
        (status = 201, description = "Endpoint created", body = AppChannel),
        (status = 400, description = "Invalid endpoint", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn create_agent_endpoint(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateAgentEndpointRequest>,
) -> Result<(StatusCode, Json<AppChannel>), (StatusCode, Json<ErrorResponse>)> {
    let endpoint = CreateAgentEndpoint { agent_id, req }
        .run(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(endpoint)))
}

#[utoipa::path(
    description = "Get one ingress endpoint owned by an Agent.",
    get,
    path = "/v1/agents/{agent_id}/endpoints/{endpoint_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("endpoint_id" = String, Path, description = "Endpoint ID")
    ),
    responses(
        (status = 200, description = "Agent endpoint", body = AppChannel),
        (status = 404, description = "Endpoint not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn get_agent_endpoint(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, endpoint_id)): Path<(String, String)>,
) -> ApiResult<AppChannel> {
    let endpoint = GetAgentEndpoint {
        agent_id,
        endpoint_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(endpoint))
}

#[utoipa::path(
    description = "Update an ingress endpoint owned by an Agent.",
    patch,
    path = "/v1/agents/{agent_id}/endpoints/{endpoint_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("endpoint_id" = String, Path, description = "Endpoint ID")
    ),
    request_body = UpdateAgentEndpointRequest,
    responses(
        (status = 200, description = "Endpoint updated", body = AppChannel),
        (status = 400, description = "Invalid endpoint", body = ErrorResponse),
        (status = 404, description = "Endpoint not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn update_agent_endpoint(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, endpoint_id)): Path<(String, String)>,
    Json(req): Json<UpdateAgentEndpointRequest>,
) -> ApiResult<AppChannel> {
    let endpoint = UpdateAgentEndpointCmd {
        agent_id,
        endpoint_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(endpoint))
}

#[utoipa::path(
    description = "Delete an ingress endpoint owned by an Agent.",
    delete,
    path = "/v1/agents/{agent_id}/endpoints/{endpoint_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("endpoint_id" = String, Path, description = "Endpoint ID")
    ),
    responses(
        (status = 200, description = "Endpoint deleted"),
        (status = 404, description = "Endpoint not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn delete_agent_endpoint(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, endpoint_id)): Path<(String, String)>,
) -> ApiResult<Value> {
    let result = DeleteAgentEndpoint {
        agent_id,
        endpoint_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(result))
}

#[utoipa::path(
    description = "Publish an Agent endpoint so it can accept ingress traffic.",
    post,
    path = "/v1/agents/{agent_id}/endpoints/{endpoint_id}/publish",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("endpoint_id" = String, Path, description = "Endpoint ID")
    ),
    responses(
        (status = 200, description = "Endpoint published", body = AppChannel),
        (status = 404, description = "Endpoint not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn publish_agent_endpoint(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, endpoint_id)): Path<(String, String)>,
) -> ApiResult<AppChannel> {
    let endpoint = PublishAgentEndpoint {
        agent_id,
        endpoint_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(endpoint))
}

#[utoipa::path(
    description = "Unpublish an Agent endpoint so it no longer accepts ingress traffic.",
    post,
    path = "/v1/agents/{agent_id}/endpoints/{endpoint_id}/unpublish",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("endpoint_id" = String, Path, description = "Endpoint ID")
    ),
    responses(
        (status = 200, description = "Endpoint unpublished", body = AppChannel),
        (status = 404, description = "Endpoint not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn unpublish_agent_endpoint(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, endpoint_id)): Path<(String, String)>,
) -> ApiResult<AppChannel> {
    let endpoint = UnpublishAgentEndpoint {
        agent_id,
        endpoint_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(endpoint))
}

#[utoipa::path(
    description = "Run a published Agent schedule endpoint now.",
    post,
    path = "/v1/agents/{agent_id}/endpoints/{endpoint_id}/trigger",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("endpoint_id" = String, Path, description = "Endpoint ID")
    ),
    responses(
        (status = 200, description = "Endpoint triggered", body = TriggerAgentEndpointOutput),
        (status = 400, description = "Endpoint cannot run", body = ErrorResponse),
        (status = 404, description = "Endpoint not found", body = ErrorResponse)
    ),
    tag = "agent-endpoints"
)]
pub async fn trigger_agent_endpoint(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, endpoint_id)): Path<(String, String)>,
) -> ApiResult<TriggerAgentEndpointOutput> {
    let result = TriggerAgentEndpoint {
        agent_id,
        endpoint_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(result))
}
