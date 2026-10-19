//! Transfer bounded causal identities at shared host boundaries.
use super::*;

impl<A: RuntimeHostAdapter> RuntimeSessionLifecycle<A> {
    pub async fn turn_started(
        &self,
        turn_id: TurnId,
        input_message_id: MessageId,
    ) -> everruns_contracts::error::Result<()> {
        // The input read is independent of the status write; run them
        // together, since each is a store round trip on the turn's path to its
        // first model call. The agent identity reads the session, so it
        // follows the write rather than racing it.
        let input_message = async {
            self.adapter
                .message_store()
                .get(self.session_id, input_message_id)
                .await
                .ok()
                .flatten()
        };
        let activated = async {
            self.set_session_status(SessionExecutionState::Active, "turn_started")
                .await?;
            Ok::<_, everruns_contracts::error::AgentLoopError>(self.agent_identity().await)
        };
        let (input_message, agent) = futures::join!(input_message, activated);
        let causes = crate::causal::CausalContext::from_metadata(
            input_message
                .as_ref()
                .and_then(|message| message.metadata.as_ref()),
        );
        let input_content = input_message.map(|message| message.content_to_llm_string());
        let agent = agent?;

        self.emit_event(EventRequest::new(
            self.session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionActivatedData {
                turn_id,
                input_message_id,
            },
        ))
        .await?;

        self.emit_event(
            EventRequest::new(
                self.session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnStartedData {
                    turn_id,
                    input_message_id,
                    input_content,
                    agent_id: agent.id,
                    agent_name: agent.name,
                    agent_description: agent.description,
                },
            )
            .with_causal_context(&causes),
        )
        .await?;
        Ok(())
    }
}

pub(super) fn reason_causes(
    messages: &[RuntimeMessage],
    input: MessageId,
) -> crate::causal::CausalContext {
    let mut causes = crate::causal::CausalContext::default();
    if let Some(start) = messages.iter().position(|message| message.id == input) {
        causes.merge(&crate::causal::CausalContext::from_metadata(
            messages[start].metadata.as_ref(),
        ));
        for message in messages[start + 1..].iter().rev().take(255) {
            if message.role == RuntimeMessageRole::User {
                causes.merge(&crate::causal::CausalContext::from_metadata(
                    message.metadata.as_ref(),
                ));
            }
        }
    }
    causes
}
