// Atomic Operations for Agent Protocol
//
// Atoms are self-contained, stateless operations that can be composed
// to build agent loops. Each atom handles its own message storage.
//
// Key concepts:
// - Atom trait: Defines atomic operations with Input → Output
// - Each Atom handles: load messages → execute → store results
// - Stateless: No internal state, all state passed in/out
// - Composable: Atoms can be orchestrated by external systems (Temporal, custom loops)

use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AgentConfig;
use crate::error::{AgentLoopError, Result};
use crate::llm::{
    LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, LlmProvider, LlmStreamEvent,
};
use crate::message::{Message, MessageRole};
use crate::traits::{MessageStore, ToolExecutor};

// ============================================================================
// Atom Trait - Core abstraction for atomic operations
// ============================================================================

/// An atomic operation in the agent protocol
///
/// Atoms are self-contained operations that:
/// 1. Take an input with all required context
/// 2. Perform their operation (may load/store messages)
/// 3. Return a result
///
/// This trait enables:
/// - Uniform execution interface for all operations
/// - Easy composition and orchestration
/// - Temporal activity integration
/// - Testing and mocking
#[async_trait]
pub trait Atom: Send + Sync {
    /// Input type for this atom
    type Input: Send;
    /// Output type for this atom
    type Output: Send;

    /// Name of this atom (for logging/debugging)
    fn name(&self) -> &'static str;

    /// Execute the atom with the given input
    async fn execute(&self, input: Self::Input) -> Result<Self::Output>;
}

// ============================================================================
// Atom Inputs and Outputs
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

/// Input for ExecuteToolAtom (single tool)
#[derive(Debug, Clone)]
pub struct ExecuteToolInput {
    /// Session ID
    pub session_id: Uuid,
    /// Tool call to execute
    pub tool_call: ToolCall,
    /// Available tool definitions for resolution
    pub tool_definitions: Vec<ToolDefinition>,
}

/// Result of executing a single tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolResult {
    /// Result of the tool call
    pub result: ToolResult,
    /// Message stored (tool result)
    pub message: Message,
}

// ============================================================================
// AddUserMessageAtom - Atom for adding user messages
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

// ============================================================================
// CallModelAtom - Atom for calling the LLM
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
// ExecuteToolAtom - Atom for executing a single tool
// ============================================================================

/// Atom that executes a single tool call
///
/// This atom:
/// 1. Resolves the tool definition from available definitions
/// 2. Executes the tool call
/// 3. Stores the tool result message
/// 4. Returns the result
pub struct ExecuteToolAtom<M, T>
where
    M: MessageStore,
    T: ToolExecutor,
{
    message_store: M,
    tool_executor: T,
}

impl<M, T> ExecuteToolAtom<M, T>
where
    M: MessageStore,
    T: ToolExecutor,
{
    /// Create a new ExecuteToolAtom
    pub fn new(message_store: M, tool_executor: T) -> Self {
        Self {
            message_store,
            tool_executor,
        }
    }
}

#[async_trait]
impl<M, T> Atom for ExecuteToolAtom<M, T>
where
    M: MessageStore + Send + Sync,
    T: ToolExecutor + Send + Sync,
{
    type Input = ExecuteToolInput;
    type Output = ExecuteToolResult;

    fn name(&self) -> &'static str {
        "execute_tool"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let ExecuteToolInput {
            session_id,
            tool_call,
            tool_definitions,
        } = input;

        // Resolve tool definition
        let tool_definition = tool_definitions
            .iter()
            .find(|def| {
                let name = match def {
                    ToolDefinition::Builtin(b) => &b.name,
                };
                name == &tool_call.name
            })
            .cloned()
            .ok_or_else(|| {
                AgentLoopError::tool(format!("Tool definition not found: {}", tool_call.name))
            })?;

        // Execute tool
        let result = self
            .tool_executor
            .execute(&tool_call, &tool_definition)
            .await?;

        // Store tool result message
        let message =
            Message::tool_result(&tool_call.id, result.result.clone(), result.error.clone());
        self.message_store
            .store(session_id, message.clone())
            .await?;

        Ok(ExecuteToolResult { result, message })
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
