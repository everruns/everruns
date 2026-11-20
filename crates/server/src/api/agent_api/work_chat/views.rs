//! The public bootstrap exposes placement only. Signed-in consumers read
//! content through the same channel and session checks as their conversation.

use super::*;
use crate::domains::agent_channels::record::work_chat::WorkChatViewPlacement;
use crate::domains::agents::view_publication::{self, AgentViewPreference};
use axum::extract::Query;
use everruns_contracts::typed_id::AgentId;
use uuid::Uuid;

pub(super) fn routes() -> Router<ChannelApiState> {
    ["/v1/channels", "/v1/e"]
        .into_iter()
        .fold(Router::new(), |router, prefix| {
            router.route(
                &format!("{prefix}/{{channel_id}}/chat/view"),
                get(get_work_chat_view).put(set_work_chat_view_preference),
            )
        })
}

pub(super) async fn placement(
    state: &ChannelApiState,
    context: &IngressContext,
    config: &WorkChatConfig,
) -> WorkChatViewPlacement {
    if config.view_placement == WorkChatViewPlacement::None {
        return WorkChatViewPlacement::None;
    }
    let Some(auth) = &state.runtime_auth else {
        return WorkChatViewPlacement::None;
    };
    let enabled = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        context.org_id,
        &auth.feature_flag_policy,
    )
    .await
    .is_ok_and(|flags| flags.agent_views);
    if enabled {
        config.view_placement
    } else {
        WorkChatViewPlacement::None
    }
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ViewQuery {
    session: Option<String>,
}

struct Target {
    org: i64,
    agent: AgentId,
    session: Option<SessionId>,
    audience: String,
    viewer: Uuid,
}

async fn target(
    state: &ChannelApiState,
    channel: &str,
    headers: &HeaderMap,
    query: ViewQuery,
) -> ApiResponse<Target> {
    // THREAT[TM-TENANT-024]: derive the Agent and viewer from the authorized channel.
    let auth = authorize_by_id(state, channel, headers, None).await?;
    let cfg = config(&auth.channel)?;
    if placement(state, &auth.context, &cfg).await == WorkChatViewPlacement::None {
        return Err(not_found());
    }
    let viewer = auth
        .caller
        .end_user_id()
        .ok_or_else(not_found)?
        .virtual_user_id
        .uuid();
    let agent = AgentId::from_uuid(auth.context.agent_internal_id);
    let session = if let Some(session) = query.session {
        let row = callers_session(state, &auth, &session).await?;
        (row.agent_id == Some(agent)).then_some(row.id)
    } else {
        None
    };
    Ok(Target {
        org: auth.context.org_id,
        agent,
        session,
        audience: format!("work_chat:{}", auth.channel.public_id),
        viewer,
    })
}

async fn read(state: &ChannelApiState, target: Target) -> ApiResponse<Response> {
    let view = view_publication::read(
        &state.db,
        target.org,
        target.agent,
        target.session,
        Some((&target.audience, target.viewer)),
        true,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    Ok(Json(view).into_response())
}

#[utoipa::path(
    get, path = "/v1/channels/{channel_id}/chat/view",
    params(("channel_id" = String, Path), ViewQuery),
    responses((status = 200, description = "Published view", body = view_publication::PublishedAgentView),
        (status = 404, description = "View unavailable")),
    tag = "agent-execution"
)]
pub(crate) async fn get_work_chat_view(
    State(state): State<ChannelApiState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Query(query): Query<ViewQuery>,
) -> Response {
    async {
        let target = target(&state, &channel, &headers, query).await?;
        read(&state, target).await
    }
    .await
    .unwrap_or_else(|response| response)
}

#[utoipa::path(
    put, path = "/v1/channels/{channel_id}/chat/view",
    params(("channel_id" = String, Path), ViewQuery),
    request_body = AgentViewPreference,
    responses((status = 200, description = "Saved personal layout", body = view_publication::PublishedAgentView),
        (status = 404, description = "View unavailable")),
    tag = "agent-execution"
)]
pub(crate) async fn set_work_chat_view_preference(
    State(state): State<ChannelApiState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Query(query): Query<ViewQuery>,
    Json(preference): Json<AgentViewPreference>,
) -> Response {
    async {
        let target = target(&state, &channel, &headers, query).await?;
        view_publication::save_preference(
            &state.db,
            target.agent,
            &target.audience,
            target.viewer,
            preference,
        )
        .await
        .map_err(|error| {
            let response: (StatusCode, Json<crate::api::common::ErrorResponse>) = error.into();
            response.into_response()
        })?;
        read(&state, target).await
    }
    .await
    .unwrap_or_else(|response| response)
}
