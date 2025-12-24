// Message service for business logic
// Messages are the PRIMARY conversation data store
//
// The message contract uses ContentPart arrays for flexible content types.
// This service handles conversion between the API contract and database storage.

use crate::messages::{ContentPart, Message, MessageRole};
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

/// Convert ContentPart array to stored JSON content
pub fn content_parts_to_json(role: &MessageRole, parts: &[ContentPart]) -> serde_json::Value {
    match role {
        MessageRole::User | MessageRole::Assistant | MessageRole::System => {
            // Collect text parts and tool call parts
            let mut texts = Vec::new();
            let mut tool_calls = Vec::new();

            for part in parts {
                match part {
                    ContentPart::Text { text } => texts.push(text.clone()),
                    ContentPart::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        tool_calls.push(serde_json::json!({
                            "id": id,
                            "name": name,
                            "arguments": arguments
                        }));
                    }
                    _ => {} // Skip image and tool_result in user/assistant/system messages
                }
            }

            let combined_text = texts.join("\n");

            if tool_calls.is_empty() {
                serde_json::json!({ "text": combined_text })
            } else {
                serde_json::json!({
                    "text": combined_text,
                    "tool_calls": tool_calls
                })
            }
        }
        MessageRole::ToolCall => {
            // Find the first tool call part
            for part in parts {
                if let ContentPart::ToolCall {
                    id,
                    name,
                    arguments,
                } = part
                {
                    return serde_json::json!({
                        "id": id,
                        "name": name,
                        "arguments": arguments
                    });
                }
            }
            serde_json::json!({})
        }
        MessageRole::ToolResult => {
            // Find the first tool result part
            for part in parts {
                if let ContentPart::ToolResult { result, error } = part {
                    return serde_json::json!({
                        "result": result,
                        "error": error
                    });
                }
            }
            serde_json::json!({})
        }
    }
}
