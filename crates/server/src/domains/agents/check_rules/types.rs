// Org-configurable agent check rule types (specs/agent-checks.md, phase 4).

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domains::agents::analysis::NlRubricRule;
use crate::domains::agents::checks::{
    DeclarativeRule, FindingCategory, FindingSeverity, MatchMode, RuleOverride,
    builtin_rule_catalog,
};
use crate::storage::models::AgentCheckRuleRow;

/// Effective rule configuration for an org, parsed from stored rows and ready
/// to apply during preview (overrides + declarative) and analyze (nl_rubric).
#[derive(Debug, Default)]
pub struct EffectiveRuleConfig {
    pub overrides: HashMap<String, RuleOverride>,
    pub declarative: Vec<DeclarativeRule>,
    pub nl_rubric: Vec<NlRubricRule>,
}

impl EffectiveRuleConfig {
    /// Build from stored rows. Malformed custom rules are skipped (advisory).
    pub fn from_rows(rows: &[AgentCheckRuleRow]) -> Self {
        let mut config = EffectiveRuleConfig::default();
        for row in rows {
            match row.kind.as_str() {
                "builtin_override" => {
                    config.overrides.insert(
                        row.rule_id.clone(),
                        RuleOverride {
                            enabled: row.enabled,
                            severity: row.severity_override.as_deref().and_then(parse_severity),
                        },
                    );
                }
                "declarative" if row.enabled => {
                    if let Some(rule) = parse_declarative(row) {
                        config.declarative.push(rule);
                    }
                }
                "nl_rubric" if row.enabled => {
                    if let Some(rule) = parse_nl_rubric(row) {
                        config.nl_rubric.push(rule);
                    }
                }
                _ => {}
            }
        }
        config
    }
}

fn parse_severity(s: &str) -> Option<FindingSeverity> {
    match s {
        "warning" => Some(FindingSeverity::Warning),
        "info" => Some(FindingSeverity::Info),
        "suggestion" => Some(FindingSeverity::Suggestion),
        _ => None,
    }
}

fn parse_category(s: &str) -> FindingCategory {
    match s {
        "completeness" => FindingCategory::Completeness,
        "effectiveness" => FindingCategory::Effectiveness,
        "safety" => FindingCategory::Safety,
        "cost" => FindingCategory::Cost,
        _ => FindingCategory::Structure,
    }
}

#[derive(Debug, Deserialize)]
struct DeclarativeConfig {
    category: String,
    message: String,
    pattern: String,
    match_mode: String,
}

fn parse_declarative(row: &AgentCheckRuleRow) -> Option<DeclarativeRule> {
    let cfg: DeclarativeConfig = serde_json::from_value(row.config.clone()?).ok()?;
    let pattern = Regex::new(&cfg.pattern).ok()?;
    let match_mode = match cfg.match_mode.as_str() {
        "required" => MatchMode::Required,
        _ => MatchMode::Forbidden,
    };
    Some(DeclarativeRule {
        rule_id: row.rule_id.clone(),
        severity: row
            .severity_override
            .as_deref()
            .and_then(parse_severity)
            .unwrap_or(FindingSeverity::Warning),
        category: parse_category(&cfg.category),
        message: cfg.message,
        pattern,
        match_mode,
    })
}

#[derive(Debug, Deserialize)]
struct NlRubricConfig {
    category: String,
    rubric: String,
}

fn parse_nl_rubric(row: &AgentCheckRuleRow) -> Option<NlRubricRule> {
    let cfg: NlRubricConfig = serde_json::from_value(row.config.clone()?).ok()?;
    Some(NlRubricRule {
        rule_id: row.rule_id.clone(),
        severity: row
            .severity_override
            .as_deref()
            .and_then(parse_severity)
            .unwrap_or(FindingSeverity::Info),
        category: parse_category(&cfg.category),
        rubric: cfg.rubric,
    })
}

// ---- API DTOs ----

/// Effective built-in rule state for an org (catalog + any override).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BuiltinRuleView {
    pub rule_id: String,
    pub description: String,
    pub category: String,
    pub default_severity: String,
    pub enabled: bool,
    pub severity: String,
}

/// A custom org-authored rule.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CustomRuleView {
    pub rule_id: String,
    pub kind: String,
    pub enabled: bool,
    pub severity: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rubric: Option<String>,
}

/// The org's full check-rule configuration: built-in catalog with effective
/// state, plus custom rules.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CheckRulesResponse {
    pub builtin: Vec<BuiltinRuleView>,
    pub custom: Vec<CustomRuleView>,
}

/// Upsert request for a single rule (keyed by rule_id in the path).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpsertCheckRuleRequest {
    /// `builtin_override`, `declarative`, or `nl_rubric`.
    pub kind: String,
    pub enabled: bool,
    /// Required for custom rules; optional override for built-ins.
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub match_mode: Option<String>,
    #[serde(default)]
    pub rubric: Option<String>,
}

/// Build the list response from stored rows + the static catalog.
pub fn build_response(rows: &[AgentCheckRuleRow]) -> CheckRulesResponse {
    let overrides: HashMap<&str, &AgentCheckRuleRow> = rows
        .iter()
        .filter(|r| r.kind == "builtin_override")
        .map(|r| (r.rule_id.as_str(), r))
        .collect();

    let builtin = builtin_rule_catalog()
        .iter()
        .map(|info| {
            let ovr = overrides.get(info.rule_id);
            BuiltinRuleView {
                rule_id: info.rule_id.to_string(),
                description: info.description.to_string(),
                category: severity_category_str(info.category).to_string(),
                default_severity: severity_str(info.default_severity).to_string(),
                enabled: ovr.map(|r| r.enabled).unwrap_or(true),
                severity: ovr
                    .and_then(|r| r.severity_override.clone())
                    .unwrap_or_else(|| severity_str(info.default_severity).to_string()),
            }
        })
        .collect();

    let custom = rows
        .iter()
        .filter(|r| r.kind != "builtin_override")
        .map(custom_view)
        .collect();

    CheckRulesResponse { builtin, custom }
}

fn custom_view(row: &AgentCheckRuleRow) -> CustomRuleView {
    let cfg = row.config.clone().unwrap_or(serde_json::Value::Null);
    let get = |k: &str| cfg.get(k).and_then(|v| v.as_str()).map(String::from);
    CustomRuleView {
        rule_id: row.rule_id.clone(),
        kind: row.kind.clone(),
        enabled: row.enabled,
        severity: row
            .severity_override
            .clone()
            .unwrap_or_else(|| "info".to_string()),
        category: get("category").unwrap_or_else(|| "structure".to_string()),
        message: get("message"),
        pattern: get("pattern"),
        match_mode: get("match_mode"),
        rubric: get("rubric"),
    }
}

fn severity_str(s: FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Warning => "warning",
        FindingSeverity::Info => "info",
        FindingSeverity::Suggestion => "suggestion",
    }
}

fn severity_category_str(c: FindingCategory) -> &'static str {
    match c {
        FindingCategory::Structure => "structure",
        FindingCategory::Completeness => "completeness",
        FindingCategory::Effectiveness => "effectiveness",
        FindingCategory::Safety => "safety",
        FindingCategory::Cost => "cost",
    }
}
