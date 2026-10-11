// End-of-message output seam for the guardrails capability (EVE-573, EVE-572).
//
// Model-backed (`moderation`, `llm_judge`) and MCP-delegated (`mcp`) checks on
// the `output` stage run here, once, on the finalized client-visible assistant
// message. They cannot run on the streaming seam: that path is sync and per
// delta, and these checks are async I/O that need the assembled message.
// Deterministic output checks stay on the streaming seam
// (`DeclarativeOutputGuardrail`).
//
// The checks mirror their `tool_use`/`tool_output` counterparts: jev-engine
// checks resolve in one batched request, utility-LLM and MCP calls are capped
// per message, advisory mode and `on_fail: log` downgrade a block to a log, and
// every error, timeout, or missing collaborator fails open.
//
// Split out of `guardrails.rs`, which is on the file-size ratchet.

use super::*;

/// Timeout for a single moderation call. Fail-open on expiry — a
/// utility-model outage must never wedge a turn.
const MODERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Maximum moderation checks evaluated per finalized message (TM-DOS: bounds
/// added latency on the end-of-message seam).
pub(super) const MAX_MODERATION_CALLS_PER_INVOCATION: usize = 4;
/// Bytes of assistant text sent to the moderation scorer and to the batched
/// jev request for the `output` stage.
const MODERATION_CONTENT_CAP: usize = 4_000;

/// System prompt sent to the utility LLM for moderation scoring.
const MODERATION_SYSTEM_PROMPT: &str = r#"You are a content-moderation decisions for an AI agent runtime.
Score the provided assistant message for each requested category from 0 to 100, where 0 means clearly absent and 100 means a clear, severe violation.
Respond ONLY with valid JSON on a single line. No explanation, no prose, no markdown.
Format: {"scores":{"<category>":<integer 0-100>, ...}} — include every requested category."#;

/// Evaluate one moderation check against `content` via the utility LLM.
/// Returns `Some(GuardrailAction::Block)` when any requested category scores
/// at or above the check's threshold, `Some(Log)` when all score below it, and
/// `None` on any error/timeout/parse failure (fail-open). The caller applies
/// advisory-mode downgrade via `compiled.async_action()`.
async fn run_moderation_check(
    service: &dyn crate::builtins::UtilityLlmService,
    check: &crate::builtins::guardrail_checks::CompiledModerationCheck,
    content: &str,
) -> Option<GuardrailAction> {
    let payload = truncate_on_char_boundary(content, MODERATION_CONTENT_CAP);
    let user_prompt = format!(
        "Categories: {}\nMessage:\n{}",
        check.categories.join(", "),
        payload,
    );
    let request = UtilityLlmRequest::new(vec![
        Message::text(MessageRole::System, MODERATION_SYSTEM_PROMPT),
        Message::text(MessageRole::User, user_prompt),
    ])
    .with_reasoning_effort(UtilityLlmReasoningEffort::Low)
    .with_max_tokens(128);

    let response = match tokio::time::timeout(MODERATION_TIMEOUT, service.chat_completion(request))
        .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            tracing::warn!(check = %check.label, error = %e, "guardrails: moderation call failed, failing open");
            return None;
        }
        Err(_) => {
            tracing::warn!(check = %check.label, "guardrails: moderation call timed out, failing open");
            return None;
        }
    };

    let Some(fragment) = json_like_fragment(response.text.trim()) else {
        tracing::warn!(check = %check.label, "guardrails: moderation response missing JSON, failing open");
        return None;
    };
    let parsed = match serde_json::from_str::<serde_json::Value>(fragment) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(check = %check.label, parse_error = %e, raw = %fragment, "guardrails: moderation response parse failed, failing open");
            return None;
        }
    };
    let Some(scores) = parsed.get("scores").and_then(|s| s.as_object()) else {
        tracing::warn!(check = %check.label, "guardrails: moderation response missing scores field, failing open");
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

/// Async output guardrail run once on the finalized assistant message. Holds
/// the compiled config; contributes only when an output-stage `moderation`,
/// `llm_judge`, or `mcp` check is configured.
pub(super) struct MessageOutputGuardrail {
    compiled: Arc<CompiledGuardrails>,
}

impl MessageOutputGuardrail {
    pub(super) fn for_compiled(compiled: Arc<CompiledGuardrails>) -> Option<Self> {
        let stage = GuardrailStage::Output;
        let any = compiled.moderation_checks_for_stage(stage).next().is_some()
            || compiled.judge_checks_for_stage(stage).next().is_some()
            || compiled.mcp_checks_for_stage(stage).next().is_some();
        any.then_some(Self { compiled })
    }

    /// Apply advisory mode / `on_fail` to a raw verdict. Returns the block
    /// decision when it enforces; logs and returns `None` otherwise.
    fn enforce(
        &self,
        raw_action: Option<GuardrailAction>,
        on_fail: crate::builtins::guardrail_checks::GuardrailOnFail,
        label: &str,
        reason_code: &str,
        replacement: &Option<String>,
    ) -> Option<GuardrailDecision> {
        // `None` is fail-open (error/timeout/parse failure); `Log` is allow.
        if raw_action != Some(GuardrailAction::Block) {
            return None;
        }
        if self.compiled.async_action(on_fail) == GuardrailAction::Log {
            tracing::warn!(check = %label, reason_code, "guardrails: output check hit (log only)");
            return None;
        }
        tracing::warn!(check = %label, reason_code, "guardrails: blocking model output");
        Some(GuardrailDecision::Block(
            crate::builtins::output_guardrail::GuardrailBlock {
                reason_code: reason_code.to_string(),
                replacement: replacement
                    .clone()
                    .unwrap_or_else(|| DEFAULT_OUTPUT_REPLACEMENT.to_string()),
            },
        ))
    }
}

#[async_trait]
impl PostGenerationOutputGuardrail for MessageOutputGuardrail {
    fn id(&self) -> &str {
        // Same id as the streaming seam: both run this capability's declarative
        // checks; the reason code names the check type that tripped.
        "guardrail_checks"
    }

    async fn check_message(&self, ctx: &PostGenerationOutputContext<'_>) -> GuardrailDecision {
        const STAGE: GuardrailStage = GuardrailStage::Output;
        let text = ctx.message_text;
        // Jev-engine moderation and judge checks resolve in one request,
        // before the per-check utility-LLM calls.
        let judged = stage_decisions(
            ctx.decisions,
            &self.compiled,
            STAGE,
            None,
            text,
            MODERATION_CONTENT_CAP,
        )
        .await;
        let utility = ctx
            .utility_llm_service
            .filter(|service| service.is_configured());

        let mut utility_calls = 0usize;
        for check in self.compiled.moderation_checks_for_stage(STAGE) {
            let raw_action = match check.engine {
                GuardrailEngine::Jev => judged.get(&check.index).copied(),
                GuardrailEngine::UtilityLlm => {
                    let Some(service) = utility else {
                        tracing::warn!(check = %check.label, "guardrails: moderation skipped — no utility LLM service (fail-open)");
                        continue;
                    };
                    if utility_calls >= MAX_MODERATION_CALLS_PER_INVOCATION {
                        tracing::warn!(
                            "guardrails: moderation call cap reached; output check skipped (fail-open)"
                        );
                        continue;
                    }
                    utility_calls += 1;
                    run_moderation_check(service.as_ref(), check, text).await
                }
            };
            let reason = "guardrail.moderation";
            if let Some(block) = self.enforce(
                raw_action,
                check.on_fail,
                &check.label,
                reason,
                &check.replacement,
            ) {
                return block;
            }
        }

        let mut utility_calls = 0usize;
        for check in self.compiled.judge_checks_for_stage(STAGE) {
            let raw_action = match check.engine {
                GuardrailEngine::Jev => judged.get(&check.index).copied(),
                GuardrailEngine::UtilityLlm => {
                    let Some(service) = utility else { continue };
                    if utility_calls >= MAX_JUDGE_CALLS_PER_INVOCATION {
                        tracing::warn!(
                            "guardrails: judge cap reached; output utility check skipped"
                        );
                        continue;
                    }
                    utility_calls += 1;
                    run_judge_check(service.as_ref(), check, STAGE, None, text).await
                }
            };
            let reason = "guardrail.llm_judge";
            if let Some(block) = self.enforce(
                raw_action,
                check.on_fail,
                &check.label,
                reason,
                &check.replacement,
            ) {
                return block;
            }
        }

        // MCP-served checks; skipped when the host wired no scoped-MCP invoker.
        let Some(invoker) = ctx.mcp_invoker else {
            return GuardrailDecision::Pass;
        };
        for (calls, check) in self.compiled.mcp_checks_for_stage(STAGE).enumerate() {
            if calls >= MAX_MCP_CALLS_PER_INVOCATION {
                tracing::warn!("guardrails: mcp call cap reached for output, skipping remaining");
                break;
            }
            let raw_action = run_mcp_check(invoker.as_ref(), check, STAGE, None, text).await;
            let reason = "guardrail.mcp";
            if let Some(block) = self.enforce(
                raw_action,
                check.on_fail,
                &check.label,
                reason,
                &check.replacement,
            ) {
                return block;
            }
        }
        GuardrailDecision::Pass
    }
}
