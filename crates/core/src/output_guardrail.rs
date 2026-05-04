// Streaming output guardrails.
//
// Capabilities can contribute guardrails that inspect the model's streamed
// output as it arrives and, post factum, replace the entire response with a
// canned message when a violation is detected. The client receives normal
// `output.message.delta` events until a guardrail trips; at that point a
// single `output.message.replaced` event tells the client to discard the
// accumulated text and show the replacement instead.
//
// Design constraints:
//  - Guardrails run on every batched delta in the streaming hot path. The
//    `check` method is intentionally synchronous so slow checks cannot back
//    up the LLM stream. Heavy guardrails (e.g. an LLM-based moderator)
//    should run asynchronously elsewhere — this trait is for cheap, in-
//    process inspection.
//  - The model's original tokens are never persisted when a guardrail trips;
//    the replacement becomes the canonical assistant message so later turns
//    can never see what was blocked.

use std::sync::Arc;

/// Provider-side definition of an output guardrail.
///
/// Contributed by capabilities via `Capability::output_guardrails()`. A single
/// provider may serve multiple sessions concurrently; per-stream mutable
/// state lives in the [`OutputGuardrailRun`] returned by [`Self::arm`].
pub trait OutputGuardrail: Send + Sync {
    /// Stable identifier (e.g. `"prompt_canary"`). Surfaced to clients in the
    /// `output.message.replaced` event so they can localize messaging or
    /// route to telemetry.
    fn id(&self) -> &str;

    /// Construct a per-stream guardrail. Called once at the start of an
    /// assistant message stream with a snapshot of the runtime context.
    /// Returning `None` skips the guardrail for this stream (e.g. no canary
    /// could be derived from the system prompt).
    fn arm(&self, ctx: &OutputGuardrailContext<'_>) -> Option<Box<dyn OutputGuardrailRun>>;
}

/// Per-stream guardrail instance. Holds whatever state the implementation
/// needs across delta callbacks (e.g. precomputed needles, position cursors).
pub trait OutputGuardrailRun: Send {
    /// Inspect the latest accumulated output. Called after each batched
    /// delta is appended. `accumulated` is the full assistant text so far;
    /// `delta` is the chunk that was just added.
    ///
    /// Returning [`GuardrailDecision::Block`] aborts the stream and triggers
    /// replacement. Subsequent calls are not made on a blocked stream.
    fn check(&mut self, accumulated: &str, delta: &str) -> GuardrailDecision;
}

/// Snapshot of the runtime configuration available when arming a guardrail.
///
/// Borrowed for the duration of the `arm` call so guardrails can read the
/// system prompt without owning a copy. Anything the guardrail needs after
/// `arm` returns must be cloned into the returned `OutputGuardrailRun`.
pub struct OutputGuardrailContext<'a> {
    /// The fully assembled system prompt for this turn.
    pub system_prompt: &'a str,
    /// Per-capability config JSON (`AgentCapabilityConfig.config`).
    pub config: &'a serde_json::Value,
}

/// Guardrail decision returned from `check`.
#[derive(Debug, Clone)]
pub enum GuardrailDecision {
    /// Output is fine; keep streaming.
    Pass,
    /// Output violated the guardrail. The stream is aborted and the client
    /// is told to replace the accumulated text with `replacement`.
    Block(GuardrailBlock),
}

/// Details of a guardrail violation, surfaced to the client in the
/// `output.message.replaced` event and persisted as the assistant message.
#[derive(Debug, Clone)]
pub struct GuardrailBlock {
    /// Stable machine-readable code (e.g. `"system_prompt_leak"`). Clients
    /// localize their copy from this rather than the human text.
    pub reason_code: String,
    /// Replacement text shown to the user and stored in the conversation.
    pub replacement: String,
}

/// Convenience constructor.
impl GuardrailDecision {
    pub fn block(reason_code: impl Into<String>, replacement: impl Into<String>) -> Self {
        GuardrailDecision::Block(GuardrailBlock {
            reason_code: reason_code.into(),
            replacement: replacement.into(),
        })
    }
}

/// One armed guardrail for a single stream. Carries the contributing
/// capability id alongside the guardrail's own id so the
/// `output.message.replaced` event can label both.
pub struct ArmedGuardrail {
    pub capability_id: String,
    pub guardrail_id: String,
    pub run: Box<dyn OutputGuardrailRun>,
}

/// Run all armed guardrails against the latest accumulated output. Returns
/// the first block, in registration order. Pure helper — no I/O.
pub fn evaluate_guardrails(
    runs: &mut [ArmedGuardrail],
    accumulated: &str,
    delta: &str,
) -> Option<TrippedGuardrail> {
    for armed in runs.iter_mut() {
        match armed.run.check(accumulated, delta) {
            GuardrailDecision::Pass => continue,
            GuardrailDecision::Block(block) => {
                return Some(TrippedGuardrail {
                    capability_id: armed.capability_id.clone(),
                    guardrail_id: armed.guardrail_id.clone(),
                    block,
                });
            }
        }
    }
    None
}

/// Result of [`evaluate_guardrails`]: which guardrail tripped and with what
/// replacement.
#[derive(Debug, Clone)]
pub struct TrippedGuardrail {
    pub capability_id: String,
    pub guardrail_id: String,
    pub block: GuardrailBlock,
}

/// Arm a set of providers for a stream. Providers that decline to arm
/// (return `None`) are skipped. Each provider carries the contributing
/// capability id so the resulting [`ArmedGuardrail`] can label events.
pub fn arm_guardrails(
    providers: &[(String, Arc<dyn OutputGuardrail>)],
    ctx: &OutputGuardrailContext<'_>,
) -> Vec<ArmedGuardrail> {
    providers
        .iter()
        .filter_map(|(cap_id, p)| {
            let guardrail_id = p.id().to_string();
            p.arm(ctx).map(|run| ArmedGuardrail {
                capability_id: cap_id.clone(),
                guardrail_id,
                run,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysBlock;
    impl OutputGuardrailRun for AlwaysBlock {
        fn check(&mut self, _accumulated: &str, _delta: &str) -> GuardrailDecision {
            GuardrailDecision::block("test_block", "[blocked]")
        }
    }

    struct NeverBlock;
    impl OutputGuardrailRun for NeverBlock {
        fn check(&mut self, _accumulated: &str, _delta: &str) -> GuardrailDecision {
            GuardrailDecision::Pass
        }
    }

    fn armed(cap: &str, guard: &str, run: Box<dyn OutputGuardrailRun>) -> ArmedGuardrail {
        ArmedGuardrail {
            capability_id: cap.to_string(),
            guardrail_id: guard.to_string(),
            run,
        }
    }

    #[test]
    fn evaluate_returns_first_block_in_order() {
        let mut runs = vec![
            armed("cap_a", "g_a", Box::new(NeverBlock)),
            armed("cap_b", "g_b", Box::new(AlwaysBlock)),
            armed("cap_c", "g_c", Box::new(AlwaysBlock)),
        ];
        let tripped = evaluate_guardrails(&mut runs, "any text", "delta").expect("blocked");
        assert_eq!(tripped.capability_id, "cap_b");
        assert_eq!(tripped.guardrail_id, "g_b");
        assert_eq!(tripped.block.reason_code, "test_block");
        assert_eq!(tripped.block.replacement, "[blocked]");
    }

    #[test]
    fn evaluate_returns_none_when_all_pass() {
        let mut runs = vec![
            armed("cap_a", "g_a", Box::new(NeverBlock)),
            armed("cap_b", "g_b", Box::new(NeverBlock)),
        ];
        assert!(evaluate_guardrails(&mut runs, "txt", "d").is_none());
    }
}
