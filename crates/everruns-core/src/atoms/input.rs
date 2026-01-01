//! InputAtom - Atom for recording user input and starting a turn
//!
//! This atom is the entry point for a turn. It:
//! 1. Retrieves the user message from the message store
//! 2. Emits input.received event
//! 3. Returns the message for further processing

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Atom, AtomContext};
use crate::error::{AgentLoopError, Result};
use crate::event::{Event, EventContext, InputReceivedData, INPUT_RECEIVED};
use crate::message::Message;
use crate::traits::{EventEmitter, MessageStore};

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for InputAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAtomInput {
    /// Atom execution context
    pub context: AtomContext,
}

/// Result of the InputAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAtomResult {
    /// The user message that triggered this turn
    pub message: Message,
}

// ============================================================================
// InputAtom
// ============================================================================

/// Atom that records user input and starts a turn
///
/// This atom:
/// 1. Retrieves the user message from the message store using input_message_id
/// 2. Emits input.received event
/// 3. Returns the message for downstream processing
///
/// The message is expected to already be stored by the API layer.
/// This atom just retrieves it and prepares for the turn.
pub struct InputAtom<M, E>
where
    M: MessageStore,
    E: EventEmitter,
{
    message_store: M,
    event_emitter: E,
}

impl<M, E> InputAtom<M, E>
where
    M: MessageStore,
    E: EventEmitter,
{
    /// Create a new InputAtom
    pub fn new(message_store: M, event_emitter: E) -> Self {
        Self {
            message_store,
            event_emitter,
        }
    }
}

#[async_trait]
impl<M, E> Atom for InputAtom<M, E>
where
    M: MessageStore + Send + Sync,
    E: EventEmitter + Send + Sync,
{
    type Input = InputAtomInput;
    type Output = InputAtomResult;

    fn name(&self) -> &'static str {
        "input"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let InputAtomInput { context } = input;

        tracing::debug!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            input_message_id = %context.input_message_id,
            exec_id = %context.exec_id,
            "InputAtom: retrieving user message"
        );

        // Retrieve the user message from the store
        let message = self
            .message_store
            .get(context.session_id, context.input_message_id)
            .await?
            .ok_or_else(|| {
                AgentLoopError::store(format!(
                    "User message not found: {}",
                    context.input_message_id
                ))
            })?;

        // Create event context from atom context
        let event_context = EventContext::atom(
            context.session_id,
            context.turn_id,
            context.input_message_id,
            context.exec_id,
        );

        // Emit input.received event
        if let Err(e) = self
            .event_emitter
            .emit(Event::new(
                INPUT_RECEIVED,
                event_context,
                InputReceivedData::new(message.clone()),
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "InputAtom: failed to emit input.received event"
            );
        }

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            message_id = %message.id,
            "InputAtom: turn started with user message"
        );

        Ok(InputAtomResult { message })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryMessageStore;
    use crate::traits::{InputMessage, NoopEventEmitter};
    use uuid::Uuid;

    #[tokio::test]
    async fn test_input_atom_retrieves_message() {
        let store = InMemoryMessageStore::new();
        let event_emitter = NoopEventEmitter;
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();

        // Add a user message to the store
        let user_message = store
            .add(session_id, InputMessage::user("Hello, world!"))
            .await
            .unwrap();

        let context = AtomContext::new(session_id, turn_id, user_message.id);
        let atom = InputAtom::new(store, event_emitter);

        let result = atom.execute(InputAtomInput { context }).await.unwrap();

        assert_eq!(result.message.id, user_message.id);
        assert_eq!(result.message.text(), Some("Hello, world!"));
    }

    #[tokio::test]
    async fn test_input_atom_not_found() {
        let store = InMemoryMessageStore::new();
        let event_emitter = NoopEventEmitter;
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();
        let missing_id = Uuid::now_v7();

        let context = AtomContext::new(session_id, turn_id, missing_id);
        let atom = InputAtom::new(store, event_emitter);

        let result = atom.execute(InputAtomInput { context }).await;

        assert!(result.is_err());
    }
}
