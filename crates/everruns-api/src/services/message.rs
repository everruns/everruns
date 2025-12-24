// Message service for business logic
// Messages are the PRIMARY conversation data store
//
// The message contract uses ContentPart arrays for flexible content types.
// This service handles conversion between the API contract and database storage.

use crate::messages::{ContentPart, InputContentPart, Message, MessageRole};
use anyhow::Result;
use everruns_storage::{models::CreateMessage, Database};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub struct MessageService {
    db: Arc<Database>,
}

impl MessageService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateMessage) -> Result<Message> {
        let row = self.db.create_message(input).await?;
        Ok(Self::row_to_message(row))
    }

    #[allow(dead_code)] // Will be used by future endpoints
    pub async fn get(&self, id: Uuid) -> Result<Option<Message>> {
        let row = self.db.get_message(id).await?;
        Ok(row.map(Self::row_to_message))
    }

    pub async fn list(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let rows = self.db.list_messages(session_id).await?;
        Ok(rows.into_iter().map(Self::row_to_message).collect())
    }

    /// Convert database row to API Message
    fn row_to_message(row: everruns_storage::MessageRow) -> Message {
        let role = MessageRole::from(row.role.as_str());
        let content = Self::json_to_content_parts(&role, &row.content);
        let metadata = row
            .metadata
            .and_then(|m| serde_json::from_value::<HashMap<String, serde_json::Value>>(m).ok());

        Message {
            id: row.id,
            session_id: row.session_id,
            sequence: row.sequence,
            role,
            content,
            metadata,
            tool_call_id: row.tool_call_id,
            created_at: row.created_at,
        }
    }

    /// Convert stored JSON content to ContentPart array
    fn json_to_content_parts(role: &MessageRole, content: &serde_json::Value) -> Vec<ContentPart> {
        match role {
            MessageRole::User | MessageRole::Assistant | MessageRole::System => {
                // Text content: { "text": "..." }
                let text = content
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                let mut parts = vec![ContentPart::Text { text }];

                // For assistant messages, also include any tool_calls
                if *role == MessageRole::Assistant {
                    if let Some(tool_calls) = content.get("tool_calls").and_then(|tc| tc.as_array())
                    {
                        for tc in tool_calls {
                            if let (Some(id), Some(name), Some(args)) = (
                                tc.get("id").and_then(|v| v.as_str()),
                                tc.get("name").and_then(|v| v.as_str()),
                                tc.get("arguments"),
                            ) {
                                parts.push(ContentPart::ToolCall {
                                    id: id.to_string(),
                                    name: name.to_string(),
                                    arguments: args.clone(),
                                });
                            }
                        }
                    }
                }

                parts
            }
            MessageRole::ToolCall => {
                // Tool call content: { "id": "...", "name": "...", "arguments": {...} }
                let id = content
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = content
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments = content
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));

                vec![ContentPart::ToolCall {
                    id,
                    name,
                    arguments,
                }]
            }
            MessageRole::ToolResult => {
                // Tool result content: { "result": {...}, "error": "..." }
                let result = content.get("result").cloned();
                let error = content
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                vec![ContentPart::ToolResult { result, error }]
            }
        }
    }
}

/// Convert InputContentPart array to stored JSON content (for user input)
pub fn input_content_parts_to_json(parts: &[InputContentPart]) -> serde_json::Value {
    // User input only contains text and images
    let mut texts = Vec::new();

    for part in parts {
        if let InputContentPart::Text { text } = part {
            texts.push(text.clone());
        }
        // TODO: Handle images when image storage is implemented
    }

    let combined_text = texts.join("\n");
    serde_json::json!({ "text": combined_text })
}
