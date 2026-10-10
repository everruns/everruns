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
    routing::{get, post},
};
use everruns_contracts::execution_api::{
    AgentCard, AgentCardAuth, AgentCardInput, AgentCardLinks, CreateAgentSessionRequest,
    SubmitToolApprovalsRequest, SubmitToolApprovalsResponse,
};
use everruns_contracts::typed_id::{EventId, SessionId};
use everruns_core::builtins::ask_user::{AskUserAnswer, AskUserStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::agent_api_auth::{CallerChecks, resolve_caller};
use super::agent_api_idempotency::{Retryable, run_once};
use super::channel_api::ChannelApiState;
use super::channel_ingress::{IngressChannel, IngressContext, channel_liveness, resolve_channel};
use super::common::ErrorResponse;
use super::events::{SessionEventStream, SseRender, session_event_sse, sse_frame};
use super::question_answers::{
    QuestionAnswersRequest, QuestionAnswersResponse, QuestionResolver, SubmittedStatus,
    question_answers_response, resolve_error_response, resolve_question_answers,
};
use super::tool_approvals::{
    ApprovalOutcome, ApprovalServices, approval_error_response, pending_tool_approvals,
    resolve_tool_approvals, validate_decisions,
};
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::domains::agent_channels::api_sessions::{
    AgentSessionView, ApiAuthError, ApiCaller, create_api_session, project_event,
    public_event_json, send_api_message, session_is_callers, session_pending_input,
    visible_event_types,
};
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::agent_channels::record::api::{AgentApiChannelConfig, ApiToolApprovals};
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
        .route("/v1/channels/{channel_id}", get(agent_api_get_card))
        .route(
            "/v1/channels/{channel_id}/sessions",
            get(agent_api_list_sessions),
        )
        .route(
            "/v1/channels/{channel_id}/sessions/{session_id}/events",
            get(agent_api_list_events),
        )
        .route(
            "/v1/channels/{channel_id}/sessions/{session_id}/sse",
            get(agent_api_stream_events),
        )
        .route(
            "/v1/channels/{channel_id}/sessions/{session_id}/question-answers",
            post(agent_api_answer_questions),
        )
        .route(
            "/v1/channels/{channel_id}/sessions/{session_id}/tool-approvals",
            post(agent_api_submit_tool_approvals),
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
    let checks = CallerChecks {
        db: &state.db,
        verifier: &state.auth_verifier,
        runtime_auth: state.runtime_auth.as_ref(),
    };
    let caller = match resolve_caller(&checks, &context, &channel, headers).await {
        Ok(Ok(caller)) => caller,
        Ok(Err(ApiAuthError::Unauthorized)) => {
            return Err(error(
                StatusCode::UNAUTHORIZED,
                "Invalid or missing credentials",
            ));
        }
        Ok(Err(ApiAuthError::Forbidden)) => {
            return Err(error(
                StatusCode::FORBIDDEN,
                "This credential may not act for an end user",
            ));
        }
        Ok(Err(ApiAuthError::Misconfigured)) => {
            return Err(error(StatusCode::FORBIDDEN, "This agent is misconfigured"));
        }
        Err(err) => return Err(internal_error(err)),
    };
    // THREAT[TM-AGENTKEY-004]: per caller and IP, after the credential check so
    // an unauthenticated caller cannot grow the limiter or probe channels.
    if let Some(limit) = caller.config.rate_limit_per_minute.filter(|l| *l > 0) {
        let scope = format!("api:{}:{}", channel.public_id, caller.rate_scope());
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
pub async fn agent_api_get_card(
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
        streaming: true,
        input: AgentCardInput::TEXT,
        auth: card_auth(&auth.caller.config),
        conversation_starters: Vec::new(),
        links: AgentCardLinks {
            sessions: format!("/api/v1/channels/{}/sessions", auth.channel.public_id),
            ag_ui: None,
            a2a: None,
        },
    })
    .into_response()
}

/// The credentials the agent card advertises: agent keys and runtime tokens
/// always, then each identity provider of the channel.
fn card_auth(config: &AgentApiChannelConfig) -> Vec<AgentCardAuth> {
    use crate::domains::agent_channels::record::{ChannelAuthMode, ChannelAuthProviderConfig};
    let mut auth = vec![AgentCardAuth::AgentKey, AgentCardAuth::RuntimeToken];
    if config.org_members {
        auth.push(AgentCardAuth::PersonalAccessToken);
    }
    for method in &config.auth_methods {
        let entry = match (&method.mode, &method.provider) {
            (ChannelAuthMode::OAuth2Introspection, _) => AgentCardAuth::OAuth2,
            (_, Some(ChannelAuthProviderConfig::Oidc { issuer, .. })) => AgentCardAuth::Oidc {
                issuer: issuer.clone(),
            },
            (ChannelAuthMode::GoogleOidc, _) => AgentCardAuth::Oidc {
                issuer: "https://accounts.google.com".to_string(),
            },
            _ => continue,
        };
        if !auth.contains(&entry) {
            auth.push(entry);
        }
    }
    auth
}

/// `POST …/sessions` on an api channel; reached through `channel_api`.
pub(crate) async fn create_session(
    state: &ChannelApiState,
    auth: Authorized,
    headers: &HeaderMap,
    body: &Bytes,
) -> Response {
    let retry = Retryable {
        org_id: auth.context.org_id,
        caller: &auth.caller,
        operation: "agent_api.create_session",
        target: auth.channel.public_id.to_string(),
        body,
    };
    run_once(
        state,
        headers,
        retry,
        create_session_now(state, &auth, body),
    )
    .await
}

async fn create_session_now(state: &ChannelApiState, auth: &Authorized, body: &Bytes) -> Response {
    if let Err(response) = within_spend_limit(state, auth).await {
        return response;
    }
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
pub async fn agent_api_list_sessions(
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
        .then(|| rows.last().map(encode_page_token))
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
    let session = match callers_session(state, &auth, session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    match session_pending_input(&state.db, &session, &auth.caller.config).await {
        Ok(pending) => Json(AgentSessionView::from(&session).with_pending(pending)).into_response(),
        Err(err) => internal_error(err),
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
    headers: &HeaderMap,
    body: &Bytes,
) -> Response {
    let retry = Retryable {
        org_id: auth.context.org_id,
        caller: &auth.caller,
        operation: "agent_api.send_message",
        target: format!("{}/{session_id}", auth.channel.public_id),
        body,
    };
    let send = send_message_now(state, &auth, session_id, request_id, body);
    run_once(state, headers, retry, send).await
}

async fn send_message_now(
    state: &ChannelApiState,
    auth: &Authorized,
    session_id: &str,
    request_id: Option<String>,
    body: &Bytes,
) -> Response {
    let body: SendMessageBody = match serde_json::from_slice(body) {
        Ok(body) => body,
        Err(err) => return error(StatusCode::BAD_REQUEST, &format!("Invalid body: {err}")),
    };
    let session = match callers_session(state, auth, session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if let Err(response) = within_spend_limit(state, auth).await {
        return response;
    }
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
pub async fn agent_api_list_events(
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
    let events = match state.event_service.list_advanced(&params).await {
        Ok(events) => events,
        Err(err) => return internal_error(err),
    };
    let data: Vec<Value> = events
        .iter()
        .filter_map(|event| project_event(public_event_json(event)?, &auth.caller.config))
        .collect();
    Json(json!({ "data": data })).into_response()
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct StreamEventsQuery {
    /// Resume after this event (the last `id:` received).
    #[serde(default)]
    since_id: Option<EventId>,
    /// Replay events after this sequence first. `0` replays the whole session.
    #[serde(default)]
    after_sequence: Option<i32>,
}

#[utoipa::path(
    description = "Follow a session's events live, as the channel's visibility allows. \
        Same framing as `/v1/sessions/{id}/sse`: `connected`, `id:` on durable events \
        (resume with `since_id`), a heartbeat, and `disconnecting` before the server \
        cycles the connection.",
    get,
    path = "/v1/channels/{channel_id}/sessions/{session_id}/sse",
    params(
        ("channel_id" = String, Path, description = "api channel ID"),
        ("session_id" = String, Path, description = "Session ID"),
        StreamEventsQuery
    ),
    responses(
        (status = 200, description = "Server-Sent Events stream of canonical event envelopes", content_type = "text/event-stream", body = Value),
        (status = 401, description = "Missing or invalid agent key", body = ErrorResponse),
        (status = 404, description = "Channel or session not found", body = ErrorResponse),
        (status = 429, description = "Too many open streams", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn agent_api_stream_events(
    State(state): State<ChannelApiState>,
    Path((channel_id, session_id)): Path<(String, String)>,
    Query(query): Query<StreamEventsQuery>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let session = match callers_session(&state, &auth, &session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    // THREAT[TM-DOS-003]: the same per-org and per-session stream caps as `/v1`.
    let guard = match state
        .sse_tracker
        .try_acquire(auth.context.org_id, session.id.uuid())
    {
        Ok(guard) => guard,
        Err(rejection) => {
            let message = rejection.report("api_channel_events", auth.context.org_id, &session.id);
            return error(StatusCode::TOO_MANY_REQUESTS, &message);
        }
    };
    let config = auth.caller.config.clone();
    // Visibility applies to every frame, replayed or live, before it is written.
    let render: SseRender = Arc::new(move |event, retry| {
        let projected = project_event(public_event_json(event)?, &config)?;
        Some(sse_frame(event, projected.to_string(), retry))
    });
    session_event_sse(SessionEventStream {
        event_service: state.event_service.clone(),
        event_broadcaster: None,
        session_id: session.id.uuid(),
        since_id: query.since_id.map(|id| id.uuid()),
        after_sequence: query.after_sequence,
        filter_types: visible_event_types(auth.caller.config.visibility).unwrap_or_default(),
        exclude_types: Vec::new(),
        guard,
        render,
    })
    .await
    .into_response()
}

#[utoipa::path(
    description = "Answer the question set the agent asked with `ask_user`, and resume the turn.",
    post,
    path = "/v1/channels/{channel_id}/sessions/{session_id}/question-answers",
    params(
        ("channel_id" = String, Path, description = "api channel ID"),
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = QuestionAnswersRequest,
    responses(
        (status = 200, description = "Answer recorded and turn resumed", body = QuestionAnswersResponse),
        (status = 400, description = "Answers that do not match what was asked, or a credential", body = ErrorResponse),
        (status = 401, description = "Missing or invalid agent key", body = ErrorResponse),
        (status = 404, description = "Channel, session or pending question set not found", body = ErrorResponse),
        (status = 409, description = "Not waiting, or already answered", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn agent_api_answer_questions(
    State(state): State<ChannelApiState>,
    Path((channel_id, session_id)): Path<(String, String)>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    body: Bytes,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let request: QuestionAnswersRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(err) => return error(StatusCode::BAD_REQUEST, &format!("Invalid body: {err}")),
    };
    let session = match callers_session(&state, &auth, &session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    // THREAT[TM-AGENT-016]: a credential never travels over the execution API.
    // A secret question can be declined here; a person completes it in Everruns.
    if request
        .answers
        .iter()
        .any(|answer| answer.secret_ref.is_some())
    {
        return error(
            StatusCode::BAD_REQUEST,
            "secret_ref is not accepted here. Decline the question, or have a person \
             complete it in Everruns",
        );
    }
    let status = match request.status {
        SubmittedStatus::Answered => AskUserStatus::Answered,
        SubmittedStatus::Declined => AskUserStatus::Declined,
    };
    let answers: Vec<AskUserAnswer> = request.answers.into_iter().map(Into::into).collect();
    let resolver = QuestionResolver {
        db: &state.db,
        session_service: &state.session_service,
        event_service: &state.event_service,
        runner: state.message_service.runner().clone(),
    };
    // Attribution is the channel's, as on A2A and AG-UI: the answer came from
    // a key holder, which is what `Caller::internal` records.
    let caller = everruns_core::Caller::internal(auth.context.org_id);
    match resolve_question_answers(
        &resolver,
        &caller,
        session.id,
        request.tool_call_id.as_deref(),
        status,
        &answers,
    )
    .await
    {
        Ok(result) => question_answers_response(&result).into_response(),
        Err(err) => resolve_error_response(err).into_response(),
    }
}

#[utoipa::path(
    description = "Allow or reject tool calls the agent's approval gate held back, and resume \
        the turn. Only on channels whose `tool_approvals` is `caller`.",
    post,
    path = "/v1/channels/{channel_id}/sessions/{session_id}/tool-approvals",
    params(
        ("channel_id" = String, Path, description = "api channel ID"),
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = SubmitToolApprovalsRequest,
    responses(
        (status = 200, description = "Decisions recorded and turn resumed", body = SubmitToolApprovalsResponse),
        (status = 400, description = "Invalid decisions", body = ErrorResponse),
        (status = 401, description = "Missing or invalid agent key", body = ErrorResponse),
        (status = 403, description = "An operator answers this agent's tool approvals", body = ErrorResponse),
        (status = 404, description = "Channel, session or pending request not found", body = ErrorResponse),
        (status = 409, description = "Not waiting, expired, or already answered", body = ErrorResponse)
    ),
    tag = "agent-execution"
)]
pub async fn agent_api_submit_tool_approvals(
    State(state): State<ChannelApiState>,
    Path((channel_id, session_id)): Path<(String, String)>,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    body: Bytes,
) -> Response {
    let auth = match authorize_by_id(&state, &channel_id, &headers, connect_info).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let request: SubmitToolApprovalsRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(err) => return error(StatusCode::BAD_REQUEST, &format!("Invalid body: {err}")),
    };
    let session = match callers_session(&state, &auth, &session_id).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    // THREAT[TM-AGENTKEY-006]: a held-back action is the caller's to allow
    // only when the owner said so.
    if auth.caller.config.tool_approvals != ApiToolApprovals::Caller {
        return error(
            StatusCode::FORBIDDEN,
            "An operator answers this agent's tool approvals",
        );
    }
    let pending = match pending_tool_approvals(&state.db, session.id).await {
        Ok(pending) => pending,
        Err(err) => return internal_error(err),
    };
    let outcomes = match validate_decisions(&pending, &request.decisions, chrono::Utc::now()) {
        Ok(outcomes) => outcomes,
        Err(err) => return approval_error_response(err).into_response(),
    };
    match resolve_tool_approvals(
        &ApprovalServices {
            db: &state.db,
            event_service: &state.event_service,
            runner: state.message_service.runner(),
        },
        auth.context.org_id,
        session.id,
        &pending,
        &outcomes,
        ApprovalOutcome::NotApproved,
        "api_channel",
    )
    .await
    {
        Ok(resolved) => Json(SubmitToolApprovalsResponse {
            resolved,
            status: "active".to_string(),
        })
        .into_response(),
        Err(err) => approval_error_response(err).into_response(),
    }
}

/// THREAT[TM-AGENTKEY-009]: one caller cannot run up the owner's bill past
/// the channel's per-caller daily limit. Checked before new work starts, so
/// the turn that crosses the limit still finishes.
async fn within_spend_limit(state: &ChannelApiState, auth: &Authorized) -> ApiResponse<()> {
    let Some(limit) = auth.caller.config.daily_spend_limit_usd else {
        return Ok(());
    };
    let since = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|midnight| midnight.and_utc())
        .unwrap_or_else(chrono::Utc::now);
    let spent = state
        .db
        .api_caller_spend_since(
            auth.context.org_id,
            auth.channel.internal_id,
            &auth.caller.session_tags(&auth.channel),
            since,
        )
        .await
        .map_err(internal_error)?;
    if spent >= limit {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "This caller reached today's spending limit for this agent",
        ));
    }
    Ok(())
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
