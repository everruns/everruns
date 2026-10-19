//! Materialize accepted messages with validated causal metadata.
use super::*;

pub(super) fn message_from_input(input: InputMessage) -> RuntimeMessage {
    message_from_input_with_id(MessageId::new(), input)
}

pub(super) fn message_from_input_with_id(
    message_id: MessageId,
    mut input: InputMessage,
) -> RuntimeMessage {
    crate::causal::sanitize_input_metadata(&mut input.metadata);
    RuntimeMessage {
        id: message_id,
        role: input.role,
        content: input.content,
        phase: None,
        phase_source: None,
        controls: input.controls,
        metadata: input.metadata,
        external_actor: input.external_actor,
        created_at: Utc::now(),
    }
}
