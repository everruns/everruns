//! Unit tests for [`super`], the session actor.
//!
//! Split out of `session.rs` rather than inlined: the tests were two
//! thirds of the file's length, and carrying them there kept the actor
//! itself on the file-size ratchet's debt list for no reason.

use std::sync::{Arc, Mutex};

use everruns_core::events::EventData;
use everruns_core::turn::TurnStopReason;
use everruns_core::{ContentPart, InputMessage, RuntimeMessageRole};
use everruns_host::{
    EventHistory, EventHistoryReadLimit, EventHistoryReadRequest, EventReadLimit, EventReadRequest,
    TurnResult,
};
use everruns_provider::typed_id::TurnId;

use super::Turn;
use crate::{Agent, InMemoryEngine, Model};

#[tokio::test]
async fn history_accumulates_across_turns() {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated_capturing("ok", capture.clone()))
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    session.run("hello").await.expect("first turn");
    session.run("continue").await.expect("second turn");

    let calls = capture.lock().unwrap();
    assert_eq!(calls.len(), 2, "two turns => two LLM calls");
    assert!(
        calls[1].len() > calls[0].len(),
        "the second turn's request must include the first turn's messages"
    );
}

#[tokio::test]
async fn normal_session_history_is_rebuilt_from_canonical_events() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");
    let session = InMemoryEngine::new().create(agent.clone());
    let session_id = session.session_id();

    session.run("hello").await.expect("turn runs");

    let event_log = session
        .inner
        .execution
        .backends()
        .await
        .unwrap_or_else(|_| panic!("run built the shared backends"))
        .host
        .event_log
        .clone();
    let events = event_log
        .read_page(EventReadRequest::new(session_id, EventReadLimit::default()))
        .await
        .expect("canonical events replay");
    assert!(
        events.events.iter().all(|event| event.sequence.is_some()),
        "durable replay excludes sequence-less live deltas"
    );

    let canonical_messages: Vec<_> = events
        .events
        .iter()
        .filter_map(|event| match &event.data {
            EventData::InputMessage(data) => Some(data.message.clone()),
            EventData::OutputMessageCompleted(data) => Some(data.message.clone()),
            _ => None,
        })
        .collect();
    let history = EventHistory::new(event_log);
    let page = history
        .read_page(EventHistoryReadRequest::new(
            session_id,
            EventHistoryReadLimit::new(8).expect("valid message limit"),
        ))
        .await
        .expect("event-derived history page");

    assert_eq!(
        serde_json::to_value(&page.messages).expect("history serializes"),
        serde_json::to_value(&canonical_messages).expect("events serialize")
    );
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].text(), Some("hello"));
    assert_eq!(page.messages[1].text(), Some("ok"));
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn two_sessions_do_not_share_history() {
    let capture = Arc::new(Mutex::new(Vec::new()));
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated_capturing("ok", capture.clone()))
        .build()
        .expect("valid agent");

    let first = InMemoryEngine::new().create(agent.clone());
    first.run("a1").await.expect("a1");
    first.run("a2").await.expect("a2");

    let second = InMemoryEngine::new().create(agent.clone());
    second.run("b1").await.expect("b1");

    assert_ne!(first.id(), second.id(), "sessions have distinct ids");

    let calls = capture.lock().unwrap();
    assert_eq!(calls.len(), 3);
    // The second session's first call starts fresh: same size as the first
    // session's first call, and smaller than its accumulated second call.
    assert_eq!(
        calls[2].len(),
        calls[0].len(),
        "a second session must not inherit the first session's history"
    );
    assert!(calls[1].len() > calls[2].len());
}

#[tokio::test]
async fn accepts_multimodal_input() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    // A rich, multi-part InputMessage goes through unchanged.
    let message = InputMessage {
        role: RuntimeMessageRole::User,
        content: vec![
            ContentPart::text("describe"),
            ContentPart::text("this attachment"),
        ],
        controls: None,
        metadata: None,
        tags: vec![],
    };
    let turn = session.run(message).await.expect("turn runs");
    assert!(turn.success);
}

#[test]
fn turn_preserves_failure_and_stop_reason() {
    let result = TurnResult {
        response: String::new(),
        iterations: 3,
        tool_calls_count: 0,
        success: false,
        error: Some("hit the ceiling".to_string()),
        stop_reason: TurnStopReason::MaxTurnRequests,
        turn_id: TurnId::new(),
    };
    let turn = Turn::from(result);
    assert!(!turn.success);
    assert_eq!(turn.stop_reason, TurnStopReason::MaxTurnRequests);
    assert_eq!(turn.error.as_deref(), Some("hit the ceiling"));
}

// --- Events and cancellation (EVE-833) -------------------------------
//
// These reach the crate-internal simulator helpers (`simulated_scripted`,
// `simulated_delayed`) that an external integration test cannot see. The
// public-surface event/cancellation behaviors live in
// `tests/facade/session_events.rs`.

use std::time::Duration;

use everruns_provider::tool_types::ToolCall;
use serde_json::json;

use crate::{CancellationToken, RunOptions, SessionEvent, SessionEventKind};

async fn drain(mut stream: crate::EventStream) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await.expect("event stream stays lossless") {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn tool_events_correlate_with_parent_turn() {
    let tool = crate::FunctionTool::new(
        "ping",
        "Respond to a ping.",
        json!({ "type": "object", "properties": {} }),
        |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "ok": true })) },
    );
    let agent = Agent::builder()
        .instructions("Call ping when asked.")
        .model(Model::simulated_scripted(
            "done",
            vec![
                vec![ToolCall {
                    id: "call_ping_1".into(),
                    name: "ping".into(),
                    arguments: json!({}),
                }],
                vec![],
            ],
        ))
        .tool(tool)
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let stream = session.events();
    let turn = session.run("please ping").await.expect("turn runs");
    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    assert_eq!(turn.tool_calls, 1);

    drop(session);
    let events = drain(stream).await;

    let tool_started = events
        .iter()
        .find(|e| matches!(e.kind, SessionEventKind::ToolStarted { .. }))
        .expect("a tool.started event");
    let tool_completed = events
        .iter()
        .find(|e| matches!(e.kind, SessionEventKind::ToolCompleted { .. }))
        .expect("a tool.completed event");

    // Both tool events carry the parent turn's id.
    assert_eq!(tool_started.turn_id.as_deref(), Some(turn.turn_id.as_str()));
    assert_eq!(
        tool_completed.turn_id.as_deref(),
        Some(turn.turn_id.as_str())
    );

    let SessionEventKind::ToolStarted {
        tool_call_id: started_id,
        tool_name,
    } = &tool_started.kind
    else {
        unreachable!("matched ToolStarted above")
    };
    assert_eq!(tool_name, "ping");
    let SessionEventKind::ToolCompleted {
        tool_call_id: completed_id,
        success,
        ..
    } = &tool_completed.kind
    else {
        unreachable!("matched ToolCompleted above")
    };
    assert_eq!(started_id, completed_id, "same tool call across the pair");
    assert!(success, "the ping tool succeeded");
}

#[tokio::test]
async fn cancellation_stops_a_running_turn_with_cancelled_stop_reason() {
    // A long TTFT delay parks the turn so we can cancel it mid-flight.
    let agent = Agent::builder()
        .instructions("You are slow.")
        .model(Model::simulated_delayed(
            "eventually",
            Duration::from_secs(30),
        ))
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let token = CancellationToken::new();

    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });

    let turn = session
        .run_with("hi", RunOptions::new().cancel_token(token))
        .await
        .expect("run_with resolves");

    assert!(!turn.success, "a cancelled turn is not a success");
    assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
}

#[tokio::test]
async fn an_uncancelled_run_with_matches_run() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let turn = session
        .run_with("hi", RunOptions::new())
        .await
        .expect("turn runs");
    assert!(turn.success);
    assert_eq!(turn.response, "ok");
    assert!(turn.hook_failures.is_empty());
}

#[tokio::test]
async fn lifecycle_hooks_wrap_a_tool_call_in_registration_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let tool_order = order.clone();
    let tool = crate::FunctionTool::new(
        "ping",
        "Respond to a ping.",
        json!({ "type": "object", "properties": {} }),
        move |_args: serde_json::Value| {
            let tool_order = tool_order.clone();
            async move {
                tool_order.lock().unwrap().push("tool");
                Ok::<_, String>(json!({ "ok": true }))
            }
        },
    );
    let start_one = order.clone();
    let start_two = order.clone();
    let end_one = order.clone();
    let end_two = order.clone();
    let completion = order.clone();
    let agent = Agent::builder()
        .instructions("Call ping when asked.")
        .model(Model::simulated_scripted(
            "done",
            vec![
                vec![ToolCall {
                    id: "call_ping_hooks".into(),
                    name: "ping".into(),
                    arguments: json!({}),
                }],
                vec![],
            ],
        ))
        .tool(tool)
        .on_tool_start(move |context| {
            let start_one = start_one.clone();
            async move {
                assert_eq!(context.tool_name, "ping");
                assert!(context.turn_id.is_some());
                start_one.lock().unwrap().push("start-1");
            }
        })
        .on_tool_start(move |_context| {
            let start_two = start_two.clone();
            async move { start_two.lock().unwrap().push("start-2") }
        })
        .on_tool_end(move |context| {
            let end_one = end_one.clone();
            async move {
                assert!(context.success());
                end_one.lock().unwrap().push("end-1");
            }
        })
        .on_tool_end(move |_context| {
            let end_two = end_two.clone();
            async move { end_two.lock().unwrap().push("end-2") }
        })
        .on_completion(move |context| {
            let completion = completion.clone();
            async move {
                assert!(context.turn.success);
                completion.lock().unwrap().push("completion");
            }
        })
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent.clone())
        .run("please ping")
        .await
        .expect("turn runs");

    assert!(turn.hook_failures.is_empty());
    assert_eq!(
        *order.lock().unwrap(),
        ["start-1", "start-2", "tool", "end-1", "end-2", "completion"]
    );
}

#[tokio::test]
async fn tool_start_error_blocks_call_and_skips_later_start_hooks() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let tool_ran = Arc::new(AtomicBool::new(false));
    let tool_ran_in_handler = tool_ran.clone();
    let later_ran = Arc::new(AtomicBool::new(false));
    let later = later_ran.clone();
    let end_context = Arc::new(Mutex::new(None));
    let end_context_in_hook = end_context.clone();
    let tool = crate::FunctionTool::new(
        "ping",
        "Respond to a ping.",
        json!({ "type": "object", "properties": {} }),
        move |_args: serde_json::Value| {
            let tool_ran_in_handler = tool_ran_in_handler.clone();
            async move {
                tool_ran_in_handler.store(true, Ordering::SeqCst);
                Ok::<_, String>(json!({ "ok": true }))
            }
        },
    );
    let agent = Agent::builder()
        .instructions("Call ping when asked.")
        .model(Model::simulated_scripted(
            "recovered",
            vec![
                vec![ToolCall {
                    id: "call_blocked_by_framework_hook".into(),
                    name: "ping".into(),
                    arguments: json!({}),
                }],
                vec![],
            ],
        ))
        .tool(tool)
        .on_tool_start(|_context| async move { Err::<(), _>("policy backend diagnostic: secret") })
        .on_tool_start(move |_context| {
            let later = later.clone();
            async move { later.store(true, Ordering::SeqCst) }
        })
        .on_tool_end(move |context| {
            let end_context_in_hook = end_context_in_hook.clone();
            async move {
                *end_context_in_hook.lock().unwrap() = Some(context);
            }
        })
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent.clone())
        .run("ping")
        .await
        .expect("turn settles");

    assert!(turn.success, "model can recover from a blocked tool call");
    assert!(!tool_ran.load(Ordering::SeqCst));
    assert!(!later_ran.load(Ordering::SeqCst));
    let end_context = end_context.lock().unwrap();
    let end_context = end_context.as_ref().expect("blocked call still ends");
    assert!(!end_context.success());
    let model_visible_error = end_context.error.as_deref().expect("blocked call error");
    assert!(model_visible_error.contains("tool call blocked by tool_start hook #0"));
    assert!(!model_visible_error.contains("secret"));
    assert_eq!(turn.hook_failures.len(), 1);
    assert_eq!(turn.hook_failures[0].point, crate::HookPoint::ToolStart);
    assert_eq!(
        turn.hook_failures[0].message,
        "policy backend diagnostic: secret"
    );
    assert_eq!(
        turn.hook_failures[0].tool_call_id.as_deref(),
        Some("call_blocked_by_framework_hook")
    );
}

#[tokio::test]
async fn tool_end_error_is_isolated_and_later_handlers_run() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let later_ran = Arc::new(AtomicBool::new(false));
    let later = later_ran.clone();
    let tool = crate::FunctionTool::new(
        "ping",
        "Respond to a ping.",
        json!({ "type": "object", "properties": {} }),
        |_args: serde_json::Value| async move { Ok::<_, String>(json!({ "ok": true })) },
    );
    let agent = Agent::builder()
        .instructions("Call ping when asked.")
        .model(Model::simulated_scripted(
            "done",
            vec![
                vec![ToolCall {
                    id: "call_post_hook_error".into(),
                    name: "ping".into(),
                    arguments: json!({}),
                }],
                vec![],
            ],
        ))
        .tool(tool)
        .on_tool_end(|_context| async move { Err::<(), _>("audit sink offline") })
        .on_tool_end(move |_context| {
            let later = later.clone();
            async move { later.store(true, Ordering::SeqCst) }
        })
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent.clone())
        .run("ping")
        .await
        .expect("turn runs");

    assert!(turn.success);
    assert!(later_ran.load(Ordering::SeqCst));
    assert_eq!(turn.hook_failures.len(), 1);
    assert_eq!(turn.hook_failures[0].point, crate::HookPoint::ToolEnd);
}

#[tokio::test]
async fn cancellation_drops_an_in_flight_hook_and_skips_remaining_hooks() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let started = Arc::new(tokio::sync::Notify::new());
    let started_in_hook = started.clone();
    let later_ran = Arc::new(AtomicBool::new(false));
    let later = later_ran.clone();
    let completion_ran = Arc::new(AtomicBool::new(false));
    let completion = completion_ran.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("unreachable"))
        .on_turn_start(move |_context| {
            let started_in_hook = started_in_hook.clone();
            async move {
                started_in_hook.notify_one();
                std::future::pending::<()>().await;
            }
        })
        .on_turn_start(move |_context| {
            let later = later.clone();
            async move { later.store(true, Ordering::SeqCst) }
        })
        .on_completion(move |_context| {
            let completion = completion.clone();
            async move { completion.store(true, Ordering::SeqCst) }
        })
        .build()
        .expect("valid agent");

    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        started.notified().await;
        canceller.cancel();
    });
    let turn = tokio::time::timeout(
        Duration::from_secs(2),
        InMemoryEngine::new()
            .create(agent)
            .run_with("hello", RunOptions::new().cancel_token(token)),
    )
    .await
    .expect("cancellation is prompt")
    .expect("run resolves");

    assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
    assert!(!later_ran.load(Ordering::SeqCst));
    assert!(!completion_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancellation_drops_an_in_flight_tool_hook() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let hook_started = Arc::new(tokio::sync::Notify::new());
    let hook_started_inside = hook_started.clone();
    let tool_ran = Arc::new(AtomicBool::new(false));
    let tool_ran_inside = tool_ran.clone();
    let completion_ran = Arc::new(AtomicBool::new(false));
    let completion = completion_ran.clone();
    let tool = crate::FunctionTool::new(
        "ping",
        "Respond to a ping.",
        json!({ "type": "object", "properties": {} }),
        move |_args: serde_json::Value| {
            let tool_ran_inside = tool_ran_inside.clone();
            async move {
                tool_ran_inside.store(true, Ordering::SeqCst);
                Ok::<_, String>(json!({ "ok": true }))
            }
        },
    );
    let agent = Agent::builder()
        .instructions("Call ping when asked.")
        .model(Model::simulated_scripted(
            "unreachable",
            vec![vec![ToolCall {
                id: "call_cancelled_hook".into(),
                name: "ping".into(),
                arguments: json!({}),
            }]],
        ))
        .tool(tool)
        .on_tool_start(move |_context| {
            let hook_started_inside = hook_started_inside.clone();
            async move {
                hook_started_inside.notify_one();
                std::future::pending::<()>().await;
            }
        })
        .on_completion(move |_context| {
            let completion = completion.clone();
            async move { completion.store(true, Ordering::SeqCst) }
        })
        .build()
        .expect("valid agent");

    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        hook_started.notified().await;
        canceller.cancel();
    });
    let turn = tokio::time::timeout(
        Duration::from_secs(2),
        InMemoryEngine::new()
            .create(agent)
            .run_with("ping", RunOptions::new().cancel_token(token)),
    )
    .await
    .expect("cancellation is prompt")
    .expect("run resolves");

    assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
    assert!(!tool_ran.load(Ordering::SeqCst));
    assert!(!completion_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn completion_finishes_after_the_runtime_commits_even_if_token_is_cancelled() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let token = CancellationToken::new();
    let cancel_inside = token.clone();
    let completions = Arc::new(AtomicUsize::new(0));
    let first = completions.clone();
    let second = completions.clone();
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .on_completion(move |_context| {
            let cancel_inside = cancel_inside.clone();
            let first = first.clone();
            async move {
                cancel_inside.cancel();
                tokio::task::yield_now().await;
                first.fetch_add(1, Ordering::SeqCst);
            }
        })
        .on_completion(move |_context| {
            let second = second.clone();
            async move {
                second.fetch_add(1, Ordering::SeqCst);
            }
        })
        .build()
        .expect("valid agent");

    let turn = InMemoryEngine::new()
        .create(agent)
        .run_with("hello", RunOptions::new().cancel_token(token))
        .await
        .expect("committed turn completes its hooks");

    assert!(turn.success);
    assert_eq!(completions.load(Ordering::SeqCst), 2);
}
