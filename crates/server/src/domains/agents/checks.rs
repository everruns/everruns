// Tier-1 deterministic agent config checks (knowledge/evaluation/agent-checks.md).
//
// Pure functions over the authored prompt and the resolved preview shape.
// Advisory only: findings never block any operation. LLM-backed checks
// (tier 2) and health checks (tier 3) are separate phases and do not live
// here.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::kernel_imports::{AgentCapabilityConfig, everruns_provider::tool_types::ToolDefinition};
use regex::Regex;
use serde::Serialize;
use utoipa::ToSchema;

/// Advisory severity. There is deliberately no `error`: checks never gate
/// save/publish (knowledge/evaluation/agent-checks.md, Non-Goals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Warning,
    Info,
    Suggestion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Structure,
    Completeness,
    Effectiveness,
    Safety,
    Cost,
}

/// Which tier produced the finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingSource {
    Builtin,
    Llm,
    HealthCheck,
}

/// Pointer to the config field (and optional byte span within it) a finding
/// refers to.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FindingLocation {
    /// Config field name, e.g. `system_prompt`, `tools`.
    pub field: String,
    /// Byte offset range into the field's authored text, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<u32>,
}

/// A single advisory finding about an agent configuration.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Finding {
    /// Stable rule identifier, e.g. `prompt.duplicate_paragraphs`.
    pub rule_id: String,
    pub severity: FindingSeverity,
    pub category: FindingCategory,
    /// Human-readable explanation. Safe to render in user-facing messages.
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<FindingLocation>,
    /// Proposed replacement text (phase 2+; always absent for builtin rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    pub source: FindingSource,
}

impl Finding {
    fn builtin(
        rule_id: &str,
        severity: FindingSeverity,
        category: FindingCategory,
        message: String,
    ) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            severity,
            category,
            message,
            location: None,
            fix: None,
            source: FindingSource::Builtin,
        }
    }

    fn at(mut self, field: &str, span: Option<(usize, usize)>) -> Self {
        self.location = Some(FindingLocation {
            field: field.to_string(),
            start: span.map(|(s, _)| s as u32),
            end: span.map(|(_, e)| e as u32),
        });
        self
    }
}

/// Authored prompt larger than this is flagged as recurring per-turn cost.
const LONG_PROMPT_BYTES: usize = 32 * 1024;
/// Resolved prompt (with harness/capability contributions) larger than this
/// is flagged even when the authored share is small. Higher threshold:
/// contributions are platform-curated, so only clearly oversized totals are
/// worth a finding.
const LONG_RESOLVED_PROMPT_BYTES: usize = 96 * 1024;
/// Paragraphs shorter than this (normalized) are too generic to call
/// duplicates ("Be helpful." legitimately repeats).
const DUP_PARAGRAPH_MIN_CHARS: usize = 80;
/// Keep advisory checks bounded: previews must not amplify a bounded prompt
/// into an unbounded list of serialized findings.
const MAX_FINDINGS_PER_RULE: usize = 25;
const MAX_MESSAGE_SNIPPET_CHARS: usize = 80;

static TEMPLATE_VAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*[A-Za-z0-9_.-]+\s*\}\}").expect("valid regex"));
// Requires at least one underscore on purpose: backticked single words are
// overwhelmingly code literals or jargon (`json`, `bash`, `main`), so flagging
// them as tool references would be mostly noise. Multi-word snake_case matches
// the platform's tool naming convention. Single-word tool names are accepted
// as a known miss for precision.
static BACKTICKED_IDENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`").expect("valid regex"));

const BREVITY_PHRASES: &[&str] = &[
    "be brief",
    "be concise",
    "keep responses short",
    "keep answers short",
    "short answers",
    "terse",
    "minimal output",
];
const VERBOSITY_PHRASES: &[&str] = &[
    "be detailed",
    "be thorough",
    "be exhaustive",
    "comprehensive",
    "verbose",
    "in-depth",
    "in depth",
];

/// Run all tier-1 rules against an agent preview.
///
/// `authored_prompt` is the user-authored system prompt; `resolved_prompt`
/// is the full prompt after harness/capability contributions; `tools` is the
/// resolved tool list.
pub fn run_builtin_checks(
    authored_prompt: &str,
    resolved_prompt: &str,
    capabilities: &[AgentCapabilityConfig],
    tools: &[ToolDefinition],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    check_empty_prompt(authored_prompt, &mut findings);
    check_long_prompt(authored_prompt, resolved_prompt, &mut findings);
    check_template_variables(authored_prompt, &mut findings);
    check_duplicate_paragraphs(authored_prompt, &mut findings);
    check_restates_contribution(authored_prompt, resolved_prompt, &mut findings);
    check_conflicting_style(authored_prompt, &mut findings);
    check_unknown_tool_reference(authored_prompt, capabilities, tools, &mut findings);
    check_duplicate_tool_names(tools, &mut findings);
    findings
}

fn check_empty_prompt(authored: &str, findings: &mut Vec<Finding>) {
    if authored.trim().is_empty() {
        findings.push(
            Finding::builtin(
                "prompt.empty",
                FindingSeverity::Info,
                FindingCategory::Completeness,
                "The agent has no system prompt of its own; its behavior comes entirely from \
                 harness and capability contributions."
                    .to_string(),
            )
            .at("system_prompt", None),
        );
    }
}

fn check_long_prompt(authored: &str, resolved: &str, findings: &mut Vec<Finding>) {
    if authored.len() > LONG_PROMPT_BYTES {
        findings.push(
            Finding::builtin(
                "prompt.very_long",
                FindingSeverity::Warning,
                FindingCategory::Cost,
                format!(
                    "The system prompt is {} KiB and is sent on every model turn. Consider \
                     moving reference material into skills or initial files.",
                    authored.len() / 1024
                ),
            )
            .at("system_prompt", None),
        );
    } else if resolved.len() > LONG_RESOLVED_PROMPT_BYTES {
        findings.push(
            Finding::builtin(
                "prompt.resolved_very_long",
                FindingSeverity::Info,
                FindingCategory::Cost,
                format!(
                    "The full system prompt is {} KiB after harness and capability \
                     contributions ({} KiB authored here) and is sent on every model turn. \
                     Consider disabling capabilities this agent does not need.",
                    resolved.len() / 1024,
                    authored.len() / 1024
                ),
            )
            .at("capabilities", None),
        );
    }
}

fn check_template_variables(authored: &str, findings: &mut Vec<Finding>) {
    if let Some(m) = TEMPLATE_VAR.find(authored) {
        findings.push(
            Finding::builtin(
                "prompt.template_variables",
                FindingSeverity::Warning,
                FindingCategory::Structure,
                format!(
                    "The prompt contains the template placeholder `{}`, which will be sent to \
                     the model literally. Agent prompts do not support template variables.",
                    m.as_str()
                ),
            )
            .at("system_prompt", Some((m.start(), m.end()))),
        );
    }
}

/// Paragraphs of `text` with their byte offsets, normalized for comparison.
fn paragraphs(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            if let Some(s) = start.take() {
                out.push((s, pos, normalize_paragraph(&text[s..pos])));
            }
        } else if start.is_none() {
            start = Some(pos);
        }
        pos += line.len();
    }
    if let Some(s) = start {
        out.push((s, text.len(), normalize_paragraph(&text[s..])));
    }
    out
}

fn normalize_paragraph(p: &str) -> String {
    p.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn paragraph_preview(normalized: &str) -> String {
    snippet_preview(normalized, 60)
}

fn snippet_preview(text: &str, max_chars: usize) -> String {
    let preview: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        format!("{preview}…")
    } else {
        preview
    }
}

fn check_duplicate_paragraphs(authored: &str, findings: &mut Vec<Finding>) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut reported = 0;
    for (start, end, normalized) in paragraphs(authored) {
        if normalized.len() < DUP_PARAGRAPH_MIN_CHARS {
            continue;
        }
        if !seen.insert(normalized.clone()) {
            if reported >= MAX_FINDINGS_PER_RULE {
                findings.push(
                    Finding::builtin(
                        "prompt.duplicate_paragraphs.summary",
                        FindingSeverity::Info,
                        FindingCategory::Structure,
                        format!(
                            "More than {MAX_FINDINGS_PER_RULE} duplicate paragraphs were found; \
                             only the first {MAX_FINDINGS_PER_RULE} are shown."
                        ),
                    )
                    .at("system_prompt", None),
                );
                break;
            }
            reported += 1;
            findings.push(
                Finding::builtin(
                    "prompt.duplicate_paragraphs",
                    FindingSeverity::Warning,
                    FindingCategory::Structure,
                    format!(
                        "The paragraph starting with \"{}\" appears more than once in the \
                         system prompt.",
                        paragraph_preview(&normalized)
                    ),
                )
                .at("system_prompt", Some((start, end))),
            );
        }
    }
}

fn check_restates_contribution(authored: &str, resolved: &str, findings: &mut Vec<Finding>) {
    if authored.trim().is_empty() {
        return;
    }
    // Contribution text = resolved prompt minus the authored portion.
    // Substring match over whitespace-normalized text: contributions wrap
    // content in XML tags that would defeat paragraph-set comparison.
    let contributions = normalize_paragraph(&resolved.replacen(authored, "", 1));
    for (start, end, normalized) in paragraphs(authored) {
        if normalized.len() >= DUP_PARAGRAPH_MIN_CHARS && contributions.contains(&normalized) {
            findings.push(
                Finding::builtin(
                    "prompt.restates_contribution",
                    FindingSeverity::Info,
                    FindingCategory::Structure,
                    format!(
                        "The paragraph starting with \"{}\" duplicates text already contributed \
                         by the harness or an enabled capability.",
                        paragraph_preview(&normalized)
                    ),
                )
                .at("system_prompt", Some((start, end))),
            );
        }
    }
}

fn check_conflicting_style(authored: &str, findings: &mut Vec<Finding>) {
    let lower = authored.to_lowercase();
    let brevity: Vec<&str> = BREVITY_PHRASES
        .iter()
        .copied()
        .filter(|p| lower.contains(p))
        .collect();
    let verbosity: Vec<&str> = VERBOSITY_PHRASES
        .iter()
        .copied()
        .filter(|p| lower.contains(p))
        .collect();
    if !brevity.is_empty() && !verbosity.is_empty() {
        findings.push(
            Finding::builtin(
                "prompt.conflicting_style",
                FindingSeverity::Info,
                FindingCategory::Structure,
                format!(
                    "The prompt asks for both brevity (\"{}\") and detail (\"{}\"). If these \
                     apply to different situations, consider stating the conditions explicitly.",
                    brevity.join("\", \""),
                    verbosity.join("\", \"")
                ),
            )
            .at("system_prompt", None),
        );
    }
}

fn check_unknown_tool_reference(
    authored: &str,
    capabilities: &[AgentCapabilityConfig],
    tools: &[ToolDefinition],
    findings: &mut Vec<Finding>,
) {
    // Tool-less, capability-less agents backtick code identifiers, not tool
    // names; once anything is enabled the agent is in "tool world" and
    // references are worth validating.
    if tools.is_empty() && capabilities.is_empty() {
        return;
    }
    let known: HashSet<&str> = tools
        .iter()
        .map(|t| t.name())
        .chain(capabilities.iter().map(|c| c.capability_id()))
        .collect();
    let mut reported: HashSet<&str> = HashSet::new();
    for caps in BACKTICKED_IDENT.captures_iter(authored) {
        let ident = caps.get(1).expect("group 1 exists");
        if !known.contains(ident.as_str()) && reported.insert(ident.as_str()) {
            if reported.len() > MAX_FINDINGS_PER_RULE {
                findings.push(
                    Finding::builtin(
                        "tools.unknown_reference.summary",
                        FindingSeverity::Info,
                        FindingCategory::Completeness,
                        format!(
                            "More than {MAX_FINDINGS_PER_RULE} unknown tool or capability \
                             references were found; only the first {MAX_FINDINGS_PER_RULE} are \
                             shown."
                        ),
                    )
                    .at("system_prompt", None),
                );
                break;
            }
            let ident_preview = snippet_preview(ident.as_str(), MAX_MESSAGE_SNIPPET_CHARS);
            findings.push(
                Finding::builtin(
                    "tools.unknown_reference",
                    FindingSeverity::Info,
                    FindingCategory::Completeness,
                    format!(
                        "The prompt references `{}`, but no enabled tool or capability has that \
                         name. The model cannot call it.",
                        ident_preview
                    ),
                )
                .at("system_prompt", Some((ident.start(), ident.end()))),
            );
        }
    }
}

fn check_duplicate_tool_names(tools: &[ToolDefinition], findings: &mut Vec<Finding>) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for tool in tools {
        *counts.entry(tool.name()).or_default() += 1;
    }
    let mut dupes: Vec<&str> = counts
        .into_iter()
        .filter(|(_, c)| *c > 1)
        .map(|(n, _)| n)
        .collect();
    dupes.sort_unstable();
    let capped = dupes.len() > MAX_FINDINGS_PER_RULE;
    for name in dupes.into_iter().take(MAX_FINDINGS_PER_RULE) {
        let name_preview = snippet_preview(name, MAX_MESSAGE_SNIPPET_CHARS);
        findings.push(
            Finding::builtin(
                "tools.duplicate_names",
                FindingSeverity::Warning,
                FindingCategory::Completeness,
                format!(
                    "Multiple tools are named `{name_preview}`. The model cannot distinguish them; \
                     rename or remove one of the sources."
                ),
            )
            .at("tools", None),
        );
    }
    if capped {
        findings.push(
            Finding::builtin(
                "tools.duplicate_names.summary",
                FindingSeverity::Info,
                FindingCategory::Completeness,
                format!(
                    "More than {MAX_FINDINGS_PER_RULE} duplicate tool names were found; only the \
                     first {MAX_FINDINGS_PER_RULE} are shown."
                ),
            )
            .at("tools", None),
        );
    }
}

// ============================================================================
// Org-configurable rules (knowledge/evaluation/agent-checks.md, phase 4)
// ============================================================================

/// Static metadata about a built-in rule, for the admin catalog so orgs can
/// see what they can toggle/override.
pub struct BuiltinRuleInfo {
    pub rule_id: &'static str,
    pub default_severity: FindingSeverity,
    pub category: FindingCategory,
    pub description: &'static str,
}

/// The full set of built-in rules an org can enable/disable or re-severity.
/// Kept in sync with the `check_*` helpers above.
pub fn builtin_rule_catalog() -> &'static [BuiltinRuleInfo] {
    use FindingCategory::*;
    use FindingSeverity::*;
    &[
        BuiltinRuleInfo {
            rule_id: "prompt.empty",
            default_severity: Info,
            category: Completeness,
            description: "Agent has no system prompt of its own.",
        },
        BuiltinRuleInfo {
            rule_id: "prompt.very_long",
            default_severity: Warning,
            category: Cost,
            description: "Authored prompt over 32 KiB, sent every turn.",
        },
        BuiltinRuleInfo {
            rule_id: "prompt.resolved_very_long",
            default_severity: Info,
            category: Cost,
            description: "Full prompt over 96 KiB after contributions.",
        },
        BuiltinRuleInfo {
            rule_id: "prompt.template_variables",
            default_severity: Warning,
            category: Structure,
            description: "Literal {{placeholder}} text reaches the model.",
        },
        BuiltinRuleInfo {
            rule_id: "prompt.duplicate_paragraphs",
            default_severity: Warning,
            category: Structure,
            description: "A paragraph appears more than once.",
        },
        BuiltinRuleInfo {
            rule_id: "prompt.restates_contribution",
            default_severity: Info,
            category: Structure,
            description: "Prompt duplicates a harness/capability contribution.",
        },
        BuiltinRuleInfo {
            rule_id: "prompt.conflicting_style",
            default_severity: Info,
            category: Structure,
            description: "Asks for both brevity and detail.",
        },
        BuiltinRuleInfo {
            rule_id: "tools.unknown_reference",
            default_severity: Info,
            category: Completeness,
            description: "Prompt references a tool/capability that is not enabled.",
        },
        BuiltinRuleInfo {
            rule_id: "tools.duplicate_names",
            default_severity: Warning,
            category: Completeness,
            description: "Two tools share a name.",
        },
    ]
}

/// Per-rule override (built-in rules): disable or re-severity.
#[derive(Debug, Clone)]
pub struct RuleOverride {
    pub enabled: bool,
    pub severity: Option<FindingSeverity>,
}

/// When a custom declarative rule's regex match produces a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Flag when the pattern is present in the prompt.
    Forbidden,
    /// Flag when the pattern is absent from the prompt.
    Required,
}

/// A custom, org-authored deterministic rule: a regex over the resolved prompt.
#[derive(Debug, Clone)]
pub struct DeclarativeRule {
    pub rule_id: String,
    pub severity: FindingSeverity,
    pub category: FindingCategory,
    pub message: String,
    pub pattern: Regex,
    pub match_mode: MatchMode,
}

/// Apply org overrides to built-in findings: drop disabled rules and apply
/// severity overrides. Findings whose rule has no override pass through.
pub fn apply_rule_overrides(
    findings: Vec<Finding>,
    overrides: &std::collections::HashMap<String, RuleOverride>,
) -> Vec<Finding> {
    findings
        .into_iter()
        .filter_map(|mut f| {
            if let Some(ovr) = overrides.get(&f.rule_id) {
                if !ovr.enabled {
                    return None;
                }
                if let Some(severity) = ovr.severity {
                    f.severity = severity;
                }
            }
            Some(f)
        })
        .collect()
}

/// Run custom declarative rules against the resolved prompt.
pub fn run_declarative_rules(rules: &[DeclarativeRule], resolved_prompt: &str) -> Vec<Finding> {
    rules
        .iter()
        .filter_map(|rule| {
            let matched = rule.pattern.is_match(resolved_prompt);
            let triggered = match rule.match_mode {
                MatchMode::Forbidden => matched,
                MatchMode::Required => !matched,
            };
            triggered.then(|| Finding {
                rule_id: rule.rule_id.clone(),
                severity: rule.severity,
                category: rule.category,
                message: rule.message.clone(),
                location: None,
                fix: None,
                source: FindingSource::Builtin,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::tool_types::ClientSideTool;

    fn client_tool(name: &str) -> ToolDefinition {
        ToolDefinition::ClientSide(ClientSideTool {
            name: name.to_string(),
            display_name: None,
            description: "test tool".to_string(),
            parameters: serde_json::json!({}),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        })
    }

    fn capability(r#ref: &str) -> AgentCapabilityConfig {
        AgentCapabilityConfig::with_config(r#ref, serde_json::Value::Null)
    }

    fn rule_ids(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.rule_id.as_str()).collect()
    }

    #[test]
    fn clean_prompt_produces_no_findings() {
        let prompt = "You are a helpful support agent. Use the `search_docs` tool to look up \
                      product documentation before answering.";
        let findings = run_builtin_checks(prompt, prompt, &[], &[client_tool("search_docs")]);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn empty_prompt_is_flagged() {
        let findings = run_builtin_checks("  \n", "harness text", &[], &[]);
        assert_eq!(rule_ids(&findings), vec!["prompt.empty"]);
        assert_eq!(findings[0].severity, FindingSeverity::Info);
    }

    #[test]
    fn long_prompt_is_flagged_as_cost() {
        let prompt = "x".repeat(LONG_PROMPT_BYTES + 1);
        let findings = run_builtin_checks(&prompt, &prompt, &[], &[]);
        assert!(rule_ids(&findings).contains(&"prompt.very_long"));
        let f = findings
            .iter()
            .find(|f| f.rule_id == "prompt.very_long")
            .unwrap();
        assert_eq!(f.category, FindingCategory::Cost);
    }

    #[test]
    fn template_variables_are_flagged_with_span() {
        let prompt = "Answer the question about {{product_name}} accurately.";
        let findings = run_builtin_checks(prompt, prompt, &[], &[]);
        assert_eq!(rule_ids(&findings), vec!["prompt.template_variables"]);
        let loc = findings[0].location.as_ref().unwrap();
        let (start, end) = (loc.start.unwrap() as usize, loc.end.unwrap() as usize);
        assert_eq!(&prompt[start..end], "{{product_name}}");
    }

    #[test]
    fn duplicate_paragraphs_are_flagged() {
        let para = "Always verify the customer's account number before discussing any billing \
                    details or making changes to their subscription plan.";
        let prompt = format!("{para}\n\nSome other instruction here.\n\n{para}\n");
        let findings = run_builtin_checks(&prompt, &prompt, &[], &[]);
        assert_eq!(rule_ids(&findings), vec!["prompt.duplicate_paragraphs"]);
    }

    #[test]
    fn short_repeated_lines_are_not_duplicates() {
        let prompt = "Be helpful.\n\nDo the work.\n\nBe helpful.";
        let findings = run_builtin_checks(prompt, prompt, &[], &[]);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn restating_capability_contribution_is_flagged() {
        let para = "When the user asks about the current date or time, use the current_time \
                    tool instead of guessing from training data.";
        let authored = format!("You are an assistant.\n\n{para}");
        let resolved =
            format!("<capability id=\"current_time\">\n{para}\n</capability>\n\n{authored}");
        let findings = run_builtin_checks(&authored, &resolved, &[], &[]);
        assert_eq!(rule_ids(&findings), vec!["prompt.restates_contribution"]);
    }

    #[test]
    fn conflicting_style_keywords_are_flagged() {
        let prompt = "Be concise in all replies. Provide comprehensive analysis of every issue.";
        let findings = run_builtin_checks(prompt, prompt, &[], &[]);
        assert_eq!(rule_ids(&findings), vec!["prompt.conflicting_style"]);
    }

    #[test]
    fn unknown_backticked_tool_reference_is_flagged() {
        let prompt = "Use `search_web` to find current information.";
        let findings = run_builtin_checks(prompt, prompt, &[], &[client_tool("web_fetch")]);
        assert_eq!(rule_ids(&findings), vec!["tools.unknown_reference"]);
    }

    #[test]
    fn unknown_backticked_tool_references_are_capped() {
        let prompt = (0..(MAX_FINDINGS_PER_RULE + 10))
            .map(|i| format!("`missing_tool_{i}`"))
            .collect::<Vec<_>>()
            .join(" ");
        let findings = run_builtin_checks(&prompt, &prompt, &[], &[client_tool("web_fetch")]);

        assert_eq!(
            findings
                .iter()
                .filter(|f| f.rule_id == "tools.unknown_reference")
                .count(),
            MAX_FINDINGS_PER_RULE
        );
        assert_eq!(
            findings.last().map(|f| f.rule_id.as_str()),
            Some("tools.unknown_reference.summary")
        );
    }

    #[test]
    fn unknown_backticked_tool_reference_message_truncates_identifier() {
        let long_name = format!("missing_{}", "tool".repeat(40));
        let prompt = format!("Use `{long_name}`.");
        let findings = run_builtin_checks(&prompt, &prompt, &[], &[client_tool("web_fetch")]);

        assert_eq!(rule_ids(&findings), vec!["tools.unknown_reference"]);
        assert!(!findings[0].message.contains(&long_name));
        assert!(findings[0].message.contains('…'));
    }

    #[test]
    fn duplicate_paragraphs_are_capped() {
        // More than MAX_FINDINGS_PER_RULE distinct paragraphs, each repeated
        // verbatim, so an attacker-controlled prompt cannot amplify into an
        // unbounded findings list.
        let unique = MAX_FINDINGS_PER_RULE + 5;
        let mut paragraphs: Vec<String> = Vec::new();
        for i in 0..unique {
            let paragraph = format!(
                "Paragraph number {i} repeats verbatim with more than enough padding text to \
                 clear the minimum duplicate-paragraph length threshold used by the checker."
            );
            paragraphs.push(paragraph.clone());
            paragraphs.push(paragraph);
        }
        let prompt = paragraphs.join("\n\n");
        let findings = run_builtin_checks(&prompt, &prompt, &[], &[]);

        assert_eq!(
            findings
                .iter()
                .filter(|f| f.rule_id == "prompt.duplicate_paragraphs")
                .count(),
            MAX_FINDINGS_PER_RULE
        );
        assert!(
            findings
                .iter()
                .any(|f| f.rule_id == "prompt.duplicate_paragraphs.summary"),
            "expected a prompt.duplicate_paragraphs.summary finding when capped"
        );
    }

    #[test]
    fn duplicate_tool_names_are_capped() {
        // More than MAX_FINDINGS_PER_RULE distinct duplicated tool names must
        // be capped, with a single summary finding.
        let unique = MAX_FINDINGS_PER_RULE + 5;
        let mut tools: Vec<ToolDefinition> = Vec::new();
        for i in 0..unique {
            let name = format!("dup_tool_{i}");
            tools.push(client_tool(&name));
            tools.push(client_tool(&name));
        }
        let findings = run_builtin_checks(
            "You are an assistant.",
            "You are an assistant.",
            &[],
            &tools,
        );

        assert_eq!(
            findings
                .iter()
                .filter(|f| f.rule_id == "tools.duplicate_names")
                .count(),
            MAX_FINDINGS_PER_RULE
        );
        assert!(
            findings
                .iter()
                .any(|f| f.rule_id == "tools.duplicate_names.summary"),
            "expected a tools.duplicate_names.summary finding when capped"
        );
    }

    #[test]
    fn known_tool_and_capability_references_are_not_flagged() {
        let prompt = "Use `web_fetch` to fetch pages. The `bashkit_shell` capability runs code.";
        let findings = run_builtin_checks(
            prompt,
            prompt,
            &[capability("bashkit_shell")],
            &[client_tool("web_fetch")],
        );
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn resolved_long_prompt_is_flagged_when_contributions_dominate() {
        let authored = "Short prompt with `web_fetch` guidance.";
        let resolved = format!("{}{}", "c".repeat(LONG_RESOLVED_PROMPT_BYTES + 1), authored);
        let findings = run_builtin_checks(authored, &resolved, &[], &[client_tool("web_fetch")]);
        assert_eq!(rule_ids(&findings), vec!["prompt.resolved_very_long"]);
        assert_eq!(findings[0].severity, FindingSeverity::Info);
    }

    #[test]
    fn unknown_reference_is_flagged_with_capabilities_but_no_tools() {
        let prompt = "Use `search_web` to find current information.";
        let findings = run_builtin_checks(prompt, prompt, &[capability("agent_instructions")], &[]);
        assert_eq!(rule_ids(&findings), vec!["tools.unknown_reference"]);
    }

    #[test]
    fn no_tool_reference_check_without_tools() {
        // Backticked identifiers in a tool-less, capability-less agent are
        // usually code, not tools.
        let prompt = "Wrap config keys like `max_retry_count` in backticks.";
        let findings = run_builtin_checks(prompt, prompt, &[], &[]);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn duplicate_tool_names_are_flagged() {
        let tools = vec![client_tool("send_email"), client_tool("send_email")];
        let findings = run_builtin_checks("Prompt.", "Prompt.", &[], &tools);
        assert_eq!(rule_ids(&findings), vec!["tools.duplicate_names"]);
    }
}
