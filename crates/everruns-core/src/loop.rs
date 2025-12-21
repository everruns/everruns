// Agent Loop v2 - Atom-based execution loop
//
// AgentLoop2 provides a high-level loop that orchestrates atoms to execute
// a complete agent turn (user message → LLM calls → tool execution → response).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::atoms::{
    AddUserMessageAtom, AddUserMessageInput, AddUserMessageResult, Atom, CallModelAtom,
    CallModelInput, CallModelResult, ExecuteToolAtom, ExecuteToolInput, ExecuteToolResult,
};
use crate::config::AgentConfig;
use crate::error::{AgentLoopError, Result};
use crate::llm::LlmProvider;
use crate::message::Message;
use crate::traits::{MessageStore, ToolExecutor};

/// Result of loading messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadMessagesResult {
    /// Loaded messages
    pub messages: Vec<Message>,
    /// Count of messages
    pub count: usize,
}

/// Atom-based agent loop
///
/// Provides a high-level interface for running agent turns using atoms.
/// Each method internally creates and executes the appropriate atom.
///
/// For direct atom access, use the individual atom factory methods.
pub struct AgentLoop2<M, L, T>
where
    M: MessageStore,
    L: LlmProvider,
    T: ToolExecutor,
{
    message_store: M,
    llm_provider: L,
    tool_executor: T,
}

impl<M, L, T> AgentLoop2<M, L, T>
where
    M: MessageStore + Clone + Send + Sync,
    L: LlmProvider + Clone + Send + Sync,
    T: ToolExecutor + Clone + Send + Sync,
{
    /// Create a new agent loop
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

    // ========================================================================
    // Atom Factory Methods
    // ========================================================================

    /// Create an AddUserMessageAtom
    pub fn add_user_message_atom(&self) -> AddUserMessageAtom<M> {
        AddUserMessageAtom::new(self.message_store.clone())
    }

    /// Create a CallModelAtom
    pub fn call_model_atom(&self) -> CallModelAtom<M, L> {
        CallModelAtom::new(self.message_store.clone(), self.llm_provider.clone())
    }

    /// Create an ExecuteToolAtom (single tool)
    pub fn execute_tool_atom(&self) -> ExecuteToolAtom<M, T> {
        ExecuteToolAtom::new(self.message_store.clone(), self.tool_executor.clone())
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

    /// Add a user message to the conversation (uses AddUserMessageAtom)
    pub async fn add_user_message(
        &self,
        session_id: Uuid,
        content: impl Into<String>,
    ) -> Result<AddUserMessageResult> {
        let atom = self.add_user_message_atom();
        atom.execute(AddUserMessageInput {
            session_id,
            content: content.into(),
        })
        .await
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

    /// Run a complete turn (user message → final response)
    ///
    /// Determines the next action based on the output of the previous atom,
    /// without re-inspecting the message store. Follows a functional data-flow pattern:
    ///
    /// ```text
    /// User Message → CallModel → ExecuteTools (parallel) → CallModel → ... → Response
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

            // Execute tools in parallel using ExecuteToolAtom
            let tool_definitions = config.tools.clone();
            let futures: Vec<_> = result
                .tool_calls
                .into_iter()
                .map(|tool_call| {
                    let atom = self.execute_tool_atom();
                    let tool_defs = tool_definitions.clone();
                    async move {
                        atom.execute(ExecuteToolInput {
                            session_id,
                            tool_call,
                            tool_definitions: tool_defs,
                        })
                        .await
                    }
                })
                .collect();

            let results: Vec<Result<ExecuteToolResult>> = futures::future::join_all(futures).await;

            // Check for errors
            for result in results {
                result?;
            }

            // Check if we've exhausted iterations
            if iteration == max_iterations {
                return Err(AgentLoopError::MaxIterationsReached(max_iterations));
            }
        }

        // Should not reach here, but just in case
        Err(AgentLoopError::MaxIterationsReached(max_iterations))
    }
}
