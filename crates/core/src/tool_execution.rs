//! Neutral contracts for tool execution and tool-scoped authorities.

use crate::error::Result;
use crate::tool_context::ToolContext;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use std::collections::HashMap;

fn build_tool_map(tool_defs: &[ToolDefinition]) -> HashMap<&str, &ToolDefinition> {
    tool_defs.iter().map(|def| (def.name(), def)).collect()
}

/// Trait for executing tool calls
///
/// Implementations handle the actual tool execution:
/// - Webhook calls
/// - Built-in function execution
/// - Mock execution for testing
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a single tool call (without context)
    ///
    /// This is the legacy method that doesn't provide context to tools.
    /// Use `execute_with_context` when context is available.
    async fn execute(&self, tool_call: &ToolCall, tool_def: &ToolDefinition) -> Result<ToolResult>;

    /// Execute a single tool call with context
    ///
    /// This method provides runtime context to tools that need it (like filesystem tools).
    /// The default implementation delegates to `execute()`.
    async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> Result<ToolResult> {
        // Default: delegate to execute(), ignoring context
        self.execute(tool_call, tool_def).await
    }

    /// Execute multiple tool calls (default: sequential)
    async fn execute_batch(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        let mut results = Vec::with_capacity(tool_calls.len());

        let tool_map = build_tool_map(tool_defs);

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

        let tool_map = build_tool_map(tool_defs);

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

/// Delegating impl so callers can hold a `ToolExecutor` as a trait object
/// (e.g. to choose between a plain registry and an MCP-routing composite at
/// runtime without monomorphizing the consumer).
#[async_trait]
impl ToolExecutor for std::sync::Arc<dyn ToolExecutor> {
    async fn execute(&self, tool_call: &ToolCall, tool_def: &ToolDefinition) -> Result<ToolResult> {
        (**self).execute(tool_call, tool_def).await
    }

    async fn execute_with_context(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> Result<ToolResult> {
        (**self)
            .execute_with_context(tool_call, tool_def, context)
            .await
    }

    async fn execute_batch(
        &self,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        (**self).execute_batch(tool_calls, tool_defs).await
    }
}

/// Trait for checking budget status from within tool execution.
///
/// Implemented by gRPC adapters (worker → server) and direct adapters (in-process).
/// Used by the `check_budget` tool to return real budget data to agents.
/// The org_id is captured at construction time by the implementing adapter.
#[async_trait]
pub trait BudgetChecker: Send + Sync {
    /// Check all budgets for a session and return a tool-friendly response.
    async fn check_budgets(&self, session_id: &str) -> Result<crate::budget::BudgetToolResponse>;
}

// ============================================================================
// PaymentAuthority - For capability-internal machine payments
// ============================================================================

/// Internal authority for paid capability operations.
///
/// Capabilities call this with fixed, typed requests. The model never receives a
/// generic paid HTTP tool, wallet credentials, or payment payloads.
#[async_trait]
pub trait PaymentAuthority: Send + Sync {
    async fn execute_machine_payment(
        &self,
        session_id: SessionId,
        request: crate::payment::MachinePaymentRequest,
    ) -> Result<crate::payment::MachinePaymentResponse>;
}

/// Per-org gate on outbound tool execution.
///
/// Returns `true` if the call is within the per-org budget, `false` if the
/// org has exceeded its outbound tool rate limit for this window.
/// Implementations must be fail-open: Valkey/backend errors should return `true`
/// rather than blocking legitimate tool calls.
#[async_trait]
pub trait OutboundToolRateLimiter: Send + Sync {
    /// Key by the public org UUID (keyed string representation).
    async fn check_org(&self, org_id: &crate::typed_id::OrgId) -> bool;
}
