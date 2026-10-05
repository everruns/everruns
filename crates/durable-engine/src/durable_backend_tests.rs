//! `DurableBackend` on the in-memory store, driven by its own workers over
//! the in-process runtime: turns, steering, cancellation and shutdown.

use std::sync::atomic::Ordering;
use std::time::Duration;

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};

use crate::core::InputMessage;
use crate::core::turn::TurnStopReason;
use crate::durable_backend::DurableBackend;
use crate::host::{
    AcceptedTurnInput, InProcessRuntime, PersistedTurn, TurnBackend, TurnInput, TurnRequest,
    TurnSteering, TurnTicket,
};

async fn runtime(sim: LlmSimConfig) -> (InProcessRuntime, SessionId) {
    let runtime = InProcessRuntime::builder()
        .llm_sim_as_default(sim)
        .single_session(|session| session)
        .build()
        .await
        .unwrap();
    let session_id = runtime.default_session_id().unwrap();
    (runtime, session_id)
}

fn message(session_id: SessionId, text: &str) -> TurnRequest {
    TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::Message(Box::new(AcceptedTurnInput::new(InputMessage::user(text)))),
    )
}

async fn finish(ticket: TurnTicket) -> crate::host::TurnResult {
    tokio::time::timeout(Duration::from_secs(10), ticket)
        .await
        .expect("the durable turn ends")
        .expect("the durable turn succeeds")
}

#[tokio::test]
async fn a_message_runs_to_its_answer_on_the_workers() {
    let (runtime, session_id) = runtime(LlmSimConfig::fixed("durably done")).await;
    let backend = DurableBackend::memory(2);
    assert_eq!(backend.running_workers(), 0, "workers start with a session");
    let session = backend.attach(session_id, runtime.clone());
    assert_eq!(backend.running_workers(), 2);

    let request = message(session_id, "hi");
    let turn_id = request.turn_id;
    let ticket = session.start_turn(request).await.unwrap();
    assert!(session.is_running(session_id).await);
    assert_eq!(session.active_count().await, 1);

    let result = finish(ticket).await;
    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "durably done");
    assert_eq!(result.turn_id, turn_id, "the turn keeps the requested id");
    assert_eq!(result.stop_reason, TurnStopReason::EndTurn);
    assert!(!session.is_running(session_id).await);
    assert_eq!(session.active_count().await, 0);

    // The input and the answer are in the session's history.
    let messages = runtime.messages(session_id).await.unwrap();
    assert_eq!(messages.len(), 2, "{messages:?}");

    // The session takes its next turn.
    let next = finish(
        session
            .start_turn(message(session_id, "again"))
            .await
            .unwrap(),
    )
    .await;
    assert!(next.success);
}

#[tokio::test]
async fn a_tool_turn_runs_reason_act_reason() {
    let (runtime, session_id) = runtime(
        LlmSimConfig::sequence(vec!["checking".into(), "tool done".into()])
            .with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call-1".into(),
                    name: "no_such_tool".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ]),
    )
    .await;
    let backend = DurableBackend::memory(1);
    let session = backend.attach(session_id, runtime);

    let result = finish(session.start_turn(message(session_id, "hi")).await.unwrap()).await;
    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "tool done");
    assert_eq!(result.tool_calls_count, 1);
    assert_eq!(result.iterations, 2);
}

#[tokio::test]
async fn steering_joins_the_running_turn_at_its_next_reason() {
    // The first reason is slow, so the steered message is queued before it
    // ends; the turn must continue into a second reason instead of finishing.
    let (runtime, session_id) = runtime(
        LlmSimConfig::sequence(vec!["first".into(), "second".into()])
            .with_response_delay(Duration::from_millis(300)),
    )
    .await;
    let backend = DurableBackend::memory(1);
    let session = backend.attach(session_id, runtime.clone());

    let steering = TurnSteering::new();
    let ticket = session
        .start_turn(message(session_id, "hi").with_steering(steering.clone()))
        .await
        .unwrap();
    // Let the first reason begin, so the push lands after it and must be
    // delivered at the boundary that would otherwise end the turn.
    tokio::time::sleep(Duration::from_millis(100)).await;
    steering
        .try_push(AcceptedTurnInput::new(InputMessage::user("also this")))
        .expect("the turn is still open for steering");

    let result = finish(ticket).await;
    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "second", "{result:?}");
    assert_eq!(result.iterations, 2);
    // Once the turn committed to completion, input belongs to the next turn.
    assert!(
        steering
            .try_push(AcceptedTurnInput::new(InputMessage::user("too late")))
            .is_err()
    );
    let texts: Vec<_> = runtime
        .messages(session_id)
        .await
        .unwrap()
        .iter()
        .map(|message| message.content_to_llm_string())
        .collect();
    assert_eq!(texts.len(), 4, "{texts:?}");
    assert!(texts[2].contains("also this"), "{texts:?}");
}

#[tokio::test]
async fn cancel_drops_the_step_and_ends_the_ticket() {
    let (runtime, session_id) =
        runtime(LlmSimConfig::fixed("eventually").with_response_delay(Duration::from_secs(30)))
            .await;
    let backend = DurableBackend::memory(1);
    let session = backend.attach(session_id, runtime.clone());
    let mut ticket = session.start_turn(message(session_id, "hi")).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut ticket)
            .await
            .is_err(),
        "the slow turn is in flight"
    );

    let cancelled = tokio::time::timeout(Duration::from_secs(5), session.cancel(session_id))
        .await
        .expect("cancel does not wait for the slow step")
        .unwrap();
    assert!(cancelled);
    assert!(matches!(ticket.await, Err(AgentLoopError::Cancelled)));
    assert!(!session.is_running(session_id).await);
    assert!(!session.cancel(session_id).await.unwrap(), "nothing left");
    assert!(
        !session.cancel(SessionId::new()).await.unwrap(),
        "another session has nothing to cancel here"
    );
}

#[tokio::test]
async fn unsupported_and_empty_inputs_and_a_second_turn_are_rejected() {
    let (runtime, session_id) =
        runtime(LlmSimConfig::fixed("eventually").with_response_delay(Duration::from_secs(30)))
            .await;
    let backend = DurableBackend::memory(1);
    let session = backend.attach(session_id, runtime);
    let persisted = TurnInput::Persisted(Box::new(PersistedTurn::ToolResolution {
        resolution_id: uuid::Uuid::now_v7(),
    }));
    let error = session
        .start_turn(TurnRequest::new(session_id, TurnId::new(), persisted))
        .await
        .expect_err("server-persisted input is the runner's");
    assert!(matches!(error, AgentLoopError::Configuration(_)), "{error}");

    // Continuations fail as in process when there is nothing to continue.
    let error = session
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::ToolResults(Vec::new()),
        ))
        .await
        .expect_err("nothing is parked");
    assert!(
        error
            .to_string()
            .contains("no turn waiting for tool results"),
        "{error}"
    );
    let error = session
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::ResumeInterrupted,
        ))
        .await
        .expect_err("nothing is interrupted");
    assert!(error.to_string().contains("no turn interrupted"), "{error}");
    assert!(!session.is_running(session_id).await);

    let _first = session
        .start_turn(message(session_id, "first"))
        .await
        .unwrap();
    let error = session
        .start_turn(message(session_id, "second"))
        .await
        .expect_err("one turn per session");
    assert!(error.to_string().contains("already runs a turn"), "{error}");

    let error = session
        .start_turn(message(SessionId::new(), "elsewhere"))
        .await
        .expect_err("the handle runs one session");
    assert!(matches!(error, AgentLoopError::Configuration(_)), "{error}");
    session.cancel(session_id).await.unwrap();
}

#[tokio::test]
async fn shutdown_stops_the_workers() {
    let (runtime, session_id) = runtime(LlmSimConfig::fixed("done")).await;
    let backend = DurableBackend::memory(3);
    let _session = backend.attach(session_id, runtime);
    assert_eq!(backend.running_workers(), 3);
    tokio::time::timeout(Duration::from_secs(5), backend.shutdown())
        .await
        .expect("shutdown does not hang");
    assert_eq!(backend.running_workers(), 0);
}

#[tokio::test]
async fn dropping_the_last_backend_handle_stops_the_workers() {
    let (runtime, session_id) =
        runtime(LlmSimConfig::fixed("eventually").with_response_delay(Duration::from_secs(30)))
            .await;
    let backend = DurableBackend::memory(2);
    let running = backend.running_worker_counter();
    let session = backend.attach(session_id, runtime);
    // A turn is in flight; dropping the backend drops its step too.
    let _ticket = session.start_turn(message(session_id, "hi")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    drop(backend);
    tokio::time::timeout(Duration::from_secs(5), async {
        while running.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("workers stop once the backend is dropped");
    drop(session);
}
