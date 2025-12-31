//! CallModelAtom - Atom for calling the LLM
//!
//! This module handles:
//! 1. Retrieving agent and session configuration from stores
//! 2. Resolving model using priority chain: controls.model_id > session.model_id > agent.default_model_id
//! 3. Building configuration with capabilities applied
//! 4. Loading messages from the store
//! 5. Patching dangling tool calls (tool calls without results)
//! 6. Calling the LLM with the messages
//! 7. Storing the assistant response

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Atom;
use crate::capabilities::CapabilityRegistry;
use crate::error::{AgentLoopError, Result};
use crate::llm_driver_registry::{
    DriverRegistry, LlmCallConfigBuilder, LlmMessage, LlmMessageContent, LlmMessageRole,
    LlmStreamEvent, ProviderConfig, ProviderType,
};
use crate::message::{Message, MessageRole};
use crate::runtime_agent::RuntimeAgentBuilder;
use crate::tool_types::ToolCall;
use crate::traits::{AgentStore, LlmProviderStore, MessageStore, ModelWithProvider, SessionStore};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallModelInput {
    /// Session ID
    pub session_id: Uuid,
    /// Agent ID - the atom will retrieve the agent and build RuntimeAgent
    pub agent_id: Uuid,
}

/// Result of calling the model
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallModelResult {
    /// Text response from the model
    pub text: String,
    /// Tool calls requested by the model (None if no tools)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Whether the loop should continue (has tool calls)
    pub needs_tool_execution: bool,
    /// Tool definitions from applied capabilities (for tool execution)
    #[serde(default)]
    pub tool_definitions: Vec<crate::tool_types::ToolDefinition>,
    /// Maximum iterations configured for the agent
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
}

fn default_max_iterations() -> usize {
    100
}

// ============================================================================
// CallModelAtom
// ============================================================================

/// Atom that calls the LLM model
///
/// This atom:
/// 1. Retrieves agent and session configuration from stores
/// 2. Resolves model using priority: controls.model_id > session.model_id > agent.default_model_id
/// 3. Builds configuration with capabilities applied
/// 4. Loads messages from the store
/// 5. Calls the LLM with the messages
/// 6. Stores the assistant response
/// 7. Returns the result with tool calls (if any)
pub struct CallModelAtom<A, S, M, P>
where
    A: AgentStore,
    S: SessionStore,
    M: MessageStore,
    P: LlmProviderStore,
{
    agent_store: A,
    session_store: S,
    message_store: M,
    provider_store: P,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
}

impl<A, S, M, P> CallModelAtom<A, S, M, P>
where
    A: AgentStore,
    S: SessionStore,
    M: MessageStore,
    P: LlmProviderStore,
{
    /// Create a new CallModelAtom
    pub fn new(
        agent_store: A,
        session_store: S,
        message_store: M,
        provider_store: P,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
    ) -> Self {
        Self {
            agent_store,
            session_store,
            message_store,
            provider_store,
            capability_registry,
            driver_registry,
        }
    }
}

#[async_trait]
impl<A, S, M, P> Atom for CallModelAtom<A, S, M, P>
where
    A: AgentStore + Send + Sync,
    S: SessionStore + Send + Sync,
    M: MessageStore + Send + Sync,
    P: LlmProviderStore + Send + Sync,
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

        // 1. Retrieve agent
        let agent = self
            .agent_store
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| AgentLoopError::agent_not_found(agent_id))?;

        // 2. Retrieve session
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;

        // 3. Load messages (needed for controls.model_id extraction)
        let messages = self.message_store.load(session_id).await?;

        if messages.is_empty() {
            return Err(AgentLoopError::NoMessages);
        }

        // 4. Extract model_id from the last user message's controls (highest priority)
        let controls_model_id = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.controls.as_ref())
            .and_then(|c| c.model_id);

        // 5. Resolve model using chain: controls.model_id > session.model_id > agent.default_model_id
        let model_with_provider = self
            .resolve_model(controls_model_id, session.model_id, agent.default_model_id)
            .await?;

        // 6. Build runtime agent from agent with capabilities applied
        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &self.capability_registry)
            .model(&model_with_provider.model)
            .build();

        // 7. Create LLM driver using factory
        let llm_driver = self.create_llm_driver(&model_with_provider)?;

        // 8. Extract reasoning effort from the last user message's controls
        let reasoning_effort = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.controls.as_ref())
            .and_then(|c| c.reasoning.as_ref())
            .and_then(|r| r.effort.clone());

        // 9. Patch dangling tool calls (add cancelled results for tool calls without responses)
        let patched_messages = patch_dangling_tool_calls(&messages);

        // 10. Build LLM messages
        let mut llm_messages = Vec::new();

        // Add system prompt
        if !runtime_agent.system_prompt.is_empty() {
            llm_messages.push(LlmMessage {
                role: LlmMessageRole::System,
                content: LlmMessageContent::Text(runtime_agent.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages (user, assistant, tool results)
        // Tool calls are embedded in Assistant messages via ContentPart::ToolCall.
        for msg in &patched_messages {
            llm_messages.push(msg.into());
        }

        // 11. Build LLM call config with reasoning effort
        let mut llm_config_builder = LlmCallConfigBuilder::from(&runtime_agent);
        if let Some(effort) = reasoning_effort.clone() {
            llm_config_builder = llm_config_builder.reasoning_effort(effort);
        }
        let llm_config = llm_config_builder.build();

        let mut stream = llm_driver
            .chat_completion_stream(llm_messages, &llm_config)
            .await?;

        // 12. Process stream
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

        // 13. Build metadata with model and reasoning effort info
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "model".to_string(),
            serde_json::Value::String(runtime_agent.model.clone()),
        );
        if let Some(ref effort) = reasoning_effort {
            metadata.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(effort.clone()),
            );
        }

        // 14. Store assistant message with metadata
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

        let tool_calls_option = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls.clone())
        };

        Ok(CallModelResult {
            text,
            tool_calls: tool_calls_option,
            needs_tool_execution: has_tool_calls,
            tool_definitions: runtime_agent.tools.clone(),
            max_iterations: runtime_agent.max_iterations,
        })
    }
}

impl<A, S, M, P> CallModelAtom<A, S, M, P>
where
    A: AgentStore,
    S: SessionStore,
    M: MessageStore,
    P: LlmProviderStore,
{
    /// Resolve model using priority chain:
    /// 1. controls.model_id (from last user message)
    /// 2. session.model_id
    /// 3. agent.default_model_id
    /// 4. system default model
    async fn resolve_model(
        &self,
        controls_model_id: Option<Uuid>,
        session_model_id: Option<Uuid>,
        agent_model_id: Option<Uuid>,
    ) -> Result<ModelWithProvider> {
        // Try controls.model_id first (highest priority)
        if let Some(model_id) = controls_model_id {
            if let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
            {
                return Ok(model_with_provider);
            }
        }

        // Try session.model_id second
        if let Some(model_id) = session_model_id {
            if let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
            {
                return Ok(model_with_provider);
            }
        }

        // Try agent.default_model_id third
        if let Some(model_id) = agent_model_id {
            if let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
            {
                return Ok(model_with_provider);
            }
        }

        // Fall back to system default model
        self.provider_store
            .get_default_model()
            .await?
            .ok_or_else(|| {
                AgentLoopError::llm(
                    "No model configured: no model_id in controls, session, or agent, and no system default model is set"
                )
            })
    }

    /// Create LLM driver using the driver registry
    ///
    /// Returns a user-friendly error if the driver is not registered for the provider type.
    fn create_llm_driver(
        &self,
        model: &ModelWithProvider,
    ) -> Result<crate::llm_driver_registry::BoxedLlmDriver> {
        let provider_type = match model.provider_type {
            crate::llm_models::LlmProviderType::Openai => ProviderType::OpenAI,
            crate::llm_models::LlmProviderType::Anthropic => ProviderType::Anthropic,
            crate::llm_models::LlmProviderType::AzureOpenAI => ProviderType::AzureOpenAI,
        };

        let mut config = ProviderConfig::new(provider_type);
        if let Some(ref api_key) = model.api_key {
            config = config.with_api_key(api_key);
        }
        if let Some(ref base_url) = model.base_url {
            config = config.with_base_url(base_url);
        }

        self.driver_registry.create_driver(&config)
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
            tool_calls: None,
            needs_tool_execution: false,
            tool_definitions: vec![],
            max_iterations: 10,
        };
        assert_eq!(result.text, "Hello");
        assert!(!result.needs_tool_execution);
        assert_eq!(result.max_iterations, 10);
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
