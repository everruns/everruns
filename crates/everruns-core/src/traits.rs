// Core traits for pluggable backends
//
// These traits allow the agent loop to be used with different backends:
// - In-memory implementations for examples and testing
// - Database implementations for production
// - Channel-based implementations for streaming

use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;
use uuid::Uuid;

use crate::error::Result;
use crate::events::LoopEvent;
use crate::message::Message;

// ============================================================================
// EventEmitter - For streaming events during execution
// ============================================================================

/// Trait for emitting events during loop execution
///
/// Implementations can:
/// - Store events in a database
/// - Send events to a channel for SSE streaming
/// - Collect events in memory for testing
/// - Do nothing (no-op implementation)
#[async_trait]
pub trait EventEmitter: Send + Sync {
    /// Emit a single event
    async fn emit(&self, event: LoopEvent) -> Result<()>;

    /// Emit multiple events
    async fn emit_batch(&self, events: Vec<LoopEvent>) -> Result<()> {
        for event in events {
            self.emit(event).await?;
        }
        Ok(())
    }
}

// ============================================================================
// MessageStore - For persisting conversation messages
// ============================================================================

/// Trait for storing and retrieving conversation messages
///
/// Implementations can:
/// - Store messages in a database
/// - Keep messages in memory for testing
/// - Store messages in a file
#[async_trait]
pub trait MessageStore: Send + Sync {
    /// Store a message
    async fn store(&self, session_id: Uuid, message: Message) -> Result<()>;

    /// Store multiple messages
    async fn store_batch(&self, session_id: Uuid, messages: Vec<Message>) -> Result<()> {
        for message in messages {
            self.store(session_id, message).await?;
        }
        Ok(())
    }

    /// Load all messages for a session
    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>>;

    /// Load messages with pagination
    async fn load_page(
        &self,
        session_id: Uuid,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let all = self.load(session_id).await?;
        Ok(all.into_iter().skip(offset).take(limit).collect())
    }

    /// Count messages in a session
    async fn count(&self, session_id: Uuid) -> Result<usize> {
        Ok(self.load(session_id).await?.len())
    }
}

// ============================================================================
// ToolExecutor - For executing tool calls
// ============================================================================

/// Trait for executing tool calls
///
/// Implementations handle the actual tool execution:
/// - Webhook calls
/// - Built-in function execution
/// - Mock execution for testing
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single tool call
    async fn execute(&self, tool_call: &ToolCall, tool_def: &ToolDefinition) -> Result<ToolResult>;

    /// Execute multiple tool calls (default: sequential)
    async fn execute_batch(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        let mut results = Vec::with_capacity(tool_calls.len());

        // Build a map of tool names to definitions
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_defs
            .iter()
            .map(|def| {
                let name = match def {
                    ToolDefinition::Builtin(b) => b.name.as_str(),
                };
                (name, def)
            })
            .collect();

        for tool_call in tool_calls {
            let tool_def = tool_map.get(tool_call.name.as_str()).ok_or_else(|| {
                crate::error::AgentLoopError::tool(format!(
                    "Tool definition not found: {}",
                    tool_call.name
                ))
            })?;

            results.push(self.execute(tool_call, tool_def).await?);
        }

        Ok(results)
    }

    /// Execute multiple tool calls in parallel
    async fn execute_parallel(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>>
    where
        Self: Sized,
    {
        use futures::future::join_all;

        // Build a map of tool names to definitions
        let tool_map: std::collections::HashMap<&str, &ToolDefinition> = tool_defs
            .iter()
            .map(|def| {
                let name = match def {
                    ToolDefinition::Builtin(b) => b.name.as_str(),
                };
                (name, def)
            })
            .collect();

        let futures: Vec<_> = tool_calls
            .iter()
            .map(|tool_call| async {
                let tool_def = tool_map.get(tool_call.name.as_str()).ok_or_else(|| {
                    crate::error::AgentLoopError::tool(format!(
                        "Tool definition not found: {}",
                        tool_call.name
                    ))
                })?;
                self.execute(tool_call, tool_def).await
            })
            .collect();

        let results = join_all(futures).await;
        results.into_iter().collect()
    }
}
