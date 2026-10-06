//! Partial-stream detection over a host's own canonical event log (EVE-532).
//!
//! The in-process runtime stores every durable event in its [`EventLog`], so
//! the stream a lost reason attempt left open is visible there: the latest
//! `output.message.started` of the turn with no completion or replacement
//! under its message id. Same contract as the server's `PgPartialStreamStore`.
//!
//! Decisions:
//! - A forward scan of the session, bounded like the other replays by
//!   [`MAX_EVENT_HISTORY_REPLAY`]. The log has no reverse or by-type read, and
//!   every reason step already replays the session's history to build its
//!   transcript, so this adds a scan of the same size, not a new order.
//! - Deltas are ephemeral and never reach the log, so `accumulated` is empty
//!   here unless a log chooses to keep them; recovery then restarts the call.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::reasoning_updates::ReasoningState;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};

use super::events::{EventLog, EventReadLimit, EventReadRequest, MAX_EVENT_HISTORY_REPLAY};
use crate::durability::{PartialStreamState, PartialStreamStore};
use crate::events::EventData;

/// [`PartialStreamStore`] answered from a host's canonical event log.
pub(crate) struct EventLogPartialStreamStore {
    log: Arc<dyn EventLog>,
}

impl EventLogPartialStreamStore {
    pub(crate) fn new(log: Arc<dyn EventLog>) -> Self {
        Self { log }
    }
}

/// The store over `log`, as the in-process runtime hands it to reason steps.
pub(crate) fn over(log: &Arc<dyn EventLog>) -> Arc<dyn PartialStreamStore> {
    Arc::new(EventLogPartialStreamStore::new(log.clone()))
}

/// The turn's latest started stream while scanning forward.
struct OpenStream {
    message_id: MessageId,
    reasoning_state: Option<ReasoningState>,
    accumulated: String,
    settled: bool,
    closed: bool,
}

#[async_trait]
impl PartialStreamStore for EventLogPartialStreamStore {
    async fn get_partial_stream(
        &self,
        session_id: SessionId,
        turn_id: &str,
    ) -> Result<Option<PartialStreamState>> {
        let turn_id = TurnId::parse(turn_id)
            .map_err(|error| AgentLoopError::store(format!("partial stream turn id: {error}")))?;
        let limit = EventReadLimit::default();
        let mut request = EventReadRequest::new(session_id, limit);
        let mut latest: Option<OpenStream> = None;
        let mut examined = 0usize;
        loop {
            let page = self
                .log
                .read_page(request)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            examined = examined.saturating_add(page.events.len());
            for event in page
                .events
                .iter()
                .filter(|event| event.context.turn_id == Some(turn_id))
            {
                match &event.data {
                    EventData::OutputMessageStarted(data) => {
                        latest = Some(OpenStream {
                            message_id: data.message_id,
                            reasoning_state: data.reasoning_state.clone(),
                            accumulated: String::new(),
                            settled: false,
                            closed: false,
                        });
                    }
                    EventData::OutputMessageDelta(data) => {
                        if let Some(open) =
                            latest.as_mut().filter(|o| o.message_id == data.message_id)
                        {
                            open.accumulated.clone_from(&data.accumulated);
                        }
                    }
                    EventData::OutputMessageCompleted(data) => {
                        if let Some(open) =
                            latest.as_mut().filter(|o| o.message_id == data.message.id)
                        {
                            open.closed = true;
                        }
                    }
                    EventData::OutputMessageReplaced(data) => {
                        if let Some(open) =
                            latest.as_mut().filter(|o| o.message_id == data.message_id)
                        {
                            open.closed = true;
                        }
                    }
                    EventData::ReasonCompleted(_) => {
                        if let Some(open) = latest.as_mut() {
                            open.settled = true;
                        }
                    }
                    _ => {}
                }
            }
            if examined > MAX_EVENT_HISTORY_REPLAY {
                return Err(AgentLoopError::store(format!(
                    "partial-stream scan exceeds the {MAX_EVENT_HISTORY_REPLAY}-event bound"
                )));
            }
            let Some(cursor) = page.next_cursor else {
                break;
            };
            request = EventReadRequest::from_cursor(cursor, limit);
        }
        Ok(latest
            .filter(|open| !open.closed)
            .map(|open| PartialStreamState {
                reasoning_state: open.reasoning_state,
                message_id: open.message_id,
                accumulated: open.accumulated,
                attempt_settled: open.settled,
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{
        EventContext, EventRequest, OutputMessageCompletedData, OutputMessageStartedData,
        ReasonCompletedData,
    };
    use crate::host::InMemoryEventLog;
    use crate::message::RuntimeMessage;

    struct Rig {
        log: Arc<InMemoryEventLog>,
        session_id: SessionId,
        turn_id: TurnId,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                log: Arc::new(InMemoryEventLog::new()),
                session_id: SessionId::new(),
                turn_id: TurnId::new(),
            }
        }

        fn context(&self, turn_id: TurnId) -> EventContext {
            EventContext {
                turn_id: Some(turn_id),
                ..EventContext::default()
            }
        }

        async fn started(&self, turn_id: TurnId) -> MessageId {
            let message_id = MessageId::new();
            self.log
                .append(EventRequest::new(
                    self.session_id,
                    self.context(turn_id),
                    OutputMessageStartedData {
                        reasoning_state: None,
                        turn_id,
                        message_id,
                        model: None,
                        iteration: Some(1),
                        phase: None,
                    },
                ))
                .await
                .unwrap();
            message_id
        }

        async fn completed(&self, message_id: MessageId) {
            self.log
                .append(EventRequest::new(
                    self.session_id,
                    self.context(self.turn_id),
                    OutputMessageCompletedData::new(
                        RuntimeMessage::assistant("done").with_id(message_id),
                    ),
                ))
                .await
                .unwrap();
        }

        async fn reason_completed(&self) {
            self.log
                .append(EventRequest::new(
                    self.session_id,
                    self.context(self.turn_id),
                    ReasonCompletedData::failure("transient".into(), None),
                ))
                .await
                .unwrap();
        }

        async fn lookup(&self) -> Option<PartialStreamState> {
            EventLogPartialStreamStore::new(self.log.clone())
                .get_partial_stream(self.session_id, &self.turn_id.to_string())
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn no_started_stream_is_no_partial() {
        assert!(Rig::new().lookup().await.is_none());
    }

    #[tokio::test]
    async fn a_started_stream_without_completion_is_open_and_unsettled() {
        let rig = Rig::new();
        let first = rig.started(rig.turn_id).await;
        rig.completed(first).await;
        let open = rig.started(rig.turn_id).await;
        let partial = rig.lookup().await.expect("the latest stream is open");
        assert_eq!(partial.message_id, open);
        assert!(partial.accumulated.is_empty());
        assert!(!partial.attempt_settled);
    }

    #[tokio::test]
    async fn a_completed_latest_stream_masks_nothing_and_reports_none() {
        let rig = Rig::new();
        let message_id = rig.started(rig.turn_id).await;
        rig.completed(message_id).await;
        assert!(rig.lookup().await.is_none());
    }

    #[tokio::test]
    async fn a_completion_under_another_id_leaves_the_stream_open_and_settled() {
        let rig = Rig::new();
        let open = rig.started(rig.turn_id).await;
        rig.completed(MessageId::new()).await;
        rig.reason_completed().await;
        let partial = rig.lookup().await.expect("the stream is still open");
        assert_eq!(partial.message_id, open);
        assert!(partial.attempt_settled);
    }

    #[tokio::test]
    async fn another_turns_stream_is_not_this_turns_partial() {
        let rig = Rig::new();
        rig.started(TurnId::new()).await;
        assert!(rig.lookup().await.is_none());
    }
}
