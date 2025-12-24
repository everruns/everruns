//! CallModelAtom - Atom for calling the LLM

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Atom;
use crate::config::AgentConfig;
use crate::error::{AgentLoopError, Result};
use crate::llm::{
    LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, LlmProvider, LlmStreamEvent,
};
use crate::message::{Message, MessageRole};
use crate::tool_types::ToolCall;
use crate::traits::MessageStore;

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for CallModelAtom
#[derive(Debug, Clone)]
pub struct CallModelInput {
    /// Session ID
    pub session_id: Uuid,
    /// Agent configuration
    pub config: AgentConfig,
}

/// Result of calling the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallModelResult {
    /// Text response from the model
    pub text: String,
    /// Tool calls requested by the model (empty if none)
    pub tool_calls: Vec<ToolCall>,
    /// Whether the loop should continue (has tool calls)
    pub needs_tool_execution: bool,
    /// The assistant message that was stored
    pub assistant_message: Message,
}

// ============================================================================
// CallModelAtom
// ============================================================================

/// Atom that calls the LLM model
///
/// This atom:
/// 1. Loads messages from the store
/// 2. Calls the LLM with the messages
/// 3. Stores the assistant response
/// 4. Returns the result with tool calls (if any)
pub struct CallModelAtom<M, L>
where
    M: MessageStore,
    L: LlmProvider,
{
    message_store: M,
    llm_provider: L,
}

impl<M, L> CallModelAtom<M, L>
where
    M: MessageStore,
    L: LlmProvider,
{
    /// Create a new CallModelAtom
    pub fn new(message_store: M, llm_provider: L) -> Self {
        Self {
            message_store,
            llm_provider,
        }
    }
}

#[async_trait]
impl<M, L> Atom for CallModelAtom<M, L>
where
    M: MessageStore + Send + Sync,
    L: LlmProvider + Send + Sync,
{
    type Input = CallModelInput;
    type Output = CallModelResult;

    fn name(&self) -> &'static str {
        "call_model"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let CallModelInput { session_id, config } = input;

        // 1. Load messages
        let messages = self.message_store.load(session_id).await?;

        if messages.is_empty() {
            return Err(AgentLoopError::NoMessages);
        }

        // 2. Build LLM messages
        let mut llm_messages = Vec::new();

        // Add system prompt
        if !config.system_prompt.is_empty() {
            llm_messages.push(LlmMessage {
                role: LlmMessageRole::System,
                content: LlmMessageContent::Text(config.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages (skip tool_call messages - they're in assistant messages)
        for msg in &messages {
            if msg.role == MessageRole::ToolCall {
                continue;
            }
            llm_messages.push(msg.into());
        }

        // 3. Call LLM
        let llm_config = LlmCallConfig::from(&config);
        let mut stream = self
            .llm_provider
            .chat_completion_stream(llm_messages, &llm_config)
            .await?;

        // Process stream
        let mut text = String::new();
        let mut tool_calls = Vec::new();

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                }
                LlmStreamEvent::ToolCalls(calls) => {
                    tool_calls = calls;
                }
                LlmStreamEvent::Done(_) => {
                    break;
                }
                LlmStreamEvent::Error(err) => {
                    return Err(AgentLoopError::llm(err));
                }
            }
        }

        // 4. Store assistant message
        let has_tool_calls = !tool_calls.is_empty();
        let assistant_message = if has_tool_calls {
            Message::assistant_with_tools(&text, tool_calls.clone())
        } else {
            Message::assistant(&text)
        };

        self.message_store
            .store(session_id, assistant_message.clone())
            .await?;

        // 5. If there are tool calls, store tool_call messages too
        if has_tool_calls {
            for tool_call in &tool_calls {
                let tool_call_msg = Message::tool_call(tool_call);
                self.message_store.store(session_id, tool_call_msg).await?;
            }
        }

        Ok(CallModelResult {
            text,
            tool_calls: tool_calls.clone(),
            needs_tool_execution: has_tool_calls,
            assistant_message,
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_model_result() {
        let result = CallModelResult {
            text: "Hello".to_string(),
            tool_calls: vec![],
            needs_tool_execution: false,
            assistant_message: Message::assistant("Hello"),
        };
        assert_eq!(result.text, "Hello");
        assert!(!result.needs_tool_execution);
    }
}
