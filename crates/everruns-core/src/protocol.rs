// Agent Protocol - Stateless Atomic Operations
//
// AgentProtocol provides a stateless, inverted approach to agent execution.
// Instead of the executor owning state and orchestrating the loop, this module
// provides atomic operations (Atoms) that are self-contained and handle their
// own message retrieval and storage.
//
// Key concepts:
// - Atoms: Atomic operations (call_model, execute_tool, etc.)
// - Each Atom handles: load messages → execute → store results
// - Stateless: No internal state, all state passed in/out
// - Composable: Atoms can be orchestrated by external systems (Temporal, custom loops)

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
// Atom Results - Output types for atomic operations
// ============================================================================

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

/// What action should be taken next
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NextAction {
    /// Call the model
    CallModel,
    /// Execute pending tool calls
    ExecuteTools { tool_calls: Vec<ToolCall> },
    /// Session is complete
    Complete { final_response: Option<String> },
    /// Error occurred
    Error { message: String },
}

impl NextAction {
    /// Check if this is a CallModel action
    pub fn is_call_model(&self) -> bool {
        matches!(self, NextAction::CallModel)
    }

    /// Check if this is an ExecuteTools action
    pub fn is_execute_tools(&self) -> bool {
        matches!(self, NextAction::ExecuteTools { .. })
    }

    /// Check if this is a Complete action
    pub fn is_complete(&self) -> bool {
        matches!(self, NextAction::Complete { .. })
    }

    /// Check if this is an Error action
    pub fn is_error(&self) -> bool {
        matches!(self, NextAction::Error { .. })
    }
}

// ============================================================================
// AgentProtocol - Stateless atomic operations
// ============================================================================

/// Stateless agent protocol with atomic operations
///
/// Each method is an "Atom" - a self-contained operation that:
/// 1. Loads required state from the message store
/// 2. Performs its operation
/// 3. Stores results back to the message store
///
/// This design enables:
/// - Temporal workflow integration (each atom = activity)
/// - Custom orchestration logic
/// - Easy testing and debugging
/// - State persistence between steps
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
    M: MessageStore,
    L: LlmProvider,
    T: ToolExecutor,
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

    // ========================================================================
    // Atom: Load Messages
    // ========================================================================

    /// Load all messages for a session
    ///
    /// This is a read-only atom that retrieves the current conversation state.
    pub async fn load_messages(&self, session_id: Uuid) -> Result<LoadMessagesResult> {
        let messages = self.message_store.load(session_id).await?;
        let count = messages.len();
        Ok(LoadMessagesResult { messages, count })
    }

    // ========================================================================
    // Atom: Add User Message
    // ========================================================================

    /// Add a user message to the conversation
    ///
    /// Stores the message and returns it.
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

    // ========================================================================
    // Atom: Call Model
    // ========================================================================

    /// Call the LLM model
    ///
    /// This atom:
    /// 1. Loads messages from the store
    /// 2. Calls the LLM with the messages
    /// 3. Stores the assistant response
    /// 4. Returns the result with tool calls (if any)
    pub async fn call_model(
        &self,
        session_id: Uuid,
        config: &AgentConfig,
    ) -> Result<CallModelResult> {
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
        let llm_config = LlmCallConfig::from(config);
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

    // ========================================================================
    // Atom: Execute Tools
    // ========================================================================

    /// Execute pending tool calls
    ///
    /// This atom:
    /// 1. Executes each tool call
    /// 2. Stores tool result messages
    /// 3. Returns the results
    pub async fn execute_tools(
        &self,
        session_id: Uuid,
        tool_calls: &[ToolCall],
        tool_definitions: &[ToolDefinition],
    ) -> Result<ExecuteToolsResult> {
        let mut results = Vec::with_capacity(tool_calls.len());
        let mut messages = Vec::with_capacity(tool_calls.len());

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

        for tool_call in tool_calls {
            // Find tool definition
            let tool_def = tool_map.get(tool_call.name.as_str()).ok_or_else(|| {
                AgentLoopError::tool(format!("Tool not found: {}", tool_call.name))
            })?;

            // Execute tool
            let result = self.tool_executor.execute(tool_call, tool_def).await?;

            // Store tool result message
            let result_msg = ConversationMessage::tool_result(
                &tool_call.id,
                result.result.clone(),
                result.error.clone(),
            );
            self.message_store
                .store(session_id, result_msg.clone())
                .await?;

            messages.push(result_msg);
            results.push(result);
        }

        Ok(ExecuteToolsResult { results, messages })
    }

    // ========================================================================
    // Atom: Determine Next Action
    // ========================================================================

    /// Determine what action should be taken next
    ///
    /// This is a read-only atom that examines the current state and determines
    /// what the orchestrator should do next.
    pub async fn determine_next_action(&self, session_id: Uuid) -> Result<NextAction> {
        let messages = self.message_store.load(session_id).await?;

        if messages.is_empty() {
            return Ok(NextAction::CallModel);
        }

        // Find the last meaningful message
        let last_msg = messages.last().unwrap();

        match last_msg.role {
            MessageRole::User => {
                // User message: call model
                Ok(NextAction::CallModel)
            }
            MessageRole::ToolResult => {
                // Tool result: call model to continue
                Ok(NextAction::CallModel)
            }
            MessageRole::Assistant => {
                // Check if assistant requested tool calls
                if let Some(ref tool_calls) = last_msg.tool_calls {
                    if !tool_calls.is_empty() {
                        // Check if we already have results for these tool calls
                        let has_all_results = tool_calls.iter().all(|tc| {
                            messages.iter().any(|m| {
                                m.role == MessageRole::ToolResult
                                    && m.tool_call_id.as_ref() == Some(&tc.id)
                            })
                        });

                        if has_all_results {
                            // All tools executed, call model
                            Ok(NextAction::CallModel)
                        } else {
                            // Need to execute tools
                            Ok(NextAction::ExecuteTools {
                                tool_calls: tool_calls.clone(),
                            })
                        }
                    } else {
                        // No tool calls, session complete
                        Ok(NextAction::Complete {
                            final_response: last_msg.text().map(|s| s.to_string()),
                        })
                    }
                } else {
                    // No tool calls, session complete
                    Ok(NextAction::Complete {
                        final_response: last_msg.text().map(|s| s.to_string()),
                    })
                }
            }
            MessageRole::ToolCall => {
                // Shouldn't happen normally, but find pending tool calls
                let pending: Vec<_> = messages
                    .iter()
                    .filter_map(|m| {
                        if m.role == MessageRole::ToolCall {
                            if let crate::message::MessageContent::ToolCall {
                                id,
                                name,
                                arguments,
                            } = &m.content
                            {
                                // Check if there's a result for this
                                let has_result = messages.iter().any(|r| {
                                    r.role == MessageRole::ToolResult
                                        && r.tool_call_id.as_ref() == Some(id)
                                });
                                if !has_result {
                                    return Some(ToolCall {
                                        id: id.clone(),
                                        name: name.clone(),
                                        arguments: arguments.clone(),
                                    });
                                }
                            }
                        }
                        None
                    })
                    .collect();

                if pending.is_empty() {
                    Ok(NextAction::CallModel)
                } else {
                    Ok(NextAction::ExecuteTools {
                        tool_calls: pending,
                    })
                }
            }
            MessageRole::System => {
                // Just system message, need user input
                Ok(NextAction::CallModel)
            }
        }
    }

    // ========================================================================
    // High-level: Run Turn
    // ========================================================================

    /// Run a complete turn (user message → final response)
    ///
    /// This is a convenience method that orchestrates atoms to run a complete
    /// turn. For more control, use the individual atoms directly.
    pub async fn run_turn(
        &self,
        session_id: Uuid,
        user_message: impl Into<String>,
        config: &AgentConfig,
        max_iterations: usize,
    ) -> Result<String> {
        // Add user message
        self.add_user_message(session_id, user_message).await?;

        let mut iteration = 0;
        let mut final_response = String::new();

        loop {
            iteration += 1;
            if iteration > max_iterations {
                return Err(AgentLoopError::MaxIterationsReached(max_iterations));
            }

            // Determine next action
            let action = self.determine_next_action(session_id).await?;

            match action {
                NextAction::CallModel => {
                    let result = self.call_model(session_id, config).await?;
                    if !result.text.is_empty() {
                        final_response = result.text;
                    }
                    if !result.needs_tool_execution {
                        // No tool calls, we're done
                        break;
                    }
                    // Continue loop to execute tools
                }
                NextAction::ExecuteTools { tool_calls } => {
                    self.execute_tools(session_id, &tool_calls, &config.tools)
                        .await?;
                    // Continue loop to call model with results
                }
                NextAction::Complete {
                    final_response: response,
                } => {
                    if let Some(resp) = response {
                        final_response = resp;
                    }
                    break;
                }
                NextAction::Error { message } => {
                    return Err(AgentLoopError::Internal(anyhow::anyhow!(message)));
                }
            }
        }

        Ok(final_response)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_action_variants() {
        let action = NextAction::CallModel;
        assert!(action.is_call_model());

        let action = NextAction::ExecuteTools {
            tool_calls: vec![],
        };
        assert!(action.is_execute_tools());

        let action = NextAction::Complete {
            final_response: Some("done".to_string()),
        };
        assert!(action.is_complete());
        if let NextAction::Complete { final_response } = action {
            assert_eq!(final_response, Some("done".to_string()));
        }

        let action = NextAction::Error {
            message: "test".to_string(),
        };
        assert!(action.is_error());
    }

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
