// Agent Execution API routes for `api` channels, rooted at the agent base URL
// `/v1/channels/{channel_id}`.
//
// Decisions:
// - Same routes and shapes as serve's per-agent API (`everruns::execution_api`),
//   so the SDK's agent client works against either host.
// - The base URL is shared with the frozen `api_endpoint` channel, whose
//   handlers own `POST …/sessions`, `GET …/sessions/{id}`, `…/messages` and
//   `…/cancel`. Those handlers resolve the channel first and hand `api`
//   channels to the functions here (`channel_api::channel_door`); the other
//   routes are registered here and answer 404 for any other channel type.
// - Only an agent key of this channel reaches these routes. They have no
//   handler path into management state (THREAT[TM-AGENTKEY-001]).
// Domain rules (caller confinement, visibility) live in
// `domains::agent_channels::api_sessions`.

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use everruns_contracts::execution_api::{
    AgentCard, AgentCardAuth, AgentCardInput, AgentCardLinks, CreateAgentSessionRequest,
};
use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use serde_json::{Value, json};

use super::channel_api::ChannelApiState;
use super::channel_auth::extract_bearer;
use super::channel_ingress::{IngressChannel, IngressContext, channel_liveness, resolve_channel};
use super::common::ErrorResponse;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::agent_channels::api_sessions::{
    AgentSessionView, ApiAuthError, ApiCaller, authorize_agent_key, create_api_session,
    project_event, send_api_message, session_is_callers, visible_event_types,
};
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::messages::types::InputMessage;
use crate::storage::SessionRow;

/// Default and largest page of `GET …/sessions` and `GET …/events`.
const DEFAULT_SESSION_PAGE: u32 = 50;
const MAX_SESSION_PAGE: u32 = 200;
const DEFAULT_EVENT_PAGE: i32 = 100;
const MAX_EVENT_PAGE: i32 = 500;

type ApiResponse<T> = Result<T, Response>;

pub fn routes(state: ChannelApiState) -> Router {
    Router::new()
        .route("/v1/channels/{channel_id}", get(get_card))
        .route("/v1/channels/{channel_id}/sessions", get(list_sessions))
        .route(
            "/v1/channels/{channel_id}/sessions/{session_id}/events",
            get(list_events),
        )
        .with_state(state)
}

/// A request that passed the channel, liveness, key and rate-limit gates.
pub(crate) struct Authorized {
    context: IngressContext,
    channel: IngressChannel,
    caller: ApiCaller,
}

/// The channel when it is an `api` channel, `None` for any other type.
async fn resolve_api_channel(
    state: &ChannelApiState,
    channel_id: &str,
) -> ApiResponse<(IngressContext, IngressChannel)> {
    match resolve_channel(&state.db, state.encryption.as_ref(), channel_id).await {
        Ok(Some((context, channel))) if channel.channel_type == ChannelType::Api => {
            Ok((context, channel))
        }
        Ok(_) => Err(not_found()),
        Err(error) => Err(internal_error(error)),
    }
}

/// Liveness, then the agent key, then the rate limit.
pub(crate) async fn authorize(
    state: &ChannelApiState,
    context: IngressContext,
    channel: IngressChannel,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
) -> ApiResponse<Authorized> {
    // THREAT[TM-AGENTKEY-005]: liveness before the key, with a generic answer.
    if channel_liveness(&context, &channel).is_err() {
        return Err(error(StatusCode::FORBIDDEN, "This agent is not available"));
    }
    let caller =
        match authorize_agent_key(&state.db, &context, &channel, extract_bearer(headers)).await {
            Ok(Ok(caller)) => caller,
            Ok(Err(ApiAuthError::Unauthorized)) => {
                return Err(error(
                    StatusCode::UNAUTHORIZED,
                    "Invalid or missing agent key",
                ));
            }
            Ok(Err(ApiAuthError::Misconfigured)) => {
                return Err(error(StatusCode::FORBIDDEN, "This agent is misconfigured"));
            }
            Err(err) => return Err(internal_error(err)),
        };
    // THREAT[TM-AGENTKEY-004]: per key and IP, after the key check so an
    // unauthenticated caller cannot grow the limiter or probe channels.
    if let Some(limit) = caller.config.rate_limit_per_minute.filter(|l| *l > 0) {
        let scope = format!("api:{}:{}", channel.public_id, caller.key_id);
        let ip = extract_client_ip_from_parts(peer, headers);
        if state.rate_limiter.check(&scope, ip, limit).await.is_err() {
            return Err(error(StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded"));
        }
    }
    Ok(Authorized {
        context,
        channel,
        caller,
    })
}

async fn authorize_by_id(
    state: &ChannelApiState,
    channel_id: &str,
    headers: &HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> ApiResponse<Authorized> {
    let (context, channel) = resolve_api_channel(state, channel_id).await?;
    authorize(state, context, channel, headers, peer(connect_info)).await
}

pub(crate) fn peer(
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Option<std::net::SocketAddr> {
    connect_info.map(|Extension(ConnectInfo(addr))| addr)
}

/// The caller's session `session_id`, or the generic 404.
async fn callers_session(
    state: &ChannelApiState,
    auth: &Authorized,
    session_id: &str,
) -> ApiResponse<SessionRow> {
    let id = session_id.parse::<SessionId>().map_err(|_| not_found())?;
    match state.db.get_session(auth.context.org_id, id).await {
        Ok(Some(session)) if session_is_callers(&session, &auth.channel, &auth.caller) => {
            Ok(session)
        }
        Ok(_) => Err(not_found()),
        Err(err) => Err(internal_error(err)),
    }
}

#[utoipa::path(
    description = "Agent card of an api channel: what a caller needs to start talking to the agent.",
    get,
    path = "/v1/channels/{channel_id}",
    params(("channel_id" = String, Path, description = "api channel ID")),
    responses(
        (status = 200, description = "Agent card", body = AgentCard),
        (status = 401, description = "Missing or invalid agent key", body = ErrorResponse),
        (status = 403, description = "Agent not available", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn get_card(
    State(state): State<ChannelApiState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    Json(AgentCard {
        name: auth.context.name.clone(),
        description: auth.context.description.clone(),
        streaming: false,
        input: AgentCardInput::TEXT,
        auth: vec![AgentCardAuth::AgentKey],
        conversation_starters: Vec::new(),
        links: AgentCardLinks {
            sessions: format!("/api/v1/channels/{}/sessions", auth.channel.public_id),
            ag_ui: None,
            a2a: None,
        },
    })
    .into_response()
}

/// `POST …/sessions` on an api channel; reached through `channel_api`.
pub(crate) async fn create_session(
    state: &ChannelApiState,
    auth: Authorized,
    body: &Bytes,
) -> Response {
    let request: CreateAgentSessionRequest = if body.iter().all(u8::is_ascii_whitespace) {
        CreateAgentSessionRequest::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(request) => request,
            Err(err) => return error(StatusCode::BAD_REQUEST, &format!("Invalid body: {err}")),
        }
    };
    if request.metadata.is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "Session metadata is not supported by this host",
        );
    }
    match create_api_session(
        &state.db,
        &state.session_service,
        &auth.context,
        &auth.channel,
        &auth.caller,
        request.title,
    )
    .await
    {
        Ok(session) => (
            StatusCode::CREATED,
            [(
                header::LOCATION,
                format!(
                    "/api/v1/channels/{}/sessions/{}",
                    auth.channel.public_id, session.id
                ),
            )],
            Json(AgentSessionView::from(&session)),
        )
            .into_response(),
        Err(err) => super::channel_api::command_error_response(err).into_response(),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListSessionsQuery {
    /// Page size, 1 to 200 (default 50).
    #[serde(default)]
    limit: Option<u32>,
    /// `next_page_token` of the previous page.
    #[serde(default)]
    page_token: Option<String>,
}

#[utoipa::path(
    description = "The calling key's sessions on an api channel, most recently active first.",
    get,
    path = "/v1/channels/{channel_id}/sessions",
    params(("channel_id" = String, Path, description = "api channel ID"), ListSessionsQuery),
    responses(
        (status = 200, description = "One page of sessions: `{data, next_page_token?}`", body = Value),
        (status = 400, description = "Invalid page size or token", body = ErrorResponse),
        (status = 401, description = "Missing or invalid agent key", body = ErrorResponse),
        (status = 404, description = "Channel not found", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn list_sessions(
    State(state): State<ChannelApiState>,
    Path(channel_id): Path<String>,
    Query(query): Query<ListSessionsQuery>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let limit = query.limit.unwrap_or(DEFAULT_SESSION_PAGE);
    if !(1..=MAX_SESSION_PAGE).contains(&limit) {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("limit must be between 1 and {MAX_SESSION_PAGE}"),
        );
    }
    let after = match query.page_token.as_deref().map(decode_page_token) {
        None => None,
        Some(Some(after)) => Some(after),
        Some(None) => return error(StatusCode::BAD_REQUEST, "Invalid page_token"),
    };
    let tags = auth.caller.session_tags(&auth.channel);
    let rows = match state
        .db
        .list_sessions_by_tags(auth.context.org_id, &tags, &[], None, after, limit)
        .await
    {
        Ok((rows, _total)) => rows,
        Err(err) => return internal_error(err),
    };
    let next_page_token = (rows.len() == limit as usize)
        .then(|| rows.last().map(|row| encode_page_token(row)))
        .flatten();
    let data: Vec<AgentSessionView> = rows
        .iter()
        .filter(|row| session_is_callers(row, &auth.channel, &auth.caller))
        .map(AgentSessionView::from)
        .collect();
    let mut body = json!({ "data": data });
    if let Some(token) = next_page_token {
        body["next_page_token"] = json!(token);
    }
    Json(body).into_response()
}

fn encode_page_token(row: &SessionRow) -> String {
    format!(
        "{}_{}",
        row.updated_at.timestamp_micros(),
        row.id.uuid().simple()
    )
}

fn decode_page_token(token: &str) -> Option<(chrono::DateTime<chrono::Utc>, uuid::Uuid)> {
    let (micros, id) = token.split_once('_')?;
    let at = chrono::DateTime::from_timestamp_micros(micros.parse().ok()?)?;
    Some((at, uuid::Uuid::parse_str(id).ok()?))
}

/// `GET …/sessions/{id}` on an api channel; reached through `channel_api`.
pub(crate) async fn get_session(
    state: &ChannelApiState,
    auth: Authorized,
    session_id: &str,
) -> Response {
    match callers_session(state, &auth, session_id).await {
        Ok(session) => Json(AgentSessionView::from(&session)).into_response(),
        Err(response) => response,
    }
}

/// Body of `POST …/sessions/{id}/messages`: the `/v1/sessions/{id}/messages`
/// body. Fields an execution caller may not set (model controls, tags,
/// participants) are ignored, as serve ignores them.
#[derive(Debug, Deserialize)]
struct SendMessageBody {
    message: InputMessage,
}

/// `POST …/sessions/{id}/messages` on an api channel; reached through `channel_api`.
pub(crate) async fn send_message(
    state: &ChannelApiState,
    auth: Authorized,
    session_id: &str,
    request_id: Option<String>,
    body: &Bytes,
) -> Response {
    let body: SendMessageBody = match serde_json::from_slice(body) {
        Ok(body) => body,
        Err(err) => return error(StatusCode::BAD_REQUEST, &format!("Invalid body: {err}")),
    };
    let session = match callers_session(state, &auth, session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    match send_api_message(
        &state.message_service,
        &auth.context,
        &auth.channel,
        &auth.caller,
        session.id,
        body.message,
        request_id,
    )
    .await
    {
        Ok(message) => (StatusCode::CREATED, Json(message)).into_response(),
        Err(err) => super::channel_api::command_error_response(err).into_response(),
    }
}

/// `POST …/sessions/{id}/cancel` on an api channel; reached through `channel_api`.
pub(crate) async fn cancel(
    state: &ChannelApiState,
    auth: Authorized,
    session_id: &str,
) -> Response {
    let session = match callers_session(state, &auth, session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if let Err(err) = super::channel_api::cancel_session_turn_for(
        &state.db,
        &state.message_service,
        session.id,
        "api channel cancel",
    )
    .await
    {
        return internal_error(err);
    }
    Json(json!({ "status": "cancelled", "message": "Turn cancelled" })).into_response()
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListEventsQuery {
    /// Return events after this sequence number (exclusive).
    #[serde(default)]
    after_sequence: Option<i32>,
    /// Page size, 1 to 500 (default 100).
    #[serde(default)]
    limit: Option<i32>,
}

#[utoipa::path(
    description = "A session's events, oldest first, as the channel's visibility allows.",
    get,
    path = "/v1/channels/{channel_id}/sessions/{session_id}/events",
    params(
        ("channel_id" = String, Path, description = "api channel ID"),
        ("session_id" = String, Path, description = "Session ID"),
        ListEventsQuery
    ),
    responses(
        (status = 200, description = "Events: `{data}`, each in the canonical event envelope", body = Value),
        (status = 400, description = "Invalid page size", body = ErrorResponse),
        (status = 401, description = "Missing or invalid agent key", body = ErrorResponse),
        (status = 404, description = "Channel or session not found", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn list_events(
    State(state): State<ChannelApiState>,
    Path((channel_id, session_id)): Path<(String, String)>,
    Query(query): Query<ListEventsQuery>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let limit = query.limit.unwrap_or(DEFAULT_EVENT_PAGE);
    if !(1..=MAX_EVENT_PAGE).contains(&limit) {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("limit must be between 1 and {MAX_EVENT_PAGE}"),
        );
    }
    let session = match callers_session(&state, &auth, &session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let params = crate::storage::ListEventsParams {
        session_id: session.id,
        after_sequence: query.after_sequence,
        filter_types: visible_event_types(auth.caller.config.visibility).unwrap_or_default(),
        limit: Some(limit),
        ..Default::default()
    };
    let rows = match state.db.list_events_advanced(&params).await {
        Ok(rows) => rows,
        Err(err) => return internal_error(err),
    };
    let data: Vec<Value> = rows
        .iter()
        .filter_map(|row| project_event(row, &auth.caller.config))
        .collect();
    Json(json!({ "data": data })).into_response()
}

fn error(status: StatusCode, message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(status)
        .into_response()
}

fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "Channel or session not found")
}

fn internal_error(err: anyhow::Error) -> Response {
    tracing::error!(error = %err, "Failed to handle agent execution API request");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}
