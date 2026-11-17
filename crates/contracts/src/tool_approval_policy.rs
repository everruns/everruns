//! Who may loosen or tighten the `tool_approval` gate, and the records that
//! carry those choices from the control plane to the gate.
//!
//! Decisions:
//!
//! - **Three layers, one direction each.** The organization caps which modes
//!   people may pick and adds rules for every agent; the agent's capability
//!   config sets its default mode and rules; a person may switch the mode for
//!   one session and add rules that only ever ask *more*. A personal rule
//!   never allows: in a shared agent it would let one person widen what an
//!   admin set.
//! - **Rules beat the reviewer.** `never` blocks before anything else and
//!   `ask` always reaches a person, whatever the mode; `allow` skips approval
//!   for a tool. When rules disagree the most cautious wins.
//! - **Mode switches live in session storage** under the reserved
//!   `tool_approval/` prefix, written only by the server's session API, so a
//!   model can neither read nor write them through `kv_store`.

use serde::{Deserialize, Serialize};

/// Approval modes, from least to most human involvement.
pub const APPROVAL_MODE_NAMES: [&str; 4] = ["off", "auto", "normal", "protective"];

/// Session-storage key of a person's mode switch for one session.
pub const SESSION_APPROVAL_MODE_KEY: &str = "tool_approval/mode";

/// Session-storage key of the auto-review block counters.
pub const AUTO_REVIEW_STREAK_KEY: &str = "tool_approval/review_streak";

/// User-preference key holding a person's own approval settings.
pub const PERSONAL_APPROVAL_PREFERENCE_KEY: &str = "tool_approval";

/// Most rules one layer may hold.
pub const MAX_APPROVAL_RULES: usize = 50;
/// Longest tool-name pattern.
pub const MAX_RULE_PATTERN_LEN: usize = 200;
/// Most trust notes an organization may write.
pub const MAX_TRUST_NOTES: usize = 20;
/// Longest single note, in bytes.
pub const MAX_NOTE_BYTES: usize = 1_000;

/// What a rule does with a matching tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ToolRuleAction {
    /// Run without approval.
    Allow,
    /// Always ask a person, whatever the mode. The auto reviewer cannot
    /// answer for them.
    Ask,
    /// Never run.
    Never,
}

/// A rule over tool names. `tool` is an exact name or a pattern with `*`
/// standing for any run of characters (`github__merge_*`, `*delete*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ToolApprovalRule {
    /// Tool name or `*` pattern.
    pub tool: String,
    /// What happens to a matching call.
    pub action: ToolRuleAction,
}

impl ToolApprovalRule {
    /// Whether `name` matches this rule's pattern.
    pub fn matches(&self, name: &str) -> bool {
        glob_matches(&self.tool, name)
    }
}

/// `*`-only glob, iterative so a hostile pattern cannot blow the stack.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

/// The most cautious action any of `rules` takes on `name`, if one matches.
pub fn strictest_rule<'a>(
    rules: impl IntoIterator<Item = &'a ToolApprovalRule>,
    name: &str,
) -> Option<ToolRuleAction> {
    rules
        .into_iter()
        .filter(|rule| rule.matches(name))
        .map(|rule| rule.action)
        .max()
}

/// An organization's approval policy. Empty means no limits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct OrgToolApprovalPolicy {
    /// Modes people and agents may use (`off`, `auto`, `normal`,
    /// `protective`). Empty allows all. A mode outside the list is raised to
    /// the next more careful allowed one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_modes: Vec<String>,
    /// Rules for every agent in the organization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ToolApprovalRule>,
    /// Plain sentences the auto reviewer reads: trusted domains, repositories,
    /// which systems count as production.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trust_notes: Vec<String>,
}

impl OrgToolApprovalPolicy {
    /// Check sizes, mode names and patterns.
    pub fn validate(&self) -> Result<(), String> {
        for mode in &self.allowed_modes {
            if !APPROVAL_MODE_NAMES.contains(&mode.as_str()) {
                return Err(format!(
                    "allowed_modes must name off, auto, normal or protective, got '{mode}'"
                ));
            }
        }
        validate_rules(&self.rules)?;
        validate_notes(&self.trust_notes, MAX_TRUST_NOTES, "trust_notes")
    }

    /// Whether `mode` may be used.
    pub fn allows(&self, mode: &str) -> bool {
        self.allowed_modes.is_empty() || self.allowed_modes.iter().any(|m| m == mode)
    }

    /// `mode`, or the next more careful mode this policy allows. Falls back
    /// to `protective`, which no policy can make less safe than asking.
    pub fn cap<'a>(&self, mode: &'a str) -> &'a str {
        let start = APPROVAL_MODE_NAMES
            .iter()
            .position(|m| *m == mode)
            .unwrap_or(2);
        APPROVAL_MODE_NAMES[start..]
            .iter()
            .copied()
            .find(|candidate| self.allows(candidate))
            .unwrap_or("protective")
    }
}

/// A person's own approval settings, stored as a user preference.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PersonalToolApproval {
    /// Mode the person's sessions start in, within the organization's
    /// allowed modes. `None` keeps each agent's own mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<String>,
    /// Rules that apply to calls made on this person's behalf. Only `ask` and
    /// `never`: a personal rule may add caution, never remove it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ToolApprovalRule>,
}

impl PersonalToolApproval {
    /// Check the mode name and that every rule only adds caution.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(mode) = &self.default_mode
            && !APPROVAL_MODE_NAMES.contains(&mode.as_str())
        {
            return Err(format!(
                "default_mode must be off, auto, normal or protective, got '{mode}'"
            ));
        }
        validate_rules(&self.rules)?;
        if self.rules.iter().any(|r| r.action == ToolRuleAction::Allow) {
            return Err(
                "personal rules may only ask or never; allow rules belong to the agent \
                        or the organization"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// The settings with anything that would loosen the gate removed, for a
    /// stored value written before validation existed or by hand.
    pub fn sanitized(mut self) -> Self {
        self.rules.retain(|r| r.action != ToolRuleAction::Allow);
        if self
            .default_mode
            .as_deref()
            .is_some_and(|m| !APPROVAL_MODE_NAMES.contains(&m))
        {
            self.default_mode = None;
        }
        self
    }
}

/// Everything outside the agent's own config that shapes the gate for one
/// session, as the control plane hands it to the worker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SessionToolApprovalPolicy {
    /// The session organization's policy.
    #[serde(default)]
    pub org: OrgToolApprovalPolicy,
    /// The session owner's own settings.
    #[serde(default)]
    pub personal: PersonalToolApproval,
}

fn validate_rules(rules: &[ToolApprovalRule]) -> Result<(), String> {
    if rules.len() > MAX_APPROVAL_RULES {
        return Err(format!("at most {MAX_APPROVAL_RULES} rules"));
    }
    for rule in rules {
        let tool = rule.tool.trim();
        if tool.is_empty() || tool.len() > MAX_RULE_PATTERN_LEN {
            return Err(format!(
                "rule tool patterns must be 1 to {MAX_RULE_PATTERN_LEN} characters"
            ));
        }
        if tool.chars().any(char::is_whitespace) {
            return Err(format!(
                "rule tool pattern '{tool}' must not contain spaces"
            ));
        }
    }
    Ok(())
}

fn validate_notes(notes: &[String], max: usize, field: &str) -> Result<(), String> {
    if notes.len() > max {
        return Err(format!("{field} holds at most {max} entries"));
    }
    if notes
        .iter()
        .any(|n| n.trim().is_empty() || n.len() > MAX_NOTE_BYTES)
    {
        return Err(format!(
            "each {field} entry must be 1 to {MAX_NOTE_BYTES} bytes"
        ));
    }
    Ok(())
}

/// Validate free-text reviewer notes on an agent.
pub fn validate_reviewer_notes(notes: &[String]) -> Result<(), String> {
    validate_notes(notes, MAX_TRUST_NOTES, "auto.notes")
}

/// Validate an agent's own rules.
pub fn validate_agent_rules(rules: &[ToolApprovalRule]) -> Result<(), String> {
    validate_rules(rules)
}

/// Counters behind "after 3 blocks in a row or 20 in a session, ask a person".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoReviewStreak {
    /// Blocks since the last allowed call or person's answer.
    pub consecutive: u32,
    /// Blocks in the session.
    pub total: u32,
}

/// Blocks in a row after which auto review hands calls to a person.
pub const AUTO_REVIEW_MAX_CONSECUTIVE_BLOCKS: u32 = 3;
/// Blocks per session after which auto review hands calls to a person.
pub const AUTO_REVIEW_MAX_TOTAL_BLOCKS: u32 = 20;

impl AutoReviewStreak {
    /// Whether the reviewer has blocked too often to keep answering alone.
    pub fn paused(&self) -> bool {
        self.consecutive >= AUTO_REVIEW_MAX_CONSECUTIVE_BLOCKS
            || self.total >= AUTO_REVIEW_MAX_TOTAL_BLOCKS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_approval_types::TOOL_APPROVAL_KV_PREFIX;

    fn rule(tool: &str, action: ToolRuleAction) -> ToolApprovalRule {
        ToolApprovalRule {
            tool: tool.into(),
            action,
        }
    }

    #[test]
    fn patterns_match_names() {
        assert!(rule("send_email", ToolRuleAction::Ask).matches("send_email"));
        assert!(!rule("send_email", ToolRuleAction::Ask).matches("send_emails"));
        assert!(rule("github__*", ToolRuleAction::Ask).matches("github__merge_pr"));
        assert!(rule("*delete*", ToolRuleAction::Ask).matches("bulk_delete_rows"));
        assert!(!rule("*delete*", ToolRuleAction::Ask).matches("create_row"));
        assert!(rule("*", ToolRuleAction::Ask).matches("anything"));
        assert!(rule("a*b*c", ToolRuleAction::Ask).matches("aXXbYYc"));
        assert!(!rule("a*b*c", ToolRuleAction::Ask).matches("aXXbYY"));
    }

    #[test]
    fn the_most_cautious_rule_wins() {
        let rules = [
            rule("github__*", ToolRuleAction::Allow),
            rule("github__delete_*", ToolRuleAction::Never),
            rule("github__merge_pr", ToolRuleAction::Ask),
        ];
        assert_eq!(
            strictest_rule(&rules, "github__list_issues"),
            Some(ToolRuleAction::Allow)
        );
        assert_eq!(
            strictest_rule(&rules, "github__merge_pr"),
            Some(ToolRuleAction::Ask)
        );
        assert_eq!(
            strictest_rule(&rules, "github__delete_repo"),
            Some(ToolRuleAction::Never)
        );
        assert_eq!(strictest_rule(&rules, "slack__post"), None);
    }

    #[test]
    fn a_disallowed_mode_is_raised_to_the_next_careful_one() {
        let policy = OrgToolApprovalPolicy {
            allowed_modes: vec!["normal".into(), "protective".into()],
            ..Default::default()
        };
        assert_eq!(policy.cap("off"), "normal");
        assert_eq!(policy.cap("auto"), "normal");
        assert_eq!(policy.cap("protective"), "protective");
        let only_auto = OrgToolApprovalPolicy {
            allowed_modes: vec!["auto".into()],
            ..Default::default()
        };
        assert_eq!(only_auto.cap("off"), "auto");
        assert_eq!(only_auto.cap("normal"), "protective");
        assert_eq!(OrgToolApprovalPolicy::default().cap("off"), "off");
    }

    #[test]
    fn personal_settings_cannot_loosen() {
        let loosen = PersonalToolApproval {
            default_mode: Some("auto".into()),
            rules: vec![rule("*", ToolRuleAction::Allow)],
        };
        assert!(loosen.validate().is_err());
        assert!(loosen.clone().sanitized().rules.is_empty());
        let tighten = PersonalToolApproval {
            default_mode: Some("auto".into()),
            rules: vec![rule("send_*", ToolRuleAction::Ask)],
        };
        assert!(tighten.validate().is_ok());
    }

    #[test]
    fn org_policy_validation_rejects_bad_input() {
        let bad_mode = OrgToolApprovalPolicy {
            allowed_modes: vec!["yolo".into()],
            ..Default::default()
        };
        assert!(bad_mode.validate().is_err());
        let blank_rule = OrgToolApprovalPolicy {
            rules: vec![rule(" ", ToolRuleAction::Never)],
            ..Default::default()
        };
        assert!(blank_rule.validate().is_err());
        let long_note = OrgToolApprovalPolicy {
            trust_notes: vec!["x".repeat(MAX_NOTE_BYTES + 1)],
            ..Default::default()
        };
        assert!(long_note.validate().is_err());
        assert!(OrgToolApprovalPolicy::default().validate().is_ok());
    }

    #[test]
    fn the_streak_pauses_after_three_in_a_row_or_twenty_total() {
        assert!(!AutoReviewStreak::default().paused());
        assert!(
            AutoReviewStreak {
                consecutive: 3,
                total: 3
            }
            .paused()
        );
        assert!(
            AutoReviewStreak {
                consecutive: 0,
                total: 20
            }
            .paused()
        );
    }

    #[test]
    fn storage_keys_live_under_the_reserved_prefix() {
        assert!(SESSION_APPROVAL_MODE_KEY.starts_with(TOOL_APPROVAL_KV_PREFIX));
        assert!(AUTO_REVIEW_STREAK_KEY.starts_with(TOOL_APPROVAL_KV_PREFIX));
    }
}
