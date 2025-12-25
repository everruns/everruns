//! ExecuteToolAtom - Atom for executing a single tool

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Atom;
use crate::error::{AgentLoopError, Result};
use crate::message::Message;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::{MessageStore, ToolContext, ToolExecutor};

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for ExecuteToolAtom (single tool)
#[derive(Debug, Clone)]
pub struct ExecuteToolInput {
    /// Session ID
    pub session_id: Uuid,
    /// Tool call to execute
    pub tool_call: ToolCall,
    /// Available tool definitions for resolution
    pub tool_definitions: Vec<ToolDefinition>,
    /// Optional tool context for context-aware tools (like filesystem tools)
    pub tool_context: Option<ToolContext>,
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
// ExecuteToolAtom
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
            tool_context,
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

        // Execute tool - use context if provided, otherwise fall back to basic execute
        let result = if let Some(context) = tool_context {
            self.tool_executor
                .execute_with_context(&tool_call, &tool_definition, &context)
                .await?
        } else {
            self.tool_executor
                .execute(&tool_call, &tool_definition)
                .await?
        };

        // Store tool result message
        let message =
            Message::tool_result(&tool_call.id, result.result.clone(), result.error.clone());
        self.message_store
            .store(session_id, message.clone())
            .await?;

        Ok(ExecuteToolResult { result, message })
    }
}
