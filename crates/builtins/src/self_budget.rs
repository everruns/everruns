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
// See knowledge/security/budgeting.md (Self-Managed vs Platform-Enforced Budgets).

use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};

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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Самокерований бюджет",
            "Лише промптові настанови для міркування про орієнтовний бюджет, заданий користувачем. Агент самостійно керує цільовим показником на основі даних про використання сесії; жодних інструментів не додається, і платформа нічого примусово не обмежує. Використовуйте разом із `budgeting` для авторитетних платформних бюджетів.",
        )]
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

const SELF_BUDGET_SYSTEM_PROMPT: &str = "User-stated budgets are agent-managed soft targets, not platform-enforced limits. Track spend with `get_session_info` around expensive phases, qualify estimates when pricing is partial, and tighten scope/output as the target nears. Do not create, modify, delete, or report them as platform budgets; if close to exhausted, inform the user and continue with a scoped-down path.";

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = SelfBudgetCapability;
        let prompt = cap.system_prompt_addition().expect("prompt present");
        assert!(prompt.contains("agent-managed soft targets"));
        assert!(prompt.contains("get_session_info"));
        assert!(prompt.contains("platform-enforced limits"));
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
