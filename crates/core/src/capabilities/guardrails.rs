// Declarative guardrails capability.
//
// Attaches the deterministic check engine (`crate::guardrail_checks`) to the
// existing interception seams — streaming output guardrails and pre/post
// tool hooks — driven entirely by per-agent config. No checks configured
// means no hooks contributed: an agent without this capability (or with an
// empty config) runs exactly as before. See specs/guardrails.md.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::atoms::{PostToolExecHook, PreToolUseDecision, PreToolUseHook};
use crate::capabilities::{Capability, CapabilityLocalization};
use crate::guardrail_checks::{
    CompiledGuardrails, DEFAULT_OUTPUT_REPLACEMENT, DEFAULT_TOOL_OUTPUT_REPLACEMENT,
    GuardrailAction, GuardrailStage, GuardrailsConfig, MAX_CHECK_ID_LEN, MAX_CHECKS,
    MAX_ENTRIES_PER_CHECK, MAX_ENTRY_LEN, MAX_REPLACEMENT_LEN,
};
use crate::output_guardrail::{
    GuardrailDecision, OutputGuardrail, OutputGuardrailContext, OutputGuardrailRun,
};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::ToolContext;

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
        "Deterministic guardrail checks over model output and tool calls: \
         regex and blocklist matching, tool-call restrictions. Checks block \
         or log per configuration; advisory mode logs without enforcing."
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
                                "enum": ["regex", "blocklist", "tool_pattern"],
                                "description": "regex/blocklist match stage text; tool_pattern matches tool names (tool_use stage only)."
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
        _context: &ToolContext,
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
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        _context: &ToolContext,
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
                    // The original content never reaches model context: the
                    // notice becomes the canonical result and error/images/raw
                    // payloads are dropped with it.
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;
    use serde_json::json;

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
}
