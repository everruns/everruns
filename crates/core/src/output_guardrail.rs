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

use async_trait::async_trait;

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
    /// Per-capability config JSON (`CapabilityRef.config`).
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

// ---------------------------------------------------------------------------
// End-of-message (post-generation) output seam (EVE-573)
// ---------------------------------------------------------------------------

/// Async, end-of-message output guardrail.
///
/// Unlike [`OutputGuardrail`] — which runs synchronously on every streamed
/// delta in the hot path and must stay cheap — this seam runs **once** on the
/// fully assembled assistant message after streaming completes and before the
/// message is finalized into context. It may perform I/O (e.g. call a
/// moderation classifier through the utility LLM).
///
/// Contract: implementations MUST be internally time-bounded and **fail open**
/// — any timeout, transport error, or missing dependency must return
/// [`GuardrailDecision::Pass`] so a guardrail outage never wedges a turn. A
/// [`GuardrailDecision::Block`] reuses the streaming seam's plumbing: the
/// finalized message is replaced with the block's `replacement` and a single
/// `output.message.replaced` event is emitted. The model's original tokens are
/// never persisted once a block fires.
#[async_trait]
pub trait PostGenerationOutputGuardrail: Send + Sync {
    /// Stable identifier (e.g. `"moderation"`), surfaced to clients in the
    /// `output.message.replaced` event.
    fn id(&self) -> &str;

    /// Inspect the finalized client-visible assistant output, including
    /// annotation metadata. Returns
    /// [`GuardrailDecision::Block`] to replace it; otherwise
    /// [`GuardrailDecision::Pass`]. Must fail open on any error.
    async fn check_message(&self, ctx: &PostGenerationOutputContext<'_>) -> GuardrailDecision;
}

/// Runtime context handed to a [`PostGenerationOutputGuardrail`]. Borrowed for
/// the duration of the `check_message` call.
pub struct PostGenerationOutputContext<'a> {
    /// The fully assembled system prompt for this turn.
    pub system_prompt: &'a str,
    /// The finalized client-visible output (post-streaming, pre-context), with
    /// persisted annotation metadata appended when present.
    pub message_text: &'a str,
    /// Utility LLM service for model-backed checks. `None` when the deployment
    /// has no utility model configured — model-backed checks then fail open.
    pub utility_llm_service: Option<&'a Arc<dyn crate::UtilityLlmService>>,
}

/// A post-generation guardrail provider paired with its contributing
/// capability id, so a trip can label the `output.message.replaced` event.
pub struct PostGenerationProvider {
    pub capability_id: String,
    pub provider: Arc<dyn PostGenerationOutputGuardrail>,
}

/// Build the complete client-visible output inspected by post-generation
/// guardrails. Citation metadata is persisted and rendered alongside the
/// assistant text, so it must cross the same output policy boundary.
pub fn post_generation_guardrail_text(
    message_text: &str,
    annotations: &[crate::message::TextAnnotation],
) -> String {
    if annotations.is_empty() {
        return message_text.to_string();
    }

    let mut output = String::from(message_text);
    for annotation in annotations {
        output.push('\n');
        output.push_str(&annotation.source.uri);
        if let Some(title) = &annotation.source.title {
            output.push('\n');
            output.push_str(title);
        }
        if let Some(snippet) = &annotation.source.snippet {
            output.push('\n');
            output.push_str(snippet);
        }
        if let Some(location) = &annotation.source.location {
            output.push('\n');
            output.push_str(&location.to_string());
        }
        if let Some(external_id) = &annotation.external_id {
            output.push('\n');
            output.push_str(external_id);
        }
    }
    output
}

/// Run post-generation guardrails in registration order, returning the first
/// block. Pure orchestration: each provider owns its timeout / fail-open
/// behavior, so a provider that errors simply yields `Pass` and the next runs.
pub async fn evaluate_post_generation_guardrails(
    providers: &[PostGenerationProvider],
    ctx: &PostGenerationOutputContext<'_>,
) -> Option<TrippedGuardrail> {
    for p in providers {
        match p.provider.check_message(ctx).await {
            GuardrailDecision::Pass => continue,
            GuardrailDecision::Block(block) => {
                return Some(TrippedGuardrail {
                    capability_id: p.capability_id.clone(),
                    guardrail_id: p.provider.id().to_string(),
                    block,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AnnotationSource, TextAnnotation};

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

    struct PostPass;
    #[async_trait]
    impl PostGenerationOutputGuardrail for PostPass {
        fn id(&self) -> &str {
            "pass"
        }
        async fn check_message(&self, _ctx: &PostGenerationOutputContext<'_>) -> GuardrailDecision {
            GuardrailDecision::Pass
        }
    }

    struct PostBlock;
    #[async_trait]
    impl PostGenerationOutputGuardrail for PostBlock {
        fn id(&self) -> &str {
            "block"
        }
        async fn check_message(&self, _ctx: &PostGenerationOutputContext<'_>) -> GuardrailDecision {
            GuardrailDecision::block("guardrail.moderation", "[removed]")
        }
    }

    fn post_ctx<'a>(text: &'a str) -> PostGenerationOutputContext<'a> {
        PostGenerationOutputContext {
            system_prompt: "",
            message_text: text,
            utility_llm_service: None,
        }
    }

    #[tokio::test]
    async fn post_generation_returns_first_block_in_order() {
        let providers = vec![
            PostGenerationProvider {
                capability_id: "cap_a".to_string(),
                provider: Arc::new(PostPass),
            },
            PostGenerationProvider {
                capability_id: "cap_b".to_string(),
                provider: Arc::new(PostBlock),
            },
        ];
        let ctx = post_ctx("hello");
        let tripped = evaluate_post_generation_guardrails(&providers, &ctx)
            .await
            .expect("blocked");
        assert_eq!(tripped.capability_id, "cap_b");
        assert_eq!(tripped.guardrail_id, "block");
        assert_eq!(tripped.block.reason_code, "guardrail.moderation");
        assert_eq!(tripped.block.replacement, "[removed]");
    }

    #[tokio::test]
    async fn post_generation_returns_none_when_all_pass() {
        let providers = vec![PostGenerationProvider {
            capability_id: "cap_a".to_string(),
            provider: Arc::new(PostPass),
        }];
        let ctx = post_ctx("hello");
        assert!(
            evaluate_post_generation_guardrails(&providers, &ctx)
                .await
                .is_none()
        );
    }

    #[test]
    fn post_generation_text_includes_client_visible_citation_metadata() {
        let annotations = vec![TextAnnotation {
            start: 0,
            end: 5,
            origin: "citation_retrieval".to_string(),
            source: AnnotationSource {
                uri: "https://example.com/private".to_string(),
                title: Some("Private roadmap".to_string()),
                snippet: Some("password S3CR3T".to_string()),
                location: Some(serde_json::json!({ "page": 4 })),
            },
            external_id: None,
            verified: None,
        }];

        let output = post_generation_guardrail_text("An answer.", &annotations);

        assert!(output.contains("An answer."));
        assert!(output.contains("https://example.com/private"));
        assert!(output.contains("Private roadmap"));
        assert!(output.contains("password S3CR3T"));
        assert!(output.contains(r#"{"page":4}"#));
    }
}
