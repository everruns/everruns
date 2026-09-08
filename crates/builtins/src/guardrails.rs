// Declarative guardrails capability.
//
// Attaches the deterministic check engine (`crate::guardrail_checks`) to the
// existing interception seams — streaming output guardrails and pre/post
// tool hooks — driven entirely by per-agent config. No checks configured
// means no hooks contributed: an agent without this capability (or with an
// empty config) runs exactly as before. See knowledge/execution/guardrails.md.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::capabilities::{Capability, CapabilityLocalization};
use crate::guardrail_checks::{
    CompiledGuardrails, DEFAULT_OUTPUT_REPLACEMENT, DEFAULT_TOOL_OUTPUT_REPLACEMENT,
    GuardrailAction, GuardrailStage, GuardrailsConfig, MAX_CHECK_ID_LEN, MAX_CHECKS,
    MAX_ENTRIES_PER_CHECK, MAX_ENTRY_LEN, MAX_JUDGE_PROMPT_LEN, MAX_MCP_REF_LEN,
    MAX_REPLACEMENT_LEN,
};
use crate::mcp_server::mcp_tool_name;
use crate::output_guardrail::{
    GuardrailDecision, OutputGuardrail, OutputGuardrailContext, OutputGuardrailRun,
    PostGenerationOutputContext, PostGenerationOutputGuardrail,
};
use crate::tool_hooks::{
    PostToolExecHook, PostToolExecHookPriority, PreToolUseDecision, PreToolUseHook,
};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::utility_llm::{UtilityLlmReasoningEffort, UtilityLlmRequest};
use crate::{LlmMessage, LlmMessageRole};
use everruns_core::tool_context::ToolContext;

pub const GUARDRAILS_CAPABILITY_ID: &str = "guardrails";

pub struct GuardrailsCapability;

impl Capability for GuardrailsCapability {
    fn id(&self) -> &str {
        GUARDRAILS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Guardrails"
    }

    fn description(&self) -> &str {
        "Guardrail checks over model output and tool calls: regex and blocklist \
         matching, tool-call restrictions, an LLM judge, and delegation to an \
         external guardrail served over scoped MCP. Checks block or log per \
         configuration; advisory mode logs without enforcing."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Запобіжники",
            "Детерміновані перевірки виводу моделі та викликів інструментів: \
             регулярні вирази, списки заборонених слів, обмеження інструментів. \
             Перевірки блокують або лише журналюють згідно з конфігурацією.",
        )]
    }

    fn category(&self) -> Option<&str> {
        Some("Safety")
    }

    fn icon(&self) -> Option<&str> {
        Some("shield")
    }

    fn is_guardrail(&self) -> bool {
        true
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["active", "advisory"],
                    "default": "active",
                    "description": "Advisory runs all checks but only logs hits — use it to tune checks against false positives before enforcing."
                },
                "checks": {
                    "type": "array",
                    "maxItems": MAX_CHECKS,
                    "items": {
                        "type": "object",
                        "required": ["stage", "type"],
                        "properties": {
                            "id": {
                                "type": "string",
                                "maxLength": MAX_CHECK_ID_LEN,
                                "description": "Stable identifier surfaced in reason codes and logs."
                            },
                            "stage": {
                                "type": "string",
                                "enum": ["output", "tool_use", "tool_output"],
                                "description": "Where the check runs: streamed model output, tool calls before execution, or tool results before they enter context."
                            },
                            "type": {
                                "type": "string",
                                "enum": ["regex", "blocklist", "tool_pattern", "llm_judge", "mcp", "moderation"],
                                "description": "regex/blocklist match stage text; tool_pattern matches tool names (tool_use stage only); llm_judge evaluates a natural-language policy via the utility LLM (tool_use/tool_output stages only); mcp delegates the decision to an external guardrail served over scoped MCP (tool_use/tool_output stages only — sends stage content off-platform); moderation scores the finalized assistant message via the utility LLM as a content classifier (output stage only — runs on the end-of-message seam, sends the message to the utility model)."
                            },
                            "patterns": {
                                "type": "array",
                                "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                                "maxItems": MAX_ENTRIES_PER_CHECK,
                                "description": "Regex patterns (type=regex)."
                            },
                            "words": {
                                "type": "array",
                                "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                                "maxItems": MAX_ENTRIES_PER_CHECK,
                                "description": "Words or phrases matched as substrings (type=blocklist)."
                            },
                            "case_sensitive": {
                                "type": "boolean",
                                "default": false,
                                "description": "Blocklist matching case sensitivity."
                            },
                            "tools": {
                                "type": "array",
                                "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                                "maxItems": MAX_ENTRIES_PER_CHECK,
                                "description": "Tool name patterns with * wildcards (type=tool_pattern)."
                            },
                            "on_fail": {
                                "type": "string",
                                "enum": ["block", "log"],
                                "default": "block",
                                "description": "block stops the output/tool call; log records the hit and continues."
                            },
                            "prompt": {
                                "type": "string",
                                "maxLength": MAX_JUDGE_PROMPT_LEN,
                                "description": "Natural-language policy prompt for llm_judge. Example: 'Block any tool call that reads files outside /home/user.' Evaluated by the utility LLM; fails open on timeout or error."
                            },
                            "server": {
                                "type": "string",
                                "maxLength": MAX_MCP_REF_LEN,
                                "description": "Scoped-MCP server reference for type=mcp (sanitized server name). Required for mcp checks."
                            },
                            "tool": {
                                "type": "string",
                                "maxLength": MAX_MCP_REF_LEN,
                                "description": "Guardrail tool/method to call on the MCP server for type=mcp. Required for mcp checks. Sends a bounded stage payload off-platform; fails open on timeout, connection error, parse failure, or server-not-configured."
                            },
                            "categories": {
                                "type": "array",
                                "items": {"type": "string", "maxLength": MAX_ENTRY_LEN},
                                "maxItems": MAX_ENTRIES_PER_CHECK,
                                "description": "Moderation categories to score (type=moderation). Defaults to a built-in safety set (hate, harassment, self_harm, sexual, violence, illicit) when omitted."
                            },
                            "threshold": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 100,
                                "default": 50,
                                "description": "Moderation block threshold as a percentage (0-100): a category scoring at or above this value trips the check (type=moderation)."
                            },
                            "replacement": {
                                "type": "string",
                                "maxLength": MAX_REPLACEMENT_LEN,
                                "description": "Text shown in place of blocked output or as the user-facing message for blocked tool calls."
                            }
                        }
                    }
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        GuardrailsConfig::from_value(config)?.compile().map(|_| ())
    }

    fn output_guardrails(&self) -> Vec<Arc<dyn OutputGuardrail>> {
        vec![Arc::new(DeclarativeOutputGuardrail)]
    }

    fn post_output_guardrails_with_config(
        &self,
        config: &serde_json::Value,
    ) -> Vec<Arc<dyn PostGenerationOutputGuardrail>> {
        // Contribute the model-backed moderation provider only when an
        // output-stage moderation check is configured. Deterministic output
        // checks run on the streaming seam (`output_guardrails`), not here.
        match compile_config_for_stage(config, GuardrailStage::Output) {
            Some(compiled)
                if compiled
                    .moderation_checks_for_stage(GuardrailStage::Output)
                    .next()
                    .is_some() =>
            {
                vec![Arc::new(ModerationOutputGuardrail { compiled })]
            }
            _ => vec![],
        }
    }

    fn pre_tool_use_hooks_with_config(
        &self,
        config: &serde_json::Value,
    ) -> Vec<Arc<dyn PreToolUseHook>> {
        match compile_config_for_stage(config, GuardrailStage::ToolUse) {
            Some(compiled) => vec![Arc::new(GuardrailPreToolHook { compiled })],
            None => vec![],
        }
    }

    fn post_tool_exec_hooks_with_config(
        &self,
        config: &serde_json::Value,
    ) -> Vec<Arc<dyn PostToolExecHook>> {
        match compile_config_for_stage(config, GuardrailStage::ToolOutput) {
            Some(compiled) => vec![Arc::new(GuardrailPostToolHook { compiled })],
            None => vec![],
        }
    }
}

/// Timeout for a single LLM judge call. Fail-open on expiry.
const JUDGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Maximum judge checks evaluated per single tool call invocation.
const MAX_JUDGE_CALLS_PER_INVOCATION: usize = 4;

/// Timeout for a single MCP guardrail call. Fail-open on expiry. Mirrors
/// `JUDGE_TIMEOUT` — an external guardrail outage must never wedge a turn.
const MCP_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Maximum MCP guardrail checks evaluated per single tool call invocation.
/// Mirrors `MAX_JUDGE_CALLS_PER_INVOCATION` to bound added latency (TM-DOS).
///
/// NOTE: this cap is per-check-type. `llm_judge` and `mcp` checks run serially
/// in the same hook, so when both are configured on a stage the additive
/// worst-case latency is
/// `(MAX_JUDGE_CALLS_PER_INVOCATION + MAX_MCP_CALLS_PER_INVOCATION) × 10 s`
/// (= 80 s today), not 40 s. There is intentionally no shared cross-type
/// budget yet — each async check type bounds itself independently and fails
/// open on its own timeout. See TM-DOS-020.
const MAX_MCP_CALLS_PER_INVOCATION: usize = 4;
/// Bound on the stage payload (UTF-8 char-boundary safe) sent to an MCP
/// guardrail endpoint. Mirrors the judge content cap.
const MCP_CONTENT_CAP: usize = 2_000;

/// System prompt sent to the utility LLM for all judge evaluations.
const JUDGE_SYSTEM_PROMPT: &str = r#"You are a guardrail policy evaluator for an AI agent runtime.
Your task: determine whether the provided content violates the given policy.
Respond ONLY with valid JSON on a single line. No explanation, no prose, no markdown.
Format: {"verdict":"allow"} or {"verdict":"block","reason":"<concise reason>"}"#;

/// Evaluate one llm_judge check against `content` via the utility LLM.
/// Returns `Some(GuardrailAction)` on a block/log verdict, `None` on error
/// (fail-open). The caller is responsible for applying advisory-mode
/// downgrade via `compiled.judge_action()`.
async fn run_judge_check(
    service: &dyn crate::UtilityLlmService,
    check: &crate::guardrail_checks::CompiledJudgeCheck,
    stage: GuardrailStage,
    tool_name: &str,
    content: &str,
) -> Option<GuardrailAction> {
    // Bound content sent to judge; find a safe UTF-8 char boundary.
    let content_cap = {
        let mut end = content.len().min(2_000);
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        end
    };
    let user_prompt = format!(
        "Policy: {}\nStage: {}\nTool: {}\nContent:\n{}",
        check.prompt,
        stage.as_str(),
        xml_escape(tool_name),
        &content[..content_cap],
    );
    let request = UtilityLlmRequest::new(vec![
        LlmMessage::text(LlmMessageRole::System, JUDGE_SYSTEM_PROMPT),
        LlmMessage::text(LlmMessageRole::User, user_prompt),
    ])
    .with_reasoning_effort(UtilityLlmReasoningEffort::Low)
    .with_max_tokens(64);

    let response = match tokio::time::timeout(JUDGE_TIMEOUT, service.chat_completion(request)).await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!(
                check = %check.label,
                error = %e,
                "guardrails: judge call failed, failing open"
            );
            return None;
        }
        Err(_) => {
            tracing::warn!(
                check = %check.label,
                "guardrails: judge call timed out, failing open"
            );
            return None;
        }
    };

    let text = response.text.trim();
    let Some(fragment) = json_like_fragment(text) else {
        tracing::warn!(
            check = %check.label,
            raw = %text,
            "guardrails: judge response missing JSON fragment, failing open"
        );
        return None;
    };

    match serde_json::from_str::<serde_json::Value>(fragment) {
        Ok(v) if v.get("verdict").and_then(|v| v.as_str()) == Some("block") => {
            tracing::warn!(
                check = %check.label,
                reason = v.get("reason").and_then(|r| r.as_str()).unwrap_or(""),
                "guardrails: judge verdict block"
            );
            Some(GuardrailAction::Block)
        }
        Ok(_) => Some(GuardrailAction::Log), // "allow" or unrecognized → no-op
        Err(e) => {
            tracing::warn!(
                check = %check.label,
                parse_error = %e,
                raw = %fragment,
                "guardrails: judge response parse failed, failing open"
            );
            None // fail-open
        }
    }
}

/// Timeout for a single moderation classifier call. Fail-open on expiry —
/// a utility-model outage must never wedge a turn.
const MODERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Maximum moderation checks evaluated per finalized message (TM-DOS: bounds
/// added latency on the end-of-message seam).
const MAX_MODERATION_CALLS_PER_INVOCATION: usize = 4;
/// Bytes of assistant text sent to the moderation classifier.
const MODERATION_CONTENT_CAP: usize = 4_000;

/// System prompt sent to the utility LLM for moderation scoring.
const MODERATION_SYSTEM_PROMPT: &str = r#"You are a content-moderation classifier for an AI agent runtime.
Score the provided assistant message for each requested category from 0 to 100, where 0 means clearly absent and 100 means a clear, severe violation.
Respond ONLY with valid JSON on a single line. No explanation, no prose, no markdown.
Format: {"scores":{"<category>":<integer 0-100>, ...}} — include every requested category."#;

/// Evaluate one moderation check against `content` via the utility LLM as a
/// classifier. Returns `Some(GuardrailAction::Block)` when any requested
/// category scores at or above the check's threshold, `Some(Log)` when all
/// score below it, and `None` on any error/timeout/parse failure (fail-open).
/// The caller applies advisory-mode downgrade via `compiled.async_action()`.
async fn run_moderation_check(
    service: &dyn crate::UtilityLlmService,
    check: &crate::guardrail_checks::CompiledModerationCheck,
    content: &str,
) -> Option<GuardrailAction> {
    let payload = truncate_on_char_boundary(content, MODERATION_CONTENT_CAP);
    let user_prompt = format!(
        "Categories: {}\nMessage:\n{}",
        check.categories.join(", "),
        payload,
    );
    let request = UtilityLlmRequest::new(vec![
        LlmMessage::text(LlmMessageRole::System, MODERATION_SYSTEM_PROMPT),
        LlmMessage::text(LlmMessageRole::User, user_prompt),
    ])
    .with_reasoning_effort(UtilityLlmReasoningEffort::Low)
    .with_max_tokens(128);

    let response =
        match tokio::time::timeout(MODERATION_TIMEOUT, service.chat_completion(request)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::warn!(
                    check = %check.label,
                    error = %e,
                    "guardrails: moderation call failed, failing open"
                );
                return None;
            }
            Err(_) => {
                tracing::warn!(
                    check = %check.label,
                    "guardrails: moderation call timed out, failing open"
                );
                return None;
            }
        };

    // Parse the scores object from the first JSON-like fragment.
    let text = response.text.trim();
    let start = text.find('{').unwrap_or(0);
    let end = text.rfind('}').map(|i| i + 1).unwrap_or(text.len());
    let fragment = &text[start..end];
    let parsed = match serde_json::from_str::<serde_json::Value>(fragment) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                check = %check.label,
                parse_error = %e,
                raw = %fragment,
                "guardrails: moderation response parse failed, failing open"
            );
            return None;
        }
    };
    let Some(scores) = parsed.get("scores").and_then(|s| s.as_object()) else {
        tracing::warn!(
            check = %check.label,
            "guardrails: moderation response missing scores field, failing open"
        );
        return None;
    };

    for category in &check.categories {
        // Accept both integer and float scores; ignore unparseable entries.
        let score = scores
            .get(category)
            .and_then(|v| v.as_f64())
            .map(|s| s.round() as i64);
        if let Some(score) = score
            && score >= check.threshold as i64
        {
            tracing::warn!(
                check = %check.label,
                category = %category,
                score,
                threshold = check.threshold,
                "guardrails: moderation category at/over threshold"
            );
            return Some(GuardrailAction::Block);
        }
    }
    Some(GuardrailAction::Log)
}

/// Truncate `content` to at most `cap` bytes on a UTF-8 char boundary.
fn truncate_on_char_boundary(content: &str, cap: usize) -> &str {
    let mut end = content.len().min(cap);
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

/// Extract the first JSON-like object fragment from `text`.
fn json_like_fragment(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?.checked_add(1)?;
    (start < end).then(|| &text[start..end])
}

/// Parse a `{"verdict":"allow"|"block","reason":"..."}` verdict out of a JSON
/// value (or a string holding such JSON). Mirrors the judge verdict shape.
/// Returns `Some(Block)` on an explicit block verdict, `Some(Log)` for allow /
/// unrecognized, and `None` (fail-open) when no verdict can be parsed.
fn parse_verdict(value: &serde_json::Value, label: &str) -> Option<GuardrailAction> {
    // The MCP result may be a JSON object directly, or a string carrying JSON
    // (servers that return text content). Handle both.
    let parsed_owned;
    let verdict_obj = match value {
        serde_json::Value::String(s) => {
            let text = s.trim();
            let Some(fragment) = json_like_fragment(text) else {
                tracing::warn!(
                    check = %label,
                    "guardrails: mcp response missing JSON fragment, failing open"
                );
                return None;
            };
            match serde_json::from_str::<serde_json::Value>(fragment) {
                Ok(v) => {
                    parsed_owned = v;
                    &parsed_owned
                }
                Err(e) => {
                    tracing::warn!(
                        check = %label,
                        parse_error = %e,
                        "guardrails: mcp verdict parse failed, failing open"
                    );
                    return None;
                }
            }
        }
        other => other,
    };
    match verdict_obj.get("verdict").and_then(|v| v.as_str()) {
        Some("block") => {
            tracing::warn!(
                check = %label,
                reason = verdict_obj.get("reason").and_then(|r| r.as_str()).unwrap_or(""),
                "guardrails: mcp verdict block"
            );
            Some(GuardrailAction::Block)
        }
        Some(_) => Some(GuardrailAction::Log), // "allow" → no-op
        None => {
            // No recognizable verdict field — fail open rather than guess.
            tracing::warn!(
                check = %label,
                "guardrails: mcp response missing verdict field, failing open"
            );
            None
        }
    }
}

/// Evaluate one `mcp` check against `content` by calling the configured
/// scoped-MCP guardrail tool. Returns `Some(GuardrailAction)` on a parsed
/// verdict, `None` on any failure (fail-open). The caller applies advisory-mode
/// downgrade via `compiled.async_action()`.
async fn run_mcp_check(
    invoker: &dyn crate::McpToolInvoker,
    check: &crate::guardrail_checks::CompiledMcpCheck,
    stage: GuardrailStage,
    tool_name: &str,
    content: &str,
) -> Option<GuardrailAction> {
    if !everruns_core::mcp_server::is_valid_mcp_server_name(&check.server) {
        tracing::warn!(check = %check.label, "guardrails: ambiguous MCP server prefix, skipping check");
        return None;
    }
    let payload = truncate_on_char_boundary(content, MCP_CONTENT_CAP);
    // The guardrail tool receives a structured payload describing the stage
    // under inspection. Tenant scoping is enforced by the host's per-session
    // connection resolver, which only resolves servers scoped to this session.
    let call = ToolCall {
        id: String::new(),
        name: mcp_tool_name(&check.server, &check.tool),
        arguments: json!({
            "stage": stage.as_str(),
            "tool": tool_name,
            "content": payload,
        }),
    };

    let result = match tokio::time::timeout(MCP_CHECK_TIMEOUT, invoker.invoke(&call)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!(
                check = %check.label,
                error = %e,
                "guardrails: mcp call failed, failing open"
            );
            return None;
        }
        Err(_) => {
            tracing::warn!(
                check = %check.label,
                "guardrails: mcp call timed out, failing open"
            );
            return None;
        }
    };

    // A tool-level error from the endpoint (server not found, transport error)
    // fails open — never block execution on a guardrail outage.
    if let Some(error) = &result.error {
        tracing::warn!(
            check = %check.label,
            error = %error,
            "guardrails: mcp endpoint returned error, failing open"
        );
        return None;
    }
    let Some(value) = &result.result else {
        tracing::warn!(
            check = %check.label,
            "guardrails: mcp endpoint returned no result, failing open"
        );
        return None;
    };
    parse_verdict(value, &check.label)
}

fn xml_escape(s: &str) -> std::borrow::Cow<'_, str> {
    if s.bytes()
        .any(|b| matches!(b, b'<' | b'>' | b'&' | b'\'' | b'"'))
    {
        std::borrow::Cow::Owned(
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\'', "&#39;")
                .replace('"', "&quot;"),
        )
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

/// Compile `config` and return it only when at least one check targets
/// `stage`. Invalid configs (possible only if persisted before validation
/// existed) are logged and treated as no checks — guardrails must never
/// take down the turn pipeline.
fn compile_config_for_stage(
    config: &serde_json::Value,
    stage: GuardrailStage,
) -> Option<Arc<CompiledGuardrails>> {
    let parsed = match GuardrailsConfig::from_value(config).and_then(|c| c.compile()) {
        Ok(compiled) => compiled,
        Err(error) => {
            tracing::warn!(%error, "guardrails: skipping invalid config");
            return None;
        }
    };
    parsed.has_stage(stage).then(|| Arc::new(parsed))
}

// ============================================================================
// Output stage: streaming output guardrail
// ============================================================================

struct DeclarativeOutputGuardrail;

impl OutputGuardrail for DeclarativeOutputGuardrail {
    fn id(&self) -> &str {
        "guardrail_checks"
    }

    fn arm(&self, ctx: &OutputGuardrailContext<'_>) -> Option<Box<dyn OutputGuardrailRun>> {
        let compiled = compile_config_for_stage(ctx.config, GuardrailStage::Output)?;
        Some(Box::new(DeclarativeOutputRun {
            compiled,
            logged: HashSet::new(),
        }))
    }
}

struct DeclarativeOutputRun {
    compiled: Arc<CompiledGuardrails>,
    /// Checks already reported as log-only hits for this stream. Without
    /// this, an advisory hit would re-log on every subsequent delta because
    /// evaluation always sees the full accumulated text.
    logged: HashSet<usize>,
}

impl OutputGuardrailRun for DeclarativeOutputRun {
    fn check(&mut self, accumulated: &str, _delta: &str) -> GuardrailDecision {
        // Evaluating against the full accumulated text keeps matches that
        // span delta boundaries correct. Cost is O(|accumulated|) per delta
        // — same asymptotics the canary guardrail accepts — and bounded by
        // assistant message size.
        let logged = &self.logged;
        let hits = self
            .compiled
            .evaluate(GuardrailStage::Output, accumulated, None, &|i| {
                logged.contains(&i)
            });
        for hit in hits {
            match hit.action {
                GuardrailAction::Block => {
                    tracing::warn!(
                        check = %hit.check_label,
                        reason_code = %hit.reason_code,
                        "guardrails: blocking model output"
                    );
                    return GuardrailDecision::Block(crate::output_guardrail::GuardrailBlock {
                        reason_code: hit.reason_code,
                        replacement: hit
                            .replacement
                            .unwrap_or_else(|| DEFAULT_OUTPUT_REPLACEMENT.to_string()),
                    });
                }
                GuardrailAction::Log => {
                    tracing::warn!(
                        check = %hit.check_label,
                        reason_code = %hit.reason_code,
                        "guardrails: output check hit (log only)"
                    );
                    self.logged.insert(hit.check_index);
                }
            }
        }
        GuardrailDecision::Pass
    }
}

// ============================================================================
// Output stage: end-of-message moderation seam (EVE-573)
// ============================================================================

/// Async, model-backed output guardrail run once on the finalized assistant
/// message. Holds the compiled output-stage moderation checks; the streaming
/// deterministic checks are handled separately by `DeclarativeOutputGuardrail`.
struct ModerationOutputGuardrail {
    compiled: Arc<CompiledGuardrails>,
}

#[async_trait]
impl PostGenerationOutputGuardrail for ModerationOutputGuardrail {
    fn id(&self) -> &str {
        "moderation"
    }

    async fn check_message(&self, ctx: &PostGenerationOutputContext<'_>) -> GuardrailDecision {
        // Model-backed: needs the utility LLM. Without it, fail open.
        let Some(service) = ctx.utility_llm_service else {
            tracing::warn!(
                "guardrails: moderation skipped — no utility LLM service available (fail-open)"
            );
            return GuardrailDecision::Pass;
        };
        if !service.is_configured() {
            tracing::warn!(
                "guardrails: moderation skipped — utility LLM not configured (fail-open)"
            );
            return GuardrailDecision::Pass;
        }

        for (calls, check) in self
            .compiled
            .moderation_checks_for_stage(GuardrailStage::Output)
            .enumerate()
        {
            if calls >= MAX_MODERATION_CALLS_PER_INVOCATION {
                tracing::warn!(
                    "guardrails: moderation call cap reached ({MAX_MODERATION_CALLS_PER_INVOCATION}); \
                     remaining output checks skipped (fail-open)"
                );
                break;
            }
            let Some(raw_action) =
                run_moderation_check(service.as_ref(), check, ctx.message_text).await
            else {
                continue; // fail-open on error/timeout/parse failure
            };
            if raw_action != GuardrailAction::Block {
                continue; // scored below threshold → allow
            }
            // Apply advisory-mode / on_fail downgrade.
            match self.compiled.async_action(check.on_fail) {
                GuardrailAction::Block => {
                    tracing::warn!(
                        check = %check.label,
                        reason_code = "guardrail.moderation",
                        "guardrails: blocking model output (moderation)"
                    );
                    return GuardrailDecision::Block(crate::output_guardrail::GuardrailBlock {
                        reason_code: "guardrail.moderation".to_string(),
                        replacement: check
                            .replacement
                            .clone()
                            .unwrap_or_else(|| DEFAULT_OUTPUT_REPLACEMENT.to_string()),
                    });
                }
                GuardrailAction::Log => {
                    tracing::warn!(
                        check = %check.label,
                        reason_code = "guardrail.moderation",
                        "guardrails: moderation hit (log only)"
                    );
                }
            }
        }
        GuardrailDecision::Pass
    }
}

// ============================================================================
// Tool-use stage: pre-tool hook
// ============================================================================

struct GuardrailPreToolHook {
    compiled: Arc<CompiledGuardrails>,
}

#[async_trait]
impl PreToolUseHook for GuardrailPreToolHook {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision {
        // tool_pattern rules match the tool name; regex/blocklist rules
        // match the serialized arguments.
        let args_text = tool_call.arguments.to_string();
        let hits = self.compiled.evaluate(
            GuardrailStage::ToolUse,
            &args_text,
            Some(&tool_call.name),
            &|_| false,
        );
        for hit in hits {
            match hit.action {
                GuardrailAction::Block => {
                    tracing::warn!(
                        check = %hit.check_label,
                        reason_code = %hit.reason_code,
                        tool = %tool_call.name,
                        "guardrails: blocking tool call"
                    );
                    return PreToolUseDecision::Block {
                        tool_call,
                        reason: format!(
                            "Tool call blocked by guardrail check '{}' ({})",
                            hit.check_label, hit.reason_code
                        ),
                        user_message: hit.replacement,
                    };
                }
                GuardrailAction::Log => {
                    tracing::warn!(
                        check = %hit.check_label,
                        reason_code = %hit.reason_code,
                        tool = %tool_call.name,
                        "guardrails: tool call check hit (log only)"
                    );
                }
            }
        }
        // LLM-judge checks run after deterministic checks; skipped when
        // the utility LLM is absent or disabled.
        if let Some(service) = &context.utility_llm_service
            && service.is_configured()
        {
            for (calls, check) in self
                .compiled
                .judge_checks_for_stage(GuardrailStage::ToolUse)
                .enumerate()
            {
                if calls >= MAX_JUDGE_CALLS_PER_INVOCATION {
                    tracing::warn!(
                        tool = %tool_call.name,
                        "guardrails: judge call cap reached for tool_use, skipping remaining"
                    );
                    break;
                }
                let Some(raw_action) = run_judge_check(
                    service.as_ref(),
                    check,
                    GuardrailStage::ToolUse,
                    &tool_call.name,
                    &args_text,
                )
                .await
                else {
                    continue; // fail-open
                };
                let action = self.compiled.judge_action(check.on_fail);
                match action {
                    GuardrailAction::Block if raw_action == GuardrailAction::Block => {
                        tracing::warn!(
                            check = %check.label,
                            tool = %tool_call.name,
                            "guardrails: judge blocking tool call"
                        );
                        return PreToolUseDecision::Block {
                            tool_call,
                            reason: format!(
                                "Tool call blocked by guardrail check '{}' (guardrail.llm_judge)",
                                check.label
                            ),
                            user_message: check.replacement.clone(),
                        };
                    }
                    _ => {
                        if raw_action == GuardrailAction::Block {
                            tracing::warn!(
                                check = %check.label,
                                tool = %tool_call.name,
                                "guardrails: judge hit (log only)"
                            );
                        }
                    }
                }
            }
        }
        // MCP-served checks run after judge checks; skipped when no scoped-MCP
        // invoker is wired into the context.
        if let Some(invoker) = &context.mcp_invoker {
            for (calls, check) in self
                .compiled
                .mcp_checks_for_stage(GuardrailStage::ToolUse)
                .enumerate()
            {
                if calls >= MAX_MCP_CALLS_PER_INVOCATION {
                    tracing::warn!(
                        tool = %tool_call.name,
                        "guardrails: mcp call cap reached for tool_use, skipping remaining"
                    );
                    break;
                }
                let Some(raw_action) = run_mcp_check(
                    invoker.as_ref(),
                    check,
                    GuardrailStage::ToolUse,
                    &tool_call.name,
                    &args_text,
                )
                .await
                else {
                    continue; // fail-open
                };
                let action = self.compiled.async_action(check.on_fail);
                match action {
                    GuardrailAction::Block if raw_action == GuardrailAction::Block => {
                        tracing::warn!(
                            check = %check.label,
                            tool = %tool_call.name,
                            "guardrails: mcp blocking tool call"
                        );
                        return PreToolUseDecision::Block {
                            tool_call,
                            reason: format!(
                                "Tool call blocked by guardrail check '{}' (guardrail.mcp)",
                                check.label
                            ),
                            user_message: check.replacement.clone(),
                        };
                    }
                    _ => {
                        if raw_action == GuardrailAction::Block {
                            tracing::warn!(
                                check = %check.label,
                                tool = %tool_call.name,
                                "guardrails: mcp hit (log only)"
                            );
                        }
                    }
                }
            }
        }
        PreToolUseDecision::Continue(tool_call)
    }
}

// ============================================================================
// Tool-output stage: post-tool hook
// ============================================================================

struct GuardrailPostToolHook {
    compiled: Arc<CompiledGuardrails>,
}

#[async_trait]
impl PostToolExecHook for GuardrailPostToolHook {
    fn priority(&self) -> PostToolExecHookPriority {
        PostToolExecHookPriority::Guardrail
    }

    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    ) {
        let mut haystack = String::new();
        if let Some(value) = &result.result {
            match value {
                serde_json::Value::String(s) => haystack.push_str(s),
                other => haystack.push_str(&other.to_string()),
            }
        }
        if let Some(error) = &result.error {
            haystack.push('\n');
            haystack.push_str(error);
        }
        // Exec-style tools budget the visible `result` JSON but keep the full,
        // untruncated content in `raw_output`, which is persisted to `/outputs`.
        // Include it in the haystack so sensitive content that only survives in
        // `raw_output` is caught before this hook clears it on a block.
        if let Some(raw_output) = &result.raw_output {
            haystack.push('\n');
            haystack.push_str(raw_output);
        }
        if haystack.is_empty() {
            return;
        }
        let hits = self
            .compiled
            .evaluate(GuardrailStage::ToolOutput, &haystack, None, &|_| false);
        for hit in hits {
            match hit.action {
                GuardrailAction::Block => {
                    tracing::warn!(
                        check = %hit.check_label,
                        reason_code = %hit.reason_code,
                        tool = %tool_call.name,
                        "guardrails: withholding tool output"
                    );
                    let notice = hit
                        .replacement
                        .unwrap_or_else(|| DEFAULT_TOOL_OUTPUT_REPLACEMENT.to_string());
                    // The original content never reaches model context.
                    result.result = Some(serde_json::Value::String(notice));
                    result.error = None;
                    result.images = None;
                    result.raw_output = None;
                    return;
                }
                GuardrailAction::Log => {
                    tracing::warn!(
                        check = %hit.check_label,
                        reason_code = %hit.reason_code,
                        tool = %tool_call.name,
                        "guardrails: tool output check hit (log only)"
                    );
                }
            }
        }
        // LLM-judge checks for tool_output; skipped when utility LLM is absent or disabled.
        if let Some(service) = &context.utility_llm_service
            && service.is_configured()
        {
            for (calls, check) in self
                .compiled
                .judge_checks_for_stage(GuardrailStage::ToolOutput)
                .enumerate()
            {
                if calls >= MAX_JUDGE_CALLS_PER_INVOCATION {
                    tracing::warn!(
                        tool = %tool_call.name,
                        "guardrails: judge call cap reached for tool_output, skipping remaining"
                    );
                    break;
                }
                let Some(raw_action) = run_judge_check(
                    service.as_ref(),
                    check,
                    GuardrailStage::ToolOutput,
                    &tool_call.name,
                    &haystack,
                )
                .await
                else {
                    continue; // fail-open
                };
                let action = self.compiled.judge_action(check.on_fail);
                match action {
                    GuardrailAction::Block if raw_action == GuardrailAction::Block => {
                        tracing::warn!(
                            check = %check.label,
                            tool = %tool_call.name,
                            "guardrails: judge withholding tool output"
                        );
                        let notice = check
                            .replacement
                            .clone()
                            .unwrap_or_else(|| DEFAULT_TOOL_OUTPUT_REPLACEMENT.to_string());
                        result.result = Some(serde_json::Value::String(notice));
                        result.error = None;
                        result.images = None;
                        result.raw_output = None;
                        return;
                    }
                    _ => {
                        if raw_action == GuardrailAction::Block {
                            tracing::warn!(
                                check = %check.label,
                                tool = %tool_call.name,
                                "guardrails: judge tool_output hit (log only)"
                            );
                        }
                    }
                }
            }
        }
        // MCP-served checks for tool_output; skipped when no scoped-MCP invoker
        // is wired into the context.
        if let Some(invoker) = &context.mcp_invoker {
            for (calls, check) in self
                .compiled
                .mcp_checks_for_stage(GuardrailStage::ToolOutput)
                .enumerate()
            {
                if calls >= MAX_MCP_CALLS_PER_INVOCATION {
                    tracing::warn!(
                        tool = %tool_call.name,
                        "guardrails: mcp call cap reached for tool_output, skipping remaining"
                    );
                    break;
                }
                let Some(raw_action) = run_mcp_check(
                    invoker.as_ref(),
                    check,
                    GuardrailStage::ToolOutput,
                    &tool_call.name,
                    &haystack,
                )
                .await
                else {
                    continue; // fail-open
                };
                let action = self.compiled.async_action(check.on_fail);
                match action {
                    GuardrailAction::Block if raw_action == GuardrailAction::Block => {
                        tracing::warn!(
                            check = %check.label,
                            tool = %tool_call.name,
                            "guardrails: mcp withholding tool output"
                        );
                        let notice = check
                            .replacement
                            .clone()
                            .unwrap_or_else(|| DEFAULT_TOOL_OUTPUT_REPLACEMENT.to_string());
                        result.result = Some(serde_json::Value::String(notice));
                        result.error = None;
                        result.images = None;
                        result.raw_output = None;
                        return;
                    }
                    _ => {
                        if raw_action == GuardrailAction::Block {
                            tracing::warn!(
                                check = %check.label,
                                tool = %tool_call.name,
                                "guardrails: mcp tool_output hit (log only)"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;
    use crate::utility_llm::UtilityLlmService;
    use crate::{AgentLoopError, LlmCompletionMetadata, LlmResponse, LlmResponseStream};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    /// Stub utility LLM that returns a fixed verdict string.
    struct StubJudge {
        response: String,
    }

    impl StubJudge {
        fn block() -> Arc<Self> {
            Arc::new(Self {
                response: r#"{"verdict":"block","reason":"test"}"#.to_string(),
            })
        }
        fn allow() -> Arc<Self> {
            Arc::new(Self {
                response: r#"{"verdict":"allow"}"#.to_string(),
            })
        }
        fn malformed(response: &str) -> Arc<Self> {
            Arc::new(Self {
                response: response.to_string(),
            })
        }

        fn error() -> Arc<Self> {
            Arc::new(Self {
                response: "".to_string(), // unused; chat_completion errors
            })
        }
    }

    #[async_trait]
    impl UtilityLlmService for StubJudge {
        fn is_configured(&self) -> bool {
            true
        }

        async fn chat_completion(
            &self,
            _request: crate::utility_llm::UtilityLlmRequest,
        ) -> crate::Result<LlmResponse> {
            if self.response.is_empty() {
                return Err(AgentLoopError::llm("stub error"));
            }
            Ok(LlmResponse {
                text: self.response.clone(),
                reasoning: Vec::new(),
                tool_calls: None,
                metadata: LlmCompletionMetadata {
                    total_tokens: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    provider_cost_usd: None,
                    model: None,
                    finish_reason: None,
                    retry_metadata: None,
                    response_id: None,
                    phase: None,
                    cache_diagnostics: None,
                },
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: crate::utility_llm::UtilityLlmRequest,
        ) -> crate::Result<LlmResponseStream> {
            Err(AgentLoopError::llm("stub: no stream"))
        }
    }

    // ---- Moderation output seam (EVE-573) ----

    fn moderation_config(threshold: u8, on_fail: &str, mode: &str) -> serde_json::Value {
        json!({
            "mode": mode,
            "checks": [{
                "stage": "output",
                "type": "moderation",
                "threshold": threshold,
                "on_fail": on_fail,
            }]
        })
    }

    async fn run_moderation_seam(
        config: &serde_json::Value,
        service: Option<Arc<dyn UtilityLlmService>>,
        text: &str,
    ) -> GuardrailDecision {
        let providers = GuardrailsCapability.post_output_guardrails_with_config(config);
        let provider = providers
            .into_iter()
            .next()
            .expect("moderation provider should be contributed");
        let ctx = PostGenerationOutputContext {
            system_prompt: "",
            message_text: text,
            utility_llm_service: service.as_ref(),
        };
        provider.check_message(&ctx).await
    }

    fn scores(json_body: &str) -> Arc<StubJudge> {
        Arc::new(StubJudge {
            response: json_body.to_string(),
        })
    }

    #[tokio::test]
    async fn moderation_blocks_when_category_at_or_over_threshold() {
        let config = moderation_config(50, "block", "active");
        let svc: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"hate":90,"violence":0}}"#);
        let decision = run_moderation_seam(&config, Some(svc), "bad text").await;
        match decision {
            GuardrailDecision::Block(b) => {
                assert_eq!(b.reason_code, "guardrail.moderation");
                assert_eq!(b.replacement, DEFAULT_OUTPUT_REPLACEMENT);
            }
            GuardrailDecision::Pass => panic!("expected block"),
        }
    }

    #[tokio::test]
    async fn moderation_blocks_exactly_at_threshold() {
        let config = moderation_config(50, "block", "active");
        let svc: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"hate":50}}"#);
        assert!(matches!(
            run_moderation_seam(&config, Some(svc), "x").await,
            GuardrailDecision::Block(_)
        ));
    }

    #[tokio::test]
    async fn moderation_allows_when_below_threshold() {
        let config = moderation_config(50, "block", "active");
        let svc: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"hate":10,"violence":5}}"#);
        assert!(matches!(
            run_moderation_seam(&config, Some(svc), "fine").await,
            GuardrailDecision::Pass
        ));
    }

    #[tokio::test]
    async fn moderation_advisory_logs_without_blocking() {
        let config = moderation_config(50, "block", "advisory");
        let svc: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"hate":99}}"#);
        assert!(matches!(
            run_moderation_seam(&config, Some(svc), "x").await,
            GuardrailDecision::Pass
        ));
    }

    #[tokio::test]
    async fn moderation_on_fail_log_does_not_block() {
        let config = moderation_config(50, "log", "active");
        let svc: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"hate":99}}"#);
        assert!(matches!(
            run_moderation_seam(&config, Some(svc), "x").await,
            GuardrailDecision::Pass
        ));
    }

    #[tokio::test]
    async fn moderation_uses_custom_replacement() {
        let config = json!({
            "checks": [{
                "stage": "output",
                "type": "moderation",
                "threshold": 50,
                "replacement": "[removed by policy]",
            }]
        });
        let svc: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"hate":80}}"#);
        match run_moderation_seam(&config, Some(svc), "x").await {
            GuardrailDecision::Block(b) => assert_eq!(b.replacement, "[removed by policy]"),
            GuardrailDecision::Pass => panic!("expected block"),
        }
    }

    #[tokio::test]
    async fn moderation_fails_open_on_service_error() {
        let config = moderation_config(50, "block", "active");
        let svc: Arc<dyn UtilityLlmService> = StubJudge::error();
        assert!(matches!(
            run_moderation_seam(&config, Some(svc), "x").await,
            GuardrailDecision::Pass
        ));
    }

    #[tokio::test]
    async fn moderation_fails_open_without_utility_service() {
        let config = moderation_config(50, "block", "active");
        assert!(matches!(
            run_moderation_seam(&config, None, "x").await,
            GuardrailDecision::Pass
        ));
    }

    #[tokio::test]
    async fn moderation_fails_open_on_unparseable_response() {
        let config = moderation_config(50, "block", "active");
        let svc: Arc<dyn UtilityLlmService> = scores("not json at all");
        assert!(matches!(
            run_moderation_seam(&config, Some(svc), "x").await,
            GuardrailDecision::Pass
        ));
    }

    #[tokio::test]
    async fn moderation_defaults_categories_when_unspecified() {
        // No `categories` → built-in set is used; the model scoring a default
        // category over threshold still trips.
        let config = moderation_config(50, "block", "active");
        let svc: Arc<dyn UtilityLlmService> = scores(r#"{"scores":{"self_harm":75}}"#);
        assert!(matches!(
            run_moderation_seam(&config, Some(svc), "x").await,
            GuardrailDecision::Block(_)
        ));
    }

    #[test]
    fn no_moderation_provider_without_output_moderation_check() {
        // A config with only a deterministic output check contributes no
        // post-generation provider (that check runs on the streaming seam).
        let config = json!({
            "checks": [{
                "stage": "output",
                "type": "regex",
                "patterns": ["secret"],
            }]
        });
        assert!(
            GuardrailsCapability
                .post_output_guardrails_with_config(&config)
                .is_empty()
        );
        // Empty config: no provider either.
        assert!(
            GuardrailsCapability
                .post_output_guardrails_with_config(&json!({}))
                .is_empty()
        );
    }

    #[test]
    fn moderation_rejected_on_tool_stages() {
        for stage in ["tool_use", "tool_output"] {
            let config = json!({
                "checks": [{ "stage": stage, "type": "moderation", "threshold": 50 }]
            });
            assert!(
                GuardrailsConfig::from_value(&config)
                    .unwrap()
                    .compile()
                    .is_err(),
                "moderation on {stage} should be rejected"
            );
        }
    }

    /// What a stubbed MCP guardrail endpoint should do for a call.
    enum McpBehavior {
        /// Return a result `Value` (object or JSON string).
        Result(serde_json::Value),
        /// Return a tool-level error.
        Error(String),
        /// Never respond in time (sleep past the timeout).
        Timeout,
        /// Return the call's `content` argument back as the verdict-bearing
        /// result string — used to assert payload truncation.
        EchoContent,
    }

    /// Stub scoped-MCP invoker returning a fixed verdict, recording calls.
    struct StubMcpInvoker {
        behavior: McpBehavior,
        last_call: std::sync::Mutex<Option<ToolCall>>,
    }

    impl StubMcpInvoker {
        fn new(behavior: McpBehavior) -> Arc<Self> {
            Arc::new(Self {
                behavior,
                last_call: std::sync::Mutex::new(None),
            })
        }
        fn block() -> Arc<Self> {
            Self::new(McpBehavior::Result(
                json!({"verdict": "block", "reason": "test"}),
            ))
        }
        fn allow() -> Arc<Self> {
            Self::new(McpBehavior::Result(json!({"verdict": "allow"})))
        }
    }

    #[async_trait]
    impl crate::McpToolInvoker for StubMcpInvoker {
        async fn invoke(&self, tool_call: &ToolCall) -> crate::Result<ToolResult> {
            *self.last_call.lock().unwrap() = Some(tool_call.clone());
            let result = match &self.behavior {
                McpBehavior::Result(v) => ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: Some(v.clone()),
                    images: None,
                    error: None,
                    connection_required: None,
                    raw_output: None,
                },
                McpBehavior::Error(msg) => {
                    return Err(AgentLoopError::tool(msg.clone()));
                }
                McpBehavior::Timeout => {
                    tokio::time::sleep(MCP_CHECK_TIMEOUT + std::time::Duration::from_secs(2)).await;
                    unreachable!("timeout fires before sleep completes")
                }
                McpBehavior::EchoContent => {
                    let content = tool_call.arguments["content"].as_str().unwrap_or_default();
                    ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        // Not a valid verdict — fails open — but the recorded
                        // call's content is what the truncation test inspects.
                        result: Some(serde_json::Value::String(content.to_string())),
                        images: None,
                        error: None,
                        connection_required: None,
                        raw_output: None,
                    }
                }
            };
            Ok(result)
        }
    }

    fn tool_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            name: name.to_string(),
            arguments: args,
        }
    }

    fn tool_def() -> ToolDefinition {
        ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: "test_tool".to_string(),
            display_name: None,
            description: "test".to_string(),
            parameters: json!({}),
            policy: crate::tool_types::ToolPolicy::Auto,
            category: None,
            deferrable: crate::tool_types::DeferrablePolicy::Never,
            hints: Default::default(),
            full_parameters: None,
        })
    }

    fn arm_output(config: serde_json::Value) -> Option<Box<dyn OutputGuardrailRun>> {
        let ctx = OutputGuardrailContext {
            system_prompt: "irrelevant",
            config: &config,
        };
        DeclarativeOutputGuardrail.arm(&ctx)
    }

    #[test]
    fn validate_config_accepts_valid_and_rejects_invalid() {
        let cap = GuardrailsCapability;
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(
            cap.validate_config(&json!({
                "checks": [{"stage": "output", "type": "blocklist", "words": ["x"]}]
            }))
            .is_ok()
        );
        assert!(
            cap.validate_config(&json!({
                "checks": [{"stage": "output", "type": "regex", "patterns": ["("]}]
            }))
            .is_err()
        );
    }

    #[test]
    fn output_guardrail_declines_to_arm_without_output_checks() {
        assert!(arm_output(json!({})).is_none());
        assert!(
            arm_output(json!({
                "checks": [{"stage": "tool_use", "type": "tool_pattern", "tools": ["bash*"]}]
            }))
            .is_none()
        );
    }

    #[test]
    fn output_guardrail_blocks_with_custom_replacement() {
        let mut run = arm_output(json!({
            "checks": [{
                "stage": "output", "type": "blocklist", "words": ["forbidden"],
                "replacement": "nope"
            }]
        }))
        .expect("armed");
        assert!(matches!(
            run.check("all good here", "here"),
            GuardrailDecision::Pass
        ));
        match run.check("this is forbidden text", " text") {
            GuardrailDecision::Block(b) => {
                assert_eq!(b.reason_code, "guardrail.blocklist");
                assert_eq!(b.replacement, "nope");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn output_guardrail_advisory_logs_once_and_passes() {
        let mut run = arm_output(json!({
            "mode": "advisory",
            "checks": [{"stage": "output", "type": "blocklist", "words": ["forbidden"]}]
        }))
        .expect("armed");
        assert!(matches!(
            run.check("forbidden", "forbidden"),
            GuardrailDecision::Pass
        ));
        // Subsequent deltas keep passing (and the hit is not re-reported).
        assert!(matches!(
            run.check("forbidden and more", " and more"),
            GuardrailDecision::Pass
        ));
    }

    #[tokio::test]
    async fn pre_tool_hook_blocks_matching_tool_name() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{
                "stage": "tool_use", "type": "tool_pattern", "tools": ["bash*"],
                "replacement": "Shell access is not allowed for this agent."
            }]
        }));
        assert_eq!(hooks.len(), 1);
        let ctx = ToolContext::new(SessionId::new());
        let decision = hooks[0]
            .before_exec(
                tool_call("bashkit_exec", json!({"cmd": "ls"})),
                &tool_def(),
                &ctx,
            )
            .await;
        match decision {
            PreToolUseDecision::Block {
                reason,
                user_message,
                ..
            } => {
                assert!(reason.contains("guardrail"), "{reason}");
                assert_eq!(
                    user_message.as_deref(),
                    Some("Shell access is not allowed for this agent.")
                );
            }
            other => panic!("expected Block, got {other:?}"),
        }
        let decision = hooks[0]
            .before_exec(tool_call("read_file", json!({})), &tool_def(), &ctx)
            .await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    }

    #[tokio::test]
    async fn pre_tool_hook_matches_arguments_with_regex() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{
                "stage": "tool_use", "type": "regex",
                "patterns": ["(?i)drop\\s+table"]
            }]
        }));
        let ctx = ToolContext::new(SessionId::new());
        let decision = hooks[0]
            .before_exec(
                tool_call("sql_query", json!({"query": "DROP TABLE users"})),
                &tool_def(),
                &ctx,
            )
            .await;
        assert!(matches!(decision, PreToolUseDecision::Block { .. }));
    }

    #[tokio::test]
    async fn pre_tool_hook_advisory_continues() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "mode": "advisory",
            "checks": [{"stage": "tool_use", "type": "tool_pattern", "tools": ["bash*"]}]
        }));
        let ctx = ToolContext::new(SessionId::new());
        let decision = hooks[0]
            .before_exec(tool_call("bashkit_exec", json!({})), &tool_def(), &ctx)
            .await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    }

    #[tokio::test]
    async fn post_tool_hook_withholds_matching_output() {
        let cap = GuardrailsCapability;
        let hooks = cap.post_tool_exec_hooks_with_config(&json!({
            "checks": [{
                "id": "aws_key", "stage": "tool_output", "type": "regex",
                "patterns": ["AKIA[0-9A-Z]{16}"]
            }]
        }));
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].priority(), PostToolExecHookPriority::Guardrail);
        let ctx = ToolContext::new(SessionId::new());
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("key is AKIAIOSFODNN7EXAMPLE ok")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        hooks[0]
            .after_exec(
                &tool_call("web_fetch", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;
        assert_eq!(
            result.result,
            Some(json!(DEFAULT_TOOL_OUTPUT_REPLACEMENT)),
            "matched output must be replaced with the notice"
        );
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn post_tool_hook_scans_raw_output_persistence_surface() {
        // Exec-style tools keep the full, untruncated content in `raw_output`
        // (persisted to /outputs) while the visible `result` is budgeted. A
        // secret that only survives in `raw_output` must still be blocked.
        let cap = GuardrailsCapability;
        let hooks = cap.post_tool_exec_hooks_with_config(&json!({
            "checks": [{
                "id": "aws_key", "stage": "tool_output", "type": "regex",
                "patterns": ["AKIA[0-9A-Z]{16}"]
            }]
        }));
        let ctx = ToolContext::new(SessionId::new());
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("(truncated output)")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: Some("full log: AKIAIOSFODNN7EXAMPLE trailing".to_string()),
        };
        hooks[0]
            .after_exec(
                &tool_call("bashkit_exec", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;
        assert_eq!(
            result.result,
            Some(json!(DEFAULT_TOOL_OUTPUT_REPLACEMENT)),
            "a secret only present in raw_output must trigger the block"
        );
        assert!(
            result.raw_output.is_none(),
            "raw_output must be cleared on a block so it is not persisted"
        );
    }

    #[tokio::test]
    async fn post_tool_hook_leaves_clean_output_untouched() {
        let cap = GuardrailsCapability;
        let hooks = cap.post_tool_exec_hooks_with_config(&json!({
            "checks": [{"stage": "tool_output", "type": "blocklist", "words": ["secret"]}]
        }));
        let ctx = ToolContext::new(SessionId::new());
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("nothing to see")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        hooks[0]
            .after_exec(
                &tool_call("web_fetch", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;
        assert_eq!(result.result, Some(json!("nothing to see")));
    }

    #[test]
    fn no_hooks_contributed_without_matching_stage_checks() {
        let cap = GuardrailsCapability;
        assert!(cap.pre_tool_use_hooks_with_config(&json!({})).is_empty());
        assert!(cap.post_tool_exec_hooks_with_config(&json!({})).is_empty());
        let output_only = json!({
            "checks": [{"stage": "output", "type": "blocklist", "words": ["x"]}]
        });
        assert!(cap.pre_tool_use_hooks_with_config(&output_only).is_empty());
        assert!(
            cap.post_tool_exec_hooks_with_config(&output_only)
                .is_empty()
        );
    }

    #[test]
    fn capability_metadata() {
        let cap = GuardrailsCapability;
        assert_eq!(cap.id(), GUARDRAILS_CAPABILITY_ID);
        assert!(cap.is_guardrail());
        assert!(cap.config_schema().is_some());
        assert_eq!(cap.output_guardrails().len(), 1);
    }

    // --- llm_judge hook tests ---

    #[tokio::test]
    async fn judge_pre_tool_hook_blocks_when_judge_says_block() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{"stage": "tool_use", "type": "llm_judge",
                        "prompt": "Block requests to delete data."}]
        }));
        assert_eq!(hooks.len(), 1);
        let ctx = ToolContext::new(SessionId::new()).with_utility_llm_service(StubJudge::block());
        let decision = hooks[0]
            .before_exec(
                tool_call("delete_record", json!({"id": 42})),
                &tool_def(),
                &ctx,
            )
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Block { .. }),
            "judge block verdict should block the tool call"
        );
    }

    #[tokio::test]
    async fn judge_pre_tool_hook_continues_when_judge_says_allow() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{"stage": "tool_use", "type": "llm_judge",
                        "prompt": "Block requests to delete data."}]
        }));
        let ctx = ToolContext::new(SessionId::new()).with_utility_llm_service(StubJudge::allow());
        let decision = hooks[0]
            .before_exec(tool_call("read_file", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "judge allow verdict should continue"
        );
    }

    #[tokio::test]
    async fn judge_pre_tool_hook_fails_open_on_error() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{"stage": "tool_use", "type": "llm_judge",
                        "prompt": "Block bad things."}]
        }));
        let ctx = ToolContext::new(SessionId::new()).with_utility_llm_service(StubJudge::error());
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "judge error must fail open"
        );
    }

    #[tokio::test]
    async fn judge_pre_tool_hook_fails_open_on_malformed_output() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{"stage": "tool_use", "type": "llm_judge",
                        "prompt": "Block bad things."}]
        }));

        for malformed in ["} bad {", "not json", "{unterminated"] {
            let ctx = ToolContext::new(SessionId::new())
                .with_utility_llm_service(StubJudge::malformed(malformed));
            let decision = hooks[0]
                .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
                .await;
            assert!(
                matches!(decision, PreToolUseDecision::Continue(_)),
                "malformed judge output must fail open: {malformed:?}"
            );
        }
    }

    #[tokio::test]
    async fn judge_pre_tool_hook_skipped_without_utility_llm() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{"stage": "tool_use", "type": "llm_judge",
                        "prompt": "Block everything."}]
        }));
        // No utility LLM service configured → judge checks are skipped
        let ctx = ToolContext::new(SessionId::new());
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "without utility LLM, judge checks are silently skipped"
        );
    }

    #[tokio::test]
    async fn judge_pre_tool_hook_skipped_when_service_not_configured() {
        // Service is present in the context but reports is_configured() == false
        // (e.g. DisabledUtilityLlmService). Judge checks must be silently skipped.
        use crate::utility_llm::DisabledUtilityLlmService;
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{"stage": "tool_use", "type": "llm_judge",
                        "prompt": "Block everything."}]
        }));
        let ctx = ToolContext::new(SessionId::new())
            .with_utility_llm_service(Arc::new(DisabledUtilityLlmService));
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "disabled utility LLM service must skip judge checks without warn logs"
        );
    }

    #[tokio::test]
    async fn judge_advisory_mode_continues_even_on_block_verdict() {
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "mode": "advisory",
            "checks": [{"stage": "tool_use", "type": "llm_judge",
                        "prompt": "Block everything.", "on_fail": "block"}]
        }));
        let ctx = ToolContext::new(SessionId::new()).with_utility_llm_service(StubJudge::block());
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "advisory mode must not block even when judge says block"
        );
    }

    #[tokio::test]
    async fn judge_post_tool_hook_withholds_output_on_block() {
        let cap = GuardrailsCapability;
        let hooks = cap.post_tool_exec_hooks_with_config(&json!({
            "checks": [{"stage": "tool_output", "type": "llm_judge",
                        "prompt": "Block PII in tool output."}]
        }));
        assert_eq!(hooks.len(), 1);
        let ctx = ToolContext::new(SessionId::new()).with_utility_llm_service(StubJudge::block());
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("user email: alice@example.com")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        hooks[0]
            .after_exec(
                &tool_call("web_fetch", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;
        assert_eq!(
            result.result,
            Some(json!(DEFAULT_TOOL_OUTPUT_REPLACEMENT)),
            "judge block should replace tool output with notice"
        );
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn judge_post_tool_hook_passes_clean_output_on_allow() {
        let cap = GuardrailsCapability;
        let hooks = cap.post_tool_exec_hooks_with_config(&json!({
            "checks": [{"stage": "tool_output", "type": "llm_judge",
                        "prompt": "Block PII."}]
        }));
        let ctx = ToolContext::new(SessionId::new()).with_utility_llm_service(StubJudge::allow());
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("no pii here")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        hooks[0]
            .after_exec(
                &tool_call("web_fetch", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;
        assert_eq!(result.result, Some(json!("no pii here")));
    }

    #[tokio::test]
    async fn judge_handles_multibyte_content_without_panic() {
        // Verifies the 2 000-byte content cap doesn't slice mid-char.
        let cap = GuardrailsCapability;
        let hooks = cap.pre_tool_use_hooks_with_config(&json!({
            "checks": [{"stage": "tool_use", "type": "llm_judge", "prompt": "p"}]
        }));
        let ctx = ToolContext::new(SessionId::new()).with_utility_llm_service(StubJudge::allow());
        // 700 × 3-byte chars = 2 100 bytes, boundary at 2 000 falls mid-char
        let multibyte_args = "€".repeat(700);
        let decision = hooks[0]
            .before_exec(
                tool_call("any_tool", json!({"x": multibyte_args})),
                &tool_def(),
                &ctx,
            )
            .await;
        // Just must not panic; allow verdict continues
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    }

    #[test]
    fn xml_escape_escapes_special_chars() {
        assert_eq!(xml_escape("normal"), "normal");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape("a&b"), "a&amp;b");
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(xml_escape("it's"), "it&#39;s");
    }

    #[test]
    fn config_schema_includes_llm_judge() {
        let cap = GuardrailsCapability;
        let schema = cap.config_schema().unwrap();
        let type_enum = &schema["properties"]["checks"]["items"]["properties"]["type"]["enum"];
        let values: Vec<&str> = type_enum
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            values.contains(&"llm_judge"),
            "schema enum must include llm_judge"
        );
    }

    // --- mcp hook tests ---

    #[test]
    fn config_schema_includes_mcp() {
        let cap = GuardrailsCapability;
        let schema = cap.config_schema().unwrap();
        let type_enum = &schema["properties"]["checks"]["items"]["properties"]["type"]["enum"];
        let values: Vec<&str> = type_enum
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(values.contains(&"mcp"), "schema enum must include mcp");
        let props = &schema["properties"]["checks"]["items"]["properties"];
        assert!(props["server"].is_object(), "schema must define server");
        assert!(props["tool"].is_object(), "schema must define tool");
    }

    fn mcp_pre_hooks(on_fail: &str, mode: &str) -> Vec<Arc<dyn PreToolUseHook>> {
        GuardrailsCapability.pre_tool_use_hooks_with_config(&json!({
            "mode": mode,
            "checks": [{"stage": "tool_use", "type": "mcp",
                        "server": "guard", "tool": "screen", "on_fail": on_fail}]
        }))
    }

    #[tokio::test]
    async fn mcp_pre_tool_hook_blocks_when_endpoint_says_block() {
        let hooks = mcp_pre_hooks("block", "active");
        assert_eq!(hooks.len(), 1);
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(StubMcpInvoker::block());
        let decision = hooks[0]
            .before_exec(
                tool_call("delete_record", json!({"id": 42})),
                &tool_def(),
                &ctx,
            )
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Block { .. }),
            "mcp block verdict should block the tool call"
        );
    }

    #[tokio::test]
    async fn mcp_pre_tool_hook_continues_when_endpoint_says_allow() {
        let hooks = mcp_pre_hooks("block", "active");
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(StubMcpInvoker::allow());
        let decision = hooks[0]
            .before_exec(tool_call("read_file", json!({})), &tool_def(), &ctx)
            .await;
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
    }

    #[tokio::test]
    async fn mcp_advisory_continues_even_on_block_verdict() {
        let hooks = mcp_pre_hooks("block", "advisory");
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(StubMcpInvoker::block());
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "advisory mode must not block even when mcp says block"
        );
    }

    #[tokio::test]
    async fn mcp_pre_tool_hook_fails_open_on_connection_error() {
        let hooks = mcp_pre_hooks("block", "active");
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(StubMcpInvoker::new(
            McpBehavior::Error("MCP server not found".into()),
        ));
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "connection error must fail open"
        );
    }

    #[tokio::test]
    async fn mcp_pre_tool_hook_fails_open_on_timeout() {
        // Pause time so the 10 s timeout fires instantly.
        tokio::time::pause();
        let hooks = mcp_pre_hooks("block", "active");
        let ctx = ToolContext::new(SessionId::new())
            .with_mcp_invoker(StubMcpInvoker::new(McpBehavior::Timeout));
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "timeout must fail open"
        );
    }

    #[tokio::test]
    async fn mcp_pre_tool_hook_fails_open_on_unparseable_response() {
        let hooks = mcp_pre_hooks("block", "active");
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(StubMcpInvoker::new(
            McpBehavior::Result(json!("not json at all, no braces")),
        ));
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "unparseable response must fail open"
        );
    }

    #[tokio::test]
    async fn mcp_pre_tool_hook_skipped_without_invoker() {
        let hooks = mcp_pre_hooks("block", "active");
        // No MCP invoker wired into the context → mcp checks are skipped.
        let ctx = ToolContext::new(SessionId::new());
        let decision = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert!(
            matches!(decision, PreToolUseDecision::Continue(_)),
            "without an MCP invoker, mcp checks are silently skipped"
        );
    }

    #[tokio::test]
    async fn mcp_call_cap_evaluates_first_n_and_skips_rest() {
        // 6 active block checks, cap is 4. The recording invoker counts how many
        // times it is called; allow-verdict so none actually block.
        let checks: Vec<_> = (0..6)
            .map(|i| {
                json!({"id": format!("m{i}"), "stage": "tool_use", "type": "mcp",
                       "server": "guard", "tool": "screen"})
            })
            .collect();
        let hooks = GuardrailsCapability.pre_tool_use_hooks_with_config(&json!({
            "checks": checks
        }));
        // A counting invoker.
        struct Counter {
            calls: std::sync::atomic::AtomicUsize,
        }
        #[async_trait]
        impl crate::McpToolInvoker for Counter {
            async fn invoke(&self, tool_call: &ToolCall) -> crate::Result<ToolResult> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    result: Some(json!({"verdict": "allow"})),
                    images: None,
                    error: None,
                    connection_required: None,
                    raw_output: None,
                })
            }
        }
        let counter = Arc::new(Counter {
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(counter.clone());
        let _ = hooks[0]
            .before_exec(tool_call("any_tool", json!({})), &tool_def(), &ctx)
            .await;
        assert_eq!(
            counter.calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_MCP_CALLS_PER_INVOCATION,
            "only the first N mcp checks should be evaluated"
        );
    }

    #[tokio::test]
    async fn mcp_payload_truncated_on_char_boundary() {
        let hooks = mcp_pre_hooks("block", "active");
        let echo = StubMcpInvoker::new(McpBehavior::EchoContent);
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(echo.clone());
        // 700 × 3-byte chars = 2 100 bytes; the 2 000-byte cap falls mid-char.
        let multibyte = "€".repeat(700);
        let decision = hooks[0]
            .before_exec(
                tool_call("any_tool", json!({"x": multibyte})),
                &tool_def(),
                &ctx,
            )
            .await;
        // Must not panic; echo result is not a valid verdict so it fails open.
        assert!(matches!(decision, PreToolUseDecision::Continue(_)));
        let call = echo
            .last_call
            .lock()
            .unwrap()
            .clone()
            .expect("invoker called");
        let sent = call.arguments["content"].as_str().unwrap();
        assert!(sent.len() <= MCP_CONTENT_CAP, "payload must be capped");
        assert!(
            std::str::from_utf8(sent.as_bytes()).is_ok(),
            "payload must remain valid UTF-8 (no mid-char split)"
        );
    }

    #[tokio::test]
    async fn mcp_post_tool_hook_withholds_output_on_block() {
        let hooks = GuardrailsCapability.post_tool_exec_hooks_with_config(&json!({
            "checks": [{"stage": "tool_output", "type": "mcp",
                        "server": "guard", "tool": "scan"}]
        }));
        assert_eq!(hooks.len(), 1);
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(StubMcpInvoker::block());
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("user email: alice@example.com")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        hooks[0]
            .after_exec(
                &tool_call("web_fetch", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;
        assert_eq!(
            result.result,
            Some(json!(DEFAULT_TOOL_OUTPUT_REPLACEMENT)),
            "mcp block should replace tool output with notice"
        );
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn mcp_post_tool_hook_passes_clean_output_on_allow() {
        let hooks = GuardrailsCapability.post_tool_exec_hooks_with_config(&json!({
            "checks": [{"stage": "tool_output", "type": "mcp",
                        "server": "guard", "tool": "scan"}]
        }));
        let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(StubMcpInvoker::allow());
        let mut result = ToolResult {
            tool_call_id: "call_1".to_string(),
            result: Some(json!("no pii here")),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        hooks[0]
            .after_exec(
                &tool_call("web_fetch", json!({})),
                &tool_def(),
                &mut result,
                &ctx,
            )
            .await;
        assert_eq!(result.result, Some(json!("no pii here")));
    }
    #[tokio::test]
    async fn mcp_check_never_invokes_a_different_server_through_an_ambiguous_name() {
        for server in ["guard_", "guard-", "guard__private"] {
            let invoker = StubMcpInvoker::block();
            let hooks = GuardrailsCapability.pre_tool_use_hooks_with_config(&json!({
                "mode":"active", "checks":[{"stage":"tool_use","type":"mcp","server":server,"tool":"screen","on_fail":"block"}]
            }));
            let ctx = ToolContext::new(SessionId::new()).with_mcp_invoker(invoker.clone());
            let decision = hooks[0]
                .before_exec(tool_call("read_file", json!({})), &tool_def(), &ctx)
                .await;
            assert!(matches!(decision, PreToolUseDecision::Continue(_)));
            assert!(
                invoker.last_call.lock().unwrap().is_none(),
                "ambiguous server {server} reached invoker"
            );
        }
    }
}
