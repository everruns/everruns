// Agent Protocol - Stateless Atomic Operations
//
// AgentProtocol provides a stateless, inverted approach to agent execution.
// Instead of the executor owning state and orchestrating the loop, this module
// provides atomic operations (Atoms) that are self-contained and handle their
// own message retrieval and storage.
//
// Key concepts:
// - Atom trait: Defines atomic operations with Input → Output
// - Each Atom handles: load messages → execute → store results
// - Stateless: No internal state, all state passed in/out
// - Composable: Atoms can be orchestrated by external systems (Temporal, custom loops)

use async_trait::async_trait;
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::AgentConfig;
use crate::error::{AgentLoopError, Result};
use crate::llm::{
    LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, LlmProvider, LlmStreamEvent,
};
use crate::message::{ConversationMessage, MessageRole};
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
    pub assistant_message: ConversationMessage,
}

/// Input for ExecuteToolAtom (single tool)
#[derive(Debug, Clone)]
pub struct ExecuteToolInput {
    /// Session ID
    pub session_id: Uuid,
    /// Tool call to execute
    pub tool_call: ToolCall,
    /// Tool definition
    pub tool_definition: ToolDefinition,
}

/// Result of executing a single tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolResult {
    /// Result of the tool call
    pub result: ToolResult,
    /// Message stored (tool result)
    pub message: ConversationMessage,
}

/// Input for ExecuteToolsAtom (multiple tools in parallel)
#[derive(Debug, Clone)]
pub struct ExecuteToolsInput {
    /// Session ID
    pub session_id: Uuid,
    /// Tool calls to execute
    pub tool_calls: Vec<ToolCall>,
    /// Available tool definitions
    pub tool_definitions: Vec<ToolDefinition>,
}

/// Result of executing tools
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolsResult {
    /// Results for each tool call
    pub results: Vec<ToolResult>,
    /// Messages stored (tool results)
    pub messages: Vec<ConversationMessage>,
}

/// Result of loading messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadMessagesResult {
    /// Loaded messages
    pub messages: Vec<ConversationMessage>,
    /// Count of messages
    pub count: usize,
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
            ConversationMessage::assistant_with_tools(&text, tool_calls.clone())
        } else {
            ConversationMessage::assistant(&text)
        };

        self.message_store
            .store(session_id, assistant_message.clone())
            .await?;

        // 5. If there are tool calls, store tool_call messages too
        if has_tool_calls {
            for tool_call in &tool_calls {
                let tool_call_msg = ConversationMessage::tool_call(tool_call);
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
/// 1. Executes the tool call
/// 2. Stores the tool result message
/// 3. Returns the result
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
            tool_definition,
        } = input;

        // Execute tool
        let result = self
            .tool_executor
            .execute(&tool_call, &tool_definition)
            .await?;

        // Store tool result message
        let message = ConversationMessage::tool_result(
            &tool_call.id,
            result.result.clone(),
            result.error.clone(),
        );
        self.message_store
            .store(session_id, message.clone())
            .await?;

        Ok(ExecuteToolResult { result, message })
    }
}

// ============================================================================
// ExecuteToolsAtom - Atom for executing multiple tools in parallel
// ============================================================================

/// Atom that executes multiple tool calls in parallel
///
/// This atom:
/// 1. Executes all tool calls concurrently using ExecuteToolAtom
/// 2. Collects and returns all results
pub struct ExecuteToolsAtom<M, T>
where
    M: MessageStore,
    T: ToolExecutor,
{
    message_store: M,
    tool_executor: T,
}

impl<M, T> ExecuteToolsAtom<M, T>
where
    M: MessageStore,
    T: ToolExecutor,
{
    /// Create a new ExecuteToolsAtom
    pub fn new(message_store: M, tool_executor: T) -> Self {
        Self {
            message_store,
            tool_executor,
        }
    }
}

#[async_trait]
impl<M, T> Atom for ExecuteToolsAtom<M, T>
where
    M: MessageStore + Clone + Send + Sync,
    T: ToolExecutor + Clone + Send + Sync,
{
    type Input = ExecuteToolsInput;
    type Output = ExecuteToolsResult;

    fn name(&self) -> &'static str {
        "execute_tools"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let ExecuteToolsInput {
            session_id,
            tool_calls,
            tool_definitions,
        } = input;

        // Build tool definition map
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_definitions
            .iter()
            .map(|def| {
                let name = match def {
                    ToolDefinition::Builtin(b) => b.name.as_str(),
                };
                (name, def)
            })
            .collect();

        // Prepare inputs for parallel execution
        let mut inputs = Vec::with_capacity(tool_calls.len());
        for tool_call in tool_calls {
            let tool_def = tool_map.get(tool_call.name.as_str()).ok_or_else(|| {
                AgentLoopError::tool(format!("Tool not found: {}", tool_call.name))
            })?;

            inputs.push(ExecuteToolInput {
                session_id,
                tool_call,
                tool_definition: (*tool_def).clone(),
            });
        }

        // Execute all tools in parallel
        let futures: Vec<_> = inputs
            .into_iter()
            .map(|input| {
                let atom =
                    ExecuteToolAtom::new(self.message_store.clone(), self.tool_executor.clone());
                async move { atom.execute(input).await }
            })
            .collect();

        let results_vec: Vec<Result<ExecuteToolResult>> = futures::future::join_all(futures).await;

        // Collect results, propagating any errors
        let mut results = Vec::with_capacity(results_vec.len());
        let mut messages = Vec::with_capacity(results_vec.len());

        for result in results_vec {
            let r = result?;
            results.push(r.result);
            messages.push(r.message);
        }

        Ok(ExecuteToolsResult { results, messages })
    }
}

// ============================================================================
// AgentProtocol - Orchestrates atoms
// ============================================================================

/// Stateless agent protocol with atomic operations
///
/// Provides convenient methods that internally use atoms.
/// For direct atom access, use the individual atom types.
pub struct AgentProtocol<M, L, T>
where
    M: MessageStore,
    L: LlmProvider,
    T: ToolExecutor,
{
    message_store: M,
    llm_provider: L,
    tool_executor: T,
}

impl<M, L, T> AgentProtocol<M, L, T>
where
    M: MessageStore + Clone + Send + Sync,
    L: LlmProvider + Clone + Send + Sync,
    T: ToolExecutor + Clone + Send + Sync,
{
    /// Create a new agent protocol
    pub fn new(message_store: M, llm_provider: L, tool_executor: T) -> Self {
        Self {
            message_store,
            llm_provider,
            tool_executor,
        }
    }

    /// Get reference to the message store
    pub fn message_store(&self) -> &M {
        &self.message_store
    }

    /// Get reference to the LLM provider
    pub fn llm_provider(&self) -> &L {
        &self.llm_provider
    }

    /// Get reference to the tool executor
    pub fn tool_executor(&self) -> &T {
        &self.tool_executor
    }

    /// Create a CallModelAtom
    pub fn call_model_atom(&self) -> CallModelAtom<M, L> {
        CallModelAtom::new(self.message_store.clone(), self.llm_provider.clone())
    }

    /// Create an ExecuteToolAtom (single tool)
    pub fn execute_tool_atom(&self) -> ExecuteToolAtom<M, T> {
        ExecuteToolAtom::new(self.message_store.clone(), self.tool_executor.clone())
    }

    /// Create an ExecuteToolsAtom (multiple tools in parallel)
    pub fn execute_tools_atom(&self) -> ExecuteToolsAtom<M, T> {
        ExecuteToolsAtom::new(self.message_store.clone(), self.tool_executor.clone())
    }

    // ========================================================================
    // Convenience Methods (use atoms internally)
    // ========================================================================

    /// Load all messages for a session
    pub async fn load_messages(&self, session_id: Uuid) -> Result<LoadMessagesResult> {
        let messages = self.message_store.load(session_id).await?;
        let count = messages.len();
        Ok(LoadMessagesResult { messages, count })
    }

    /// Add a user message to the conversation
    pub async fn add_user_message(
        &self,
        session_id: Uuid,
        content: impl Into<String>,
    ) -> Result<ConversationMessage> {
        let message = ConversationMessage::user(content);
        self.message_store
            .store(session_id, message.clone())
            .await?;
        Ok(message)
    }

    /// Call the LLM model (uses CallModelAtom)
    pub async fn call_model(
        &self,
        session_id: Uuid,
        config: &AgentConfig,
    ) -> Result<CallModelResult> {
        let atom = self.call_model_atom();
        atom.execute(CallModelInput {
            session_id,
            config: config.clone(),
        })
        .await
    }

    /// Execute pending tool calls (uses ExecuteToolsAtom)
    pub async fn execute_tools(
        &self,
        session_id: Uuid,
        tool_calls: &[ToolCall],
        tool_definitions: &[ToolDefinition],
    ) -> Result<ExecuteToolsResult> {
        let atom = self.execute_tools_atom();
        atom.execute(ExecuteToolsInput {
            session_id,
            tool_calls: tool_calls.to_vec(),
            tool_definitions: tool_definitions.to_vec(),
        })
        .await
    }

    /// Run a complete turn (user message → final response)
    ///
    /// Determines the next action based on the output of the previous atom,
    /// without re-inspecting the message store. Follows a functional data-flow pattern:
    ///
    /// ```text
    /// User Message → CallModel → ExecuteTools → CallModel → ... → Response
    /// ```
    pub async fn run_turn(
        &self,
        session_id: Uuid,
        user_message: impl Into<String>,
        config: &AgentConfig,
        max_iterations: usize,
    ) -> Result<String> {
        self.add_user_message(session_id, user_message).await?;

        let mut final_response = String::new();

        for iteration in 1..=max_iterations {
            // Call the model
            let result = self.call_model(session_id, config).await?;

            // Capture the response text
            if !result.text.is_empty() {
                final_response = result.text;
            }

            // If no tool calls, we're done
            if !result.needs_tool_execution {
                return Ok(final_response);
            }

            // Execute tools within this iteration
            self.execute_tools(session_id, &result.tool_calls, &config.tools)
                .await?;

            // Check if we've exhausted iterations
            if iteration == max_iterations {
                return Err(AgentLoopError::MaxIterationsReached(max_iterations));
            }
        }

        // Should not reach here, but just in case
        Err(AgentLoopError::MaxIterationsReached(max_iterations))
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
            assistant_message: ConversationMessage::assistant("Hello"),
        };
        assert_eq!(result.text, "Hello");
        assert!(!result.needs_tool_execution);
    }
}
