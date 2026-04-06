// Budgeting Capability
//
// Adds budget awareness to agents:
// 1. System prompt section informing the agent about budget constraints
// 2. A check_budget tool to query remaining balance
//
// The actual budget data is injected into the system prompt at runtime
// by the capability's system_prompt_contribution() method, which the
// service layer calls with budget data from the database.
//
// See specs/budgeting.md (Phase 4: Agent awareness)

use super::{Capability, CapabilityStatus};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::Value;

/// Budgeting capability — budget-aware agent behavior.
pub struct BudgetingCapability;

impl Capability for BudgetingCapability {
    fn id(&self) -> &str {
        "budgeting"
    }

    fn name(&self) -> &str {
        "Budgeting"
    }

    fn description(&self) -> &str {
        "Enables budget awareness. The agent receives information about active budgets \
         and can check remaining balance. When budget is running low, the agent will \
         prioritize completing current tasks efficiently."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("wallet")
    }

    fn category(&self) -> Option<&str> {
        Some("System")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(BUDGET_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(CheckBudgetTool)]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["budgeting"]
    }
}

const BUDGET_SYSTEM_PROMPT: &str = "\
## Budget Awareness

This session has budget constraints. You should be mindful of token usage and cost.

**Guidelines:**
- Use the `check_budget` tool to see remaining budget before starting expensive operations.
- If budget is running low (< 20% remaining), prioritize completing the current task efficiently.
- Avoid unnecessary verbose output when budget is constrained.
- If budget is exhausted, the session will be paused or stopped automatically.

Use the `check_budget` tool to check current budget status at any time.";

// ============================================================================
// Tool: check_budget
// ============================================================================

/// Tool that returns budget status for the current session.
///
/// In practice, the tool result is populated by the worker which has
/// access to the BudgetService. This implementation returns a placeholder
/// that guides the agent to check via the session context.
pub struct CheckBudgetTool;

#[async_trait]
impl Tool for CheckBudgetTool {
    fn name(&self) -> &str {
        "check_budget"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Check Budget")
    }

    fn description(&self) -> &str {
        "Check the remaining budget for this session. Returns budget balance, limit, currency, and status."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        // Fallback when no context is available (shouldn't happen in practice).
        // Returns the same BudgetToolResponse shape for contract stability.
        ToolExecutionResult::success(serde_json::json!({
            "status": "no_budgets",
            "budgets": [],
            "hint": "No budgets are configured for this session. You can proceed without budget constraints."
        }))
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(ref checker) = context.budget_checker else {
            // No budget checker wired — return the no_budgets fallback.
            return self.execute(_arguments).await;
        };

        let session_id = context.session_id.to_string();

        match checker.check_budgets(&session_id).await {
            Ok(response) => {
                ToolExecutionResult::success(serde_json::to_value(&response).unwrap_or_else(
                    |_| serde_json::json!({"status": "no_budgets", "budgets": [], "hint": null}),
                ))
            }
            Err(_) => ToolExecutionResult::tool_error(
                "Budget check is temporarily unavailable. You can proceed normally.",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = BudgetingCapability;
        assert_eq!(cap.id(), "budgeting");
        assert_eq!(cap.name(), "Budgeting");
        assert_eq!(cap.icon(), Some("wallet"));
        assert_eq!(cap.category(), Some("System"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = BudgetingCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "check_budget");
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = BudgetingCapability;
        assert!(cap.system_prompt_addition().is_some());
        assert!(
            cap.system_prompt_addition()
                .unwrap()
                .contains("Budget Awareness")
        );
    }

    #[test]
    fn test_capability_features() {
        let cap = BudgetingCapability;
        assert_eq!(cap.features(), vec!["budgeting"]);
    }

    #[tokio::test]
    async fn test_check_budget_tool_no_budgets_fallback() {
        let tool = CheckBudgetTool;
        // Without context, falls back to no_budgets with stable shape
        let result = tool.execute(serde_json::json!({})).await;
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("status").unwrap().as_str().unwrap(), "no_budgets");
            assert!(value.get("budgets").unwrap().as_array().unwrap().is_empty());
            assert!(value.get("hint").is_some());
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_check_budget_tool_with_context_no_checker() {
        use crate::typed_id::SessionId;
        let tool = CheckBudgetTool;
        // With context but no budget_checker, also falls back
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("status").unwrap().as_str().unwrap(), "no_budgets");
            assert!(value.get("budgets").unwrap().as_array().unwrap().is_empty());
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_check_budget_tool_with_mock_checker() {
        use crate::budget::{BudgetSummary, BudgetToolResponse};
        use crate::traits::BudgetChecker;
        use crate::typed_id::SessionId;
        use std::sync::Arc;

        struct MockBudgetChecker;

        #[async_trait]
        impl BudgetChecker for MockBudgetChecker {
            async fn check_budgets(
                &self,
                _session_id: &str,
            ) -> crate::error::Result<BudgetToolResponse> {
                Ok(BudgetToolResponse {
                    status: "active".into(),
                    budgets: vec![BudgetSummary {
                        currency: "usd".into(),
                        limit: 5.0,
                        balance: 2.56,
                        soft_limit: None,
                        percent_remaining: 51.2,
                        status: "active".into(),
                    }],
                    hint: Some("51.2% of budget remaining.".into()),
                })
            }
        }

        let tool = CheckBudgetTool;
        let mut context = ToolContext::new(SessionId::new());
        context.budget_checker = Some(Arc::new(MockBudgetChecker));

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("status").unwrap().as_str().unwrap(), "active");
            let budgets = value.get("budgets").unwrap().as_array().unwrap();
            assert_eq!(budgets.len(), 1);
            assert_eq!(budgets[0].get("currency").unwrap().as_str().unwrap(), "usd");
            assert_eq!(budgets[0].get("balance").unwrap().as_f64().unwrap(), 2.56);
            assert_eq!(
                budgets[0]
                    .get("percent_remaining")
                    .unwrap()
                    .as_f64()
                    .unwrap(),
                51.2
            );
            assert!(
                value
                    .get("hint")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .contains("51.2%")
            );
        } else {
            panic!("Expected success");
        }
    }
}
