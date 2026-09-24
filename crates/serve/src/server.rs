//! The `/v1` wire API. The same routes are served by `dev`, by a self-hosted
//! `start`, and (by contract) by a hosted deploy.
//!
//! One change from eve: there is no continuation token. Every event carries
//! its position in the session's log as the SSE `id`, and a client resumes by
//! sending that id back as `Last-Event-ID` (or `?after=`).

use std::convert::Infallible;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};

use crate::app::Mode;
use crate::channel::Inbound;
use crate::host::{ApiError, Host};
use crate::store::WireEvent;

pub(crate) fn router(host: Arc<Host>) -> Router {
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/v1/agent", get(agent_card))
        .route("/v1/sessions", post(create_default))
        .route("/v1/agents/{name}/sessions", post(create_named))
        .route("/v1/sessions/{id}", get(session))
        .route("/v1/sessions/{id}/messages", post(message))
        .route("/v1/sessions/{id}/events", get(events))
        .route("/v1/sessions/{id}/cancel", post(cancel))
        .route("/v1/sessions/{id}/approvals/{approval}", post(approval))
        .route("/v1/channels/{name}", post(channel));
    if host.mode == Mode::Dev {
        // Dev only: fire a schedule without waiting for its cron.
        router = router.route("/dev/schedules/{name}", post(run_schedule));
    }
    router.with_state(host)
}

struct Failure(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for Failure {
    fn from(err: E) -> Self {
        Failure(err.into())
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let message = format!("{:#}", self.0);
        let (status, build) = match self.0.downcast_ref::<ApiError>() {
            Some(ApiError::NotFound(_)) => (StatusCode::NOT_FOUND, None),
            Some(ApiError::BadRequest(_)) => (StatusCode::BAD_REQUEST, None),
            Some(ApiError::Pinned { build_id }) => (StatusCode::CONFLICT, Some(build_id.clone())),
            None => (StatusCode::INTERNAL_SERVER_ERROR, None),
        };
        let mut response = (status, Json(json!({ "error": message }))).into_response();
        if let Some(build) = build.and_then(|b| HeaderValue::from_str(&b).ok()) {
            response.headers_mut().insert("x-serve-build", build);
        }
        response
    }
}

type ApiResult<T> = Result<T, Failure>;

async fn health(State(host): State<Arc<Host>>) -> Json<Value> {
    Json(json!({ "status": "ok", "build_id": host.build_id, "experimental": true }))
}

async fn agent_card(State(host): State<Arc<Host>>) -> Json<Value> {
    let manifest = host.app.manifest();
    Json(json!({
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
    }))
}

#[derive(Deserialize, Default)]
struct CreateSession {
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    metadata: Value,
}

async fn create_default(
    State(host): State<Arc<Host>>,
    body: Option<Json<CreateSession>>,
) -> ApiResult<Response> {
    create(host, None, body.map(|b| b.0).unwrap_or_default()).await
}

async fn create_named(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
    body: Option<Json<CreateSession>>,
) -> ApiResult<Response> {
    create(host, Some(name), body.map(|b| b.0).unwrap_or_default()).await
}

async fn create(
    host: Arc<Host>,
    agent: Option<String>,
    body: CreateSession,
) -> ApiResult<Response> {
    let id = host
        .create_session(agent.as_deref(), body.metadata, None)
        .await?;
    if let Some(input) = body.input.filter(|input| !input.is_empty()) {
        // The turn runs on; the client follows it on the event stream.
        host.send(&id, input).await?;
    }
    let row = host.session_row(&id)?;
    let location = format!("/v1/sessions/{id}");
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(json!({ "id": id, "agent": row.agent, "build_id": row.build_id })),
    )
        .into_response())
}

async fn session(State(host): State<Arc<Host>>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(host.session_row(&id)?)?))
}

#[derive(Deserialize)]
struct Message {
    input: String,
}

async fn message(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Json(body): Json<Message>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    host.send(&id, body.input).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "accepted": true }))))
}

async fn cancel(State(host): State<Arc<Host>>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let cancelled = host.cancel(&id).await?;
    Ok(Json(json!({ "cancelled": cancelled })))
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
    #[serde(default)]
    note: Option<String>,
}

async fn approval(
    State(host): State<Arc<Host>>,
    Path((id, approval)): Path<(String, String)>,
    Json(body): Json<ApprovalBody>,
) -> ApiResult<Json<Value>> {
    host.session_row(&id)?;
    let approve = matches!(body.decision, DecisionKind::Approve);
    if !host.resolve_approval(&id, &approval, approve, body.note)? {
        return Err(ApiError::NotFound(format!("pending approval {approval}")).into());
    }
    Ok(Json(json!({ "resolved": true })))
}

async fn channel(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let inbound = Inbound {
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
    Ok(Json(host.inbound(&name, inbound).await?))
}

async fn run_schedule(
    State(host): State<Arc<Host>>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    host.run_schedule(&name).await?;
    Ok(Json(json!({ "ran": name })))
}

#[derive(Deserialize)]
struct EventsQuery {
    /// Resume after this sequence number (same as `Last-Event-ID`).
    after: Option<i64>,
    /// `false` replays the log and closes instead of following it live.
    follow: Option<bool>,
}

async fn events(
    State(host): State<Arc<Host>>,
    Path(id): Path<String>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    host.session_row(&id)?;
    let cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .or(query.after)
        .unwrap_or(0);
    let follow = query.follow.unwrap_or(true);

    // Subscribe before replaying, then skip anything the replay covered, so no
    // event falls between the two.
    let live = host.events.subscribe();
    let (tx, rx) = mpsc::channel::<WireEvent>(256);
    tokio::spawn(forward(host, id, cursor, follow, live, tx));
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        let event = rx.recv().await?;
        let sse = Event::default()
            .id(event.seq.to_string())
            .event(event.kind.clone())
            .data(serde_json::to_string(&event).unwrap_or_default());
        Some((Ok(sse), rx))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn forward(
    host: Arc<Host>,
    id: String,
    mut last: i64,
    follow: bool,
    mut live: broadcast::Receiver<WireEvent>,
    tx: mpsc::Sender<WireEvent>,
) {
    let replay = |host: &Host, after: i64| host.events_after(&id, after).unwrap_or_default();
    for event in replay(&host, last) {
        last = event.seq;
        if tx.send(event).await.is_err() {
            return;
        }
    }
    if !follow {
        return;
    }
    loop {
        match live.recv().await {
            Ok(event) if event.session_id == id && event.seq > last => {
                last = event.seq;
                if tx.send(event).await.is_err() {
                    return;
                }
            }
            Ok(_) => {}
            // Fell behind the in-memory fan-out: the log has everything.
            Err(broadcast::error::RecvError::Lagged(_)) => {
                for event in replay(&host, last) {
                    last = event.seq;
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}
