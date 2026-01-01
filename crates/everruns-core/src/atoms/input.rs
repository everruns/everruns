//! InputAtom - Atom for recording user input and starting a turn
//!
//! This atom is the entry point for a turn. It:
//! 1. Retrieves the user message from the message store
//! 2. Emits a message.user event
//! 3. Returns the message for further processing

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Atom, AtomContext};
use crate::error::{AgentLoopError, Result};
use crate::message::Message;
use crate::traits::MessageStore;

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
/// 2. Returns the message for downstream processing
///
/// The message is expected to already be stored by the API layer.
/// This atom just retrieves it and prepares for the turn.
pub struct InputAtom<M>
where
    M: MessageStore,
{
    message_store: M,
}

impl<M> InputAtom<M>
where
    M: MessageStore,
{
    /// Create a new InputAtom
    pub fn new(message_store: M) -> Self {
        Self { message_store }
    }
}

#[async_trait]
impl<M> Atom for InputAtom<M>
where
    M: MessageStore + Send + Sync,
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
    use crate::traits::InputMessage;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_input_atom_retrieves_message() {
        let store = InMemoryMessageStore::new();
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();

        // Add a user message to the store
        let user_message = store
            .add(session_id, InputMessage::user("Hello, world!"))
            .await
            .unwrap();

        let context = AtomContext::new(session_id, turn_id, user_message.id);
        let atom = InputAtom::new(store);

        let result = atom.execute(InputAtomInput { context }).await.unwrap();

        assert_eq!(result.message.id, user_message.id);
        assert_eq!(result.message.text(), Some("Hello, world!"));
    }

    #[tokio::test]
    async fn test_input_atom_not_found() {
        let store = InMemoryMessageStore::new();
        let session_id = Uuid::now_v7();
        let turn_id = Uuid::now_v7();
        let missing_id = Uuid::now_v7();

        let context = AtomContext::new(session_id, turn_id, missing_id);
        let atom = InputAtom::new(store);

        let result = atom.execute(InputAtomInput { context }).await;

        assert!(result.is_err());
    }
}
