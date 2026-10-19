//! The `/v1` wire API: a subset of the everruns server's session API, so the
//! everruns SDK, CLI and UI chat view can drive a serve app. The same routes
//! are served by `dev`, by a self-hosted `start`, and (by contract) by a
//! hosted deploy.
//!
//! Decisions:
//! - Shapes match the everruns server where a route exists there: `Session`,
//!   `Message`, the canonical event envelope, `/sse` framing (`connected`,
//!   `id:` only on durable events, `retry:`, `:heartbeat`), resume with
//!   `since_id` or `after_sequence`, `/question-answers`, and problem+json
//!   errors. serve adds optional fields (`build_id`, `agent_name`,
//!   `pending_approvals`, `pending_questions`) and a few serve-only routes
//!   (approvals, channels, the agent card, dev schedules). With the `ag-ui`
//!   feature, `POST /v1/channels/{agent}/ag-ui` mirrors the server's AG-UI channel
//!   route (see `ag_ui`); with `a2a`, `POST /v1/channels/{agent}/a2a` and its Agent
//!   Card mirror the server's A2A endpoint (see `a2a`); with `voice`,
//!   `/v1/channels/{agent}/voice` takes calls (see `voice`).
//! - Each top-level agent also has the Agent Execution API under
//!   `/v1/channels/{agent}` (see `agent_api`).
//! - Open unless auth is configured; then every route but discovery needs a
//!   bearer credential (see `auth`).
//! - Events are the engine's durable canonical log, replayed then followed
//!   live through `Session::events_from`; serve writes none.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::Query;
use everruns::ask_user::Status;
use everruns::{EventStreamError, SessionEvent};
use futures::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::app::Mode;
use crate::channels::ChannelRequest;
use crate::host::{ApiError, Host, NewSession, wire_json};

/// The fixed organization a serve app reports; it has no tenants.
pub(crate) const ORGANIZATION_ID: &str = "org_00000000000000000000000000000000";

/// SSE reconnect hint, milliseconds.
const RETRY: Duration = Duration::from_millis(1000);
const HEARTBEAT: Duration = Duration::from_secs(30);

pub(crate) fn router(host: Arc<Host>) -> Router {
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/v1/agent", get(agent_card))
        .route("/v1/sessions", post(create_session))
        .route("/v1/sessions/{id}", get(get_session))
        .route("/v1/sessions/{id}/messages", post(create_message))
        .route("/v1/sessions/{id}/cancel", post(cancel))
        .route("/v1/sessions/{id}/admissions/{message_id}", get(admission))
        .route("/v1/sessions/{id}/sse", get(sse))
        .route("/v1/sessions/{id}/events", get(list_events))
        .route("/v1/sessions/{id}/question-answers", post(question_answers))
        .route("/v1/sessions/{id}/approvals/{tool_call_id}", post(approval))
        .route(
            "/v1/channels/{name}",
            get(crate::agent_api::card).post(channel),
        );
    router = crate::agent_api::routes(router);
    #[cfg(feature = "ag-ui")]
    {
        router = router
            .route("/v1/channels/{name}/ag-ui", post(crate::ag_ui::run))
            .route("/v1/e/{name}/ag-ui", post(crate::ag_ui::run));
    }
    #[cfg(feature = "a2a")]
    {
        router = crate::a2a::routes(&host, router);
    }
    #[cfg(feature = "voice")]
    {
        router = router
            .route("/v1/channels/{name}/voice", get(crate::voice::page))
            .route("/v1/channels/{name}/voice/calls", post(crate::voice::call))
            .route("/v1/channels/{name}/voice/ws", get(crate::voice::socket))
            .route(
                "/v1/channels/{name}/voice/calls/{call_id}/end",
                post(crate::voice::end),
            );
    }
    if host.mode == Mode::Dev {
        // Dev only: fire a schedule without waiting for its cron.
        router = router.route("/dev/schedules/{name}", post(run_schedule));
    }
    router
        .layer(axum::middleware::from_fn_with_state(
            host.clone(),
            crate::auth::require,
        ))
        .with_state(host)
}

pub(crate) struct Failure(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for Failure {
    fn from(err: E) -> Self {
        Failure(err.into())
    }
}

/// RFC 9457 problem details, like the everruns server's `ErrorResponse`.
fn problem(status: StatusCode, detail: String) -> Response {
    let body = json!({
        "title": status.canonical_reason().unwrap_or("Error"),
        "status": status.as_u16(),
        "detail": detail,
    });
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let detail = format!("{:#}", self.0);
        let (status, build) = match self.0.downcast_ref::<ApiError>() {
            Some(ApiError::NotFound(_)) => (StatusCode::NOT_FOUND, None),
            Some(ApiError::BadRequest(_)) => (StatusCode::BAD_REQUEST, None),
            Some(ApiError::Conflict(_)) => (StatusCode::CONFLICT, None),
            Some(ApiError::Pinned { build_id }) => (StatusCode::CONFLICT, Some(build_id.clone())),
            None => {
                let status = self
                    .0
                    .chain()
                    .find_map(|cause| {
                        cause.downcast_ref::<everruns_contracts::error::AgentLoopError>()
                    })
                    .map(|error| match error {
                        everruns_contracts::error::AgentLoopError::SessionNotFound(_) => {
                            StatusCode::NOT_FOUND
                        }
                        everruns_contracts::error::AgentLoopError::Configuration(_) => {
                            StatusCode::BAD_REQUEST
                        }
                        _ => StatusCode::INTERNAL_SERVER_ERROR,
                    })
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                (status, None)
            }
        };
        let mut response = problem(status, detail);
        if let Some(build) = build.and_then(|b| HeaderValue::from_str(&b).ok()) {
            response.headers_mut().insert("x-serve-build", build);
        }
        response
    }
}

pub(crate) type ApiResult<T> = Result<T, Failure>;

pub(crate) fn bad_request(why: impl Into<String>) -> Failure {
    ApiError::BadRequest(why.into()).into()
}

async fn health(State(host): State<Arc<Host>>) -> Json<Value> {
    Json(json!({ "status": "ok", "build_id": host.build_id, "experimental": true }))
}

async fn agent_card(State(host): State<Arc<Host>>) -> Json<Value> {
    let manifest = host.app.manifest();
    let card = json!({
        "name": manifest.app.name,
        "version": manifest.app.version,
        "build_id": host.build_id,
        "experimental": true,
        "agents": manifest.agents,
        "tools": manifest.tools.iter().map(|tool| json!({
            "name": tool.name,
            "description": tool.description,
            "needs_approval": tool.needs_approval,
        })).collect::<Vec<_>>(),
        "skills": manifest.skills,
        "channels": manifest.channels,
        "schedules": manifest.schedules,
    });
    // The AG-UI endpoint of each top-level agent, by agent name.
    #[cfg(feature = "ag-ui")]
    let card = {
        let mut card = card;
        card["ag_ui"] = manifest
            .agents
            .iter()
            .filter(|agent| !agent.sub)
            .map(|agent| (agent.name.clone(), json!(crate::ag_ui::route(&agent.name))))
            .collect::<serde_json::Map<_, _>>()
            .into();
        card
    };
    // The A2A endpoint of each top-level agent, by agent name.
    #[cfg(feature = "a2a")]
    let card = {
        let mut card = card;
        card["a2a"] = manifest
            .agents
            .iter()
            .filter(|agent| !agent.sub)
            .map(|agent| (agent.name.clone(), json!(crate::a2a::route(&agent.name))))
            .collect::<serde_json::Map<_, _>>()
            .into();
        card
    };
    // The voice channel's settings and each top-level agent's endpoint.
    #[cfg(feature = "voice")]
    let card = {
        let mut card = card;
        card["voice"] = crate::voice::card(&host);
        card
    };
    Json(card)
}

/// The subset of the server's `CreateSessionRequest` serve understands.
/// Unknown fields are accepted and ignored.
#[derive(Deserialize, Default)]
struct CreateSessionBody {
    /// Optional stable identity for retryable session creation.
    #[serde(default)]
    id: Option<everruns::SessionId>,
    /// The serve agent; the app's default agent when absent.
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    hints: Option<Value>,
    /// serve extra: free-form metadata kept with the session.
    #[serde(default)]
    metadata: Option<Value>,
}

/// A session in the server's `Session` shape, plus serve's optional extras.
pub(crate) async fn session_json(host: &Host, id: &str) -> crate::Result<Value> {
    let authority = host.authoritative_status(id).await?;
    if authority.is_some() {
        host.session(id).await?;
    }
    let row = host.session_row(id)?;
    let mut session = json!({
        "id": row.id,
        "organization_id": ORGANIZATION_ID,
        "harness_id": harness_id(host),
        "agent_id": row.agent,
        "status": host.status(id),
        "tags": row.tags,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "build_id": row.build_id,
        "agent_name": row.agent,
        "pending_approvals": host.pending_approvals(id),
        "pending_questions": host.pending_questions(id),
    });
    if let Some(title) = row.title {
        session["title"] = json!(title);
    }
    if let Some(hints) = row.hints {
        session["hints"] = hints;
    }
    if let Some(metadata) = row.metadata {
        session["metadata"] = metadata;
    }
    if let Some(authority) = authority {
        session["build_id"] = json!(authority.binding.behavior.build_id);
        session["agent_name"] = json!(authority.binding.behavior.agent);
        session["agent_id"] = session["agent_name"].clone();
        session["created_at"] = json!(authority.created_at);
        session["status"] = json!(authority.phase());
        let approvals:Vec<_>=authority.waiting.iter().filter(|call|call.name=="approve_tool_call").map(|call|json!({"tool_call_id":call.id,"tool_name":call.arguments["tool"],"arguments":call.arguments["arguments"]})).collect();
        let questions: Vec<_> = authority
            .waiting
            .iter()
            .filter(|call| call.name == "ask_user")
            .map(|call| json!({"tool_call_id":call.id,"questions":call.arguments["questions"]}))
            .collect();
        session["turn_id"] = json!(authority.open_turn);
        session["suspension_reason"] = json!(authority.suspension_reason);
        session["pending_approvals"] = json!(approvals);
        session["pending_questions"] = json!(questions);
    }
    Ok(session)
}

/// Deterministic per app, in the server's `harness_{32-hex}` format.
fn harness_id(host: &Host) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("serve:{}", host.app.name()).as_bytes());
    format!("harness_{}", &hex::encode(digest)[..32])
}

async fn create_session(
    State(host): State<Arc<Host>>,
    body: Option<Json<CreateSessionBody>>,
) -> ApiResult<Response> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let id = host
        .create_session(NewSession {
            id: body.id,
            agent: body.agent_name,
            title: body.title,
            tags: body.tags,
            hints: body.hints,
            metadata: body.metadata,
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/sessions/{id}"))],
        Json(session_json(&host, &id).await?),
    )
        .into_response())
}

pub(crate) async fn get_session(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    // After a restart, a turn waiting on a person shows as waiting again.
    host.wake(&id).await;
    Ok(Json(session_json(&host, &id).await?))
}

#[derive(Deserialize)]
pub(crate) struct CreateMessageBody {
    #[serde(default)]
    message_id: Option<everruns::MessageId>,
    message: InputMessage,
    #[serde(default)]
    metadata: Option<Value>,
    /// Files sent with the message, as on the server's Agent API.
    #[serde(default)]
    files: Vec<everruns::execution_api::MessageFile>,
    /// Accepted for compatibility; serve has no per-message controls.
    #[serde(default)]
    #[allow(dead_code)]
    controls: Option<Value>,
}

#[derive(Deserialize)]
struct InputMessage {
    #[serde(default)]
    role: Option<String>,
    content: Vec<Value>,
}

/// Text of a user message. serve accepts text parts only; files come in
/// the body's `files`. Empty text is allowed when `files` is not.
fn message_text(message: &InputMessage) -> Result<String, Failure> {
    let text = joined_text(message)?;
    if text.trim().is_empty() {
        return Err(bad_request("message has no text"));
    }
    Ok(text)
}

fn joined_text(message: &InputMessage) -> Result<String, Failure> {
    if message.role.as_deref().is_some_and(|role| role != "user") {
        return Err(bad_request("only user messages can be created"));
    }
    let mut texts = Vec::new();
    for part in &message.content {
        match (part["type"].as_str(), part["text"].as_str()) {
            (Some("text"), Some(text)) => texts.push(text),
            (kind, _) => {
                return Err(bad_request(format!(
                    "unsupported content part {:?}; serve accepts text parts only",
                    kind.unwrap_or("?")
                )));
            }
        }
    }
    Ok(texts.join("\n"))
}

/// `POST /v1/sessions/{id}/messages`: starts a turn when idle, steers the
/// running turn otherwise. Answers with the server's `Message` shape.
pub(crate) async fn create_message(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Json(body): Json<CreateMessageBody>,
) -> ApiResult<Response> {
    let text = if body.files.is_empty() {
        message_text(&body.message)?
    } else {
        joined_text(&body.message)?
    };
    host.session(&id).await?;
    let mut content = Vec::new();
    if !text.trim().is_empty() {
        content.push(everruns::ContentPart::text(text));
    }
    if !body.files.is_empty() {
        let session = host.session(&id).await?;
        content.extend(crate::files::receive(session, body.files).await);
    }
    let mut input = everruns::InputMessage::user("");
    input.content = content;
    let parts = serde_json::to_value(&input.content)?;
    let sent = if let Some(message_id) = body.message_id {
        host.send_message_with_id(&id, message_id, input).await?
    } else {
        host.send_message(&id, input).await?
    };
    let mut message = json!({
        "id": sent.message_id,
        "session_id": id,
        "role": "user",
        "content": parts,
        "created_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    });
    // The canonical `input.message` event carries the message's sequence. It
    // is committed as the turn picks the message up, usually at once; the
    // field is left out if it is not there yet.
    for _ in 0..5 {
        let sequence = host.events_after(&id, 0).await.ok().and_then(|events| {
            events.iter().rev().find_map(|event| {
                (event.event_type() == "input.message"
                    && event.canonical_json()["data"]["message"]["id"] == sent.message_id.as_str())
                .then(|| {
                    event.sequence().map(|sequence| {
                        (
                            sequence,
                            event.canonical_json()["data"]["message"]["created_at"].clone(),
                        )
                    })
                })
                .flatten()
            })
        });
        if let Some((sequence, created_at)) = sequence {
            message["sequence"] = json!(sequence);
            if !created_at.is_null() {
                message["created_at"] = created_at;
            }
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if let Some(metadata) = body.metadata {
        message["metadata"] = metadata;
    }
    Ok((StatusCode::CREATED, Json(message)).into_response())
}

async fn admission(
    State(host): State<Arc<Host>>,
    Path((id, message)): Path<(String, everruns::MessageId)>,
) -> ApiResult<Json<Value>> {
    let session = host.session(&id).await?;
    let accepted = session
        .admission(message)
        .await?
        .ok_or_else(|| ApiError::NotFound("admission receipt".into()))?;
    Ok(Json(serde_json::to_value(accepted.state().await?)?))
}

#[derive(Deserialize, Default)]
pub(crate) struct CancelBody {
    #[serde(default)]
    command_id: Option<uuid::Uuid>,
    #[serde(default)]
    message_id: Option<everruns::MessageId>,
}

pub(crate) async fn cancel(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    body: Option<Json<Option<CancelBody>>>,
) -> ApiResult<Response> {
    let body = body.and_then(|body| body.0).unwrap_or_default();
    host.session(&id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            if host
                .cancel_target(
                    &id,
                    body.command_id.unwrap_or_else(uuid::Uuid::new_v4),
                    body.message_id,
                )
                .await?
            {
                json!({ "status": "accepted", "message": "Cancellation requested" })
            } else {
                json!({ "status": "no_op", "message": "No turn was running" })
            },
        ),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum DecisionKind {
    Approve,
    Deny,
}

#[derive(Deserialize)]
struct ApprovalBody {
    decision: DecisionKind,
    /// Kept for the operator's record; the runtime tells the model only that
    /// the call was rejected.
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
}

/// serve-only: approve or deny a pending tool call.
async fn approval(
    State(host): State<Arc<Host>>,
    Path((id, tool_call_id)): Path<(String, String)>,
    Json(body): Json<ApprovalBody>,
) -> ApiResult<Json<Value>> {
    host.session(&id).await?;
    // After a restart, the approval is pending again once the session is.
    host.session(&id).await?;
    let approve = matches!(body.decision, DecisionKind::Approve);
    host.resolve_approval(&id, &tool_call_id, approve).await?;
    Ok(Json(json!({
        "status": if approve { "approved" } else { "denied" },
        "tool_call_id": tool_call_id,
        "session_status": host.status(&id),
    })))
}

pub(crate) type QuestionAnswersBody = everruns::hosting::QuestionAnswersRequest;

pub(crate) async fn question_answers(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Json(body): Json<QuestionAnswersBody>,
) -> ApiResult<Json<Value>> {
    host.session(&id).await?;
    // After a restart, the question is pending again once the session is.
    host.session(&id).await?;
    let status = body.status;
    if host.engine.has_session_hosting() {
        let session = host.session(&id).await?;
        if body.command_id.is_none() {
            let pending = session.pending_tools().await?;
            if !pending.iter().any(|call| {
                call.name == "ask_user"
                    && body.tool_call_id.as_ref().is_none_or(|id| id == &call.id)
            }) {
                if let Some(call) = &body.tool_call_id {
                    let completed = session.events_after(0).await?.iter().any(|event| {
                        event.event_type() == "tool.completed"
                            && event.canonical_json()["data"]["tool_call_id"].as_str()
                                == Some(call.as_str())
                            && event.canonical_json()["data"]["tool_name"] == "ask_user"
                    });
                    if completed {
                        return Err(ApiError::Conflict(
                            "question set has already been answered".into(),
                        )
                        .into());
                    }
                }
                return Err(ApiError::NotFound("pending question set".into()).into());
            }
        }
        session.answer_questions(body).await?;
        let phase = host.engine.session_status(session.session_id()).await?;
        return Ok(Json(
            json!({"status":status,"answered_by":"user","session_status":phase.phase()}),
        ));
    }
    let answers = body.answers;
    host.answer_questions(&id, body.tool_call_id.as_deref(), status, answers)
        .await?;
    Ok(Json(json!({
        "status": match status { Status::Declined => "declined", _ => "answered" },
        "answered_by": "user",
        "session_status": host.status(&id),
    })))
}

async fn channel(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let request = ChannelRequest {
        path: String::new(),
        headers: headers
            .iter()
            .filter_map(|(key, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (key.as_str().to_string(), value.to_string()))
            })
            .collect(),
        body: body.to_vec(),
    };
    let response = host.channels().handle(&name, &request).await;
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(response.body)).into_response()
}

async fn run_schedule(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    host.run_schedule(&name).await?;
    Ok(Json(json!({ "ran": name })))
}

// --- Events ---------------------------------------------------------------------

/// Type filters shared by `/sse` and `/events` (`types`, `exclude`; each
/// may repeat).
#[derive(Default)]
struct Filters {
    types: Vec<String>,
    exclude: Vec<String>,
}

impl Filters {
    fn pass(&self, event_type: &str) -> bool {
        (self.types.is_empty() || self.types.iter().any(|t| t == event_type))
            && !self.exclude.iter().any(|t| t == event_type)
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct SseQuery {
    #[serde(default)]
    since_id: Option<String>,
    #[serde(default)]
    after_sequence: Option<i32>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

/// The durable sequence to replay after for `since_id`, with the server's
/// semantics: events whose id sorts after `since_id` (event ids are
/// time-ordered). An ephemeral event's id works too.
async fn resolve_since_id(host: &Host, id: &str, since_id: &str) -> crate::Result<i32> {
    if !since_id.starts_with("event_") {
        return Err(ApiError::BadRequest(format!("invalid since_id {since_id:?}")).into());
    }
    if host.authoritative_status(id).await?.is_some() {
        return host
            .session(id)
            .await?
            .event_sequence(since_id)
            .await?
            .ok_or_else(|| {
                ApiError::BadRequest("event cursor does not belong to this session".into()).into()
            });
    }
    Ok(host
        .events_after(id, 0)
        .await?
        .iter()
        .filter(|event| event.event_id.as_str() <= since_id)
        .filter_map(SessionEvent::sequence)
        .max()
        .unwrap_or(0))
}

fn sse_event(event: &SessionEvent) -> Event {
    let mut sse = Event::default()
        .event(event.event_type())
        .data(wire_json(event).to_string())
        .retry(RETRY);
    if event.sequence().is_some() {
        sse = sse.id(event.event_id.clone());
    }
    sse
}

/// `GET /v1/sessions/{id}/sse`: `connected`, then durable events after the
/// cursor, then live events (deltas included). Without a cursor the stream
/// starts live, as on the server.
pub(crate) async fn sse(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Query(mut query): Query<SseQuery>,
    headers: HeaderMap,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    if query.since_id.is_none()
        && query.after_sequence.is_none()
        && let Some(value) = headers.get("last-event-id")
    {
        query.since_id = Some(
            value
                .to_str()
                .map_err(|_| bad_request("invalid Last-Event-ID"))?
                .to_owned(),
        );
    }
    if query.since_id.is_some() && query.after_sequence.is_some() {
        return Err(bad_request(
            "since_id and after_sequence are mutually exclusive",
        ));
    }
    if query.after_sequence.is_some_and(|seq| seq < 0) {
        return Err(bad_request("after_sequence must be >= 0"));
    }
    host.session(&id).await?;
    let session = host.session(&id).await?;
    let after = match (&query.since_id, query.after_sequence) {
        (Some(since_id), _) => Some(resolve_since_id(&host, &id, since_id).await?),
        (None, None) if host.authoritative_status(&id).await?.is_some() => {
            Some(session.event_page(None, 1).await?.high_watermark)
        }
        (None, after) => after,
    };
    let events = match after {
        Some(after) => session.events_from(after).await?,
        None => session.events(),
    };

    let (tx, rx) = mpsc::channel::<Event>(256);
    let connected = Event::default()
        .event("connected")
        .data(r#"{"status":"connected"}"#)
        .retry(RETRY);
    let _ = tx.send(connected).await;
    let filters = Filters {
        types: query.types,
        exclude: query.exclude,
    };
    tokio::spawn(forward(session, events, after, filters, tx));
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        let event = rx.recv().await?;
        Some((Ok(event), rx))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(HEARTBEAT).text("heartbeat")))
}

async fn forward(
    session: everruns::Session,
    mut events: everruns::EventStream,
    after: Option<i32>,
    filters: Filters,
    tx: mpsc::Sender<Event>,
) {
    let mut last = after.unwrap_or(0);
    loop {
        let next = tokio::select! {
            next = events.recv() => next,
            () = tx.closed() => return,
        };
        match next {
            Ok(Some(event)) => {
                if let Some(sequence) = event.sequence() {
                    last = last.max(sequence);
                }
                if filters.pass(event.event_type()) && tx.send(sse_event(&event)).await.is_err() {
                    return;
                }
            }
            Ok(None) => return,
            // Fell behind the live feed: the durable log has everything.
            Err(EventStreamError::Lagged { .. }) => match session.events_from(last).await {
                Ok(fresh) => events = fresh,
                Err(_) => return,
            },
            Err(_) => return,
        }
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct ListQuery {
    #[serde(default)]
    since_id: Option<String>,
    #[serde(default)]
    after_sequence: Option<i32>,
    #[serde(default)]
    before_sequence: Option<i32>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

/// `GET /v1/sessions/{id}/events`: durable events as `{data: [...]}`, oldest
/// first. `limit` keeps the last N and sets `X-Total-Count`.
pub(crate) async fn list_events(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Response> {
    if query
        .limit
        .is_some_and(|limit| !(1..=1000).contains(&limit))
    {
        return Err(bad_request("limit must be between 1 and 1000"));
    }
    host.session(&id).await?;
    let after = match (&query.since_id, query.after_sequence) {
        (Some(since_id), _) => resolve_since_id(&host, &id, since_id).await?,
        (None, after) => after.unwrap_or(0),
    };
    let filters = Filters {
        types: query.types,
        exclude: query.exclude,
    };
    let all = host.events_after(&id, 0).await?;
    let total = all.len();
    let mut events: Vec<&SessionEvent> = all
        .iter()
        .filter(|event| event.sequence().is_some_and(|seq| seq > after))
        .filter(|event| {
            query
                .before_sequence
                .is_none_or(|before| event.sequence().is_some_and(|seq| seq < before))
        })
        .filter(|event| filters.pass(event.event_type()))
        .collect();
    let mut headers = HeaderMap::new();
    if let Some(limit) = query.limit {
        events = events.split_off(events.len().saturating_sub(limit));
        headers.insert("x-total-count", HeaderValue::from(total));
        headers.insert(
            "access-control-expose-headers",
            HeaderValue::from_static("X-Total-Count"),
        );
    }
    let data: Vec<Value> = events.into_iter().map(wire_json).collect();
    Ok((headers, Json(json!({ "data": data }))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_accept_text_parts_only() {
        let text = |content: Value| {
            message_text(&InputMessage {
                role: None,
                content: serde_json::from_value(content).unwrap(),
            })
        };
        assert_eq!(
            text(json!([{ "type": "text", "text": "hi" }]))
                .ok()
                .unwrap(),
            "hi"
        );
        assert!(text(json!([{ "type": "image", "url": "x" }])).is_err());
        assert!(text(json!([])).is_err());
        let assistant = message_text(&InputMessage {
            role: Some("agent".into()),
            content: vec![json!({ "type": "text", "text": "hi" })],
        });
        assert!(assistant.is_err());
    }

    #[test]
    fn repeated_filters_parse_from_the_query_string() {
        let query: SseQuery = serde_html_form_like(
            "types=turn.started&types=turn.completed&exclude=x&after_sequence=3",
        );
        assert_eq!(query.types, ["turn.started", "turn.completed"]);
        assert_eq!(query.after_sequence, Some(3));
        let filters = Filters {
            types: query.types,
            exclude: query.exclude,
        };
        assert!(filters.pass("turn.started"));
        assert!(!filters.pass("x"));
    }

    fn serde_html_form_like(qs: &str) -> SseQuery {
        let uri: axum::http::Uri = format!("/?{qs}").parse().unwrap();
        Query::<SseQuery>::try_from_uri(&uri).unwrap().0
    }
}
