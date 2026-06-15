// Commands for managing org agent check rules (specs/agent-checks.md, phase 4).

use regex::Regex;
use serde::Deserialize;
use utoipa::ToSchema;

use super::AGENT_CHECKS_MANAGE;
use super::types::{CheckRulesResponse, UpsertCheckRuleRequest, build_response};
use crate::domains::agents::checks::builtin_rule_catalog;
use crate::domains::common::*;
use crate::storage::models::UpsertAgentCheckRuleRow;

const VALID_SEVERITIES: &[&str] = &["warning", "info", "suggestion"];
const VALID_CATEGORIES: &[&str] = &[
    "structure",
    "completeness",
    "effectiveness",
    "safety",
    "cost",
];
/// Bound custom rule text fields (TM-DOS).
const MAX_RULE_TEXT: usize = 4_000;

/// List the org's effective check-rule configuration (built-ins + custom).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAgentCheckRules {}

impl Command for ListAgentCheckRules {
    type Output = CheckRulesResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_check_rules",
            category: "agents",
            description: "List the org's agent check rules: built-in rules with their effective \
                          enabled/severity, plus custom rules.",
            method: "GET",
            path: "/v1/agents/check-rules",
        }
    }

    fn read_only() -> bool {
        true
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        // Viewing the config requires the same manage permission; it exposes
        // org-wide quality policy, not per-agent data.
        Some(&AGENT_CHECKS_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<CheckRulesResponse, CommandError> {
        let rows = ctx
            .db
            .list_agent_check_rules(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        Ok(build_response(&rows))
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentCheckRules>() }

/// Create or update a single check rule.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAgentCheckRule {
    /// Built-in rule id (for `builtin_override`) or `custom.<slug>` (custom).
    pub rule_id: String,
    #[serde(flatten)]
    pub req: UpsertCheckRuleRequest,
}

impl Command for UpsertAgentCheckRule {
    type Output = CheckRulesResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "upsert_agent_check_rule",
            category: "agents",
            description: "Create or update an agent check rule (built-in override or custom rule).",
            method: "PUT",
            path: "/v1/agents/check-rules/{rule_id}",
        }
    }

    fn read_only() -> bool {
        false
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_CHECKS_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<CheckRulesResponse, CommandError> {
        let row = build_upsert_row(&self.rule_id, &self.req)?;
        ctx.db
            .upsert_agent_check_rule(ctx.org_id(), row)
            .await
            .map_err(classify_anyhow)?;
        let rows = ctx
            .db
            .list_agent_check_rules(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        Ok(build_response(&rows))
    }
}

inventory::submit! { CommandDescriptor::of::<UpsertAgentCheckRule>() }

/// Delete a check rule (removes a built-in override or a custom rule).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteAgentCheckRule {
    pub rule_id: String,
}

impl Command for DeleteAgentCheckRule {
    type Output = CheckRulesResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_agent_check_rule",
            category: "agents",
            description: "Delete an agent check rule (built-in override or custom rule).",
            method: "DELETE",
            path: "/v1/agents/check-rules/{rule_id}",
        }
    }

    fn read_only() -> bool {
        false
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_CHECKS_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<CheckRulesResponse, CommandError> {
        ctx.db
            .delete_agent_check_rule(ctx.org_id(), &self.rule_id)
            .await
            .map_err(classify_anyhow)?;
        let rows = ctx
            .db
            .list_agent_check_rules(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        Ok(build_response(&rows))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteAgentCheckRule>() }

/// Validate the request and build the storage row.
fn build_upsert_row(
    rule_id: &str,
    req: &UpsertCheckRuleRequest,
) -> Result<UpsertAgentCheckRuleRow, CommandError> {
    if let Some(sev) = &req.severity
        && !VALID_SEVERITIES.contains(&sev.as_str())
    {
        return Err(CommandError::bad_request("Invalid severity"));
    }
    match req.kind.as_str() {
        "builtin_override" => {
            let known = builtin_rule_catalog().iter().any(|r| r.rule_id == rule_id);
            if !known {
                return Err(CommandError::bad_request("Unknown built-in rule id"));
            }
            Ok(UpsertAgentCheckRuleRow {
                rule_id: rule_id.to_string(),
                kind: "builtin_override".to_string(),
                enabled: req.enabled,
                severity_override: req.severity.clone(),
                config: None,
            })
        }
        "declarative" => {
            require_custom_id(rule_id)?;
            let category = require_category(&req.category)?;
            let message = require_text(&req.message, "message")?;
            let pattern = require_text(&req.pattern, "pattern")?;
            Regex::new(&pattern).map_err(|_| CommandError::bad_request("Invalid regex pattern"))?;
            let match_mode = match req.match_mode.as_deref() {
                Some("required") => "required",
                Some("forbidden") | None => "forbidden",
                Some(_) => return Err(CommandError::bad_request("Invalid match_mode")),
            };
            Ok(UpsertAgentCheckRuleRow {
                rule_id: rule_id.to_string(),
                kind: "declarative".to_string(),
                enabled: req.enabled,
                severity_override: Some(req.severity.clone().unwrap_or_else(|| "warning".into())),
                config: Some(serde_json::json!({
                    "category": category,
                    "message": message,
                    "pattern": pattern,
                    "match_mode": match_mode,
                })),
            })
        }
        "nl_rubric" => {
            require_custom_id(rule_id)?;
            let category = require_category(&req.category)?;
            let rubric = require_text(&req.rubric, "rubric")?;
            Ok(UpsertAgentCheckRuleRow {
                rule_id: rule_id.to_string(),
                kind: "nl_rubric".to_string(),
                enabled: req.enabled,
                severity_override: Some(req.severity.clone().unwrap_or_else(|| "info".into())),
                config: Some(serde_json::json!({
                    "category": category,
                    "rubric": rubric,
                })),
            })
        }
        _ => Err(CommandError::bad_request("Invalid rule kind")),
    }
}

fn require_custom_id(rule_id: &str) -> Result<(), CommandError> {
    if rule_id.starts_with("custom.") && rule_id.len() > "custom.".len() {
        Ok(())
    } else {
        Err(CommandError::bad_request(
            "Custom rule id must start with 'custom.'",
        ))
    }
}

fn require_category(category: &Option<String>) -> Result<String, CommandError> {
    let category = category
        .clone()
        .ok_or_else(|| CommandError::bad_request("category is required"))?;
    if !VALID_CATEGORIES.contains(&category.as_str()) {
        return Err(CommandError::bad_request("Invalid category"));
    }
    Ok(category)
}

fn require_text(value: &Option<String>, field: &str) -> Result<String, CommandError> {
    let text = value
        .clone()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| CommandError::bad_request(format!("{field} is required")))?;
    if text.len() > MAX_RULE_TEXT {
        return Err(CommandError::bad_request(format!("{field} is too long")));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(kind: &str) -> UpsertCheckRuleRequest {
        UpsertCheckRuleRequest {
            kind: kind.to_string(),
            enabled: true,
            severity: None,
            category: Some("structure".to_string()),
            message: Some("msg".to_string()),
            pattern: Some("TODO".to_string()),
            match_mode: Some("forbidden".to_string()),
            rubric: Some("must do x".to_string()),
        }
    }

    #[test]
    fn builtin_override_requires_known_rule() {
        assert!(build_upsert_row("prompt.empty", &req("builtin_override")).is_ok());
        assert!(build_upsert_row("prompt.nope", &req("builtin_override")).is_err());
    }

    #[test]
    fn custom_rules_require_custom_prefix() {
        assert!(build_upsert_row("custom.no_todo", &req("declarative")).is_ok());
        assert!(build_upsert_row("no_todo", &req("declarative")).is_err());
    }

    #[test]
    fn declarative_rejects_bad_regex() {
        let mut r = req("declarative");
        r.pattern = Some("(unclosed".to_string());
        assert!(build_upsert_row("custom.bad", &r).is_err());
    }

    #[test]
    fn nl_rubric_requires_rubric() {
        let mut r = req("nl_rubric");
        r.rubric = Some("  ".to_string());
        assert!(build_upsert_row("custom.x", &r).is_err());
    }

    #[test]
    fn invalid_kind_rejected() {
        assert!(build_upsert_row("custom.x", &req("bogus")).is_err());
    }
}
