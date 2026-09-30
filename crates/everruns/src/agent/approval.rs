use std::collections::HashMap;
use std::sync::Arc;

use super::AgentBuilder;

impl AgentBuilder {
    /// Answer approval requests for tools marked with
    /// [`FunctionTool::needs_approval`](crate::FunctionTool::needs_approval).
    ///
    /// The approver runs inside the turn: the gated call waits for its
    /// [`ApprovalDecision`](crate::approval::ApprovalDecision). Setting an
    /// approver while no tool needs approval registers nothing. Calling this
    /// again replaces the earlier approver.
    ///
    /// Stability: alpha — may change without a major bump; see
    /// [`stability`](crate::stability).
    ///
    /// # Example
    ///
    /// ```
    /// use everruns::approval::{ApprovalDecision, ToolApprover, async_trait};
    /// use everruns::{Agent, FunctionTool, Model, SessionId, ToolCall, ToolDefinition};
    /// use serde_json::{Value, json};
    ///
    /// struct DenyDeletes;
    ///
    /// #[async_trait]
    /// impl ToolApprover for DenyDeletes {
    ///     async fn approve(
    ///         &self,
    ///         _session_id: SessionId,
    ///         call: &ToolCall,
    ///         _definition: &ToolDefinition,
    ///     ) -> ApprovalDecision {
    ///         if call.arguments["path"].as_str() == Some("/") {
    ///             ApprovalDecision::Reject
    ///         } else {
    ///             ApprovalDecision::Allow
    ///         }
    ///     }
    /// }
    ///
    /// let delete = FunctionTool::new(
    ///     "delete",
    ///     "Delete a path.",
    ///     json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
    ///     |_args: Value| async move { Ok::<_, String>(json!({ "deleted": true })) },
    /// )
    /// .always_needs_approval();
    ///
    /// let agent = Agent::builder()
    ///     .instructions("Clean up when asked.")
    ///     .model(Model::simulated("Done."))
    ///     .tool(delete)
    ///     .approver(DenyDeletes)
    ///     .build()?;
    /// # let _ = agent;
    /// # Ok::<(), everruns::BuildError>(())
    /// ```
    pub fn approver(mut self, approver: impl crate::approval::ToolApprover + 'static) -> Self {
        self.approver = Some(Arc::new(approver));
        self
    }
}

/// Build the approval gate for the function tools that asked for one.
///
/// The policy consults only the tools in `predicates`, by name, so built-in and
/// capability tools are never gated by it.
pub(super) fn approval_capability(
    approver: Arc<dyn everruns_builtins::ToolApprover>,
    predicates: HashMap<String, crate::tool::ApprovalPredicate>,
) -> everruns_builtins::ToolApprovalCapability {
    let policy: everruns_builtins::ToolApprovalPolicy = Arc::new(
        move |call: &crate::ToolCall, _definition: &crate::ToolDefinition| {
            predicates
                .get(&call.name)
                .is_some_and(|predicate| predicate(&call.arguments))
        },
    );
    everruns_builtins::ToolApprovalCapability::new(approver).with_policy(policy)
}
