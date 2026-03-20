// Session lifecycle management
//
// Decision: Consolidates session status transitions and lifecycle event emission
// into one place. Previously scattered across unified_worker, durable_worker,
// and activities — each with near-identical code.
//
// Workers call lifecycle methods; this module owns the event emission and
// session status updates that always move in lockstep.

use everruns_core::events::{
    EventContext, EventRequest, OutputMessageCompletedData, SessionActivatedData, SessionIdledData,
    TurnCompletedData, TurnFailedData, TurnStartedData,
};
use everruns_core::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::{DependencyBlocker, Message, TokenUsage};
use tracing::warn;

use crate::worker_adapters::WorkerAdapters;

/// Encapsulates session/turn lifecycle transitions.
///
/// Each method handles the event emission + session status update for a given
/// lifecycle transition on a best-effort basis (individual operations may fail
/// independently). Workers call these instead of manually emitting events and
/// setting status.
pub struct SessionLifecycle<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
    session_id: SessionId,
}

impl<A: WorkerAdapters> SessionLifecycle<A> {
    pub fn new(adapters: A, org_id: i64, session_id: SessionId) -> Self {
        Self {
            adapters,
            org_id,
            session_id,
        }
    }

    /// Turn is starting: set session to "active", emit session.activated + turn.started.
    pub async fn turn_started(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        input_content: Option<String>,
    ) {
        if let Err(e) = self
            .adapters
            .set_session_status(self.org_id, self.session_id.uuid(), "active")
            .await
        {
            warn!(error = %e, "Failed to set session status to active");
        }

        let activated_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionActivatedData {
                turn_id,
                input_message_id,
            },
        );
        if let Err(e) = self.adapters.emit_event(activated_event).await {
            warn!(error = %e, "Failed to emit session.activated event");
        }

        let turn_started_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            TurnStartedData {
                turn_id,
                input_message_id,
                input_content,
            },
        );
        if let Err(e) = self.adapters.emit_event(turn_started_event).await {
            warn!(error = %e, "Failed to emit turn.started event");
        }
    }

    /// Turn completed successfully: emit turn.completed + session.idled, set session to "idle".
    pub async fn turn_completed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        iterations: u32,
        usage: Option<TokenUsage>,
        input_content: Option<String>,
    ) {
        if let Err(e) = self
            .adapters
            .set_session_status(self.org_id, self.session_id.uuid(), "idle")
            .await
        {
            warn!(error = %e, "Failed to set session status to idle");
        }

        let turn_completed_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            TurnCompletedData {
                turn_id,
                iterations,
                duration_ms: None,
                usage: usage.clone(),
                input_content,
            },
        );
        if let Err(e) = self.adapters.emit_event(turn_completed_event).await {
            warn!(error = %e, "Failed to emit turn.completed event");
        }

        let idled_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: Some(iterations),
                usage,
            },
        );
        if let Err(e) = self.adapters.emit_event(idled_event).await {
            warn!(error = %e, "Failed to emit session.idled event");
        }
    }

    /// Turn failed: emit turn.failed + session.idled, set session to "idle".
    pub async fn turn_failed(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        error: &str,
        error_code: Option<&str>,
    ) {
        if let Err(e) = self
            .adapters
            .set_session_status(self.org_id, self.session_id.uuid(), "idle")
            .await
        {
            warn!(error = %e, "Failed to set session status to idle");
        }

        let turn_failed_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            TurnFailedData {
                turn_id,
                error: error.to_string(),
                error_code: error_code.map(|s| s.to_string()),
            },
        );
        if let Err(e) = self.adapters.emit_event(turn_failed_event).await {
            warn!(error = %e, "Failed to emit turn.failed event");
        }

        let idled_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None,
                usage: None,
            },
        );
        if let Err(e) = self.adapters.emit_event(idled_event).await {
            warn!(error = %e, "Failed to emit session.idled event");
        }
    }

    /// Act needs external input: set session to "waiting_for_tool_results".
    /// Events are already emitted by ActAtom's hooks — this only sets status.
    pub async fn waiting_for_tool_results(&self) {
        if let Err(e) = self
            .adapters
            .set_session_status(
                self.org_id,
                self.session_id.uuid(),
                "waiting_for_tool_results",
            )
            .await
        {
            warn!(error = %e, "Failed to set session status to waiting_for_tool_results");
        }
    }

    /// Dependency blocked: emit user-facing error message + turn.failed + session.idled,
    /// set session to "idle".
    pub async fn dependency_blocked(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
        blocker: DependencyBlocker,
    ) {
        let message = blocker.message();

        let output_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            OutputMessageCompletedData::new(Message::assistant(message)),
        );
        if let Err(e) = self.adapters.emit_event(output_event).await {
            warn!(error = %e, "Failed to emit dependency blocked message");
        }

        self.turn_failed(
            turn_id,
            input_message_id,
            message,
            Some(blocker.error_code()),
        )
        .await;
    }

    /// DLQ error: emit user-facing error + session.idled, set session to "idle".
    pub async fn dlq_error(&self, turn_id: TurnId, input_message_id: MessageId) {
        let error_message = Message::assistant(
            "I encountered an error while processing your request. Please try again later.",
        );
        let context = EventContext::turn(turn_id, input_message_id);

        if let Err(e) = self
            .adapters
            .emit_event(EventRequest::new(
                self.session_id,
                context,
                OutputMessageCompletedData::new(error_message),
            ))
            .await
        {
            warn!(error = %e, "Failed to emit DLQ error event");
        }

        let idled_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None,
                usage: None,
            },
        );
        if let Err(e) = self.adapters.emit_event(idled_event).await {
            warn!(error = %e, "Failed to emit session.idled after DLQ");
        }

        if let Err(e) = self
            .adapters
            .set_session_status(self.org_id, self.session_id.uuid(), "idle")
            .await
        {
            warn!(error = %e, "Failed to set session idle after DLQ");
        }
    }

    /// Cancelled: emit cancellation message + session.idled, set session to "idle".
    pub async fn cancelled(&self, turn_id: TurnId, input_message_id: MessageId) {
        let cancel_message = Message::assistant("Work was cancelled by user.");
        let message_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            OutputMessageCompletedData::new(cancel_message),
        );
        if let Err(e) = self.adapters.emit_event(message_event).await {
            warn!(error = %e, "Failed to emit cancellation message");
        }

        let idled_event = EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None,
                usage: None,
            },
        );
        if let Err(e) = self.adapters.emit_event(idled_event).await {
            warn!(error = %e, "Failed to emit session.idled event");
        }

        if let Err(e) = self
            .adapters
            .set_session_status(self.org_id, self.session_id.uuid(), "idle")
            .await
        {
            warn!(error = %e, "Failed to set session status to idle");
        }
    }
}
