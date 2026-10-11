//! The server's turns on the framework's turn entry point.
//!
//! Decisions:
//! - The server starts, continues and cancels every turn through
//!   `everruns_core::host::TurnBackend`, the same trait the framework's
//!   sessions use; on the server that is [`ActorTurnBackend`], which marks
//!   the session's actor runnable for a worker to run the whole turn. There
//!   is no server-only runner trait.
//! - The server persists input itself before it starts a turn (the message's
//!   `input.message` event, or a waiting-turn resolution's events), so it
//!   always starts from stored input: `TurnInput::StoredMessage` with the
//!   session's `TurnScope`, or `TurnInput::RecordedToolResults` keyed by the
//!   resolution id.
//! - The turn id on these requests only labels the ticket: a platform turn's
//!   id is minted by the worker that starts it. Callers that need no result
//!   drop the ticket; the actor was marked before `start_turn` returned, in
//!   the caller's transaction when it has one.

mod actor_backend;
pub use actor_backend::{
    ActorTurnBackend, SESSION_ACTOR_CANCEL_NOTIFICATION, SESSION_ACTOR_NOTIFICATION,
};

use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use everruns_core::host::{TurnBackend, TurnInput, TurnRequest, TurnScope};
use uuid::Uuid;

/// The event context of `session_id`'s open turn: what a waiting-turn
/// resolution (tool results, an answer, an approval, a consent) records its
/// events under.
///
/// Decision: resolutions carry the real turn id of the turn they resolve.
/// A worker resumes a parked turn by folding its log, and the fold counts a
/// `tool.completed` only under the open turn's id; one recorded under any
/// other id would leave the call unfinished, so the resumed turn would run
/// the client-side call again and park forever. A session with no open turn
/// (nothing to resume) falls back to ids derived from the session, as every
/// resolution used before session actors.
pub async fn open_turn_context(
    db: &crate::storage::StorageBackend,
    session_id: SessionId,
) -> anyhow::Result<everruns_core::events::EventContext> {
    let started = db
        .find_open_turn_started(session_id)
        .await?
        .and_then(|data| {
            serde_json::from_value::<everruns_core::events::TurnStartedData>(data).ok()
        });
    Ok(match started {
        Some(started) => {
            everruns_core::events::EventContext::turn(started.turn_id, started.input_message_id)
        }
        None => everruns_core::events::EventContext::turn(
            TurnId::from_uuid(session_id.uuid()),
            MessageId::from_uuid(session_id.uuid()),
        ),
    })
}

/// Start the turn `request` describes on `backend` and drop its ticket.
pub async fn start(backend: &dyn TurnBackend, request: TurnRequest) -> anyhow::Result<()> {
    backend.start_turn(request).await.map(drop).map_err(error)
}

/// A turn backend failure as the server reports it: a store failure keeps
/// the store's own message, which is what the server always logged and
/// returned.
pub fn error(error: AgentLoopError) -> anyhow::Error {
    match error {
        AgentLoopError::MessageStore(message) => anyhow::anyhow!(message),
        other => anyhow::Error::new(other),
    }
}

/// The scope of a session in `org_id` that runs `harness_id` and `agent_id`.
pub fn scope(org_id: i64, harness_id: HarnessId, agent_id: Option<AgentId>) -> TurnScope {
    TurnScope::new(org_id, harness_id, agent_id)
}

/// Start (or steer) the session's turn from the stored message `message_id`.
pub fn stored_message(
    session_id: SessionId,
    scope: TurnScope,
    message_id: MessageId,
    request_id: Option<String>,
) -> TurnRequest {
    TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::StoredMessage { message_id },
    )
    .with_scope(scope)
    .with_request_id(request_id)
}

/// Continue the session's parked turn from the tool resolution
/// `resolution_id`, whose events the server already recorded.
pub fn recorded_tool_results(session_id: SessionId, resolution_id: Uuid) -> TurnRequest {
    TurnRequest::new(
        session_id,
        TurnId::new(),
        TurnInput::RecordedToolResults { resolution_id },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_message_names_the_scope_and_request() {
        let session_id = SessionId::new();
        let harness_id = HarnessId::new();
        let message_id = MessageId::new();
        let request = stored_message(
            session_id,
            scope(7, harness_id, None),
            message_id,
            Some("req".to_string()),
        );
        assert_eq!(request.session_id, session_id);
        assert_eq!(request.scope, Some(TurnScope::new(7, harness_id, None)));
        assert_eq!(request.request_id.as_deref(), Some("req"));
        assert!(matches!(
            request.input,
            TurnInput::StoredMessage { message_id: id } if id == message_id
        ));
    }

    #[test]
    fn store_failures_keep_their_message() {
        let failure = error(AgentLoopError::store("Failed to get workflow status"));
        assert_eq!(failure.to_string(), "Failed to get workflow status");
        let cancelled = error(AgentLoopError::Cancelled);
        assert!(cancelled.downcast_ref::<AgentLoopError>().is_some());
    }

    #[test]
    fn recorded_tool_results_carry_the_resolution() {
        let resolution_id = Uuid::now_v7();
        let request = recorded_tool_results(SessionId::new(), resolution_id);
        assert_eq!(request.scope, None);
        assert!(matches!(
            request.input,
            TurnInput::RecordedToolResults { resolution_id: id } if id == resolution_id
        ));
    }

    // --- On a real database --------------------------------------------------

    use crate::storage::test_database::test_session_row;
    use crate::storage::{ActorCancel, CreateEventRow, StorageBackend};
    use everruns_core::events::{EventContext, InputMessageData, TurnStartedData};
    use std::sync::Arc;

    const ORG: i64 = 2;

    async fn session(db: &StorageBackend) -> SessionId {
        db.create_session(test_session_row(ORG))
            .await
            .expect("create session")
            .id
    }

    async fn append(
        db: &StorageBackend,
        session_id: SessionId,
        event_type: &str,
        context: serde_json::Value,
        data: serde_json::Value,
    ) -> i32 {
        db.create_event(CreateEventRow {
            session_id,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context,
            data,
            metadata: None,
            tags: None,
        })
        .await
        .expect("create event")
        .sequence
    }

    /// A person's message in the log; returns its id and sequence.
    async fn message(db: &StorageBackend, session_id: SessionId) -> (MessageId, i32) {
        let data = InputMessageData::new(everruns_core::message::RuntimeMessage::user("hi"));
        let id = data.message.id;
        let sequence = append(
            db,
            session_id,
            "input.message",
            serde_json::json!({}),
            serde_json::to_value(&data).unwrap(),
        )
        .await;
        (id, sequence)
    }

    async fn turn_started(db: &StorageBackend, session_id: SessionId) -> (TurnId, MessageId) {
        let (turn_id, input_message_id) = (TurnId::new(), MessageId::new());
        let data = TurnStartedData {
            turn_id,
            input_message_id,
            input_content: None,
            agent_id: None,
            agent_name: None,
            agent_description: None,
        };
        append(
            db,
            session_id,
            "turn.started",
            serde_json::to_value(EventContext::turn(turn_id, input_message_id)).unwrap(),
            serde_json::to_value(&data).unwrap(),
        )
        .await;
        (turn_id, input_message_id)
    }

    fn backend(db: &StorageBackend) -> ActorTurnBackend {
        ActorTurnBackend::new(Arc::new(db.clone()), None)
    }

    #[tokio::test]
    async fn a_stored_message_marks_the_actor_at_its_own_sequence() {
        let db = StorageBackend::test_database();
        let session_id = session(&db).await;
        let (message_id, sequence) = message(&db, session_id).await;
        // Later events (a backfill, a resolution) do not move a fresh turn's start.
        message(&db, session_id).await;

        let request = stored_message(
            session_id,
            scope(ORG, HarnessId::new(), None),
            message_id,
            None,
        );
        drop(backend(&db).start_turn(request).await.expect("marks"));

        let row = db
            .get_session_actor(session_id)
            .await
            .unwrap()
            .expect("row");
        assert!(row.runnable_since.is_some());
        assert_eq!(row.inbox_seq, i64::from(sequence));
        assert_eq!(row.next_input_seq, i64::from(sequence));
        assert!(backend(&db).is_running(session_id).await);
    }

    #[tokio::test]
    async fn a_stored_message_needs_its_scope_and_its_event() {
        let db = StorageBackend::test_database();
        let session_id = session(&db).await;
        let (message_id, _) = message(&db, session_id).await;
        let unscoped = TurnRequest::new(
            session_id,
            TurnId::new(),
            TurnInput::StoredMessage { message_id },
        );
        assert!(backend(&db).start_turn(unscoped).await.is_err());
        let missing = stored_message(
            session_id,
            scope(ORG, HarnessId::new(), None),
            MessageId::new(),
            None,
        );
        assert!(backend(&db).start_turn(missing).await.is_err());
        assert!(db.get_session_actor(session_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn recorded_results_point_the_inbox_at_the_log_head() {
        let db = StorageBackend::test_database();
        let session_id = session(&db).await;
        let (message_id, first) = message(&db, session_id).await;
        let request = stored_message(
            session_id,
            scope(ORG, HarnessId::new(), None),
            message_id,
            None,
        );
        drop(backend(&db).start_turn(request).await.unwrap());
        let head = append(
            &db,
            session_id,
            "tool.completed",
            serde_json::json!({}),
            serde_json::json!({}),
        )
        .await;

        drop(
            backend(&db)
                .start_turn(recorded_tool_results(session_id, Uuid::now_v7()))
                .await
                .expect("marks"),
        );

        let row = db.get_session_actor(session_id).await.unwrap().unwrap();
        assert_eq!(row.inbox_seq, i64::from(head));
        assert_eq!(
            row.next_input_seq,
            i64::from(first),
            "a fresh turn's start is left alone"
        );
    }

    #[tokio::test]
    async fn cancel_is_the_holders_only_while_a_live_holder_runs_the_turn() {
        let db = StorageBackend::test_database();
        // No worker runs here: the wait for the holder only times out.
        let backend = backend(&db).with_cancel_wait(std::time::Duration::from_millis(200));
        let session_id = session(&db).await;
        assert!(!backend.cancel(session_id).await.unwrap(), "nothing runs");

        let (message_id, _) = message(&db, session_id).await;
        let request = stored_message(
            session_id,
            scope(ORG, HarnessId::new(), None),
            message_id,
            None,
        );
        drop(backend.start_turn(request).await.unwrap());
        assert!(
            !backend.cancel(session_id).await.unwrap(),
            "a waiting actor is withdrawn; the caller records the cancel"
        );
        assert!(!backend.is_running(session_id).await);

        let (message_id, _) = message(&db, session_id).await;
        let request = stored_message(
            session_id,
            scope(ORG, HarnessId::new(), None),
            message_id,
            None,
        );
        drop(backend.start_turn(request).await.unwrap());
        let leased = db
            .lease_session_actors("w1", 1, std::time::Duration::from_secs(10), 32)
            .await
            .unwrap();
        assert_eq!(leased.len(), 1);
        assert_eq!(backend.active_count().await, 1);
        assert!(
            backend.cancel(session_id).await.unwrap(),
            "the holder records it"
        );
        let row = db.get_session_actor(session_id).await.unwrap().unwrap();
        assert!(row.cancel_requested_at.is_some());
        assert_eq!(
            db.request_session_actor_cancel(session_id).await.unwrap(),
            ActorCancel::Requested
        );
    }

    /// Inside a command transaction the holder cannot see the cancel before
    /// commit; the cancel still answers only once the holder recorded it,
    /// because the wait runs after the commit.
    #[tokio::test]
    async fn a_cancel_in_a_command_transaction_answers_after_the_holder_recorded_it() {
        let db = StorageBackend::test_database();
        let backend = Arc::new(backend(&db).with_cancel_wait(std::time::Duration::from_secs(15)));
        let session_id = session(&db).await;
        let (message_id, _) = message(&db, session_id).await;
        let request = stored_message(
            session_id,
            scope(ORG, HarnessId::new(), None),
            message_id,
            None,
        );
        drop(backend.start_turn(request).await.unwrap());
        let leased = db
            .lease_session_actors("w1", 1, std::time::Duration::from_secs(30), 32)
            .await
            .unwrap();
        let fence = leased[0].fence;

        // The holder: renews until it hears of the cancel, records it, releases.
        let holder = {
            let db = db.clone();
            tokio::spawn(async move {
                loop {
                    let renewal = db
                        .renew_session_actor_lease(
                            session_id,
                            "w1",
                            fence,
                            std::time::Duration::from_secs(30),
                            0,
                        )
                        .await
                        .unwrap()
                        .expect("held");
                    if renewal.cancel_requested {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                append(
                    &db,
                    session_id,
                    "turn.cancelled",
                    serde_json::json!({}),
                    serde_json::json!({}),
                )
                .await;
                let inbox = db
                    .get_session_actor(session_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .inbox_seq;
                db.release_session_actor(session_id, "w1", fence, inbox)
                    .await
                    .unwrap();
            })
        };

        let started = std::time::Instant::now();
        let recorded_by_holder = crate::storage::transaction::scope(
            &db,
            async {
                assert!(crate::storage::transaction::in_transaction());
                backend.cancel(session_id).await.map_err(super::error)
            },
            |error| error,
        )
        .await
        .unwrap();
        assert!(recorded_by_holder, "a live holder records the cancel");
        let row = db.get_session_actor(session_id).await.unwrap().unwrap();
        assert!(
            row.lease_holder.is_none() && row.cancel_requested_at.is_none(),
            "answered only after the holder recorded the cancel and released"
        );
        let cancelled = db
            .list_events(
                session_id,
                None,
                None,
                &["turn.cancelled".to_string()],
                &[],
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            cancelled.len(),
            1,
            "the holder's turn.cancelled is in the log"
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        holder.await.unwrap();
    }

    #[tokio::test]
    async fn resolutions_carry_the_open_turns_id() {
        let db = StorageBackend::test_database();
        let session_id = session(&db).await;
        let fallback = open_turn_context(&db, session_id).await.unwrap();
        assert_eq!(fallback.turn_id, Some(TurnId::from_uuid(session_id.uuid())));

        let (turn_id, input_message_id) = turn_started(&db, session_id).await;
        let open = open_turn_context(&db, session_id).await.unwrap();
        assert_eq!(open.turn_id, Some(turn_id));
        assert_eq!(open.input_message_id, Some(input_message_id));

        for terminal in [
            "turn.completed",
            "turn.cancelled",
            "turn.sealed",
            "turn.failed",
        ] {
            let (turn_id, _) = turn_started(&db, session_id).await;
            assert_eq!(
                open_turn_context(&db, session_id).await.unwrap().turn_id,
                Some(turn_id)
            );
            append(
                &db,
                session_id,
                terminal,
                serde_json::json!({}),
                serde_json::json!({}),
            )
            .await;
            assert_eq!(
                open_turn_context(&db, session_id).await.unwrap().turn_id,
                Some(TurnId::from_uuid(session_id.uuid())),
                "{terminal} closes the turn"
            );
        }
    }
}
