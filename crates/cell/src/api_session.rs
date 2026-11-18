//! A node's session commands behind the Everruns `/v1` session API
//! (`gateway_v1.rs`): answering tool approvals, client-side tool results,
//! cancelling the running turn, the session's schedules, a channel's
//! prior conversation (step 11.5), and the events the control plane records
//! on a session and its storage (11.6). Each answers
//! a [`CellReply`], so the gateway follows moves and freezes the same way
//! it does for admissions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns::hosting::SessionHosting as _;
use everruns_contracts::execution_api::ToolApprovalAnswer;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::events::EventData;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{CellReply, Lookup, reply_of};
use crate::node::Node;
use crate::state::Placement;

type ApiError = (StatusCode, String);

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/cell/sessions/{session}/approvals", post(approvals))
        .route("/cell/sessions/{session}/tool-results", post(tool_results))
        .route("/cell/sessions/{session}/cancel-turn", post(cancel_turn))
        .route("/cell/sessions/{session}/seed", post(seed))
        .route("/cell/sessions/{session}/schedules", post(schedules))
        .route("/cell/sessions/{session}/tasks", post(tasks))
        .route("/cell/sessions/{session}/task", post(task))
        .route("/cell/sessions/{session}/record", post(record))
        .route(
            "/cell/sessions/{session}/storage",
            get(storage).post(change_storage),
        )
        .route("/cell/sessions/{session}/secrets", get(secrets))
        .route("/cell/sessions/{session}/sender", post(sender))
        .with_state(node)
}

fn parse(session: &str) -> Result<SessionId, ApiError> {
    SessionId::parse(session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

/// The session through this node's Engine, or why not.
pub(crate) async fn resume(node: &Node, id: SessionId) -> Result<everruns::Session, CellReply> {
    match node.engine.resume(id).await {
        Ok(handle) => Ok(handle),
        // Engine folds authority errors into `Unavailable`; ask the replica
        // why.
        Err(error) => Err(match crate::api::lookup(node, id, |s| s.placement) {
            Lookup::Moved { cell } => CellReply::Moved { cell },
            Lookup::Missing => CellReply::NotFound,
            Lookup::Here(Placement::Frozen | Placement::Prepared) => CellReply::Frozen,
            Lookup::Here(_) => reply_of(error),
        }),
    }
}

async fn open_turn(node: &Node, id: SessionId) -> Result<TurnId, CellReply> {
    match node.hosting.status(id).await {
        Ok(status) => status.open_turn.ok_or_else(|| CellReply::Refused {
            reason: "the session is not waiting for an answer".into(),
        }),
        Err(error) => Err(reply_of(error)),
    }
}

#[derive(Deserialize)]
struct Approvals {
    command: uuid::Uuid,
    decisions: Vec<ToolApprovalAnswer>,
}

/// Answer the approvals the session's turn is parked on, atomically.
async fn approvals(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<Approvals>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    let reply = async {
        let handle = resume(&node, id).await?;
        let turn = open_turn(&node, id).await?;
        let response = handle
            .submit_approvals(body.command, turn, body.decisions)
            .await
            .map_err(reply_of)?;
        Ok::<_, CellReply>(CellReply::Ok {
            value: serde_json::to_value(response).unwrap_or(Value::Null),
        })
    }
    .await;
    Ok(Json(reply.unwrap_or_else(|reply| reply)))
}

#[derive(Deserialize)]
struct ClientResult {
    tool_call_id: String,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct ToolResults {
    command: uuid::Uuid,
    tool_results: Vec<ClientResult>,
}

/// Resolve client-side tool calls the turn is parked on. The tool's name
/// comes from the pending call, never from the request.
async fn tool_results(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<ToolResults>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    let accepted = body.tool_results.len();
    let reply = async {
        let handle = resume(&node, id).await?;
        let pending = handle.pending_tools().await.map_err(reply_of)?;
        let mut answers = Vec::with_capacity(accepted);
        for result in body.tool_results {
            let call = pending
                .iter()
                .find(|call| call.id == result.tool_call_id)
                .ok_or_else(|| CellReply::Refused {
                    reason: format!("no pending tool call {}", result.tool_call_id),
                })?;
            answers.push(everruns::ToolAnswer {
                tool_call_id: result.tool_call_id,
                tool_name: call.name.clone(),
                result: result.result.unwrap_or(Value::Null),
                error: result.error,
            });
        }
        let turn = open_turn(&node, id).await?;
        handle
            .resolve_tools(body.command, turn, answers)
            .await
            .map_err(reply_of)?;
        Ok::<_, CellReply>(CellReply::Ok {
            value: json!({ "accepted": accepted, "status": "active" }),
        })
    }
    .await;
    Ok(Json(reply.unwrap_or_else(|reply| reply)))
}

#[derive(Deserialize)]
struct CancelTurn {
    command: uuid::Uuid,
}

/// The input message of the turn still open in `events`, if any.
fn running_input(events: &[everruns_core::events::Event]) -> Option<MessageId> {
    let mut open: Option<(TurnId, MessageId)> = None;
    for event in events {
        match &event.data {
            EventData::TurnStarted(data) => open = Some((data.turn_id, data.input_message_id)),
            EventData::TurnCompleted(data) if open.is_some_and(|(t, _)| t == data.turn_id) => {
                open = None
            }
            EventData::TurnFailed(data) if open.is_some_and(|(t, _)| t == data.turn_id) => {
                open = None
            }
            EventData::TurnCancelled(data) if open.is_some_and(|(t, _)| t == data.turn_id) => {
                open = None
            }
            _ => {}
        }
    }
    open.map(|(_, input)| input)
}

/// Cancel whichever turn runs now (`POST /v1/sessions/{id}/cancel`): a
/// no-op when none does.
async fn cancel_turn(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<CancelTurn>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    let events = match node.shared.read_events(id, 0).await {
        Some(Ok(events)) => events,
        Some(Err(error)) => return Err((StatusCode::SERVICE_UNAVAILABLE, error.to_string())),
        None => {
            return Ok(Json(match crate::api::lookup(&node, id, |_| ()) {
                Lookup::Moved { cell } => CellReply::Moved { cell },
                _ => CellReply::NotFound,
            }));
        }
    };
    let Some(input) = running_input(&events) else {
        return Ok(Json(CellReply::Ok {
            value: json!({ "status": "no_op", "message": "No turn is running" }),
        }));
    };
    let reply = match node.hosting.cancel(id, body.command, input).await {
        Ok(()) => CellReply::Ok {
            value: json!({ "status": "cancelled", "message": "Turn cancelled successfully" }),
        },
        Err(error) => reply_of(error),
    };
    Ok(Json(reply))
}

#[derive(Deserialize)]
struct Seed {
    /// The command's own id (retry-safe).
    command: uuid::Uuid,
    messages: Vec<everruns_core::RuntimeMessage>,
}

/// Record `messages` as the session's prior conversation, only while it has
/// none: a channel joining a thread mid-way brings what was said before.
/// `seeded` is false for a session that already holds history.
async fn seed(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<Seed>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    if let Err(reply) = resume(&node, id).await {
        return Ok(Json(reply));
    }
    let reply = match node
        .hosting
        .seed_history(id, body.command, body.messages)
        .await
    {
        Ok(seeded) => CellReply::Ok {
            value: json!({ "seeded": seeded }),
        },
        Err(error) => reply_of(error),
    };
    Ok(Json(reply))
}

#[derive(Deserialize)]
struct Schedules {
    /// A change to make; none lists them.
    #[serde(default)]
    action: Option<crate::platform::Op>,
}

/// List the session's schedules, or change one (`platform.rs`).
async fn schedules(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<Schedules>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    // Senders are recorded only with the message (`api.rs` `send_message`).
    if matches!(body.action, Some(crate::platform::Op::Sender { .. })) {
        return Ok(Json(CellReply::Refused {
            reason: "a sender is recorded with its message".into(),
        }));
    }
    let Some(action) = body.action else {
        let listed = crate::api::lookup(&node, id, |s| {
            let live: Vec<_> = s
                .platform
                .schedules
                .values()
                .filter(|schedule| schedule.deleted_at.is_none())
                .collect();
            json!(live)
        });
        return Ok(Json(match listed {
            Lookup::Here(value) => CellReply::Ok { value },
            Lookup::Moved { cell } => CellReply::Moved { cell },
            Lookup::Missing => CellReply::NotFound,
        }));
    };
    Ok(Json(platform(&node, id, action).await?))
}

/// Propose a platform change to `id`, as a [`CellReply`].
pub(crate) async fn platform(
    node: &Node,
    id: SessionId,
    action: crate::platform::Op,
) -> Result<CellReply, ApiError> {
    use crate::state::{Command, Response};
    let command = Command::Platform {
        session: id,
        action,
        now_ms: crate::node::now_ms(),
        seed: everruns_contracts::typed_id::EventId::new(),
    };
    Ok(match node.propose(command).await {
        Ok((Response::Value { value }, _)) => CellReply::Ok { value },
        Ok((Response::Moved { cell }, _)) => CellReply::Moved { cell },
        Ok((Response::Frozen, _)) => CellReply::Frozen,
        Ok((Response::NotFound, _)) => CellReply::NotFound,
        Ok((Response::Rejected { reason }, _)) => CellReply::Refused { reason },
        Ok((other, _)) => CellReply::Refused {
            reason: format!("unexpected reply {other:?}"),
        },
        Err(error) => return Err((StatusCode::SERVICE_UNAVAILABLE, error.to_string())),
    })
}

#[derive(Deserialize)]
struct Record {
    /// The event's id, which makes a retry find it.
    event_id: everruns_contracts::typed_id::EventId,
    event: everruns_core::events::EventRequest,
}

/// Record an event the control plane produced on the session (a reaction,
/// `platform::RECORDABLE`); answers the event.
async fn record(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<Record>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    // Checked again when the command applies; refused here so an event the
    // control plane may not write never enters the log.
    if let Err(reason) = crate::platform::recordable(id, &body.event) {
        return Ok(Json(CellReply::Refused { reason }));
    }
    let action = crate::platform::Op::Record {
        event_id: body.event_id,
        request: Box::new(body.event),
    };
    Ok(Json(platform(&node, id, action).await?))
}

/// The session's values and secret names (`session_storage.rs`).
async fn storage(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<Lookup<Value>>, ApiError> {
    let id = parse(&session)?;
    Ok(Json(crate::api::lookup(
        &node,
        id,
        crate::session_storage::listed,
    )))
}

/// The session's secrets as stored, sealed (`session_storage.rs`), for
/// the control plane.
async fn secrets(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<Lookup<Value>>, ApiError> {
    let id = parse(&session)?;
    Ok(Json(crate::api::lookup(
        &node,
        id,
        crate::session_storage::sealed_secrets,
    )))
}

/// A person's change to the session's storage, through the control plane.
async fn change_storage(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(op): Json<crate::session_storage::StorageOp>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    if let Some(reason) = crate::session_storage::refused(id, &op) {
        return Ok(Json(CellReply::Refused { reason }));
    }
    let action = crate::platform::Op::Storage(op);
    Ok(Json(platform(&node, id, action).await?))
}

#[derive(Deserialize)]
struct Tasks {
    /// One task's detail; none lists them.
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    after_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// List the session's tasks, or one with its messages (`tasks.rs`).
async fn tasks(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<Tasks>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    let found = crate::api::lookup(&node, id, |s| match &body.task {
        None => crate::tasks::listed(s),
        Some(task) => crate::tasks::detail(
            s,
            task,
            body.after_id.as_deref(),
            body.limit.unwrap_or(50).clamp(1, 500),
        ),
    });
    Ok(Json(match found {
        Lookup::Here(value) => CellReply::Ok { value },
        Lookup::Moved { cell } => CellReply::Moved { cell },
        Lookup::Missing => CellReply::NotFound,
    }))
}

/// Cancel one of the session's tasks, or send it a person's message
/// (`tasks::api_op`); answers the task or the message, null for a task the
/// session does not have.
async fn task(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(write): Json<crate::tasks::ApiWrite>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    let kind = match crate::api::lookup(&node, id, |s| crate::tasks::kind_of(s, &write.task)) {
        Lookup::Here(Some(kind)) => kind,
        Lookup::Here(None) => {
            return Ok(Json(CellReply::Ok {
                value: serde_json::Value::Null,
            }));
        }
        Lookup::Moved { cell } => return Ok(Json(CellReply::Moved { cell })),
        Lookup::Missing => return Ok(Json(CellReply::NotFound)),
    };
    match crate::tasks::api_op(write, &kind) {
        Ok(op) => Ok(Json(
            platform(&node, id, crate::platform::Op::Task(op)).await?,
        )),
        Err(reason) => Ok(Json(CellReply::Refused { reason })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{Event, EventContext, TurnCompletedData, TurnStartedData};

    fn started(session: SessionId, turn: TurnId, input: MessageId) -> Event {
        Event::new(
            session,
            EventContext::default(),
            TurnStartedData {
                turn_id: turn,
                input_message_id: input,
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
                origin: None,
            },
        )
    }

    #[test]
    fn only_a_turn_with_no_end_is_running() {
        let session = SessionId::new();
        let (first, second) = (TurnId::new(), TurnId::new());
        let (a, b) = (MessageId::new(), MessageId::new());
        let mut events = vec![started(session, first, a)];
        assert_eq!(running_input(&events), Some(a));
        events.push(Event::new(
            session,
            EventContext::default(),
            TurnCompletedData {
                turn_id: first,
                ..Default::default()
            },
        ));
        assert_eq!(running_input(&events), None);
        events.push(started(session, second, b));
        assert_eq!(running_input(&events), Some(b));
    }
}

#[derive(Deserialize)]
struct SenderOf {
    message: MessageId,
}

/// Who sent the session's input message `message` (`senders.rs`):
/// `{"user": <user id or null>}`.
async fn sender(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<SenderOf>,
) -> Result<Json<CellReply>, ApiError> {
    let id = parse(&session)?;
    let found = crate::api::lookup(&node, id, |s| {
        s.platform.senders.of(body.message).map(str::to_owned)
    });
    Ok(Json(match found {
        Lookup::Here(user) => CellReply::Ok {
            value: json!({ "user": user }),
        },
        Lookup::Moved { cell } => CellReply::Moved { cell },
        Lookup::Missing => CellReply::NotFound,
    }))
}
