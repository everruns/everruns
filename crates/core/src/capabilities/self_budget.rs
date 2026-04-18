// Self-Budget Capability
//
// Prompt-only capability that teaches agents how to reason about a user-requested
// *indicative* budget ("you have $7") using session usage data. Distinct from the
// `budgeting` capability, which wires platform-enforced budgets and the
// `check_budget` tool. `self_budget` ships NO tools — it relies on
// `get_session_info` (already provided by the `session` capability in the generic
// harness) for cumulative usage and on the agent's own judgment about when and
// how to adapt behavior.
//
// See specs/budgeting.md (Self-Managed vs Platform-Enforced Budgets).

use super::{Capability, CapabilityStatus};

pub const SELF_BUDGET_CAPABILITY_ID: &str = "self_budget";

/// Self-budget capability — prompt-only guidance for agent-managed indicative budgets.
pub struct SelfBudgetCapability;

impl Capability for SelfBudgetCapability {
    fn id(&self) -> &str {
        SELF_BUDGET_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Self-Budget"
    }

    fn description(&self) -> &str {
        "Prompt-only guidance for reasoning about a user-requested indicative budget. \
         The agent self-manages the target using session usage data; no tools are added \
         and no platform enforcement is performed. Use alongside `budgeting` for \
         authoritative platform budgets."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("gauge")
    }

    fn category(&self) -> Option<&str> {
        Some("System")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SELF_BUDGET_SYSTEM_PROMPT)
    }

    fn features(&self) -> Vec<&'static str> {
        vec![]
    }
}

const SELF_BUDGET_SYSTEM_PROMPT: &str = "\
## Self-Managed Budget

If the user gives you an indicative budget (e.g. \"you have $7\" or \"keep this under 20k tokens\"), \
treat it as an **agent-managed soft target**, not as a platform-enforced limit.

**This is different from platform budgets.** The Everruns platform may also enforce \
session-level budgets — those are authoritative and will pause or stop the session \
automatically. A self-managed budget is a goal *you* are trying to respect on the \
user's behalf. Do not conflate the two when reporting progress or remaining spend.

**How to track it:**

- Use `get_session_info` to read cumulative session usage (tokens, and cost where \
  available) as the source of truth for current spend.
- Decide for yourself when to start tracking, when to re-check, and when to stop. \
  You do not need to check on every turn — re-check around expensive or long-running \
  work, or before kicking off a new phase.
- Avoid claiming exact cost certainty when only token counts or partial pricing \
  information are available. Qualify estimates (\"roughly\", \"on the order of\").

**How to adapt as the budget tightens:**

- Prefer shorter, more direct outputs over verbose narration.
- Reduce retries and exploratory tool calls; commit to a plan sooner.
- Narrow the scope — finish the core request well rather than covering every adjacent \
  concern.
- Skip redundant confirmations and intermediate summaries unless the user asked for them.

**What not to do:**

- Do not create, modify, or delete platform budgets in response to a self-managed \
  target. Self-managed budgets live only in this conversation.
- Do not refuse work solely because a self-managed target is close to exhausted; \
  inform the user and offer a scoped-down option instead.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = SelfBudgetCapability;
        assert_eq!(cap.id(), "self_budget");
        assert_eq!(cap.name(), "Self-Budget");
        assert_eq!(cap.icon(), Some("gauge"));
        assert_eq!(cap.category(), Some("System"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_has_no_tools() {
        let cap = SelfBudgetCapability;
        assert!(cap.tools().is_empty());
        assert!(cap.tool_definitions().is_empty());
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SelfBudgetCapability;
        let prompt = cap.system_prompt_addition().expect("prompt present");
        assert!(prompt.contains("Self-Managed Budget"));
        assert!(prompt.contains("agent-managed soft target"));
        assert!(prompt.contains("get_session_info"));
        assert!(prompt.contains("different from platform budgets"));
    }

    #[test]
    fn test_capability_has_no_features() {
        let cap = SelfBudgetCapability;
        assert!(cap.features().is_empty());
    }

    #[test]
    fn test_prompt_distinguishes_from_budgeting() {
        let cap = SelfBudgetCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        // Must NOT direct the agent to the platform-budget tool
        assert!(!prompt.contains("check_budget"));
        // Must NOT direct the agent to mutate budgets
        assert!(!prompt.to_lowercase().contains("create budget"));
    }
}
