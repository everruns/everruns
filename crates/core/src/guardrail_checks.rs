// Declarative guardrail checks engine.
//
// Pure check-evaluation core behind the `guardrails` capability (see
// specs/guardrails.md): a typed, declarative config of deterministic checks
// that the capability compiles onto the existing interception seams
// (streaming output guardrails, pre/post tool hooks) and that the dry-run
// API evaluates against sample text without a session.
//
// Design constraints:
//  - Deterministic only. Checks here run in the streaming hot path and the
//    per-tool-call path, so every rule must evaluate in linear time with no
//    I/O. Model-based checks (classifiers, LLM judges) are a later phase and
//    will never share this code path.
//  - Compiled once, evaluated many times. `GuardrailsConfig::compile`
//    validates and pre-compiles all rules; evaluation never allocates
//    regexes or lowercases word lists.
//  - The `regex` crate guarantees linear-time matching (no backtracking),
//    and `MAX_*` limits bound compile cost, so user-authored patterns cannot
//    DoS the worker (TM-API input validation, TM-DOS resource exhaustion).

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Maximum number of checks in one guardrails config.
pub const MAX_CHECKS: usize = 64;
/// Maximum patterns/words/tool globs per check.
pub const MAX_ENTRIES_PER_CHECK: usize = 64;
/// Maximum length of a single pattern/word/tool glob.
pub const MAX_ENTRY_LEN: usize = 512;
/// Maximum length of a custom replacement message.
pub const MAX_REPLACEMENT_LEN: usize = 2_000;
/// Maximum length of a check id.
pub const MAX_CHECK_ID_LEN: usize = 64;
/// Compiled regex size budget per pattern (bytes). Keeps pathological
/// patterns from ballooning compile time/memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Cap on the matched-snippet excerpt carried in a hit. Keeps audit
/// logs and dry-run responses bounded.
const MAX_MATCH_SNIPPET: usize = 200;

/// Default replacement when an output-stage block has no custom text.
pub const DEFAULT_OUTPUT_REPLACEMENT: &str = "[Response withheld by a guardrail.]";
/// Default notice replacing tool output suppressed by a block.
pub const DEFAULT_TOOL_OUTPUT_REPLACEMENT: &str = "[Tool output withheld by a guardrail.]";

/// Whether hits take effect or are only logged. Advisory mode is how a
/// guardrail is tuned against false positives before being made active:
/// checks run and report, but never block or replace anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum GuardrailMode {
    #[default]
    Active,
    Advisory,
}

/// Pipeline stage a check applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "output"))]
#[serde(rename_all = "snake_case")]
pub enum GuardrailStage {
    /// The model's streamed assistant text (evaluated per delta against the
    /// accumulated output).
    Output,
    /// A tool call before execution (tool name and serialized arguments).
    ToolUse,
    /// A tool result after execution, before it enters model context.
    ToolOutput,
}

impl GuardrailStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            GuardrailStage::Output => "output",
            GuardrailStage::ToolUse => "tool_use",
            GuardrailStage::ToolOutput => "tool_output",
        }
    }
}

/// Per-check failure action. Effective action is downgraded to `Log` when
/// the whole config is in advisory mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum GuardrailOnFail {
    #[default]
    Block,
    Log,
}

/// Deterministic rule variants. Tagged as `"type"` in JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuardrailRule {
    /// Match any of the regex patterns against the stage text.
    Regex { patterns: Vec<String> },
    /// Match any of the words/phrases as substrings of the stage text.
    Blocklist {
        words: Vec<String>,
        #[serde(default)]
        case_sensitive: bool,
    },
    /// Match the tool name against `*`-wildcard patterns. Only valid for
    /// the `tool_use` stage.
    ToolPattern { tools: Vec<String> },
}

impl GuardrailRule {
    pub fn rule_type(&self) -> &'static str {
        match self {
            GuardrailRule::Regex { .. } => "regex",
            GuardrailRule::Blocklist { .. } => "blocklist",
            GuardrailRule::ToolPattern { .. } => "tool_pattern",
        }
    }
}

/// One declarative check: a rule bound to a stage with a failure action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailCheck {
    /// Optional stable identifier surfaced in reason codes and audit logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub stage: GuardrailStage,
    #[serde(default)]
    pub on_fail: GuardrailOnFail,
    /// Replacement text used when a block suppresses output (output and
    /// tool_output stages) or shown to the user for blocked tool calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
    #[serde(flatten)]
    pub rule: GuardrailRule,
}

/// Declarative guardrails config — the `guardrails` capability's per-agent
/// config payload and the dry-run API's input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    #[serde(default)]
    pub mode: GuardrailMode,
    #[serde(default)]
    pub checks: Vec<GuardrailCheck>,
}

impl GuardrailsConfig {
    /// Parse a config out of arbitrary JSON (the capability config value).
    pub fn from_value(value: &serde_json::Value) -> Result<Self, String> {
        if value.is_null() {
            return Ok(Self::default());
        }
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid guardrails config: {e}"))
    }

    /// Validate and compile all checks. Errors are human-readable and refer
    /// to checks by index/id so config editors can surface them.
    pub fn compile(&self) -> Result<CompiledGuardrails, String> {
        if self.checks.len() > MAX_CHECKS {
            return Err(format!(
                "too many checks: {} (max {MAX_CHECKS})",
                self.checks.len()
            ));
        }
        let mut compiled = Vec::with_capacity(self.checks.len());
        for (index, check) in self.checks.iter().enumerate() {
            compiled.push(compile_check(index, check)?);
        }
        Ok(CompiledGuardrails {
            mode: self.mode,
            checks: compiled,
        })
    }
}

/// Effective action of a hit after applying the config mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "block"))]
#[serde(rename_all = "snake_case")]
pub enum GuardrailAction {
    Block,
    Log,
}

/// One triggered check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailHit {
    /// Index of the check in the config's `checks` array.
    pub check_index: usize,
    /// The check's `id`, or `"<type>#<index>"` when none was set.
    pub check_label: String,
    pub stage: GuardrailStage,
    pub rule_type: &'static str,
    /// Effective action (advisory mode downgrades Block to Log).
    pub action: GuardrailAction,
    /// Stable machine-readable code, `guardrail.<rule_type>`.
    pub reason_code: String,
    /// The check's custom replacement, if configured.
    pub replacement: Option<String>,
    /// Bounded excerpt of the matched content (pattern source for regex,
    /// matched word for blocklist, tool name for tool_pattern).
    pub matched: Option<String>,
}

/// Validated, pre-compiled guardrails ready for evaluation.
#[derive(Debug)]
pub struct CompiledGuardrails {
    mode: GuardrailMode,
    checks: Vec<CompiledCheck>,
}

#[derive(Debug)]
struct CompiledCheck {
    index: usize,
    label: String,
    stage: GuardrailStage,
    on_fail: GuardrailOnFail,
    replacement: Option<String>,
    rule_type: &'static str,
    matcher: CompiledRule,
}

#[derive(Debug)]
enum CompiledRule {
    Regex(Vec<regex::Regex>),
    Blocklist {
        /// Lowercased ahead of time for case-insensitive matching.
        words: Vec<String>,
        case_sensitive: bool,
    },
    ToolPattern(Vec<String>),
}

impl CompiledGuardrails {
    pub fn mode(&self) -> GuardrailMode {
        self.mode
    }

    /// Whether any check applies to `stage`.
    pub fn has_stage(&self, stage: GuardrailStage) -> bool {
        self.checks.iter().any(|c| c.stage == stage)
    }

    /// Evaluate all checks for `stage` against `text`. For the `tool_use`
    /// stage, `tool_name` feeds `tool_pattern` rules and `text` is the
    /// serialized tool arguments; other stages ignore `tool_name`.
    /// `skip` suppresses checks by index (used by the streaming run to
    /// avoid re-reporting an already-logged advisory hit on every delta).
    pub fn evaluate(
        &self,
        stage: GuardrailStage,
        text: &str,
        tool_name: Option<&str>,
        skip: &dyn Fn(usize) -> bool,
    ) -> Vec<GuardrailHit> {
        let lowercased: std::cell::OnceCell<String> = std::cell::OnceCell::new();
        let mut hits = Vec::new();
        for check in self.checks.iter() {
            if check.stage != stage || skip(check.index) {
                continue;
            }
            let matched = match &check.matcher {
                CompiledRule::Regex(patterns) => patterns
                    .iter()
                    .find_map(|re| re.find(text).map(|m| snippet(m.as_str()))),
                CompiledRule::Blocklist {
                    words,
                    case_sensitive,
                } => {
                    let haystack: &str = if *case_sensitive {
                        text
                    } else {
                        lowercased.get_or_init(|| text.to_lowercase())
                    };
                    words
                        .iter()
                        .find(|w| haystack.contains(w.as_str()))
                        .map(|w| snippet(w))
                }
                CompiledRule::ToolPattern(patterns) => tool_name.and_then(|name| {
                    patterns
                        .iter()
                        .find(|p| wildcard_match(p, name))
                        .map(|_| snippet(name))
                }),
            };
            if matched.is_some() {
                let action = match (self.mode, check.on_fail) {
                    (GuardrailMode::Advisory, _) | (_, GuardrailOnFail::Log) => {
                        GuardrailAction::Log
                    }
                    (GuardrailMode::Active, GuardrailOnFail::Block) => GuardrailAction::Block,
                };
                hits.push(GuardrailHit {
                    check_index: check.index,
                    check_label: check.label.clone(),
                    stage: check.stage,
                    rule_type: check.rule_type,
                    action,
                    reason_code: format!("guardrail.{}", check.rule_type),
                    replacement: check.replacement.clone(),
                    matched,
                });
            }
        }
        hits
    }
}

fn compile_check(index: usize, check: &GuardrailCheck) -> Result<CompiledCheck, String> {
    let label = match &check.id {
        Some(id) => {
            if id.is_empty() || id.chars().count() > MAX_CHECK_ID_LEN {
                return Err(format!(
                    "check #{index}: id must be 1..={MAX_CHECK_ID_LEN} characters"
                ));
            }
            id.clone()
        }
        None => format!("{}#{}", check.rule.rule_type(), index),
    };
    if let Some(replacement) = &check.replacement
        && replacement.len() > MAX_REPLACEMENT_LEN
    {
        return Err(format!(
            "check '{label}': replacement exceeds {MAX_REPLACEMENT_LEN} bytes"
        ));
    }
    let matcher = match &check.rule {
        GuardrailRule::Regex { patterns } => {
            validate_entries(&label, "patterns", patterns)?;
            let mut compiled = Vec::with_capacity(patterns.len());
            for pattern in patterns {
                let re = regex::RegexBuilder::new(pattern)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|e| format!("check '{label}': invalid regex '{pattern}': {e}"))?;
                compiled.push(re);
            }
            CompiledRule::Regex(compiled)
        }
        GuardrailRule::Blocklist {
            words,
            case_sensitive,
        } => {
            validate_entries(&label, "words", words)?;
            let words = if *case_sensitive {
                words.clone()
            } else {
                words.iter().map(|w| w.to_lowercase()).collect()
            };
            CompiledRule::Blocklist {
                words,
                case_sensitive: *case_sensitive,
            }
        }
        GuardrailRule::ToolPattern { tools } => {
            if check.stage != GuardrailStage::ToolUse {
                return Err(format!(
                    "check '{label}': tool_pattern is only valid for the tool_use stage"
                ));
            }
            validate_entries(&label, "tools", tools)?;
            CompiledRule::ToolPattern(tools.clone())
        }
    };
    Ok(CompiledCheck {
        index,
        label,
        stage: check.stage,
        on_fail: check.on_fail,
        replacement: check.replacement.clone(),
        rule_type: check.rule.rule_type(),
        matcher,
    })
}

fn validate_entries(label: &str, field: &str, entries: &[String]) -> Result<(), String> {
    if entries.is_empty() {
        return Err(format!("check '{label}': {field} must not be empty"));
    }
    if entries.len() > MAX_ENTRIES_PER_CHECK {
        return Err(format!(
            "check '{label}': too many {field}: {} (max {MAX_ENTRIES_PER_CHECK})",
            entries.len()
        ));
    }
    for entry in entries {
        if entry.is_empty() {
            return Err(format!(
                "check '{label}': {field} entries must not be empty"
            ));
        }
        if entry.len() > MAX_ENTRY_LEN {
            return Err(format!(
                "check '{label}': {field} entry exceeds {MAX_ENTRY_LEN} bytes"
            ));
        }
    }
    Ok(())
}

fn snippet(s: &str) -> String {
    let mut end = MAX_MATCH_SNIPPET.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// `*`-wildcard match over tool names: `*` matches any (possibly empty)
/// run of characters; everything else is literal. Linear in input length.
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let segments: Vec<&str> = pattern.split('*').collect();
    if segments.len() == 1 {
        return pattern == name;
    }
    let mut rest = name;
    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            continue;
        }
        if i == 0 {
            match rest.strip_prefix(seg) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == segments.len() - 1 {
            return rest.ends_with(seg);
        } else {
            match rest.find(seg) {
                Some(pos) => rest = &rest[pos + seg.len()..],
                None => return false,
            }
        }
    }
    // Pattern ends with '*' (last segment empty) — any remainder matches.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn no_skip() -> impl Fn(usize) -> bool {
        |_| false
    }

    fn compile(value: serde_json::Value) -> Result<CompiledGuardrails, String> {
        GuardrailsConfig::from_value(&value)?.compile()
    }

    #[test]
    fn parses_and_compiles_minimal_config() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "output", "type": "blocklist", "words": ["forbidden"]},
            ]
        }))
        .expect("compiles");
        assert_eq!(compiled.mode(), GuardrailMode::Active);
        assert!(compiled.has_stage(GuardrailStage::Output));
        assert!(!compiled.has_stage(GuardrailStage::ToolUse));
    }

    #[test]
    fn empty_or_null_config_compiles_to_no_checks() {
        let compiled = compile(json!({})).expect("compiles");
        assert!(!compiled.has_stage(GuardrailStage::Output));
        let compiled = GuardrailsConfig::from_value(&serde_json::Value::Null)
            .unwrap()
            .compile()
            .unwrap();
        assert!(!compiled.has_stage(GuardrailStage::Output));
    }

    #[test]
    fn blocklist_matches_case_insensitive_by_default() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "output", "type": "blocklist", "words": ["Secret Word"]},
            ]
        }))
        .unwrap();
        let hits = compiled.evaluate(
            GuardrailStage::Output,
            "this contains a SECRET word inside",
            None,
            &no_skip(),
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].action, GuardrailAction::Block);
        assert_eq!(hits[0].reason_code, "guardrail.blocklist");
        assert_eq!(hits[0].matched.as_deref(), Some("secret word"));
    }

    #[test]
    fn blocklist_case_sensitive_only_matches_exact_case() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "output", "type": "blocklist", "words": ["Secret"], "case_sensitive": true},
            ]
        }))
        .unwrap();
        assert!(
            compiled
                .evaluate(GuardrailStage::Output, "a secret here", None, &no_skip())
                .is_empty()
        );
        assert_eq!(
            compiled
                .evaluate(GuardrailStage::Output, "a Secret here", None, &no_skip())
                .len(),
            1
        );
    }

    #[test]
    fn regex_matches_and_reports_pattern_source() {
        let compiled = compile(json!({
            "checks": [
                {"id": "ssn", "stage": "output", "type": "regex",
                 "patterns": ["\\b\\d{3}-\\d{2}-\\d{4}\\b"]},
            ]
        }))
        .unwrap();
        let hits = compiled.evaluate(
            GuardrailStage::Output,
            "my ssn is 123-45-6789 ok",
            None,
            &no_skip(),
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].check_label, "ssn");
        assert_eq!(hits[0].rule_type, "regex");
    }

    #[test]
    fn invalid_regex_fails_compile_with_check_label() {
        let err = compile(json!({
            "checks": [
                {"id": "bad", "stage": "output", "type": "regex", "patterns": ["("]},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("check 'bad'"), "{err}");
    }

    #[test]
    fn tool_pattern_matches_wildcards_on_tool_use_stage() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "tool_pattern", "tools": ["mcp_*", "bash*"]},
            ]
        }))
        .unwrap();
        let hits = compiled.evaluate(
            GuardrailStage::ToolUse,
            "{\"cmd\":\"ls\"}",
            Some("mcp_github__create_issue"),
            &no_skip(),
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].matched.as_deref(), Some("mcp_github__create_issue"));
        assert!(
            compiled
                .evaluate(GuardrailStage::ToolUse, "{}", Some("read_file"), &no_skip())
                .is_empty()
        );
    }

    #[test]
    fn tool_pattern_rejected_outside_tool_use_stage() {
        let err = compile(json!({
            "checks": [
                {"stage": "output", "type": "tool_pattern", "tools": ["bash*"]},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("only valid for the tool_use stage"), "{err}");
    }

    #[test]
    fn advisory_mode_downgrades_block_to_log() {
        let compiled = compile(json!({
            "mode": "advisory",
            "checks": [
                {"stage": "output", "type": "blocklist", "words": ["x"], "on_fail": "block"},
            ]
        }))
        .unwrap();
        let hits = compiled.evaluate(GuardrailStage::Output, "x", None, &no_skip());
        assert_eq!(hits[0].action, GuardrailAction::Log);
    }

    #[test]
    fn on_fail_log_yields_log_action_in_active_mode() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "output", "type": "blocklist", "words": ["x"], "on_fail": "log"},
            ]
        }))
        .unwrap();
        let hits = compiled.evaluate(GuardrailStage::Output, "x", None, &no_skip());
        assert_eq!(hits[0].action, GuardrailAction::Log);
    }

    #[test]
    fn skip_suppresses_checks_by_index() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "output", "type": "blocklist", "words": ["x"]},
                {"stage": "output", "type": "blocklist", "words": ["y"]},
            ]
        }))
        .unwrap();
        let hits = compiled.evaluate(GuardrailStage::Output, "x y", None, &|i| i == 0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].check_index, 1);
    }

    #[test]
    fn enforces_limits() {
        let too_many_checks: Vec<_> = (0..=MAX_CHECKS)
            .map(|_| json!({"stage": "output", "type": "blocklist", "words": ["x"]}))
            .collect();
        assert!(
            compile(json!({"checks": too_many_checks}))
                .unwrap_err()
                .contains("too many checks")
        );

        let long_entry = "a".repeat(MAX_ENTRY_LEN + 1);
        assert!(
            compile(json!({
                "checks": [{"stage": "output", "type": "blocklist", "words": [long_entry]}]
            }))
            .unwrap_err()
            .contains("exceeds")
        );

        assert!(
            compile(json!({
                "checks": [{"stage": "output", "type": "blocklist", "words": []}]
            }))
            .unwrap_err()
            .contains("must not be empty")
        );
    }

    #[test]
    fn unknown_fields_are_rejected_gracefully_by_value_parse() {
        // serde keeps unknown fields by default for forward-compat; the
        // important part is that a wrong type errors with a clear message.
        let err = GuardrailsConfig::from_value(&json!({"checks": "nope"})).unwrap_err();
        assert!(err.contains("invalid guardrails config"), "{err}");
    }

    #[test]
    fn wildcard_match_covers_anchors_and_inner_stars() {
        assert!(wildcard_match("bash*", "bashkit_exec"));
        assert!(wildcard_match("*_file", "read_file"));
        assert!(wildcard_match("mcp_*__delete_*", "mcp_github__delete_repo"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("exact", "exact"));
        assert!(!wildcard_match("exact", "exact_no"));
        assert!(!wildcard_match("bash*", "zsh"));
        assert!(!wildcard_match(
            "mcp_*__delete_*",
            "mcp_github__create_repo"
        ));
    }

    #[test]
    fn config_roundtrips_serde() {
        let cfg = GuardrailsConfig {
            mode: GuardrailMode::Advisory,
            checks: vec![GuardrailCheck {
                id: Some("c1".into()),
                stage: GuardrailStage::ToolUse,
                on_fail: GuardrailOnFail::Log,
                replacement: None,
                rule: GuardrailRule::ToolPattern {
                    tools: vec!["bash*".into()],
                },
            }],
        };
        let value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(value["checks"][0]["type"], "tool_pattern");
        assert_eq!(value["checks"][0]["stage"], "tool_use");
        let back = GuardrailsConfig::from_value(&value).unwrap();
        assert_eq!(back, cfg);
    }
}
