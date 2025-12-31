//! CallModelAtom - Atom for calling the LLM
//!
//! This module handles:
//! 1. Retrieving agent configuration from the store
//! 2. Building configuration with capabilities applied
//! 3. Loading messages from the store
//! 4. Patching dangling tool calls (tool calls without results)
//! 5. Calling the LLM with the messages
//! 6. Storing the assistant response

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Atom;
use crate::capabilities::CapabilityRegistry;
use crate::config::AgentConfigBuilder;
use crate::error::{AgentLoopError, Result};
use crate::llm::{
    LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, LlmProvider, LlmStreamEvent,
};
use crate::message::{Message, MessageRole};
use crate::tool_types::ToolCall;
use crate::traits::{AgentStore, MessageStore};

// ============================================================================
// Helper Functions
// ============================================================================

/// Patch dangling tool calls by adding synthetic "cancelled" results.
///
/// This ensures every tool call has a corresponding tool result,
/// preventing LLM API errors (e.g., OpenAI requires every tool_call to have a result).
///
/// Based on langchain's patch_tool_calls middleware:
/// https://github.com/langchain-ai/deepagents/blob/master/libs/deepagents/deepagents/middleware/patch_tool_calls.py
fn patch_dangling_tool_calls(messages: &[Message]) -> Vec<Message> {
    let mut result = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        result.push(msg.clone());

        // After an assistant message with tool calls, add cancelled results for any missing ones
        if msg.role == MessageRole::Assistant && msg.has_tool_calls() {
            for tc in msg.tool_calls() {
                // Look for a matching tool result in ALL subsequent messages
                let has_result = messages[(i + 1)..]
                    .iter()
                    .any(|m| m.role == MessageRole::ToolResult && m.tool_call_id() == Some(&tc.id));

                if !has_result {
                    result.push(Message::tool_result(
                        &tc.id,
                        None,
                        Some(
                            "cancelled - another message came in before it could be completed"
                                .to_string(),
                        ),
                    ));
                }
            }
        }
    }

    result
}

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for CallModelAtom
#[derive(Debug, Clone)]
pub struct CallModelInput {
    /// Session ID
    pub session_id: Uuid,
    /// Agent ID - the atom will retrieve the agent and build AgentConfig
    pub agent_id: Uuid,
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
/// 1. Retrieves agent configuration from the store
/// 2. Builds configuration with capabilities applied
/// 3. Loads messages from the store
/// 4. Calls the LLM with the messages
/// 5. Stores the assistant response
/// 6. Returns the result with tool calls (if any)
pub struct CallModelAtom<A, M, L>
where
    A: AgentStore,
    M: MessageStore,
    L: LlmProvider,
{
    agent_store: A,
    message_store: M,
    llm_provider: L,
    capability_registry: CapabilityRegistry,
}

impl<A, M, L> CallModelAtom<A, M, L>
where
    A: AgentStore,
    M: MessageStore,
    L: LlmProvider,
{
    /// Create a new CallModelAtom
    pub fn new(
        agent_store: A,
        message_store: M,
        llm_provider: L,
        capability_registry: CapabilityRegistry,
    ) -> Self {
        Self {
            agent_store,
            message_store,
            llm_provider,
            capability_registry,
        }
    }
}

#[async_trait]
impl<A, M, L> Atom for CallModelAtom<A, M, L>
where
    A: AgentStore + Send + Sync,
    M: MessageStore + Send + Sync,
    L: LlmProvider + Send + Sync,
{
    type Input = CallModelInput;
    type Output = CallModelResult;

    fn name(&self) -> &'static str {
        "call_model"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let CallModelInput {
            session_id,
            agent_id,
        } = input;

        // 1. Retrieve agent and build config with capabilities
        let agent = self
            .agent_store
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| AgentLoopError::agent_not_found(agent_id))?;

        // Build config from agent with capabilities applied
        // TODO: resolve model from agent.default_model_id
        let config = AgentConfigBuilder::new()
            .with_agent(&agent, &self.capability_registry)
            .model("gpt-4o")
            .build();

        // 2. Load messages
        let messages = self.message_store.load(session_id).await?;

        if messages.is_empty() {
            return Err(AgentLoopError::NoMessages);
        }

        // 3. Extract reasoning effort from the last user message's controls
        let reasoning_effort = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.controls.as_ref())
            .and_then(|c| c.reasoning.as_ref())
            .and_then(|r| r.effort.clone());

        // 4. Patch dangling tool calls (add cancelled results for tool calls without responses)
        let patched_messages = patch_dangling_tool_calls(&messages);

        // 5. Build LLM messages
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

        // Add conversation messages (user, assistant, tool results)
        // Tool calls are embedded in Assistant messages via ContentPart::ToolCall.
        for msg in &patched_messages {
            llm_messages.push(msg.into());
        }

        // 6. Call LLM with reasoning effort
        let mut llm_config = LlmCallConfig::from(&config);
        llm_config.reasoning_effort = reasoning_effort.clone();

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

        // 7. Build metadata with model and reasoning effort info
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "model".to_string(),
            serde_json::Value::String(config.model.clone()),
        );
        if let Some(ref effort) = reasoning_effort {
            metadata.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(effort.clone()),
            );
        }

        // 8. Store assistant message with metadata
        let has_tool_calls = !tool_calls.is_empty();
        let mut assistant_message = if has_tool_calls {
            Message::assistant_with_tools(&text, tool_calls.clone())
        } else {
            Message::assistant(&text)
        };
        assistant_message.metadata = Some(metadata);

        self.message_store
            .store(session_id, assistant_message.clone())
            .await?;

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

    #[test]
    fn test_patch_dangling_tool_calls_no_tool_calls() {
        // Messages without tool calls should be unchanged
        let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];

        let patched = patch_dangling_tool_calls(&messages);

        assert_eq!(patched.len(), 2);
        assert_eq!(patched[0].role, MessageRole::User);
        assert_eq!(patched[1].role, MessageRole::Assistant);
    }

    #[test]
    fn test_patch_dangling_tool_calls_with_result() {
        // Tool call with matching result should not be patched
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let messages = vec![
            Message::user("What's the weather?"),
            Message::assistant_with_tools("Let me check", vec![tool_call]),
            Message::tool_result("call_123", Some(serde_json::json!({"temp": 72})), None),
        ];

        let patched = patch_dangling_tool_calls(&messages);

        assert_eq!(patched.len(), 3);
        assert_eq!(patched[0].role, MessageRole::User);
        assert_eq!(patched[1].role, MessageRole::Assistant);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
    }

    #[test]
    fn test_patch_dangling_tool_calls_missing_result() {
        // Tool call without result should get a cancelled result added
        let tool_call = ToolCall {
            id: "call_456".to_string(),
            name: "search_web".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };

        let messages = vec![
            Message::user("Search for rust"),
            Message::assistant_with_tools("Searching...", vec![tool_call]),
            // No tool result - simulating interruption
            Message::user("Actually, never mind"),
        ];

        let patched = patch_dangling_tool_calls(&messages);

        // Should have added a cancelled result before the new user message
        assert_eq!(patched.len(), 4);
        assert_eq!(patched[0].role, MessageRole::User);
        assert_eq!(patched[1].role, MessageRole::Assistant);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
        assert_eq!(patched[2].tool_call_id(), Some("call_456"));
        // Check the error message
        let result_content = patched[2].tool_result_content().unwrap();
        assert!(result_content.error.is_some());
        assert!(result_content.error.as_ref().unwrap().contains("cancelled"));
        assert_eq!(patched[3].role, MessageRole::User);
    }

    #[test]
    fn test_patch_dangling_tool_calls_multiple_calls() {
        // Multiple tool calls where one is missing result
        let tool_call_1 = ToolCall {
            id: "call_1".to_string(),
            name: "tool_a".to_string(),
            arguments: serde_json::json!({}),
        };
        let tool_call_2 = ToolCall {
            id: "call_2".to_string(),
            name: "tool_b".to_string(),
            arguments: serde_json::json!({}),
        };

        let messages = vec![
            Message::user("Do two things"),
            Message::assistant_with_tools("On it", vec![tool_call_1, tool_call_2]),
            Message::tool_result("call_1", Some(serde_json::json!("done")), None),
            // call_2 has no result
        ];

        let patched = patch_dangling_tool_calls(&messages);

        // Should have added a cancelled result for call_2 right after the assistant message
        // Order: [0] user, [1] assistant, [2] cancelled call_2, [3] real result call_1
        assert_eq!(patched.len(), 4);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
        assert_eq!(patched[2].tool_call_id(), Some("call_2"));
        // Verify the cancelled result has the error message
        let result_content = patched[2].tool_result_content().unwrap();
        assert!(result_content.error.as_ref().unwrap().contains("cancelled"));
    }

    #[test]
    fn test_patch_dangling_tool_calls_at_end() {
        // Dangling tool call at the end of conversation
        let tool_call = ToolCall {
            id: "call_end".to_string(),
            name: "final_tool".to_string(),
            arguments: serde_json::json!({}),
        };

        let messages = vec![
            Message::user("Do something"),
            Message::assistant_with_tools("Running tool", vec![tool_call]),
            // Conversation ends without tool result
        ];

        let patched = patch_dangling_tool_calls(&messages);

        // Should have added a cancelled result at the end
        assert_eq!(patched.len(), 3);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
        assert_eq!(patched[2].tool_call_id(), Some("call_end"));
    }
}
