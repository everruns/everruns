// `message/stream` / `SendStreamingMessage`: the SSE body.
//
// THREAT[TM-A2A-011]: Streaming widens the per-channel ingress surface from a
// single JSON-RPC response to a long-lived SSE connection that mirrors session
// events. Auth and the method gate run before the stream opens (no events leak
// before authn). Only a small allowlist of session events is translated into
// A2A frames, and agent output passes through the same final-text projection
// `tasks/get` uses, so tool calls, tool results and commentary never reach the
// caller. The stream is bounded by the turn: it closes after the first
// terminal or interrupted state.
//
// Frame order follows A2A 1.0 §3.1.2 for a task lifecycle stream (and is valid
// 0.3): the `Task` first, then `artifact-update` frames carrying agent output,
// then the closing `status-update`.

use std::convert::Infallible;

use axum::response::{
    IntoResponse, Response,
    sse::{Event as SseEvent, KeepAlive, Sse},
};
use everruns_core::events::EventData;
use futures::stream::{self, StreamExt};
use serde_json::{Value, json};
use uuid::Uuid;

use super::wire::{self, WireVersion};
use super::{ask_user, task_view};

pub(super) struct StreamContext {
    pub subscription: crate::live_updates::event_delivery::EventSubscription,
    pub rpc_id: Value,
    pub task_id: String,
    pub context_id: String,
    pub session_id: Uuid,
    /// UI origin, for the `auth-required` projection of a secret question.
    pub frontend_url: String,
    pub version: WireVersion,
    pub binding: wire::Binding,
    /// The first frame: the task as it stands when the stream opens.
    pub initial_task: Value,
}

struct StreamState {
    ctx: StreamContext,
    finished: bool,
}

/// Build the SSE response. `guard` is held for the stream's lifetime so the
/// connection-limit slot is released only when the client disconnects.
pub(super) fn sse_response<G: Send + Sync + 'static>(ctx: StreamContext, guard: G) -> Response {
    let initial_frame = sse_frame(&ctx, ctx.initial_task.clone());
    let initial = stream::iter(vec![Ok::<SseEvent, Infallible>(initial_frame)]);

    let body = stream::unfold(
        StreamState {
            ctx,
            finished: false,
        },
        |mut s| async move {
            if s.finished {
                return None;
            }
            loop {
                let Some(event) = s.ctx.subscription.recv().await else {
                    // Subscription closed without a terminal turn event: emit
                    // a synthetic failure so clients do not hang.
                    let frame = sse_frame(
                        &s.ctx,
                        status_update(
                            &s.ctx.task_id,
                            &s.ctx.context_id,
                            json!({ "state": "failed" }),
                        ),
                    );
                    s.finished = true;
                    return Some((Ok::<SseEvent, Infallible>(frame), s));
                };
                if event.session_id.uuid() != s.ctx.session_id {
                    continue;
                }
                if let Some(frame) = translate_session_event(
                    &event.data,
                    &s.ctx.task_id,
                    &s.ctx.context_id,
                    &s.ctx.frontend_url,
                ) {
                    // `final` exists only in the 0.3 frame; read it before the
                    // 1.0 rendering drops it.
                    s.finished = frame.get("final").and_then(Value::as_bool) == Some(true);
                    let frame = sse_frame(&s.ctx, frame);
                    return Some((Ok::<SseEvent, Infallible>(frame), s));
                }
            }
        },
    );

    let with_guard = initial.chain(body).map(move |event| {
        let _guard = &guard;
        event
    });
    Sse::new(with_guard)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response()
}

/// A closing `status-update` (internal 0.3 shape).
fn status_update(task_id: &str, context_id: &str, status: Value) -> Value {
    json!({
        "kind": "status-update",
        "taskId": task_id,
        "contextId": context_id,
        "status": status,
        "final": true,
    })
}

/// Translate a small allowlist of session events into an A2A frame body (the
/// JSON inside the JSON-RPC `result`, internal 0.3 shape). `None` filters the
/// event out of the stream.
pub(super) fn translate_session_event(
    data: &EventData,
    task_id: &str,
    context_id: &str,
    frontend_url: &str,
) -> Option<Value> {
    match data {
        // EVE-1062: a parked `ask_user` call is the one non-terminal stop this
        // stream has. Without a frame the caller waits on a question it cannot
        // see; the stream closes because the answer arrives as a fresh
        // `message/send` on this task.
        EventData::ToolCallRequested(requested) => {
            let pending = ask_user::pending_ask_user_from_request(requested)?;
            let projection = ask_user::project_ask_user(&pending, task_id, frontend_url);
            Some(status_update(
                task_id,
                context_id,
                json!({ "state": projection.state, "message": projection.message }),
            ))
        }
        EventData::OutputMessageCompleted(d) => {
            let (message_id, text) = task_view::final_output(d)?;
            Some(json!({
                "kind": "artifact-update",
                "taskId": task_id,
                "contextId": context_id,
                "artifact": task_view::output_artifact(&message_id, &text),
            }))
        }
        EventData::TurnCompleted(_) => Some(status_update(
            task_id,
            context_id,
            json!({ "state": "completed" }),
        )),
        EventData::TurnFailed(_) => Some(status_update(
            task_id,
            context_id,
            json!({ "state": "failed" }),
        )),
        EventData::TurnCancelled(_) => Some(status_update(
            task_id,
            context_id,
            json!({ "state": "canceled" }),
        )),
        _ => None,
    }
}

/// One SSE event: a JSON-RPC envelope around the frame, or under HTTP+JSON
/// (spec §11.7) the bare 1.0 `StreamResponse`.
fn sse_frame(ctx: &StreamContext, frame: Value) -> SseEvent {
    match ctx.binding {
        wire::Binding::JsonRpc => jsonrpc_sse_frame(&ctx.rpc_id, ctx.version, frame),
        wire::Binding::HttpJson => {
            SseEvent::default().data(wire::stream_frame(WireVersion::V1_0, frame).to_string())
        }
    }
}

fn jsonrpc_sse_frame(rpc_id: &Value, version: WireVersion, result: Value) -> SseEvent {
    let envelope = json!({
        "jsonrpc": "2.0",
        "id": rpc_id,
        "result": wire::stream_frame(version, result),
    });
    SseEvent::default().data(envelope.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_turn_completed_emits_terminal_status_update() {
        use everruns_contracts::typed_id::TurnId;
        use everruns_core::events::TurnCompletedData;
        let data = EventData::TurnCompleted(TurnCompletedData {
            turn_id: TurnId::new(),
            iterations: 1,
            duration_ms: Some(10),
            usage: None,
            input_content: None,
            final_message_id: None,
            final_answer_preview: None,
            time_to_first_token_ms: None,
            tool_call_count: None,
            llm_call_count: None,
            status: None,
            stop_reason: None,
        });
        let frame = translate_session_event(&data, "task-1", "ctx-1", "").unwrap();
        assert_eq!(frame["kind"], "status-update");
        assert_eq!(frame["taskId"], "task-1");
        assert_eq!(frame["contextId"], "ctx-1");
        assert_eq!(frame["status"]["state"], "completed");
        assert_eq!(frame["final"], true);
    }

    #[test]
    fn translate_turn_failed_emits_terminal_status_update() {
        use everruns_contracts::typed_id::TurnId;
        use everruns_core::events::TurnFailedData;
        let data = EventData::TurnFailed(TurnFailedData {
            turn_id: TurnId::new(),
            error: "boom".into(),
            error_code: None,
            error_fields: None,
            error_disclosure: None,
        });
        let frame = translate_session_event(&data, "task-1", "ctx-1", "").unwrap();
        assert_eq!(frame["status"]["state"], "failed");
        assert_eq!(frame["final"], true);
    }

    #[test]
    fn translate_agent_output_emits_an_artifact_update() {
        use everruns_core::events::OutputMessageCompletedData;
        use everruns_core::message::RuntimeMessage;
        let data = EventData::OutputMessageCompleted(OutputMessageCompletedData::new(
            RuntimeMessage::assistant("the answer"),
        ));
        let frame = translate_session_event(&data, "task-1", "ctx-1", "").unwrap();
        assert_eq!(frame["kind"], "artifact-update");
        assert_eq!(frame["artifact"]["parts"][0]["text"], "the answer");
        let v1 = wire::stream_frame(WireVersion::V1_0, frame);
        assert_eq!(
            v1["artifactUpdate"]["artifact"]["parts"][0],
            json!({ "text": "the answer" })
        );
    }

    #[test]
    fn translate_unrelated_event_returns_none() {
        use everruns_contracts::typed_id::{MessageId, TurnId};
        use everruns_core::events::OutputMessageStartedData;
        let data = EventData::OutputMessageStarted(OutputMessageStartedData {
            reasoning_state: None,
            turn_id: TurnId::new(),
            message_id: MessageId::new(),
            model: None,
            iteration: None,
            phase: None,
        });
        assert!(translate_session_event(&data, "t", "c", "").is_none());
    }

    #[test]
    fn jsonrpc_sse_frame_wraps_result_in_envelope() {
        let frame = jsonrpc_sse_frame(
            &Value::String("req-1".into()),
            WireVersion::V0_3,
            json!({"hello": "world"}),
        );
        let json_field = format!("{frame:?}");
        assert!(json_field.contains("req-1"));
        assert!(json_field.contains("hello"));
    }
}
