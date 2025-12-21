// Database-backed adapters for core traits
//
// These implementations connect the core agent-loop abstraction to the
// actual database used in production.

use async_trait::async_trait;
use everruns_contracts::tools::ToolCall;
use everruns_core::{
    message::MessageContent,
    traits::MessageStore,
    AgentLoopError, Message, MessageRole, Result,
};
use everruns_storage::models::CreateMessage;
use everruns_storage::repositories::Database;
use uuid::Uuid;

// ============================================================================
// DbMessageStore - Stores messages in database
// ============================================================================

/// Database-backed message store
///
/// Stores conversation messages in the messages table.
/// Used by activities to load/store messages during workflow execution.
#[derive(Clone)]
pub struct DbMessageStore {
    db: Database,
}

impl DbMessageStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl MessageStore for DbMessageStore {
    async fn store(&self, session_id: Uuid, message: Message) -> Result<()> {
        let role = message.role.to_string();
        let content = match &message.content {
            MessageContent::Text(text) => {
                serde_json::json!({ "text": text })
            }
            MessageContent::ToolCall {
                id,
                name,
                arguments,
            } => {
                serde_json::json!({
                    "id": id,
                    "name": name,
                    "arguments": arguments
                })
            }
            MessageContent::ToolResult { result, error } => {
                serde_json::json!({
                    "result": result,
                    "error": error
                })
            }
        };

        let create_msg = CreateMessage {
            session_id,
            role,
            content,
            tool_call_id: message.tool_call_id,
        };

        self.db
            .create_message(create_msg)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let messages = self
            .db
            .list_messages(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let converted: Vec<Message> = messages
            .into_iter()
            .map(|msg| {
                let role = MessageRole::from(msg.role.as_str());
                let content = match role {
                    MessageRole::User | MessageRole::Assistant | MessageRole::System => {
                        let text = msg
                            .content
                            .get("text")
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string();
                        MessageContent::Text(text)
                    }
                    MessageRole::ToolCall => {
                        let id = msg
                            .content
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = msg
                            .content
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let arguments = msg
                            .content
                            .get("arguments")
                            .cloned()
                            .unwrap_or(serde_json::json!({}));
                        MessageContent::ToolCall {
                            id,
                            name,
                            arguments,
                        }
                    }
                    MessageRole::ToolResult => {
                        let result = msg.content.get("result").cloned();
                        let error = msg
                            .content
                            .get("error")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        MessageContent::ToolResult { result, error }
                    }
                };

                // Parse tool_calls from assistant messages if present
                let tool_calls = if role == MessageRole::Assistant {
                    msg.content
                        .get("tool_calls")
                        .and_then(|tc| serde_json::from_value::<Vec<ToolCall>>(tc.clone()).ok())
                } else {
                    None
                };

                Message {
                    id: msg.id,
                    role,
                    content,
                    tool_call_id: msg.tool_call_id,
                    tool_calls,
                    created_at: msg.created_at,
                }
            })
            .collect();

        Ok(converted)
    }

    async fn count(&self, session_id: Uuid) -> Result<usize> {
        let messages = self.load(session_id).await?;
        Ok(messages.len())
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed message store
pub fn create_db_message_store(db: Database) -> DbMessageStore {
    DbMessageStore::new(db)
}
