//! A node's session commands behind the Everruns `/v1` session API
//! (`gateway_v1.rs`): answering tool approvals, client-side tool results,
//! and cancelling the running turn. Each goes through the node's Engine
//! and answers a [`CellReply`], so the gateway follows moves and freezes
//! the same way it does for admissions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::post;
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
