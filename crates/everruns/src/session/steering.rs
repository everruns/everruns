//! Explicit redirect and stable-ID admission.

use super::{
    AcceptedTurnInput, AgentLoopError, InputMessage, MessageId, RunError, SentMessage, Session,
    TurnId,
};

impl Session {
    /// Explicitly redirect one identified running turn. Already running tools
    /// settle normally; tools still queued for dispatch return skipped results.
    /// If the target turn has ended, this input is an ordinary follow-up and
    /// cannot redirect a different turn. Waiting approvals are not resolved.
    pub async fn redirect_with_id(
        &self,
        message_id: MessageId,
        expected_turn_id: TurnId,
        input: impl Into<InputMessage>,
    ) -> Result<SentMessage, RunError> {
        let input = input.into().with_metadata(
            everruns_core::engine::tool_steering::REDIRECT_TURN,
            serde_json::Value::String(expected_turn_id.to_string()),
        );
        self.admit_with_id(message_id, input, false, None).await
    }

    /// Admit input under a caller-retained message identity. Use the same id
    /// and content after an uncertain reply, including after frontend loss.
    pub async fn send_with_id(
        &self,
        message_id: MessageId,
        input: impl Into<InputMessage>,
    ) -> Result<SentMessage, RunError> {
        self.admit_with_id(message_id, ordinary_input(input.into()), false, None)
            .await
    }

    /// [`send_with_id`](Self::send_with_id) on behalf of verified identities
    /// the turn acts for (see [`crate::hosting::InputContext`]). For trusted
    /// hosts only: the context is taken as verified. Stability: experimental.
    pub async fn send_as(
        &self,
        message_id: MessageId,
        input: impl Into<InputMessage>,
        context: crate::hosting::InputContext,
    ) -> Result<SentMessage, RunError> {
        self.admit_with_id(
            message_id,
            ordinary_input(input.into()),
            false,
            Some(context),
        )
        .await
    }

    pub(super) async fn admit_with_id(
        &self,
        message_id: MessageId,
        input: InputMessage,
        cancelled: bool,
        context: Option<crate::hosting::InputContext>,
    ) -> Result<SentMessage, RunError> {
        let profile =
            self.inner.execution.hosting().ok_or_else(|| {
                AgentLoopError::config("stable admission requires Engine hosting")
            })?;
        if self.inner.execution.frontend_environment() {
            self.start().await.map_err(RunError::Environment)?;
        }
        self.inner
            .execution
            .ensure_cataloged()
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        let mut message = AcceptedTurnInput::new(input).into_message();
        message.id = message_id;
        let already_accepted = profile
            .receipt(self.session_id(), message_id)
            .await?
            .is_some();
        let receipt = profile
            .admit_as(self.session_id(), message, cancelled, context)
            .await?;
        // Admission commits outside the runtime emitter. Publish its canonical
        // envelope before activating, just as runtime events publish after commit.
        if !already_accepted {
            use everruns_core::host::{EventCursor, EventReadLimit, EventReadRequest, EventSink};
            let page = profile
                .reader(self.session_id())
                .event_log
                .read_page(EventReadRequest::from_cursor(
                    EventCursor::continuation(
                        self.session_id(),
                        receipt.sequence - 1,
                        receipt.sequence,
                    )
                    .map_err(|error| AgentLoopError::store(error.to_string()))?,
                    EventReadLimit::new(1)
                        .map_err(|error| AgentLoopError::store(error.to_string()))?,
                ))
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            for event in page.events {
                let _ = self.inner.event_bus.try_send(event);
            }
        }
        let admission = crate::hosting::Admission {
            hosting: profile,
            session: self.session_id(),
            receipt,
        };
        self.inner.execution.activate();
        Ok(SentMessage::admitted(self.clone(), admission))
    }
}

// Ordinary ingress cannot opt into redirection through caller metadata.
pub(super) fn ordinary_input(mut input: InputMessage) -> InputMessage {
    if let Some(metadata) = &mut input.metadata {
        metadata.remove(everruns_core::engine::tool_steering::REDIRECT_TURN);
    }
    input
}
