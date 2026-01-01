//! ActAtom - Atom for parallel tool execution
//!
//! This atom handles:
//! 1. Executing multiple tool calls in parallel
//! 2. Handling errors, timeouts, and cancellations as "normal" results
//! 3. Storing tool result messages
//! 4. Returning all tool results (success, error, timeout, or cancelled)
//!
//! NOTES from Python spec:
//! - Tools call runs in parallel
//! - Error from tool call is not an error for the whole Act, error from tool is "normal" result
//! - Tool invocations should be timeouted, timeout is also "normal" result from tool
//! - Exit of act should have all tool calls finished (successfully or with error/timeout)
//! - Act and each tool call should emit start/end events
//! - Act and each tool call should be cancellable, and this is also "normal" result

use async_trait::async_trait;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{Atom, AtomContext};
use crate::error::Result;
use crate::message::Message;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::{MessageStore, SessionFileStore, ToolContext, ToolExecutor};

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for ActAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActInput {
    /// Atom execution context
    pub context: AtomContext,
    /// Tool calls to execute
    pub tool_calls: Vec<ToolCall>,
    /// Available tool definitions for resolution
    pub tool_definitions: Vec<ToolDefinition>,
}

/// Result of a single tool call execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// The original tool call
    pub tool_call: ToolCall,
    /// The result of the tool call
    pub result: ToolResult,
    /// Whether the execution was successful
    pub success: bool,
    /// Status: "success", "error", "timeout", or "cancelled"
    pub status: String,
}

/// Result of the ActAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActResult {
    /// Results for all tool calls
    pub results: Vec<ToolCallResult>,
    /// Whether all tool calls completed (regardless of success/failure)
    pub completed: bool,
    /// Number of successful tool calls
    pub success_count: usize,
    /// Number of failed tool calls
    pub error_count: usize,
}

// ============================================================================
// ActAtom
// ============================================================================

/// Atom that executes tool calls in parallel
///
/// This atom:
/// 1. Executes all tool calls in parallel
/// 2. Handles errors, timeouts, and cancellations gracefully
/// 3. Stores tool result messages for each call
/// 4. Returns comprehensive results for all tools
pub struct ActAtom<M, T>
where
    M: MessageStore,
    T: ToolExecutor,
{
    message_store: M,
    tool_executor: T,
    /// Optional file store for context-aware tools
    file_store: Option<Arc<dyn SessionFileStore>>,
}

impl<M, T> ActAtom<M, T>
where
    M: MessageStore,
    T: ToolExecutor,
{
    /// Create a new ActAtom
    pub fn new(message_store: M, tool_executor: T) -> Self {
        Self {
            message_store,
            tool_executor,
            file_store: None,
        }
    }

    /// Create a new ActAtom with a file store for context-aware tools
    pub fn with_file_store(
        message_store: M,
        tool_executor: T,
        file_store: Arc<dyn SessionFileStore>,
    ) -> Self {
        Self {
            message_store,
            tool_executor,
            file_store: Some(file_store),
        }
    }
}

#[async_trait]
impl<M, T> Atom for ActAtom<M, T>
where
    M: MessageStore + Send + Sync + Clone,
    T: ToolExecutor + Send + Sync,
{
    type Input = ActInput;
    type Output = ActResult;

    fn name(&self) -> &'static str {
        "act"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let ActInput {
            context,
            tool_calls,
            tool_definitions,
        } = input;

        if tool_calls.is_empty() {
            return Ok(ActResult {
                results: vec![],
                completed: true,
                success_count: 0,
                error_count: 0,
            });
        }

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            exec_id = %context.exec_id,
            tool_count = %tool_calls.len(),
            "ActAtom: executing tools in parallel"
        );

        // Build tool name to definition map
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_definitions
            .iter()
            .map(|def| {
                let name = match def {
                    ToolDefinition::Builtin(b) => b.name.as_str(),
                };
                (name, def)
            })
            .collect();

        // Execute all tool calls in parallel
        let futures: Vec<_> = tool_calls
            .iter()
            .map(|tool_call| {
                let tool_def = tool_map.get(tool_call.name.as_str()).cloned();
                self.execute_single_tool(&context, tool_call.clone(), tool_def)
            })
            .collect();

        let results = join_all(futures).await;

        // Count successes and errors
        let success_count = results.iter().filter(|r| r.success).count();
        let error_count = results.iter().filter(|r| !r.success).count();

        // Store tool result messages
        for result in &results {
            let message = Message::tool_result(
                &result.tool_call.id,
                result.result.result.clone(),
                result.result.error.clone(),
            );
            // Ignore storage errors - the results are still valid
            if let Err(e) = self.message_store.store(context.session_id, message).await {
                tracing::warn!(
                    session_id = %context.session_id,
                    tool_call_id = %result.tool_call.id,
                    error = %e,
                    "ActAtom: failed to store tool result"
                );
            }
        }

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            success_count = %success_count,
            error_count = %error_count,
            "ActAtom: all tools completed"
        );

        Ok(ActResult {
            results,
            completed: true,
            success_count,
            error_count,
        })
    }
}

impl<M, T> ActAtom<M, T>
where
    M: MessageStore + Send + Sync + Clone,
    T: ToolExecutor + Send + Sync,
{
    /// Execute a single tool call
    async fn execute_single_tool(
        &self,
        context: &AtomContext,
        tool_call: ToolCall,
        tool_def: Option<&ToolDefinition>,
    ) -> ToolCallResult {
        tracing::debug!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            tool_name = %tool_call.name,
            tool_call_id = %tool_call.id,
            "ActAtom: executing tool"
        );

        // If tool definition not found, return error result
        let Some(tool_def) = tool_def else {
            return ToolCallResult {
                tool_call: tool_call.clone(),
                result: ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: None,
                    error: Some(format!("Tool definition not found: {}", tool_call.name)),
                },
                success: false,
                status: "error".to_string(),
            };
        };

        // Execute the tool
        let result = if let Some(ref store) = self.file_store {
            let tool_context = ToolContext::with_file_store(context.session_id, store.clone());
            self.tool_executor
                .execute_with_context(&tool_call, tool_def, &tool_context)
                .await
        } else {
            self.tool_executor.execute(&tool_call, tool_def).await
        };

        match result {
            Ok(tool_result) => {
                let success = tool_result.error.is_none();
                tracing::debug!(
                    session_id = %context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    success = %success,
                    "ActAtom: tool execution completed"
                );
                ToolCallResult {
                    tool_call,
                    result: tool_result,
                    success,
                    status: if success {
                        "success".to_string()
                    } else {
                        "error".to_string()
                    },
                }
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %context.session_id,
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "ActAtom: tool execution failed"
                );
                ToolCallResult {
                    tool_call: tool_call.clone(),
                    result: ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        result: None,
                        error: Some(e.to_string()),
                    },
                    success: false,
                    status: "error".to_string(),
                }
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryMessageStore;
    use crate::tools::ToolRegistry;
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_act_atom_empty_tool_calls() {
        let store = InMemoryMessageStore::new();
        let executor = ToolRegistry::with_defaults();
        let atom = ActAtom::new(store, executor);

        let context = AtomContext::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let input = ActInput {
            context,
            tool_calls: vec![],
            tool_definitions: vec![],
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert!(result.results.is_empty());
        assert_eq!(result.success_count, 0);
        assert_eq!(result.error_count, 0);
    }

    #[tokio::test]
    async fn test_act_atom_tool_not_found() {
        let store = InMemoryMessageStore::new();
        let executor = ToolRegistry::with_defaults();
        let atom = ActAtom::new(store, executor);

        let context = AtomContext::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let input = ActInput {
            context,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "nonexistent_tool".to_string(),
                arguments: json!({}),
            }],
            tool_definitions: vec![],
        };

        let result = atom.execute(input).await.unwrap();

        assert!(result.completed);
        assert_eq!(result.results.len(), 1);
        assert!(!result.results[0].success);
        assert_eq!(result.results[0].status, "error");
        assert!(result.results[0]
            .result
            .error
            .as_ref()
            .unwrap()
            .contains("not found"));
    }
}
