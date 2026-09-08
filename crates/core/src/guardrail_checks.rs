// Declarative guardrail checks engine.
//
// Pure check-evaluation core behind the `guardrails` capability (see
// knowledge/execution/guardrails.md): a typed, declarative config of deterministic checks
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
/// Maximum byte length of an `llm_judge` policy prompt.
pub const MAX_JUDGE_PROMPT_LEN: usize = 4_000;
/// Maximum byte length of an `mcp` check's server reference or tool name.
pub const MAX_MCP_REF_LEN: usize = 128;
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
///
/// `LlmJudge` and `Mcp` are the async variants — they are excluded from the
/// sync `evaluate()` path and handled separately by capability hooks via
/// `CompiledGuardrails::judge_checks_for_stage()` and
/// `CompiledGuardrails::mcp_checks_for_stage()`.
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
    /// Natural-language policy evaluated by the utility LLM.
    /// Valid only on `tool_use` and `tool_output` stages.
    /// Runs asynchronously in the hook path, not in `evaluate()`.
    /// Cost flows through utility-LLM accounting (not the session budget).
    /// Fails open on timeout or LLM error: the verdict defaults to `allow`
    /// so a judge outage never wedges a turn.
    LlmJudge { prompt: String },
    /// Delegate the guardrail decision to a third-party guardrail served as an
    /// external MCP endpoint, called over Everruns' existing scoped-MCP client.
    /// `server` is a scoped-MCP server reference (sanitized server name) and
    /// `tool` is the guardrail tool/method to call on it.
    /// Valid only on `tool_use` and `tool_output` stages.
    /// Runs asynchronously in the hook path, not in `evaluate()`.
    /// External/higher-risk: the stage payload is sent off-platform to the
    /// configured MCP endpoint (data egress). Tenant scoping is enforced by the
    /// host's per-session MCP connection resolver, which only resolves servers
    /// scoped to the current session/org.
    /// Fails open on timeout, connection error, parse failure, or
    /// server-not-configured: the verdict defaults to `allow` so a guardrail
    /// outage never wedges a turn.
    Mcp { server: String, tool: String },
    /// Model-backed moderation/classifier check (EVE-573). Sends the stage
    /// text to the utility LLM as a content classifier and blocks/logs when
    /// any configured category scores at or above `threshold` (a percentage,
    /// `0..=100`). `categories` defaults to a built-in safety set when empty.
    /// Valid only on the `output` stage — it runs on the end-of-message
    /// post-generation seam, not in the sync `evaluate()` path.
    /// Model-backed and higher-risk: the finalized assistant text is sent to
    /// the org's configured utility model (data egress to that provider —
    /// TM-LLM / TM-DOS). Fails open on timeout, LLM error, or parse failure:
    /// the verdict defaults to `allow` so a moderation outage never wedges a
    /// turn.
    Moderation {
        #[serde(default)]
        categories: Vec<String>,
        #[serde(default = "default_moderation_threshold")]
        threshold: u8,
    },
}

/// Default block threshold (percent) for a moderation check when unspecified.
pub fn default_moderation_threshold() -> u8 {
    50
}

/// Built-in moderation categories used when a check specifies none.
pub const DEFAULT_MODERATION_CATEGORIES: &[&str] = &[
    "hate",
    "harassment",
    "self_harm",
    "sexual",
    "violence",
    "illicit",
];

impl GuardrailRule {
    pub fn rule_type(&self) -> &'static str {
        match self {
            GuardrailRule::Regex { .. } => "regex",
            GuardrailRule::Blocklist { .. } => "blocklist",
            GuardrailRule::ToolPattern { .. } => "tool_pattern",
            GuardrailRule::LlmJudge { .. } => "llm_judge",
            GuardrailRule::Mcp { .. } => "mcp",
            GuardrailRule::Moderation { .. } => "moderation",
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
        let mut judge_checks = Vec::new();
        let mut mcp_checks = Vec::new();
        let mut moderation_checks = Vec::new();
        for (index, check) in self.checks.iter().enumerate() {
            match &check.rule {
                GuardrailRule::LlmJudge { prompt } => {
                    judge_checks.push(compile_judge_check(index, check, prompt)?);
                }
                GuardrailRule::Mcp { server, tool } => {
                    mcp_checks.push(compile_mcp_check(index, check, server, tool)?);
                }
                GuardrailRule::Moderation {
                    categories,
                    threshold,
                } => {
                    moderation_checks.push(compile_moderation_check(
                        index, check, categories, *threshold,
                    )?);
                }
                _ => compiled.push(compile_check(index, check)?),
            }
        }
        Ok(CompiledGuardrails {
            mode: self.mode,
            checks: compiled,
            judge_checks,
            mcp_checks,
            moderation_checks,
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
    /// Bounded excerpt of the matched content (matched text for regex,
    /// matched word for blocklist, tool name for tool_pattern).
    pub matched: Option<String>,
}

/// A compiled `llm_judge` check, carried separately from the sync checks
/// because it must be evaluated asynchronously by the capability hooks.
#[derive(Debug)]
pub struct CompiledJudgeCheck {
    pub index: usize,
    pub label: String,
    pub stage: GuardrailStage,
    pub on_fail: GuardrailOnFail,
    pub replacement: Option<String>,
    /// The natural-language policy prompt.
    pub prompt: String,
}

/// A compiled `mcp` check, carried separately from the sync checks because it
/// must be evaluated asynchronously (network I/O to an external MCP endpoint)
/// by the capability hooks.
#[derive(Debug)]
pub struct CompiledMcpCheck {
    pub index: usize,
    pub label: String,
    pub stage: GuardrailStage,
    pub on_fail: GuardrailOnFail,
    pub replacement: Option<String>,
    /// Scoped-MCP server reference (sanitized server name).
    pub server: String,
    /// Guardrail tool/method to call on the server.
    pub tool: String,
}

/// A compiled `moderation` check, carried separately from the sync checks
/// because it must be evaluated asynchronously (utility-LLM classifier call)
/// on the end-of-message output seam (EVE-573).
#[derive(Debug)]
pub struct CompiledModerationCheck {
    pub index: usize,
    pub label: String,
    pub stage: GuardrailStage,
    pub on_fail: GuardrailOnFail,
    pub replacement: Option<String>,
    /// Categories to score. Never empty after compile — defaults to
    /// [`DEFAULT_MODERATION_CATEGORIES`] when the config specifies none.
    pub categories: Vec<String>,
    /// Block threshold as a percentage (`0..=100`): a category scoring at or
    /// above this value trips the check.
    pub threshold: u8,
}

/// Validated, pre-compiled guardrails ready for evaluation.
#[derive(Debug)]
pub struct CompiledGuardrails {
    mode: GuardrailMode,
    checks: Vec<CompiledCheck>,
    /// LLM-judge checks, separated from the sync deterministic checks.
    /// Evaluated asynchronously by capability hooks; never by `evaluate()`.
    judge_checks: Vec<CompiledJudgeCheck>,
    /// MCP-served checks, separated from the sync deterministic checks.
    /// Evaluated asynchronously by capability hooks; never by `evaluate()`.
    mcp_checks: Vec<CompiledMcpCheck>,
    /// Model-backed moderation checks, evaluated asynchronously on the
    /// end-of-message output seam; never by `evaluate()`.
    moderation_checks: Vec<CompiledModerationCheck>,
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

    /// Whether any check (deterministic, llm_judge, mcp, or moderation)
    /// applies to `stage`.
    pub fn has_stage(&self, stage: GuardrailStage) -> bool {
        self.checks.iter().any(|c| c.stage == stage)
            || self.judge_checks.iter().any(|c| c.stage == stage)
            || self.mcp_checks.iter().any(|c| c.stage == stage)
            || self.moderation_checks.iter().any(|c| c.stage == stage)
    }

    /// Moderation checks that target `stage`. Empty when no `moderation` rule
    /// targets `stage`. Callers run these asynchronously via the utility LLM
    /// on the end-of-message output seam.
    pub fn moderation_checks_for_stage(
        &self,
        stage: GuardrailStage,
    ) -> impl Iterator<Item = &CompiledModerationCheck> {
        self.moderation_checks
            .iter()
            .filter(move |c| c.stage == stage)
    }

    /// LLM-judge checks that target `stage`. Empty when no `llm_judge` rule
    /// targets `stage`. Callers run these asynchronously via the utility LLM.
    pub fn judge_checks_for_stage(
        &self,
        stage: GuardrailStage,
    ) -> impl Iterator<Item = &CompiledJudgeCheck> {
        self.judge_checks.iter().filter(move |c| c.stage == stage)
    }

    /// MCP-served checks that target `stage`. Empty when no `mcp` rule targets
    /// `stage`. Callers run these asynchronously via the scoped-MCP client.
    pub fn mcp_checks_for_stage(
        &self,
        stage: GuardrailStage,
    ) -> impl Iterator<Item = &CompiledMcpCheck> {
        self.mcp_checks.iter().filter(move |c| c.stage == stage)
    }

    /// Effective action for an async check hit (llm_judge / mcp), applying
    /// advisory mode.
    pub fn async_action(&self, on_fail: GuardrailOnFail) -> GuardrailAction {
        match (self.mode, on_fail) {
            (GuardrailMode::Advisory, _) | (_, GuardrailOnFail::Log) => GuardrailAction::Log,
            (GuardrailMode::Active, GuardrailOnFail::Block) => GuardrailAction::Block,
        }
    }

    /// Effective action for a judge check hit, applying advisory mode.
    /// Retained as a name-stable alias of [`Self::async_action`].
    pub fn judge_action(&self, on_fail: GuardrailOnFail) -> GuardrailAction {
        self.async_action(on_fail)
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
        GuardrailRule::LlmJudge { .. } => {
            unreachable!(
                "llm_judge checks are routed to compile_judge_check before compile_check is called"
            )
        }
        GuardrailRule::Mcp { .. } => {
            unreachable!(
                "mcp checks are routed to compile_mcp_check before compile_check is called"
            )
        }
        GuardrailRule::Moderation { .. } => {
            unreachable!(
                "moderation checks are routed to compile_moderation_check before compile_check is called"
            )
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

fn compile_judge_check(
    index: usize,
    check: &GuardrailCheck,
    prompt: &str,
) -> Result<CompiledJudgeCheck, String> {
    let label = match &check.id {
        Some(id) => {
            if id.is_empty() || id.chars().count() > MAX_CHECK_ID_LEN {
                return Err(format!(
                    "check #{index}: id must be 1..={MAX_CHECK_ID_LEN} characters"
                ));
            }
            id.clone()
        }
        None => format!("llm_judge#{index}"),
    };
    if prompt.is_empty() {
        return Err(format!(
            "check '{label}': llm_judge prompt must not be empty"
        ));
    }
    if prompt.len() > MAX_JUDGE_PROMPT_LEN {
        return Err(format!(
            "check '{label}': llm_judge prompt exceeds {MAX_JUDGE_PROMPT_LEN} bytes"
        ));
    }
    match check.stage {
        GuardrailStage::ToolUse | GuardrailStage::ToolOutput => {}
        GuardrailStage::Output => {
            return Err(format!(
                "check '{label}': llm_judge is not supported on the 'output' stage in this phase; \
                 use 'tool_use' or 'tool_output'"
            ));
        }
    }
    if let Some(replacement) = &check.replacement
        && replacement.len() > MAX_REPLACEMENT_LEN
    {
        return Err(format!(
            "check '{label}': replacement exceeds {MAX_REPLACEMENT_LEN} bytes"
        ));
    }
    Ok(CompiledJudgeCheck {
        index,
        label,
        stage: check.stage,
        on_fail: check.on_fail,
        replacement: check.replacement.clone(),
        prompt: prompt.to_string(),
    })
}

fn compile_mcp_check(
    index: usize,
    check: &GuardrailCheck,
    server: &str,
    tool: &str,
) -> Result<CompiledMcpCheck, String> {
    let label = match &check.id {
        Some(id) => {
            if id.is_empty() || id.chars().count() > MAX_CHECK_ID_LEN {
                return Err(format!(
                    "check #{index}: id must be 1..={MAX_CHECK_ID_LEN} characters"
                ));
            }
            id.clone()
        }
        None => format!("mcp#{index}"),
    };
    // The mcp check, like llm_judge, only makes sense at the tool seams in this
    // phase; the `output` stage depends on the end-of-message seam (EVE-573).
    match check.stage {
        GuardrailStage::ToolUse | GuardrailStage::ToolOutput => {}
        GuardrailStage::Output => {
            return Err(format!(
                "check '{label}': mcp is not supported on the 'output' stage in this phase; \
                 use 'tool_use' or 'tool_output'"
            ));
        }
    }
    for (field, value) in [("server", server), ("tool", tool)] {
        if value.is_empty() {
            return Err(format!("check '{label}': mcp {field} must not be empty"));
        }
        if value.len() > MAX_MCP_REF_LEN {
            return Err(format!(
                "check '{label}': mcp {field} exceeds {MAX_MCP_REF_LEN} bytes"
            ));
        }
    }
    if let Some(replacement) = &check.replacement
        && replacement.len() > MAX_REPLACEMENT_LEN
    {
        return Err(format!(
            "check '{label}': replacement exceeds {MAX_REPLACEMENT_LEN} bytes"
        ));
    }
    Ok(CompiledMcpCheck {
        index,
        label,
        stage: check.stage,
        on_fail: check.on_fail,
        replacement: check.replacement.clone(),
        server: server.to_string(),
        tool: tool.to_string(),
    })
}

fn compile_moderation_check(
    index: usize,
    check: &GuardrailCheck,
    categories: &[String],
    threshold: u8,
) -> Result<CompiledModerationCheck, String> {
    let label = match &check.id {
        Some(id) => {
            if id.is_empty() || id.chars().count() > MAX_CHECK_ID_LEN {
                return Err(format!(
                    "check #{index}: id must be 1..={MAX_CHECK_ID_LEN} characters"
                ));
            }
            id.clone()
        }
        None => format!("moderation#{index}"),
    };
    // Moderation is the first output-stage model-backed check; the seam it runs
    // on (the end-of-message output seam) only exists for the `output` stage.
    match check.stage {
        GuardrailStage::Output => {}
        GuardrailStage::ToolUse | GuardrailStage::ToolOutput => {
            return Err(format!(
                "check '{label}': moderation is only supported on the 'output' stage"
            ));
        }
    }
    if threshold > 100 {
        return Err(format!(
            "check '{label}': moderation threshold must be 0..=100 (got {threshold})"
        ));
    }
    // Categories are optional; an empty list means "use the built-in set".
    // A provided list is validated like other entry lists.
    let categories = if categories.is_empty() {
        DEFAULT_MODERATION_CATEGORIES
            .iter()
            .map(|c| (*c).to_string())
            .collect()
    } else {
        validate_entries(&label, "categories", categories)?;
        categories.to_vec()
    };
    if let Some(replacement) = &check.replacement
        && replacement.len() > MAX_REPLACEMENT_LEN
    {
        return Err(format!(
            "check '{label}': replacement exceeds {MAX_REPLACEMENT_LEN} bytes"
        ));
    }
    Ok(CompiledModerationCheck {
        index,
        label,
        stage: check.stage,
        on_fail: check.on_fail,
        replacement: check.replacement.clone(),
        categories,
        threshold,
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
        for stage in [
            GuardrailStage::Output,
            GuardrailStage::ToolUse,
            GuardrailStage::ToolOutput,
        ] {
            assert!(!compiled.has_stage(stage));
            assert!(
                compiled
                    .evaluate(stage, "x", Some("tool"), &no_skip())
                    .is_empty()
            );
        }
        let compiled = GuardrailsConfig::from_value(&serde_json::Value::Null)
            .unwrap()
            .compile()
            .unwrap();
        for stage in [
            GuardrailStage::Output,
            GuardrailStage::ToolUse,
            GuardrailStage::ToolOutput,
        ] {
            assert!(!compiled.has_stage(stage));
        }
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
        assert!(
            compiled
                .evaluate(GuardrailStage::Output, "ordinary prose", None, &no_skip())
                .is_empty()
        );
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
    fn regex_reports_complete_hit_and_matched_text() {
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
        assert_eq!(
            hits,
            vec![GuardrailHit {
                check_index: 0,
                check_label: "ssn".into(),
                stage: GuardrailStage::Output,
                rule_type: "regex",
                action: GuardrailAction::Block,
                reason_code: "guardrail.regex".into(),
                replacement: None,
                matched: Some("123-45-6789".into()),
            }]
        );
        assert!(
            compiled
                .evaluate(GuardrailStage::Output, "123-45-678", None, &no_skip())
                .is_empty()
        );
        assert!(
            compiled
                .evaluate(GuardrailStage::ToolOutput, "123-45-6789", None, &no_skip())
                .is_empty()
        );
    }

    #[test]
    fn invalid_regex_fails_compile_with_check_label() {
        let err = compile(json!({
            "checks": [
                {"id": "bad", "stage": "output", "type": "regex", "patterns": ["("]},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("check 'bad': invalid regex '('"), "{err}");
        assert!(err.contains("unclosed group"), "{err}");
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
        assert_eq!(hits[0].check_label, "blocklist#1");
        assert_eq!(hits[0].matched.as_deref(), Some("y"));
        assert_eq!(
            compiled
                .evaluate(GuardrailStage::Output, "x y", None, &no_skip())
                .iter()
                .map(|h| h.check_index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(
            compiled
                .evaluate(GuardrailStage::Output, "x y", None, &|_| true)
                .is_empty()
        );
    }

    #[test]
    fn check_and_entry_limits_accept_boundary_and_reject_next_byte() {
        let check = json!({"stage":"output", "type":"blocklist", "words":["x"]});
        assert!(compile(json!({"checks":vec![check.clone();64]})).is_ok());
        assert_eq!(
            compile(json!({"checks":vec![check;65]})).unwrap_err(),
            "too many checks: 65 (max 64)"
        );
        for (kind, field, stage) in [
            ("blocklist", "words", "output"),
            ("regex", "patterns", "output"),
            ("tool_pattern", "tools", "tool_use"),
            ("moderation", "categories", "output"),
        ] {
            let mut check = json!({"id":"bounded", "type":kind,"stage":stage});
            check[field] = json!(vec!["x"; 64]);
            assert!(compile(json!({"checks":[check.clone()]})).is_ok(), "{kind}");
            check[field] = json!(vec!["x"; 65]);
            assert_eq!(
                compile(json!({"checks":[check.clone()]})).unwrap_err(),
                format!("check 'bounded': too many {field}: 65 (max 64)")
            );
            for (entry, valid) in [
                ("é".repeat(256), true),
                (format!("{}x", "é".repeat(256)), false),
                (String::new(), false),
            ] {
                check[field] = json!([entry]);
                assert_eq!(
                    compile(json!({"checks":[check.clone()]})).is_ok(),
                    valid,
                    "{kind}"
                );
            }
            check[field] = json!([]);
            assert_eq!(
                compile(json!({"checks":[check]})).is_ok(),
                kind == "moderation",
                "{kind}"
            );
        }
    }

    #[test]
    fn malformed_config_reports_parse_context() {
        // Invalid container types must retain config context for callers.
        let err = GuardrailsConfig::from_value(&json!({"checks": "nope"})).unwrap_err();
        assert!(err.contains("invalid guardrails config"), "{err}");
    }

    #[test]
    fn wildcard_match_covers_anchors_and_inner_stars() {
        for (pattern, name, expected) in [
            ("", "", true),
            ("", "x", false),
            ("*", "", true),
            ("a*b", "ab", true),
            ("a*b", "aba", false),
            ("a*b", "xab", false),
            ("a*a", "a", false),
            ("a**b", "ab", true),
            ("a*b*c", "acb", false),
            ("é*界", "é中界", true),
            ("file?.*", "file1.txt", false),
            ("file?.*", "file?.txt", true),
            ("[ab]*", "abc", false),
        ] {
            assert_eq!(
                wildcard_match(pattern, name),
                expected,
                "{pattern:?}/{name:?}"
            );
        }
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

    // --- llm_judge tests ---

    #[test]
    fn llm_judge_compiles_for_tool_stages() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "llm_judge", "prompt": "Block requests to delete data."},
                {"id": "tj2", "stage": "tool_output", "type": "llm_judge",
                 "prompt": "Block responses containing PII.", "on_fail": "log"},
            ]
        }))
        .expect("compiles");
        assert!(compiled.has_stage(GuardrailStage::ToolUse));
        assert!(compiled.has_stage(GuardrailStage::ToolOutput));
        // llm_judge checks are not in the sync path
        assert!(
            compiled
                .evaluate(
                    GuardrailStage::ToolUse,
                    "{}",
                    Some("delete_user"),
                    &no_skip()
                )
                .is_empty()
        );
        let use_checks: Vec<_> = compiled
            .judge_checks_for_stage(GuardrailStage::ToolUse)
            .collect();
        assert_eq!(use_checks.len(), 1);
        assert_eq!(use_checks[0].prompt, "Block requests to delete data.");
        assert_eq!(use_checks[0].on_fail, GuardrailOnFail::Block);

        let out_checks: Vec<_> = compiled
            .judge_checks_for_stage(GuardrailStage::ToolOutput)
            .collect();
        assert_eq!(out_checks.len(), 1);
        assert_eq!(out_checks[0].label, "tj2");
        assert_eq!(out_checks[0].on_fail, GuardrailOnFail::Log);
    }

    #[test]
    fn llm_judge_rejected_on_output_stage() {
        let err = compile(json!({
            "checks": [
                {"stage": "output", "type": "llm_judge", "prompt": "Block bad content."},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("not supported on the 'output' stage"), "{err}");
    }

    #[test]
    fn llm_judge_empty_prompt_rejected() {
        let err = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "llm_judge", "prompt": ""},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("prompt must not be empty"), "{err}");
    }

    #[test]
    fn llm_judge_prompt_too_long_rejected() {
        let long_prompt = "x".repeat(4_001);
        let err = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "llm_judge", "prompt": long_prompt},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("exceeds"), "{err}");
    }

    #[test]
    fn llm_judge_serde_roundtrip() {
        let cfg = GuardrailsConfig {
            mode: GuardrailMode::Active,
            checks: vec![GuardrailCheck {
                id: Some("pii-judge".into()),
                stage: GuardrailStage::ToolOutput,
                on_fail: GuardrailOnFail::Log,
                replacement: None,
                rule: GuardrailRule::LlmJudge {
                    prompt: "Block responses that contain PII.".into(),
                },
            }],
        };
        let value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(value["checks"][0]["type"], "llm_judge");
        assert_eq!(value["checks"][0]["stage"], "tool_output");
        assert_eq!(
            value["checks"][0]["prompt"],
            "Block responses that contain PII."
        );
        let back = GuardrailsConfig::from_value(&value).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn mixed_sync_and_judge_checks_compile_independently() {
        // A config that has both sync and judge checks: sync checks land in
        // evaluate(), judge checks in judge_checks_for_stage().
        let compiled = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "tool_pattern", "tools": ["bash*"]},
                {"stage": "tool_use", "type": "llm_judge", "prompt": "Block policy violations."},
            ]
        }))
        .unwrap();
        // Sync path catches bash tool
        let hits = compiled.evaluate(GuardrailStage::ToolUse, "{}", Some("bash_exec"), &no_skip());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule_type, "tool_pattern");
        // Judge check is available for async evaluation
        let judges: Vec<_> = compiled
            .judge_checks_for_stage(GuardrailStage::ToolUse)
            .collect();
        assert_eq!(judges.len(), 1);
    }

    // --- mcp tests ---

    #[test]
    fn mcp_compiles_for_tool_stages() {
        let compiled = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "mcp", "server": "guard", "tool": "screen"},
                {"id": "mc2", "stage": "tool_output", "type": "mcp",
                 "server": "guard", "tool": "scan", "on_fail": "log"},
            ]
        }))
        .expect("compiles");
        assert!(compiled.has_stage(GuardrailStage::ToolUse));
        assert!(compiled.has_stage(GuardrailStage::ToolOutput));
        // mcp checks are not in the sync path
        assert!(
            compiled
                .evaluate(
                    GuardrailStage::ToolUse,
                    "{}",
                    Some("delete_user"),
                    &no_skip()
                )
                .is_empty()
        );
        let use_checks: Vec<_> = compiled
            .mcp_checks_for_stage(GuardrailStage::ToolUse)
            .collect();
        assert_eq!(use_checks.len(), 1);
        assert_eq!(use_checks[0].server, "guard");
        assert_eq!(use_checks[0].tool, "screen");
        assert_eq!(use_checks[0].on_fail, GuardrailOnFail::Block);

        let out_checks: Vec<_> = compiled
            .mcp_checks_for_stage(GuardrailStage::ToolOutput)
            .collect();
        assert_eq!(out_checks.len(), 1);
        assert_eq!(out_checks[0].label, "mc2");
        assert_eq!(out_checks[0].on_fail, GuardrailOnFail::Log);
    }

    #[test]
    fn mcp_rejected_on_output_stage() {
        let err = compile(json!({
            "checks": [
                {"stage": "output", "type": "mcp", "server": "guard", "tool": "scan"},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("not supported on the 'output' stage"), "{err}");
    }

    #[test]
    fn mcp_empty_server_or_tool_rejected() {
        let err = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "mcp", "server": "", "tool": "scan"},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("server must not be empty"), "{err}");
        let err = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "mcp", "server": "guard", "tool": ""},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("tool must not be empty"), "{err}");
    }

    #[test]
    fn mcp_ref_too_long_rejected() {
        let long = "x".repeat(129);
        let err = compile(json!({
            "checks": [
                {"stage": "tool_use", "type": "mcp", "server": long, "tool": "scan"},
            ]
        }))
        .unwrap_err();
        assert!(err.contains("exceeds"), "{err}");
    }

    #[test]
    fn mcp_serde_roundtrip() {
        let cfg = GuardrailsConfig {
            mode: GuardrailMode::Active,
            checks: vec![GuardrailCheck {
                id: Some("ext-guard".into()),
                stage: GuardrailStage::ToolOutput,
                on_fail: GuardrailOnFail::Log,
                replacement: None,
                rule: GuardrailRule::Mcp {
                    server: "guard".into(),
                    tool: "scan".into(),
                },
            }],
        };
        let value = serde_json::to_value(&cfg).unwrap();
        assert_eq!(value["checks"][0]["type"], "mcp");
        assert_eq!(value["checks"][0]["stage"], "tool_output");
        assert_eq!(value["checks"][0]["server"], "guard");
        assert_eq!(value["checks"][0]["tool"], "scan");
        let back = GuardrailsConfig::from_value(&value).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn effective_actions_cover_every_mode_and_failure_policy() {
        for (mode, on_fail, expected) in [
            ("active", "block", GuardrailAction::Block),
            ("active", "log", GuardrailAction::Log),
            ("advisory", "block", GuardrailAction::Log),
            ("advisory", "log", GuardrailAction::Log),
        ] {
            let compiled=compile(json!({"mode":mode,"checks":[
                {"stage":"output","type":"blocklist","words":["x"],"on_fail":on_fail,"replacement":"withheld"},
                {"stage":"tool_use","type":"llm_judge","prompt":"policy","on_fail":on_fail},
                {"stage":"tool_output","type":"mcp","server":"guard","tool":"scan","on_fail":on_fail},
                {"stage":"output","type":"moderation","on_fail":on_fail}
            ]})).unwrap();
            let hits = compiled.evaluate(GuardrailStage::Output, "x", None, &no_skip());
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].action, expected, "{mode}/{on_fail}");
            assert_eq!(hits[0].replacement.as_deref(), Some("withheld"));
            let judge = compiled
                .judge_checks_for_stage(GuardrailStage::ToolUse)
                .next()
                .unwrap();
            let mcp = compiled
                .mcp_checks_for_stage(GuardrailStage::ToolOutput)
                .next()
                .unwrap();
            let moderation = compiled
                .moderation_checks_for_stage(GuardrailStage::Output)
                .next()
                .unwrap();
            assert_eq!(compiled.judge_action(judge.on_fail), expected);
            assert_eq!(compiled.async_action(mcp.on_fail), expected);
            assert_eq!(compiled.async_action(moderation.on_fail), expected);
        }
    }

    #[test]
    fn moderation_compiles_defaults_overrides_and_stays_out_of_sync_evaluation() {
        let compiled=compile(json!({"checks":[
            {"stage":"output","type":"moderation"},
            {"id":"custom","stage":"output","type":"moderation","categories":["private"],"threshold":100,"on_fail":"log","replacement":"hidden"}
        ]})).unwrap();
        let checks: Vec<_> = compiled
            .moderation_checks_for_stage(GuardrailStage::Output)
            .collect();
        assert_eq!(checks.len(), 2);
        assert_eq!(
            (
                checks[0].index,
                checks[0].label.as_str(),
                checks[0].threshold
            ),
            (0, "moderation#0", 50)
        );
        assert_eq!(
            checks[0].categories,
            [
                "hate",
                "harassment",
                "self_harm",
                "sexual",
                "violence",
                "illicit"
            ]
        );
        assert_eq!(checks[0].on_fail, GuardrailOnFail::Block);
        assert_eq!(checks[0].replacement, None);
        assert_eq!(
            (
                checks[1].index,
                checks[1].label.as_str(),
                checks[1].threshold
            ),
            (1, "custom", 100)
        );
        assert_eq!(checks[1].categories, ["private"]);
        assert_eq!(checks[1].on_fail, GuardrailOnFail::Log);
        assert_eq!(checks[1].replacement.as_deref(), Some("hidden"));
        for stage in [
            GuardrailStage::Output,
            GuardrailStage::ToolUse,
            GuardrailStage::ToolOutput,
        ] {
            assert_eq!(compiled.has_stage(stage), stage == GuardrailStage::Output);
            assert!(
                compiled
                    .evaluate(stage, "private violence", Some("tool"), &no_skip())
                    .is_empty()
            );
            if stage != GuardrailStage::Output {
                assert_eq!(compiled.moderation_checks_for_stage(stage).count(), 0);
            }
        }
        for (threshold, valid) in [(0, true), (100, true), (101, false), (255, false)] {
            assert_eq!(
                compile(
                    json!({"checks":[{"stage":"output","type":"moderation","threshold":threshold}]})
                )
                .is_ok(),
                valid
            );
        }
        for stage in ["tool_use", "tool_output"] {
            assert_eq!(
                compile(json!({"checks":[{"stage":stage,"type":"moderation"}]})).unwrap_err(),
                "check 'moderation#0': moderation is only supported on the 'output' stage"
            );
        }
    }

    #[test]
    fn common_metadata_limits_apply_to_every_compiler() {
        for mut check in [
            json!({"stage":"output","type":"blocklist","words":["x"]}),
            json!({"stage":"tool_use","type":"llm_judge","prompt":"p"}),
            json!({"stage":"tool_use","type":"mcp","server":"g","tool":"t"}),
            json!({"stage":"output","type":"moderation"}),
        ] {
            check["id"] = json!("é".repeat(64));
            check["replacement"] = json!("é".repeat(1000));
            assert!(compile(json!({"checks":[check.clone()]})).is_ok());
            check["replacement"] = json!(format!("{}x", "é".repeat(1000)));
            assert!(
                compile(json!({"checks":[check.clone()]}))
                    .unwrap_err()
                    .ends_with("replacement exceeds 2000 bytes")
            );
            check.as_object_mut().unwrap().remove("replacement");
            for id in [String::new(), "é".repeat(65)] {
                check["id"] = json!(id);
                assert_eq!(
                    compile(json!({"checks":[check.clone()]})).unwrap_err(),
                    "check #0: id must be 1..=64 characters"
                );
            }
        }
    }

    #[test]
    fn async_reference_boundaries_are_bytes_and_include_both_mcp_fields() {
        for (kind, field, limit) in [
            ("llm_judge", "prompt", 4000),
            ("mcp", "server", 128),
            ("mcp", "tool", 128),
        ] {
            let mut check =
                json!({"stage":"tool_use","type":kind,"prompt":"p","server":"g","tool":"t"});
            check[field] = json!("é".repeat(limit / 2));
            assert!(compile(json!({"checks":[check.clone()]})).is_ok());
            check[field] = json!(format!("{}x", "é".repeat(limit / 2)));
            assert!(
                compile(json!({"checks":[check]}))
                    .unwrap_err()
                    .contains(&format!("{field} exceeds {limit} bytes"))
            );
        }
    }

    #[test]
    fn match_excerpt_is_utf8_safe_and_bounded_for_each_sync_rule() {
        let text = format!("{}界tail", "a".repeat(199));
        for check in [
            json!({"stage":"tool_use","type":"regex","patterns":[".+"]}),
            json!({"stage":"tool_use","type":"blocklist","words":[text.clone()]}),
            json!({"stage":"tool_use","type":"tool_pattern","tools":["*"]}),
        ] {
            let compiled = compile(json!({"checks":[check]})).unwrap();
            let hits = compiled.evaluate(GuardrailStage::ToolUse, &text, Some(&text), &no_skip());
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].matched, Some("a".repeat(199)));
        }
    }
}
