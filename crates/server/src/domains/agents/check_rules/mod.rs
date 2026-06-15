// Org-configurable agent check rules (specs/agent-checks.md, phase 4):
// admins enable/disable or re-severity built-in rules and add custom rules
// (declarative regex or natural-language rubric).

pub mod commands;
pub mod types;

pub use types::EffectiveRuleConfig;

use everruns_core::{Permission, Policy, Rule};

/// Managing org check rules is org-wide quality configuration, gated with
/// other org settings rather than per-agent management.
pub const AGENT_CHECKS_MANAGE: Policy = Policy {
    id: "agent_checks.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

/// Load and parse an org's effective rule configuration. Returns an empty
/// config (no overrides, no custom rules) on any storage error — checks are
/// advisory, so a config-load failure must never break preview/analyze.
pub async fn load_effective_config(
    db: &crate::storage::StorageBackend,
    org_id: i64,
) -> EffectiveRuleConfig {
    match db.list_agent_check_rules(org_id).await {
        Ok(rows) => EffectiveRuleConfig::from_rows(&rows),
        Err(e) => {
            tracing::warn!(error = %e, org_id, "failed to load agent check rules; using defaults");
            EffectiveRuleConfig::default()
        }
    }
}
