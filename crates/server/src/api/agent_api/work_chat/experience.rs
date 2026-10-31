//! Consumer presentation controls. Authorization remains on the existing
//! channel/caller/session boundary; published links carry read authority only.
use super::*;
use everruns_contracts::typed_id::ModelId;

pub(super) fn routes() -> Router<ChannelApiState> {
    let mut routes = Router::new();
    for prefix in ["/v1/channels", "/v1/e"] {
        routes = routes
            .route(
                &format!("{prefix}/{{channel_id}}/sessions/{{session_id}}/settings"),
                post(update_work_chat_conversation),
            )
            .route(
                &format!("{prefix}/{{channel_id}}/sessions/{{session_id}}/share"),
                post(share_work_chat_conversation).delete(revoke_work_chat_share),
            )
            .route(
                &format!("{prefix}/{{channel_id}}/chat/shared/{{token}}"),
                get(read_work_chat_share),
            );
    }
    routes
}
pub(in crate::api::agent_api) fn controls(
    auth: &Authorized,
    value: Option<Value>,
) -> ApiResponse<Option<everruns_contracts::runtime::Controls>> {
    // Other execution channels retain their existing fixed-model contract.
    if auth.channel.channel_type != ChannelType::WorkChat {
        return Ok(None);
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Controls {
        model_id: Option<ModelId>,
        reasoning: Option<everruns_contracts::runtime::ReasoningConfig>,
    }
    let Some(value) = value else {
        return Ok(None);
    };
    let controls: Controls = serde_json::from_value(value)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid chat controls"))?;
    if controls.model_id.is_some() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "This chat uses the Agent's model",
        ));
    }
    Ok(Some(everruns_contracts::runtime::Controls {
        reasoning: controls.reasoning,
        ..Default::default()
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    title: Option<String>,
    archived: Option<bool>,
    pinned: Option<bool>,
}
#[utoipa::path(post, operation_id="update_work_chat_conversation", path="/v1/channels/{channel_id}/sessions/{session_id}/settings", params(("channel_id"=String,Path),("session_id"=String,Path)), responses((status=200,description="Updated"),(status=404,description="Unavailable")),tag="agent-execution")]
pub(crate) async fn update_work_chat_conversation(
    State(state): State<ChannelApiState>,
    Path((channel, session)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Settings>,
) -> Response {
    let result = async {
        let auth = authorize_by_id(&state, &channel, &headers, None).await?;
        let cfg = config(&auth.channel)?;
        let row = callers_session(&state, &auth, &session).await?;
        if cfg.interface_mode == ChatInterface::SingleWithThreads && row.parent_session_id.is_none()
        {
            return Err(error(
                StatusCode::CONFLICT,
                "The main conversation stays open",
            ));
        }
        if body
            .title
            .as_ref()
            .is_some_and(|title| title.trim().is_empty() || title.chars().count() > 120)
        {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "Use a title between 1 and 120 characters",
            ));
        }
        let tags = body.pinned.map(|pinned| {
            let mut tags = row.tags.clone();
            tags.retain(|t| t != "work_chat:pinned");
            if pinned {
                tags.push("work_chat:pinned".into());
            }
            tags
        });
        state
            .db
            .update_session(
                auth.context.org_id,
                row.id,
                crate::storage::UpdateSession {
                    title: body.title,
                    tags,
                    ..Default::default()
                },
            )
            .await
            .map_err(internal_error)?;
        if let Some(archived) = body.archived {
            state
                .db
                .set_session_archived(auth.context.org_id, row.id, archived)
                .await
                .map_err(internal_error)?;
        }
        let row = callers_session(&state, &auth, &session).await?;
        Ok(Json(AgentSessionView::from(&row)).into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}
async fn publish(
    state: ChannelApiState,
    channel: String,
    session: String,
    headers: HeaderMap,
    revoke: bool,
) -> Response {
    let result = async {
        let auth = authorize_by_id(&state, &channel, &headers, None).await?;
        config(&auth.channel)?;
        let row = callers_session(&state, &auth, &session).await?;
        let token = uuid::Uuid::new_v4().to_string();
        state
            .db
            .replace_work_chat_share(
                auth.channel.internal_id,
                row.id.uuid(),
                (!revoke).then_some(token.as_str()),
            )
            .await
            .map_err(internal_error)?;
        Ok(Json(if revoke {
            json!({"revoked":true})
        } else {
            json!({"token":token})
        })
        .into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}
#[utoipa::path(post, operation_id="share_work_chat_conversation", path="/v1/channels/{channel_id}/sessions/{session_id}/share", params(("channel_id"=String,Path),("session_id"=String,Path)), responses((status=200,description="Revocable link token"),(status=404,description="Unavailable")),tag="agent-execution")]
pub(crate) async fn share_work_chat_conversation(
    State(state): State<ChannelApiState>,
    Path((channel, session)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    publish(state, channel, session, headers, false).await
}
#[utoipa::path(delete, operation_id="revoke_work_chat_share", path="/v1/channels/{channel_id}/sessions/{session_id}/share", params(("channel_id"=String,Path),("session_id"=String,Path)), responses((status=200,description="Revoked"),(status=404,description="Unavailable")),tag="agent-execution")]
pub(crate) async fn revoke_work_chat_share(
    State(state): State<ChannelApiState>,
    Path((channel, session)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    publish(state, channel, session, headers, true).await
}

#[derive(Deserialize)]
pub(crate) struct History {
    before: Option<i32>,
}
#[utoipa::path(get, operation_id="read_work_chat_share", path="/v1/channels/{channel_id}/chat/shared/{token}", params(("channel_id"=String,Path),("token"=String,Path),("before"=Option<i32>,Query)), responses((status=200,description="Read-only transcript page"),(status=404,description="Unavailable")),tag="agent-execution")]
pub(crate) async fn read_work_chat_share(
    State(state): State<ChannelApiState>,
    Path((channel, token)): Path<(String, String)>,
    Query(query): Query<History>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let result = async {
        let (context, channel) = resolve_api_channel(&state, &channel).await?;
        channel_liveness(&context, &channel).map_err(|_| not_found())?;
        let cfg = config(&channel)?;
        let peer = connect_info.map(|Extension(ConnectInfo(peer))| peer);
        if state
            .rate_limiter
            .check(
                &format!("work_chat_shared:{}", channel.public_id),
                crate::auth::rate_limit::extract_client_ip_from_parts(peer, &headers),
                60,
            )
            .await
            .is_err()
        {
            return Err(error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many requests. Please try again shortly",
            ));
        }
        if uuid::Uuid::parse_str(&token).is_err() {
            return Err(not_found());
        }
        // THREAT[TM-WEB-023]: a share authorizes transcript reads only; it is never
        // accepted by caller/session/file/approval routes or used to mint a token.
        let id = state
            .db
            .channel_binding(channel.internal_id, &format!("work_chat:share:{token}"))
            .await
            .map_err(internal_error)?
            .ok_or_else(not_found)?;
        let row = state
            .db
            .get_session(context.org_id, SessionId::from_uuid(id))
            .await
            .map_err(internal_error)?
            .filter(|row| row.channel_id == Some(channel.internal_id))
            .ok_or_else(not_found)?;
        let mut events = state
            .db
            .work_chat_share_events(row.id, query.before)
            .await
            .map_err(internal_error)?;
        let before = if events.len() > 100 {
            events.remove(0);
            events.first().map(|event| event.sequence)
        } else {
            None
        };
        let data: Vec<Value> = events
            .into_iter()
            .filter_map(|event| {
                project(
                    json!({
                        "id": event.id,
                        "type": event.event_type,
                        "ts": event.ts,
                        "sequence": event.sequence,
                        "data": event.data,
                    }),
                    &cfg.execution,
                )
            })
            .collect();
        Ok((
            [("cache-control", "no-store")],
            Json(json!({"title": row.title, "data": data, "before": before})),
        )
            .into_response())
    }
    .await;
    result.unwrap_or_else(|response| response)
}
