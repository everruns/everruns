//! AddUserMessageAtom - Atom for adding user messages

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Atom;
use crate::error::Result;
use crate::message::Message;
use crate::traits::MessageStore;

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for AddUserMessageAtom
#[derive(Debug, Clone)]
pub struct AddUserMessageInput {
    /// Session ID
    pub session_id: Uuid,
    /// Message content
    pub content: String,
}

/// Result of adding a user message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddUserMessageResult {
    /// The stored message
    pub message: Message,
}

// ============================================================================
// AddUserMessageAtom
// ============================================================================

/// Atom that adds a user message to the conversation
///
/// This atom:
/// 1. Creates a user message
/// 2. Stores it in the message store
/// 3. Returns the stored message
pub struct AddUserMessageAtom<M>
where
    M: MessageStore,
{
    message_store: M,
}

impl<M> AddUserMessageAtom<M>
where
    M: MessageStore,
{
    /// Create a new AddUserMessageAtom
    pub fn new(message_store: M) -> Self {
        Self { message_store }
    }
}

#[async_trait]
impl<M> Atom for AddUserMessageAtom<M>
where
    M: MessageStore + Send + Sync,
{
    type Input = AddUserMessageInput;
    type Output = AddUserMessageResult;

    fn name(&self) -> &'static str {
        "add_user_message"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let AddUserMessageInput {
            session_id,
            content,
        } = input;

        let message = Message::user(content);
        self.message_store
            .store(session_id, message.clone())
            .await?;

        Ok(AddUserMessageResult { message })
    }
}
