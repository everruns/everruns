//! Unit tests for [`InProcessBackend`], the turn backend the session actor
//! runs its turns on.
//!
//! They live in the facade because it is the crate that can build a runtime
//! over the deterministic simulated model.

use std::time::Duration;

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_core::host::{
    AcceptedTurnInput, HostBackends, InProcessBackend, PersistedTurn, TurnBackend, TurnInput,
    TurnRequest, TurnTicket,
};
use everruns_core::turn::TurnStopReason;

use crate::{Agent, Model};

async fn backend(model: Model) -> (InProcessBackend, SessionId) {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(model)
        .build()
        .expect("valid agent");
    let session_id = SessionId::new();
    let runtime = agent
        .build_runtime_with_backends(
            HostBackends::in_memory(),
            session_id,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("runtime builds");
    (InProcessBackend::new(runtime), session_id)
}

async fn start(backend: &InProcessBackend, session_id: SessionId, text: &str) -> TurnTicket {
    let input = AcceptedTurnInput::new(everruns_core::InputMessage::user(text));
    backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Message(Box::new(input)),
        ))
        .await
        .expect("turn starts")
}

/// A slow model keeps the turn in flight; polling its ticket briefly lets
/// the turn begin without letting it finish.
async fn begin(ticket: &mut TurnTicket) {
    assert!(
        tokio::time::timeout(Duration::from_millis(50), ticket)
            .await
            .is_err(),
        "the slow turn is still running"
    );
}

#[tokio::test]
async fn a_started_turn_runs_to_its_result_and_then_leaves_the_backend() {
    let (backend, session_id) = backend(Model::simulated("Sure.")).await;
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    let ticket = start(&backend, session_id, "hi").await;
    assert_eq!(ticket.session_id(), session_id);
    assert!(backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 1);

    let turn_id = ticket.turn_id();
    let result = ticket.await.expect("turn completes");
    assert!(result.success);
    assert_eq!(result.response, "Sure.");
    assert_eq!(result.turn_id, turn_id, "the turn keeps the requested id");
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    // The session is free for its next turn.
    let next = start(&backend, session_id, "again").await;
    assert!(next.await.expect("second turn completes").success);
}

#[tokio::test]
async fn cancel_stops_a_running_turn_and_resolves_its_ticket_as_cancelled() {
    let (backend, session_id) = backend(Model::simulated_delayed(
        "eventually",
        Duration::from_secs(30),
    ))
    .await;
    let mut ticket = start(&backend, session_id, "hi").await;
    begin(&mut ticket).await;
    assert!(backend.is_running(session_id).await);

    assert!(backend.cancel(session_id).await.expect("cancel succeeds"));
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    let error = ticket.await.expect_err("a cancelled turn has no result");
    assert!(matches!(error, AgentLoopError::Cancelled), "{error:?}");
    assert!(
        !backend.cancel(session_id).await.expect("cancel succeeds"),
        "nothing is left to cancel"
    );
}

#[tokio::test]
async fn a_session_runs_one_turn_at_a_time_and_a_dropped_ticket_frees_it() {
    let (backend, session_id) = backend(Model::simulated_delayed(
        "eventually",
        Duration::from_secs(30),
    ))
    .await;
    let mut first = start(&backend, session_id, "first").await;
    begin(&mut first).await;

    let input = AcceptedTurnInput::new(everruns_core::InputMessage::user("second"));
    let error = backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Message(Box::new(input)),
        ))
        .await
        .expect_err("the session already runs a turn");
    assert!(error.to_string().contains("already runs a turn"), "{error}");

    // Dropping the ticket drops the in-process turn with it.
    drop(first);
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);
    let _second = start(&backend, session_id, "second").await;
    assert!(backend.is_running(session_id).await);
}

#[tokio::test]
async fn turns_of_other_sessions_are_not_running() {
    let (backend, session_id) = backend(Model::simulated_delayed(
        "eventually",
        Duration::from_secs(30),
    ))
    .await;
    let mut ticket = start(&backend, session_id, "hi").await;
    begin(&mut ticket).await;

    let other = SessionId::new();
    assert!(!backend.is_running(other).await);
    assert!(!backend.cancel(other).await.expect("cancel succeeds"));
    assert!(
        backend.is_running(session_id).await,
        "cancelling another session leaves this turn running"
    );
}

#[tokio::test]
async fn resuming_without_a_parked_turn_fails_through_the_ticket() {
    let (backend, session_id) = backend(Model::simulated("Sure.")).await;
    let ticket = backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::ToolResults(Vec::new()),
        ))
        .await
        .expect("the backend accepts the turn");
    let error = ticket.await.expect_err("nothing is parked");
    assert!(error.to_string().contains("no turn waiting"), "{error}");
    assert!(!backend.is_running(session_id).await);

    // The failed resume released the session.
    let result = start(&backend, session_id, "hi").await.await;
    assert_ne!(
        result.expect("turn completes").stop_reason,
        TurnStopReason::Cancelled
    );
}

#[tokio::test]
async fn server_persisted_input_is_rejected_in_process() {
    let (backend, session_id) = backend(Model::simulated("Sure.")).await;
    let error = backend
        .start_turn(TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::Persisted(Box::new(PersistedTurn::ToolResolution {
                resolution_id: uuid::Uuid::now_v7(),
            })),
        ))
        .await
        .expect_err("only a durable backend reads server-persisted input");
    assert!(matches!(error, AgentLoopError::Configuration(_)), "{error}");
    assert!(error.to_string().contains("Persisted"), "{error}");
    assert!(!backend.is_running(session_id).await);
    assert_eq!(backend.active_count().await, 0);

    // The rejection registered nothing, so the session still takes a turn.
    start(&backend, session_id, "hi")
        .await
        .await
        .expect("turn completes");
}
