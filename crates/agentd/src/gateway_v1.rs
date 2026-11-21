//! The Everruns `/v1` session API on a data plane (design.md "Gateways"):
//! the calls the console's chat, the CLI and the SDKs make on a running
//! session, in the control plane's shapes.
//!
//! Decisions:
//! - Stage 1 is chat: send and list messages, list events (with the
//!   control plane's filters and cursors), the SSE stream, cancelling the
//!   running turn, tool approvals and client-side tool results; the
//!   session's files are `gateway_fs.rs` (step 11.1), and its schedules
//!   are here since step 11.4 (`platform.rs`), with its tasks (`tasks.rs`;
//!   cancelling and messaging one since 11.6, their push configs since
//!   parity Phase B), its
//!   images and files (`gateway_input.rs`) and its Work tab reads
//!   (`gateway_work.rs`). Every other call on a
//!   session answers 501 with the feature's name, and the console hides
//!   those features for a v2 org, so a client never gets half a shape.
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
use everruns_contracts::typed_id::{MessageId, ScheduleId, SessionId};
use everruns_core::events::Event;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::Lookup;
use crate::gateway::{Gateway, parse};
use crate::stream::StreamQuery;

type ApiError = (StatusCode, String);

pub fn routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route("/v1/sessions/{session}/messages", post(send).get(messages))
        .route("/v1/sessions/{session}/events", get(events))
        .route("/v1/sessions/{session}/events/summary", get(events_summary))
        .route("/v1/sessions/{session}/sse", get(sse))
        .route("/v1/sessions/{session}/cancel", post(cancel_turn))
        .route("/v1/sessions/{session}/tool-approvals", post(approvals))
        .route("/v1/sessions/{session}/tool-results", post(tool_results))
        .route("/v1/sessions/{session}/schedules", get(list_schedules))
        .route(
            "/v1/sessions/{session}/schedules/{schedule}",
            get(get_schedule)
                .patch(update_schedule)
                .delete(delete_schedule),
        )
        .route(
            "/v1/sessions/{session}/schedules/{schedule}/trigger",
            post(trigger_schedule),
        )
        .route("/v1/sessions/{session}/tasks", get(list_tasks))
        .route("/v1/sessions/{session}/tasks/{task}", get(get_task))
        .route(
            "/v1/sessions/{session}/tasks/{task}/cancel",
            post(cancel_task),
        )
        .route(
            "/v1/sessions/{session}/work",
            get(crate::gateway_work::work),
        )
        .route(
            "/v1/sessions/{session}/resources",
            get(crate::gateway_work::resources),
        )
        .route(
            "/v1/sessions/{session}/tasks/{task}/messages",
            post(message_task),
        )
        .route(
            "/v1/sessions/{session}/tasks/{task}/push-configs",
            get(list_push_configs).post(create_push_config),
        )
        .route(
            "/v1/sessions/{session}/tasks/{task}/push-configs/{config}",
            axum::routing::delete(delete_push_config),
        )
        // The session's SQL databases (`sqldb.rs`).
        .merge(crate::sqldb::gateway_routes())
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

/// An input message's parts, inline images written to the session's
/// files first (`gateway_input.rs`).
async fn input_parts(
    gateway: &Gateway,
    session: SessionId,
    message: &Value,
) -> Result<Vec<everruns_core::ContentPart>, Response> {
    use crate::gateway_input::{Part, Refused};
    let parts = match crate::gateway_input::parts(message) {
        Ok(parts) => parts,
        Err(Refused::Kind(kind)) => return Err(unsupported_content(&kind)),
        Err(Refused::Invalid(reason)) => {
            return Err((StatusCode::BAD_REQUEST, reason).into_response());
        }
    };
    let mut content = Vec::with_capacity(parts.len());
    for part in parts {
        match part {
            Part::Ready(part) => content.push(part),
            Part::Inline(part, file) => {
                let op = json!({ "op": "seed", "files": [file] });
                let reply = gateway
                    .session_command(session, "fs", &op)
                    .await
                    .map_err(IntoResponse::into_response)?;
                if reply["result"] == "failed" {
                    let status = reply["status"]
                        .as_u64()
                        .and_then(|s| StatusCode::from_u16(s as u16).ok())
                        .unwrap_or(StatusCode::BAD_REQUEST);
                    let message = reply["message"]
                        .as_str()
                        .unwrap_or("the image was not kept");
                    return Err((status, message.to_owned()).into_response());
                }
                content.push(part);
            }
        }
    }
    Ok(content)
}

/// 501 for an input content kind the data plane does not take yet.
fn unsupported_content(kind: &str) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "title": "Not Implemented",
            "status": 501,
            "code": NOT_YET,
            "feature": format!("message content {kind}"),
            "detail": format!("{kind} content is not available yet for sessions on a v2 data plane"),
        })),
    )
        .into_response()
}

/// `POST /v1/sessions/{id}/messages`: admit a user message. Answers 201
/// with the message, as the control plane does, once its admission is
/// committed on a quorum.
async fn send(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    RawQuery(raw): RawQuery,
    claims: Option<axum::Extension<crate::org_token::OrgTokenClaims>>,
    Json(body): Json<SendBody>,
) -> Response {
    let session = match parse(&session) {
        Ok(session) => session,
        Err(error) => return error.into_response(),
    };
    let wait = crate::gateway_wait::requested(raw.as_deref());
    let id = match body.id.map(|id| MessageId::parse(&id)).transpose() {
        Ok(id) => id.unwrap_or_default(),
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let parts = match (body.text, &body.message) {
        (Some(text), _) => vec![everruns_core::ContentPart::text(text)],
        (None, Some(message)) => match input_parts(&gateway, session, message).await {
            Ok(parts) => parts,
            Err(refused) => return refused,
        },
        (None, None) => {
            return (StatusCode::BAD_REQUEST, "message or text required").into_response();
        }
    };
    // The person the verified org token names (the control plane mints
    // `sub` as the user id, for a console session or the user's API key),
    // so the turn's platform tools act as them.
    let sender = claims
        .and_then(|axum::Extension(claims)| uuid::Uuid::parse_str(&claims.sub).ok())
        .map(|user| user.to_string());
    let admitted = match crate::gateway_input::text_only(&parts) {
        Some(text) => {
            gateway
                .admit_from(session, text, Vec::new(), Some(id), sender)
                .await
        }
        None => {
            gateway
                .admit_from(session, String::new(), parts.clone(), Some(id), sender)
                .await
        }
    };
    if let Err(error) = admitted {
        return error.into_response();
    }
    let accepted = json!({
        "id": id,
        "session_id": session,
        "sequence": 0,
        "role": "user",
        "content": parts,
        "metadata": body.metadata,
        "created_at": chrono::Utc::now(),
        "status": "accepted",
    });
    match wait {
        Some(budget) => crate::gateway_wait::answer(&gateway, session, id, accepted, budget).await,
        None => (StatusCode::CREATED, Json(accepted)).into_response(),
    }
}

/// Every committed event of `session` after `after`.
pub(crate) async fn read_events(
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
pub(crate) fn message_of(event: &Event) -> Option<Value> {
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

/// Longest `q` taken, as on the control plane.
const MAX_Q_BYTES: usize = 512;

/// An RFC 3339 time.
fn timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// `GET /v1/sessions/{id}/events`'s query, as the control plane takes it.
#[derive(Debug, Default)]
struct EventsQuery {
    stream: StreamQuery,
    limit: Option<usize>,
    before_sequence: Option<i32>,
    order_desc: bool,
    search: Search,
}

/// The control plane's debugging filters (`ListEvents`), applied to the
/// session's events here (step 11.6). `q` is a case-insensitive substring
/// match over the event's data, the control plane's in-memory fallback for
/// its full-text search.
#[derive(Debug, Default)]
struct Search {
    around: Option<String>,
    window: Option<usize>,
    from_ts: Option<chrono::DateTime<chrono::Utc>>,
    to_ts: Option<chrono::DateTime<chrono::Utc>>,
    turn_id: Option<String>,
    exec_id: Option<String>,
    trace_id: Option<String>,
    tags: Vec<String>,
    tool_name: Option<String>,
    q: Option<String>,
}

/// Events on each side of `around` by default, and at most.
const AROUND_WINDOW: usize = 50;
const MAX_AROUND_WINDOW: usize = 500;

impl Search {
    fn passes(&self, event: &Event) -> bool {
        let context = &event.context;
        let text = |id: Option<String>, want: &Option<String>| {
            want.as_ref().is_none_or(|want| id.as_deref() == Some(want))
        };
        self.from_ts.is_none_or(|from| event.ts >= from)
            && self.to_ts.is_none_or(|to| event.ts <= to)
            && text(context.turn_id.map(|t| t.to_string()), &self.turn_id)
            && text(context.exec_id.map(|t| t.to_string()), &self.exec_id)
            && text(context.trace_id.clone(), &self.trace_id)
            && (self.tags.is_empty()
                || event
                    .tags
                    .as_ref()
                    .is_some_and(|tags| tags.iter().any(|t| self.tags.contains(t))))
            && self.data_passes(event)
    }

    fn data_passes(&self, event: &Event) -> bool {
        if self.tool_name.is_none() && self.q.is_none() {
            return true;
        }
        let Ok(data) = serde_json::to_value(&event.data) else {
            return false;
        };
        if let Some(tool) = &self.tool_name {
            let named = data["tool_name"]
                .as_str()
                .or_else(|| data["tool_call"]["name"].as_str());
            if named != Some(tool.as_str()) {
                return false;
            }
        }
        match &self.q {
            Some(q) => data.to_string().to_lowercase().contains(&q.to_lowercase()),
            None => true,
        }
    }

    /// `events` cut to the window around the anchor, when one is named.
    fn around(&self, events: Vec<Event>) -> Result<Vec<Event>, ApiError> {
        let Some(anchor) = &self.around else {
            return Ok(events);
        };
        let at = events
            .iter()
            .position(|e| e.id.to_string() == *anchor)
            .ok_or((StatusCode::NOT_FOUND, format!("event {anchor} not found")))?;
        let window = self
            .window
            .unwrap_or(AROUND_WINDOW)
            .clamp(1, MAX_AROUND_WINDOW);
        let end = (at + window + 1).min(events.len());
        Ok(events
            .into_iter()
            .take(end)
            .skip(at.saturating_sub(window))
            .collect())
    }
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
                "around" => query.search.around = Some(value.into_owned()),
                "window" => query.search.window = Some(value.parse().map_err(|_| bad("window"))?),
                "from_ts" => query.search.from_ts = Some(timestamp(&value).ok_or(bad("from_ts"))?),
                "to_ts" => query.search.to_ts = Some(timestamp(&value).ok_or(bad("to_ts"))?),
                "turn_id" => query.search.turn_id = Some(value.into_owned()),
                "exec_id" => query.search.exec_id = Some(value.into_owned()),
                "trace_id" => query.search.trace_id = Some(value.into_owned()),
                "tags" | "tags[]" => query.search.tags.push(value.into_owned()),
                "tool_name" => query.search.tool_name = Some(value.into_owned()),
                "q" if value.len() > MAX_Q_BYTES => return Err(bad("q: too long")),
                "q" => query.search.q = Some(value.into_owned()),
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
            .filter(|e| self.search.passes(e))
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
        let mut events =
            read_events(&gateway, session, query.stream.after_sequence.unwrap_or(0)).await?;
        if let Some(since) = &query.stream.since_id {
            let at = events
                .iter()
                .position(|e| e.id.to_string() == *since)
                .ok_or((StatusCode::BAD_REQUEST, format!("unknown since_id {since}")))?;
            events.drain(..=at);
        }
        let events = query.search.around(events)?;
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

/// `GET /v1/sessions/{id}/events/summary`: counts by type (by name), first
/// and last times, turns and errors, as the control plane's
/// `events_summary` counts them.
async fn events_summary(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let events = read_events(&gateway, session, 0).await?;
    Ok(Json(summary(&events)))
}

fn summary(events: &[Event]) -> Value {
    let mut by_type: std::collections::BTreeMap<&str, i64> = Default::default();
    for event in events {
        *by_type.entry(event.event_type.as_str()).or_default() += 1;
    }
    let turn_count = by_type.get("turn.started").copied().unwrap_or(0);
    let error_count: i64 = by_type
        .iter()
        .filter(|(kind, _)| {
            matches!(**kind, "turn.failed" | "tool.failed" | "subagent.failed")
                || kind.ends_with(".error")
        })
        .map(|(_, count)| count)
        .sum();
    json!({
        "total": events.len(),
        "by_type": by_type
            .iter()
            .map(|(kind, count)| json!({ "event_type": kind, "count": count }))
            .collect::<Vec<_>>(),
        "first_ts": events.iter().map(|e| e.ts).min(),
        "last_ts": events.iter().map(|e| e.ts).max(),
        "turn_count": turn_count,
        "error_count": error_count,
    })
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
        return Err((
            StatusCode::BAD_REQUEST,
            "decisions must not be empty".into(),
        ));
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
    fn summary_counts_by_type() {
        let summary = summary(&events(3));
        assert_eq!(summary["total"], 3);
        assert_eq!(summary["by_type"][0]["event_type"], "session.activated");
        assert_eq!(summary["by_type"][0]["count"], 3);
        assert_eq!(summary["turn_count"], 0);
    }

    #[test]
    fn search_filters_match_the_control_planes() {
        let all = events(3);
        let anchor = all[1].id.to_string();
        let query = EventsQuery::parse(Some(&format!("around={anchor}&window=1"))).unwrap();
        assert_eq!(
            seqs(&query.search.around(all.clone()).unwrap()),
            vec![1, 2, 3]
        );
        let query = EventsQuery::parse(Some(&format!("around={anchor}&window=0"))).unwrap();
        assert_eq!(
            seqs(&query.search.around(all.clone()).unwrap()),
            vec![1, 2, 3]
        );
        let turn = serde_json::to_value(&all[1].data).unwrap()["turn_id"]
            .as_str()
            .unwrap()
            .to_uppercase();
        let query = EventsQuery::parse(Some(&format!("q={turn}"))).unwrap();
        assert_eq!(seqs(&query.select(all.clone())), vec![2]);
        let query = EventsQuery::parse(Some("turn_id=turn_x")).unwrap();
        assert!(query.select(all.clone()).is_empty());
        let query = EventsQuery::parse(Some("from_ts=2000-01-01T00:00:00Z")).unwrap();
        assert_eq!(query.select(all.clone()).len(), 3);
        assert!(EventsQuery::parse(Some("from_ts=yesterday")).is_err());
        assert!(EventsQuery::parse(Some(&format!("q={}", "a".repeat(513)))).is_err());
    }
}

/// A schedule command on the session's cell; a schedule it does not hold
/// answers 404, as on the control plane.
async fn schedule_command(
    gateway: &Gateway,
    session: SessionId,
    action: Option<crate::platform::Op>,
) -> Result<Value, ApiError> {
    gateway
        .session_command(session, "schedules", &json!({ "action": action }))
        .await
        .map_err(|(status, reason)| {
            if reason == crate::platform::SCHEDULE_NOT_FOUND {
                (StatusCode::NOT_FOUND, "Schedule not found".into())
            } else {
                (status, reason)
            }
        })
}

fn schedule_id(id: &str) -> Result<ScheduleId, ApiError> {
    ScheduleId::parse(id).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

/// `GET /v1/sessions/{id}/schedules`.
async fn list_schedules(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    Ok(Json(schedule_command(&gateway, session, None).await?))
}

/// `GET /v1/sessions/{id}/schedules/{schedule_id}`.
async fn get_schedule(
    State(gateway): State<Arc<Gateway>>,
    Path((session, schedule)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let schedule = schedule_id(&schedule)?.to_string();
    let listed = schedule_command(&gateway, session, None).await?;
    listed
        .as_array()
        .and_then(|all| all.iter().find(|s| s["id"] == schedule.as_str()))
        .cloned()
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "Schedule not found".into()))
}

/// `PATCH /v1/sessions/{id}/schedules/{schedule_id}`: the control plane's
/// body; an empty one enables the schedule. `budget_usd` caps what its
/// runs spend (`platform.rs`).
async fn update_schedule(
    State(gateway): State<Arc<Gateway>>,
    Path((session, schedule)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let id = schedule_id(&schedule)?;
    let mut change: crate::platform::ScheduleChange =
        serde_json::from_value(body).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if change
        .budget_usd
        .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "budget_usd must be a positive amount in USD".into(),
        ));
    }
    if change.enabled.is_none()
        && change.ends_at.is_none()
        && change.max_runs.is_none()
        && change.budget_usd.is_none()
    {
        change.enabled = Some(true);
    }
    change.deleted = false;
    if matches!(change.max_runs, Some(Some(0))) {
        return Err((
            StatusCode::BAD_REQUEST,
            "max_runs must be at least 1".into(),
        ));
    }
    let action = crate::platform::Op::UpdateSchedule { id, change };
    Ok(Json(
        schedule_command(&gateway, session, Some(action)).await?,
    ))
}

/// `DELETE /v1/sessions/{id}/schedules/{schedule_id}`.
async fn delete_schedule(
    State(gateway): State<Arc<Gateway>>,
    Path((session, schedule)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let session = parse(&session)?;
    let id = schedule_id(&schedule)?;
    let change = crate::platform::ScheduleChange {
        deleted: true,
        ..Default::default()
    };
    let action = crate::platform::Op::UpdateSchedule { id, change };
    schedule_command(&gateway, session, Some(action)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/sessions/{id}/schedules/{schedule_id}/trigger`: run it now.
async fn trigger_schedule(
    State(gateway): State<Arc<Gateway>>,
    Path((session, schedule)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let id = schedule_id(&schedule)?;
    let action = crate::platform::Op::TriggerSchedule { id };
    let fired = schedule_command(&gateway, session, Some(action)).await?;
    if fired["fired"] != true {
        let reason = fired["reason"]
            .as_str()
            .unwrap_or("the session refused the run");
        return Err((StatusCode::CONFLICT, reason.to_owned()));
    }
    Ok(Json(fired["schedule"].clone()))
}

#[derive(Debug, Default, Deserialize)]
struct TaskQuery {
    state: Option<String>,
    kind: Option<String>,
    after_id: Option<String>,
    limit: Option<usize>,
}

/// `GET /v1/sessions/{id}/tasks`, with the control plane's filters.
async fn list_tasks(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    axum::extract::Query(query): axum::extract::Query<TaskQuery>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let listed = gateway
        .session_command(session, "tasks", &json!({}))
        .await?;
    let keep = |task: &Value, field: &str, wanted: &Option<String>| {
        wanted.as_deref().is_none_or(|w| task[field] == w)
    };
    let tasks: Vec<Value> = listed
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| keep(t, "state", &query.state) && keep(t, "kind", &query.kind))
        .collect();
    Ok(Json(json!(tasks)))
}

/// `GET /v1/sessions/{id}/tasks/{task_id}`: the task and a page of its
/// messages.
async fn get_task(
    State(gateway): State<Arc<Gateway>>,
    Path((session, task)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<TaskQuery>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let body = json!({ "task": task, "after_id": query.after_id, "limit": query.limit });
    let detail = gateway.session_command(session, "tasks", &body).await?;
    if detail.is_null() {
        return Err((StatusCode::NOT_FOUND, "Task not found".into()));
    }
    Ok(Json(detail))
}

/// A task write (`tasks::api_op`) on the session's cell; 404 for a task
/// the session does not have.
async fn write_task(
    gateway: &Gateway,
    session: &str,
    body: Value,
) -> Result<Json<Value>, ApiError> {
    let session = parse(session)?;
    let written = gateway.session_command(session, "task", &body).await?;
    if written.is_null() {
        return Err((StatusCode::NOT_FOUND, "Session task not found".into()));
    }
    Ok(Json(written))
}

/// `POST /v1/sessions/{id}/tasks/{task_id}/cancel`: request the task's
/// cooperative cancel; answers the task.
async fn cancel_task(
    State(gateway): State<Arc<Gateway>>,
    Path((session, task)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    write_task(&gateway, &session, json!({ "task": task, "cancel": true })).await
}

/// `POST /v1/sessions/{id}/tasks/{task_id}/messages`: a person's message
/// to the task (`text` or `content`, `in_reply_to`); answers the message.
async fn message_task(
    State(gateway): State<Arc<Gateway>>,
    Path((session, task)): Path<(String, String)>,
    Json(mut body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if !body.is_object() {
        return Err((StatusCode::BAD_REQUEST, "a JSON object is required".into()));
    }
    body["task"] = json!(task);
    body["cancel"] = json!(false);
    write_task(&gateway, &session, body).await
}

/// `GET /v1/sessions/{id}/tasks/{task_id}/push-configs`: the task's push
/// configs, secrets never shown.
async fn list_push_configs(
    State(gateway): State<Arc<Gateway>>,
    Path((session, task)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let listed = gateway
        .session_command(
            session,
            "push-configs",
            &json!({ "op": "list", "task": task }),
        )
        .await?;
    if listed.is_null() {
        return Err((StatusCode::NOT_FOUND, "Session task not found".into()));
    }
    Ok(Json(listed))
}

#[derive(Deserialize)]
struct PushConfigBody {
    url: String,
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    event_filter: Option<Vec<String>>,
}

/// `POST /v1/sessions/{id}/tasks/{task_id}/push-configs`: add a webhook
/// for the task's transitions, with the control plane's checks: an address
/// that is not private or internal (delivery pins DNS too), and known
/// event names.
async fn create_push_config(
    State(gateway): State<Arc<Gateway>>,
    Path((session, task)): Path<(String, String)>,
    Json(body): Json<PushConfigBody>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    everruns_contracts::url_validation::validate_safe_url(&body.url).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid webhook URL: {error}"),
        )
    })?;
    let event_filter = crate::tasks::event_filter(body.event_filter)
        .map_err(|reason| (StatusCode::BAD_REQUEST, reason))?;
    let now = chrono::Utc::now();
    // Made once, before the gateway's retries, so a retry adds it once.
    let config = crate::tasks::PushConfig {
        id: format!("tpc_{}", uuid::Uuid::new_v4().simple()),
        url: body.url,
        has_secret: false,
        event_filter,
        created_at: now,
        updated_at: now,
    };
    let added = gateway
        .session_command(
            session,
            "push-configs",
            &json!({ "op": "add", "task": task, "config": config, "secret": body.secret }),
        )
        .await?;
    if added.is_null() {
        return Err((StatusCode::NOT_FOUND, "Session task not found".into()));
    }
    Ok(Json(added))
}

/// `DELETE /v1/sessions/{id}/tasks/{task_id}/push-configs/{config_id}`.
async fn delete_push_config(
    State(gateway): State<Arc<Gateway>>,
    Path((session, task, config)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let session = parse(&session)?;
    let deleted = gateway
        .session_command(
            session,
            "push-configs",
            &json!({ "op": "delete", "task": task, "config_id": config }),
        )
        .await?;
    if deleted.is_null() {
        return Err((StatusCode::NOT_FOUND, "Session task not found".into()));
    }
    if deleted["deleted"] != true {
        return Err((StatusCode::NOT_FOUND, "Push config not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The public API's session routes against the declared plane table
/// (`everruns_contracts::session_commands`): every route of a
/// [`Plane::Cell`](everruns_contracts::session_commands::Plane::Cell)
/// segment is served here, unless it is a declared gap, and every
/// declared gap still answers "not available yet", so closing one fails
/// until the table says so.
#[cfg(test)]
mod route_tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::Request;
    use everruns_contracts::session_commands::{
        CELL_ROUTE_GAPS, MESSAGE_RECORD_ROUTES, Plane, plane,
    };
    use tower::ServiceExt as _;

    use super::*;
    use crate::cluster::{Topology, TopologySource};

    /// `(method, path relative to the session)` of every session route in
    /// the public API's document.
    fn session_routes() -> Vec<(String, String)> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/api/openapi.json");
        let spec: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut routes = Vec::new();
        for (path, item) in spec["paths"].as_object().unwrap() {
            let Some(rest) = path.strip_prefix("/v1/sessions/{session_id}/") else {
                continue;
            };
            for method in item.as_object().unwrap().keys() {
                if ["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                    routes.push((method.to_uppercase(), rest.to_owned()));
                }
            }
        }
        assert!(routes.len() > 20, "the API document lists session routes");
        routes
    }

    /// Whether the cell answered "not available yet" (or not at all).
    async fn not_served(app: &Router, method: &str, path: &str) -> bool {
        let session = SessionId::new();
        let path = path
            .split('/')
            .map(|part| if part.starts_with('{') { "x" } else { part })
            .collect::<Vec<_>>()
            .join("/");
        let request = Request::builder()
            .method(method)
            .uri(format!("/v1/sessions/{session}/{path}"))
            .body(Body::empty())
            .unwrap();
        // A served route fails on the missing body or the unreachable cell;
        // a slow one is still served.
        match tokio::time::timeout(Duration::from_secs(10), app.clone().oneshot(request)).await {
            Ok(response) => {
                let status = response.unwrap().status();
                status == StatusCode::NOT_IMPLEMENTED || status == StatusCode::METHOD_NOT_ALLOWED
            }
            Err(_) => false,
        }
    }

    #[tokio::test]
    async fn cell_routes_are_served_and_gaps_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("topology.json");
        // A cell nobody listens on: every call that reaches it fails fast.
        let topology = Topology {
            directory_cell: 0,
            cells: BTreeMap::from([(0, BTreeMap::from([(1, "127.0.0.1:1".to_owned())]))]),
        };
        std::fs::write(&path, serde_json::to_vec(&topology).unwrap()).unwrap();
        let app = routes().with_state(Gateway::new(TopologySource { path }));

        let mut wrong = Vec::new();
        for (method, path) in session_routes() {
            let mut parts = path.split('/');
            let first = parts.next().unwrap_or_default();
            if first == "messages"
                && parts
                    .nth(1)
                    .is_some_and(|call| MESSAGE_RECORD_ROUTES.contains(&call))
            {
                continue;
            }
            let gap = match plane(first) {
                Plane::Record => continue,
                Plane::Gap => true,
                Plane::Cell => CELL_ROUTE_GAPS.contains(&path.as_str()),
            };
            if not_served(&app, &method, &path).await != gap {
                wrong.push(format!(
                    "{method} {path}: {}",
                    if gap {
                        "served by the cell, but declared a gap"
                    } else {
                        "not served by the cell, but declared the cell's"
                    }
                ));
            }
        }
        assert!(
            wrong.is_empty(),
            "update crates/contracts/src/session_commands.rs:\n{}",
            wrong.join("\n")
        );
    }
}
