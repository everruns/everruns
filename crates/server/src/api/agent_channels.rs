use crate::auth::ResolvedOrg;
use crate::domains::agent_channels::types::{CreateAgentChannelRequest, UpdateAgentChannelRequest};
use crate::domains::agent_channels::{
    CreateAgentChannel, DeleteAgentChannel, GetAgentChannel, ListAgentChannels,
    PublishAgentChannel, TriggerAgentChannel, TriggerAgentChannelOutput, UnpublishAgentChannel,
    UpdateAgentChannelCmd,
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
            "/v1/agents/{agent_id}/channels",
            get(list_agent_channels).post(create_agent_channel),
        )
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}",
            get(get_agent_channel)
                .patch(update_agent_channel)
                .delete(delete_agent_channel),
        )
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}/publish",
            post(publish_agent_channel),
        )
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}/unpublish",
            post(unpublish_agent_channel),
        )
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}/trigger",
            post(trigger_agent_channel),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "List ingress channels owned by an Agent.",
    get,
    path = "/v1/agents/{agent_id}/channels",
    params(("agent_id" = String, Path, description = "Agent ID or name")),
    responses(
        (status = 200, description = "Agent channels", body = Vec<AppChannel>),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn list_agent_channels(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<Vec<AppChannel>> {
    let channels = ListAgentChannels { agent_id }.run(&state.ctx(&org)).await?;
    Ok(Json(channels))
}

#[utoipa::path(
    description = "Create an ingress channel owned by an Agent.",
    post,
    path = "/v1/agents/{agent_id}/channels",
    params(("agent_id" = String, Path, description = "Agent ID or name")),
    request_body = CreateAgentChannelRequest,
    responses(
        (status = 201, description = "Channel created", body = AppChannel),
        (status = 400, description = "Invalid channel", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn create_agent_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateAgentChannelRequest>,
) -> Result<(StatusCode, Json<AppChannel>), (StatusCode, Json<ErrorResponse>)> {
    let channel = CreateAgentChannel { agent_id, req }
        .run(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(channel)))
}

#[utoipa::path(
    description = "Get one ingress channel owned by an Agent.",
    get,
    path = "/v1/agents/{agent_id}/channels/{channel_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "Channel ID")
    ),
    responses(
        (status = 200, description = "Agent channel", body = AppChannel),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn get_agent_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
) -> ApiResult<AppChannel> {
    let channel = GetAgentChannel {
        agent_id,
        channel_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(channel))
}

#[utoipa::path(
    description = "Update an ingress channel owned by an Agent.",
    patch,
    path = "/v1/agents/{agent_id}/channels/{channel_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "Channel ID")
    ),
    request_body = UpdateAgentChannelRequest,
    responses(
        (status = 200, description = "Channel updated", body = AppChannel),
        (status = 400, description = "Invalid channel", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn update_agent_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
    Json(req): Json<UpdateAgentChannelRequest>,
) -> ApiResult<AppChannel> {
    let channel = UpdateAgentChannelCmd {
        agent_id,
        channel_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(channel))
}

#[utoipa::path(
    description = "Delete an ingress channel owned by an Agent.",
    delete,
    path = "/v1/agents/{agent_id}/channels/{channel_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "Channel ID")
    ),
    responses(
        (status = 200, description = "Channel deleted"),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn delete_agent_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
) -> ApiResult<Value> {
    let result = DeleteAgentChannel {
        agent_id,
        channel_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(result))
}

#[utoipa::path(
    description = "Publish an Agent channel so it can accept ingress traffic.",
    post,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/publish",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "Channel ID")
    ),
    responses(
        (status = 200, description = "Channel published", body = AppChannel),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn publish_agent_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
) -> ApiResult<AppChannel> {
    let channel = PublishAgentChannel {
        agent_id,
        channel_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(channel))
}

#[utoipa::path(
    description = "Unpublish an Agent channel so it no longer accepts ingress traffic.",
    post,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/unpublish",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "Channel ID")
    ),
    responses(
        (status = 200, description = "Channel unpublished", body = AppChannel),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn unpublish_agent_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
) -> ApiResult<AppChannel> {
    let channel = UnpublishAgentChannel {
        agent_id,
        channel_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(channel))
}

#[utoipa::path(
    description = "Run a published Agent schedule channel now.",
    post,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/trigger",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "Channel ID")
    ),
    responses(
        (status = 200, description = "Channel triggered", body = TriggerAgentChannelOutput),
        (status = 400, description = "Channel cannot run", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn trigger_agent_channel(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
) -> ApiResult<TriggerAgentChannelOutput> {
    let result = TriggerAgentChannel {
        agent_id,
        channel_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(result))
}
