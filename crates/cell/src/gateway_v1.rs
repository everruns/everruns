//! The Everruns `/v1` session API on a data plane (design.md "Gateways"):
//! the calls the console's chat, the CLI and the SDKs make on a running
//! session, in the control plane's shapes.
//!
//! Decisions:
//! - Stage 1 is chat: send and list messages, list events (with the
//!   control plane's filters and cursors), the SSE stream, cancelling the
//!   running turn, tool approvals and client-side tool results. Every other
//!   call on a session answers 501 with the feature's name, and the console
//!   hides those features for a v2 org, so a client never gets half a shape.
//! - The session record itself (get, list, title, pin, archive) stays with
//!   the control plane, which keeps it for every session it creates; the
//!   edge sends those calls there. A cell knows only what it runs.
//! - Writes carry a command id, so the gateway retries them safely across
//!   a leader change or a move.
//! - The stream is served by the session's stream node and relayed by
//!   `gateway_sse.rs`.

use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::events::Event;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::Lookup;
use crate::gateway::{Gateway, parse};
use crate::stream::StreamQuery;

type ApiError = (StatusCode, String);

pub fn routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            "/v1/sessions/{session}/messages",
            post(send).get(messages),
        )
        .route("/v1/sessions/{session}/events", get(events))
        .route("/v1/sessions/{session}/sse", get(sse))
        .route("/v1/sessions/{session}/cancel", post(cancel_turn))
        .route("/v1/sessions/{session}/tool-approvals", post(approvals))
        .route("/v1/sessions/{session}/tool-results", post(tool_results))
        .route("/v1/sessions/{session}/{*feature}", any(not_yet))
}

/// The code a stage 2 feature answers with, for clients that branch on it.
pub const NOT_YET: &str = "data_plane_v2_unsupported";

/// A session call this data plane does not serve yet.
async fn not_yet(method: Method, Path((_, feature)): Path<(String, String)>) -> Response {
    let feature = feature.split('/').next().unwrap_or_default().to_owned();
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "type": "https://everruns.com/docs/errors/data_plane_v2_unsupported",
            "title": "Not Implemented",
            "status": 501,
            "code": NOT_YET,
            "feature": feature,
            "detail": format!(
                "{method} {feature} is not available yet for sessions on a v2 data plane"
            ),
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct SendBody {
    /// `{"role": "user", "content": [{"type": "text", "text": ...}]}`, as
    /// the control plane takes it.
    #[serde(default)]
    message: Option<Value>,
    /// Or plain text.
    #[serde(default)]
    text: Option<String>,
    /// Makes a retry idempotent.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

/// The text of an input message's content. Anything but text parts is a
/// stage 2 feature.
fn input_text(message: &Value) -> Result<String, Response> {
    let parts = message["content"].as_array().cloned().unwrap_or_default();
    let mut text = Vec::new();
    for part in &parts {
        match part["type"].as_str() {
            Some("text") => text.push(part["text"].as_str().unwrap_or_default().to_owned()),
            other => {
                let kind = other.unwrap_or("unknown").to_owned();
                return Err((
                    StatusCode::NOT_IMPLEMENTED,
                    Json(json!({
                        "title": "Not Implemented",
                        "status": 501,
                        "code": NOT_YET,
                        "feature": format!("message content {kind}"),
                        "detail": format!(
                            "{kind} content is not available yet for sessions on a v2 data plane"
                        ),
                    })),
                )
                    .into_response());
            }
        }
    }
    Ok(text.join("\n"))
}

/// `POST /v1/sessions/{id}/messages`: admit a user message. Answers 201
/// with the message, as the control plane does, once its admission is
/// committed on a quorum.
async fn send(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    RawQuery(raw): RawQuery,
    Json(body): Json<SendBody>,
) -> Response {
    let session = match parse(&session) {
        Ok(session) => session,
        Err(error) => return error.into_response(),
    };
    if raw.as_deref().is_some_and(|q| q.contains("wait=true")) {
        return not_yet(
            Method::POST,
            Path((String::new(), "messages?wait".into())),
        )
        .await;
    }
    let text = match (body.text, &body.message) {
        (Some(text), _) => text,
        (None, Some(message)) => match input_text(message) {
            Ok(text) => text,
            Err(response) => return response,
        },
        (None, None) => {
            return (StatusCode::BAD_REQUEST, "message or text required").into_response();
        }
    };
    let id = match body.id.map(|id| MessageId::parse(&id)).transpose() {
        Ok(id) => id.unwrap_or_default(),
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    match gateway.admit(session, text.clone(), Some(id)).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(json!({
                "id": id,
                "session_id": session,
                "sequence": 0,
                "role": "user",
                "content": [{ "type": "text", "text": text }],
                "metadata": body.metadata,
                "created_at": chrono::Utc::now(),
                "status": "accepted",
            })),
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}

/// Every committed event of `session` after `after`.
async fn read_events(
    gateway: &Arc<Gateway>,
    session: SessionId,
    after: i32,
) -> Result<Vec<Event>, ApiError> {
    let gw = gateway.clone();
    gateway
        .with_cell(session, |cell| {
            let gw = gw.clone();
            async move {
                gw.call::<Lookup<Vec<Event>>>(
                    cell,
                    &format!("/cell/sessions/{session}/events?after={after}"),
                    None,
                )
                .await
            }
        })
        .await
}

/// A message as the control plane lists it, from the event that carries
/// it (`input.message` is the user's, `output.message.completed` the
/// agent's).
fn message_of(event: &Event) -> Option<Value> {
    let role = match event.event_type.as_str() {
        "input.message" => "user",
        "output.message.completed" => "agent",
        _ => return None,
    };
    let data = serde_json::to_value(&event.data).ok()?;
    let message = &data["message"];
    let mut out = json!({
        "id": message["id"],
        "session_id": event.session_id,
        "sequence": event.sequence.unwrap_or(0),
        "role": role,
        "content": message["content"],
        "created_at": event.ts,
    });
    for field in ["phase", "phase_source", "controls", "metadata"] {
        if !message[field].is_null() {
            out[field] = message[field].clone();
        }
    }
    Some(out)
}

/// `GET /v1/sessions/{id}/messages`.
async fn messages(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let events = read_events(&gateway, session, 0).await?;
    let data: Vec<Value> = events.iter().filter_map(message_of).collect();
    Ok(Json(json!({ "data": data })))
}

/// `GET /v1/sessions/{id}/events`'s query, as the control plane takes it.
#[derive(Debug, Default)]
struct EventsQuery {
    stream: StreamQuery,
    limit: Option<usize>,
    before_sequence: Option<i32>,
    order_desc: bool,
}

impl EventsQuery {
    fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        let mut query = Self {
            stream: StreamQuery::parse(raw)?,
            ..Default::default()
        };
        let bad = |what: &str| (StatusCode::BAD_REQUEST, format!("invalid {what}"));
        for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
            match key.as_ref() {
                "limit" => query.limit = Some(value.parse().map_err(|_| bad("limit"))?),
                "before_sequence" => {
                    query.before_sequence = Some(value.parse().map_err(|_| bad("before_sequence"))?)
                }
                "order_desc" => query.order_desc = value == "true",
                // Stage 2 filters: answered by the control plane's trace,
                // not by a cell.
                "around" | "from_ts" | "to_ts" | "turn_id" | "exec_id" | "trace_id" | "tags"
                | "tool_name" | "q" => {
                    return Err((
                        StatusCode::NOT_IMPLEMENTED,
                        format!("the {key} filter is not available yet on a v2 data plane"),
                    ));
                }
                _ => {}
            }
        }
        Ok(query)
    }

    /// Select from `events` (oldest first, after the cursor): filters,
    /// `before_sequence`, then the last `limit` (or the first, newest
    /// first, with `order_desc`).
    fn select(&self, events: Vec<Event>) -> Vec<Event> {
        let mut chosen: Vec<Event> = events
            .into_iter()
            .filter(|e| self.stream.passes(&e.event_type))
            .filter(|e| {
                self.before_sequence
                    .is_none_or(|before| e.sequence.is_some_and(|s| s < before))
            })
            .collect();
        if self.order_desc {
            chosen.reverse();
            if let Some(limit) = self.limit {
                chosen.truncate(limit);
            }
        } else if let Some(limit) = self.limit
            && chosen.len() > limit
        {
            chosen.drain(..chosen.len() - limit);
        }
        chosen
    }
}

/// `GET /v1/sessions/{id}/events`.
async fn events(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    RawQuery(raw): RawQuery,
) -> Response {
    let run = async {
        let session = parse(&session)?;
        let query = EventsQuery::parse(raw.as_deref())?;
        let mut events = read_events(&gateway, session, query.stream.after_sequence.unwrap_or(0))
            .await?;
        if let Some(since) = &query.stream.since_id {
            let at = events
                .iter()
                .position(|e| e.id.to_string() == *since)
                .ok_or((StatusCode::BAD_REQUEST, format!("unknown since_id {since}")))?;
            events.drain(..=at);
        }
        let total = events.len();
        let chosen = query.select(events);
        let mut headers = HeaderMap::new();
        if query.limit.is_some() {
            headers.insert("x-total-count", HeaderValue::from(total));
            headers.insert(
                header::ACCESS_CONTROL_EXPOSE_HEADERS,
                HeaderValue::from_static("X-Total-Count"),
            );
        }
        Ok::<_, ApiError>((headers, Json(json!({ "data": chosen }))).into_response())
    };
    run.await.unwrap_or_else(IntoResponse::into_response)
}

/// `POST /v1/sessions/{id}/cancel`: cancel the running turn.
async fn cancel_turn(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let body = json!({ "command": uuid::Uuid::new_v4() });
    Ok(Json(
        gateway
            .session_command(session, "cancel-turn", &body)
            .await?,
    ))
}

#[derive(Deserialize)]
struct ApprovalsBody {
    decisions: Vec<Value>,
    #[serde(default)]
    command_id: Option<uuid::Uuid>,
}

/// `POST /v1/sessions/{id}/tool-approvals`.
async fn approvals(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Json(body): Json<ApprovalsBody>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    if body.decisions.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "decisions must not be empty".into()));
    }
    let command = body.command_id.unwrap_or_else(uuid::Uuid::new_v4);
    let body = json!({ "command": command, "decisions": body.decisions });
    Ok(Json(
        gateway.session_command(session, "approvals", &body).await?,
    ))
}

#[derive(Deserialize)]
struct ToolResultsBody {
    tool_results: Vec<Value>,
    #[serde(default)]
    command_id: Option<uuid::Uuid>,
}

/// `POST /v1/sessions/{id}/tool-results`.
async fn tool_results(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Json(body): Json<ToolResultsBody>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    if body.tool_results.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "tool_results must not be empty".into(),
        ));
    }
    let command = body.command_id.unwrap_or_else(uuid::Uuid::new_v4);
    let body = json!({ "command": command, "tool_results": body.tool_results });
    Ok(Json(
        gateway
            .session_command(session, "tool-results", &body)
            .await?,
    ))
}

/// `GET /v1/sessions/{id}/sse`: from the session's stream node.
async fn sse(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    RawQuery(raw): RawQuery,
) -> Response {
    match crate::gateway_sse::stream(&gateway, &session, raw.as_deref()).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{EventContext, SessionActivatedData};

    fn events(n: i32) -> Vec<Event> {
        let session = SessionId::new();
        (1..=n)
            .map(|seq| {
                let mut event = Event::new(
                    session,
                    EventContext::default(),
                    SessionActivatedData {
                        turn_id: everruns_contracts::typed_id::TurnId::new(),
                        input_message_id: MessageId::new(),
                    },
                );
                event.sequence = Some(seq);
                event
            })
            .collect()
    }

    fn seqs(events: &[Event]) -> Vec<i32> {
        events.iter().filter_map(|e| e.sequence).collect()
    }

    #[test]
    fn limit_takes_the_last_events_oldest_first() {
        let query = EventsQuery::parse(Some("limit=2")).unwrap();
        assert_eq!(seqs(&query.select(events(5))), vec![4, 5]);
    }

    #[test]
    fn order_desc_takes_the_newest_first() {
        let query = EventsQuery::parse(Some("limit=1&order_desc=true")).unwrap();
        assert_eq!(seqs(&query.select(events(5))), vec![5]);
    }

    #[test]
    fn before_sequence_pages_backwards() {
        let query = EventsQuery::parse(Some("limit=2&before_sequence=4")).unwrap();
        assert_eq!(seqs(&query.select(events(5))), vec![2, 3]);
    }

    #[test]
    fn types_and_exclude_filter() {
        let query = EventsQuery::parse(Some("types=session.activated&exclude=x")).unwrap();
        assert_eq!(query.select(events(3)).len(), 3);
        let query = EventsQuery::parse(Some("exclude=session.activated")).unwrap();
        assert!(query.select(events(3)).is_empty());
    }

    #[test]
    fn stage_two_filters_are_refused() {
        assert_eq!(
            EventsQuery::parse(Some("q=hello")).unwrap_err().0,
            StatusCode::NOT_IMPLEMENTED
        );
    }
}
