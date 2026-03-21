//! InputAtom - Atom for recording user input and starting a turn
//!
//! This atom is the entry point for a turn. It:
//! 1. Retrieves the user message from the message store
//! 2. Returns the message for further processing
//!
//! Note: The input.message event is emitted by the API when the message is stored,
//! not by this atom. This atom simply retrieves the already-stored message.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Atom, AtomContext};
use crate::error::{AgentLoopError, Result};
use crate::message::Message;
use crate::message_retriever::MessageRetriever;

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

/// Atom that retrieves user input for a turn
///
/// This atom:
/// 1. Retrieves the user message from the message retriever using input_message_id
/// 2. Returns the message for downstream processing
///
/// The message is expected to already be stored by the API layer (which emits input.message).
/// This atom just retrieves it and prepares for the turn.
pub struct InputAtom<M>
where
    M: MessageRetriever,
{
    message_retriever: M,
}

impl<M> InputAtom<M>
where
    M: MessageRetriever,
{
    /// Create a new InputAtom
    pub fn new(message_retriever: M) -> Self {
        Self { message_retriever }
    }
}

#[async_trait]
impl<M> Atom for InputAtom<M>
where
    M: MessageRetriever + Send + Sync,
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

        // Retrieve the user message from the retriever
        let message = self
            .message_retriever
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
    use crate::memory::InMemoryMessageRetriever;
    use crate::message_retriever::InputMessage;
    use crate::typed_id::{MessageId, SessionId, TurnId};

    #[tokio::test]
    async fn test_input_atom_retrieves_message() {
        let retriever = InMemoryMessageRetriever::new();
        let session_id = SessionId::new();
        let turn_id = TurnId::new();

        // Add a user message to the retriever
        let user_message = retriever
            .add(session_id, InputMessage::user("Hello, world!"))
            .await
            .unwrap();

        let context = AtomContext::new(session_id, turn_id, user_message.id);
        let atom = InputAtom::new(retriever);

        let result = atom.execute(InputAtomInput { context }).await.unwrap();

        assert_eq!(result.message.id, user_message.id);
        assert_eq!(result.message.text(), Some("Hello, world!"));
    }

    #[tokio::test]
    async fn test_input_atom_not_found() {
        let retriever = InMemoryMessageRetriever::new();
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let missing_id = MessageId::new();

        let context = AtomContext::new(session_id, turn_id, missing_id);
        let atom = InputAtom::new(retriever);

        let result = atom.execute(InputAtomInput { context }).await;

        assert!(result.is_err());
    }
}
