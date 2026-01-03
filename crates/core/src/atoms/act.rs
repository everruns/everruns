//! ActAtom - Atom for parallel tool execution
//!
//! This atom handles:
//! 1. Emitting act.started event
//! 2. Executing multiple tool calls in parallel (with tool.call_started/completed events)
//! 3. Handling errors, timeouts, and cancellations as "normal" results
//! 4. Emitting act.completed event
//! 5. Returning all tool results (success, error, timeout, or cancelled)
//!
//! Tool results are emitted as `tool.call_completed` events and returned in ActResult.
//! Messages are derived from events - no separate message storage is needed.
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
use crate::events::{
    ActCompletedData, ActStartedData, EventContext, EventRequest, ToolCallCompletedData,
    ToolCallStartedData,
};
use crate::message::ContentPart;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::{EventEmitter, SessionFileStore, ToolContext, ToolExecutor};

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
/// 1. Emits act.started event
/// 2. Executes all tool calls in parallel (emitting tool.call_started/completed for each)
/// 3. Handles errors, timeouts, and cancellations gracefully
/// 4. Emits act.completed event
/// 5. Returns comprehensive results for all tools
///
/// Tool results are emitted as events and returned in ActResult.
/// Messages are derived from events by the message store.
pub struct ActAtom<T, E>
where
    T: ToolExecutor,
    E: EventEmitter,
{
    tool_executor: T,
    event_emitter: E,
    /// Optional file store for context-aware tools
    file_store: Option<Arc<dyn SessionFileStore>>,
}

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor,
    E: EventEmitter,
{
    /// Create a new ActAtom
    pub fn new(tool_executor: T, event_emitter: E) -> Self {
        Self {
            tool_executor,
            event_emitter,
            file_store: None,
        }
    }

    /// Create a new ActAtom with a file store for context-aware tools
    pub fn with_file_store(
        tool_executor: T,
        event_emitter: E,
        file_store: Arc<dyn SessionFileStore>,
    ) -> Self {
        Self {
            tool_executor,
            event_emitter,
            file_store: Some(file_store),
        }
    }
}

#[async_trait]
impl<T, E> Atom for ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync,
    E: EventEmitter + Send + Sync,
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

        // Create event context from atom context
        let event_context = EventContext::from_atom_context(&context);

        // Emit act.started event
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context.clone(),
                ActStartedData::new(&tool_calls),
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ActAtom: failed to emit act.started event"
            );
        }

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

        // Emit act.completed event
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context,
                ActCompletedData {
                    completed: true,
                    success_count,
                    error_count,
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ActAtom: failed to emit act.completed event"
            );
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

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync,
    E: EventEmitter + Send + Sync,
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

        // Create event context from atom context
        let event_context = EventContext::from_atom_context(context);

        // Emit tool.call_started event
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context.clone(),
                ToolCallStartedData {
                    tool_call: tool_call.clone(),
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                tool_call_id = %tool_call.id,
                error = %e,
                "ActAtom: failed to emit tool.call_started event"
            );
        }

        // If tool definition not found, return error result
        let Some(tool_def) = tool_def else {
            let error_msg = format!("Tool definition not found: {}", tool_call.name);

            // Emit tool.call_completed event for error
            if let Err(e) = self
                .event_emitter
                .emit(EventRequest::new(
                    context.session_id,
                    event_context,
                    ToolCallCompletedData::failure(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        "error".to_string(),
                        error_msg.clone(),
                    ),
                ))
                .await
            {
                tracing::warn!(
                    session_id = %context.session_id,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "ActAtom: failed to emit tool.call_completed event"
                );
            }

            return ToolCallResult {
                tool_call: tool_call.clone(),
                result: ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: None,
                    error: Some(error_msg),
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

        let tool_call_result = match result {
            Ok(tool_result) => {
                let success = tool_result.error.is_none();
                let status = if success { "success" } else { "error" };

                // Emit tool.call_completed event
                let completed_data = if success {
                    // Convert result to ContentPart (text representation of JSON)
                    let result_content = tool_result
                        .result
                        .as_ref()
                        .map(|r| vec![ContentPart::text(r.to_string())])
                        .unwrap_or_default();
                    ToolCallCompletedData::success(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        result_content,
                    )
                } else {
                    ToolCallCompletedData::failure(
                        tool_call.id.clone(),
                        tool_call.name.clone(),
                        status.to_string(),
                        tool_result.error.clone().unwrap_or_default(),
                    )
                };

                if let Err(e) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context.clone(),
                        completed_data,
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        tool_call_id = %tool_call.id,
                        error = %e,
                        "ActAtom: failed to emit tool.call_completed event"
                    );
                }

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
                    status: status.to_string(),
                }
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Emit tool.call_completed event for error
                if let Err(emit_err) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context,
                        ToolCallCompletedData::failure(
                            tool_call.id.clone(),
                            tool_call.name.clone(),
                            "error".to_string(),
                            error_msg.clone(),
                        ),
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        tool_call_id = %tool_call.id,
                        error = %emit_err,
                        "ActAtom: failed to emit tool.call_completed event"
                    );
                }

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
                        error: Some(error_msg),
                    },
                    success: false,
                    status: "error".to_string(),
                }
            }
        };

        tool_call_result
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use crate::traits::NoopEventEmitter;
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_act_atom_empty_tool_calls() {
        let executor = ToolRegistry::with_defaults();
        let event_emitter = NoopEventEmitter;
        let atom = ActAtom::new(executor, event_emitter);

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
        let executor = ToolRegistry::with_defaults();
        let event_emitter = NoopEventEmitter;
        let atom = ActAtom::new(executor, event_emitter);

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
