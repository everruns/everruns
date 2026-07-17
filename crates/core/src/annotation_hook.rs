//! End-of-message citation annotation seam.
//!
//! A mutating sibling of the post-generation guardrail family in
//! [`crate::output_guardrail`]. Where a guardrail runs once on the finalized
//! assistant message and returns a block/allow decision, an annotation hook
//! runs at the same point and returns citation [`TextAnnotation`]s to attach to
//! the message text — optionally rewriting the text first (e.g. to strip
//! citation markers the model emitted). Contributed by citation capabilities
//! via `Capability::post_output_annotation_hooks_with_config()`. See
//! `specs/citations.md`.
//!
//! Contract: implementations MUST be internally time-bounded and **fail open** —
//! any error must yield an empty [`AnnotationResult`] so an annotator outage
//! never wedges a turn. Returned spans MUST fall within the char bounds of the
//! text the hook received; the runner discards out-of-range spans.

use std::sync::Arc;

use async_trait::async_trait;

use crate::message::{Message, TextAnnotation};

/// Async, end-of-message hook that attaches citation annotations to the
/// finalized assistant text.
#[async_trait]
pub trait PostGenerationAnnotationHook: Send + Sync {
    /// Stable identifier, usually the contributing capability id.
    fn id(&self) -> &str;

    /// Produce annotations (and optional text rewrite) for the finalized
    /// message. Must fail open (empty result) on any error.
    async fn annotate(&self, ctx: &AnnotationContext<'_>) -> AnnotationResult;
}

/// Runtime context handed to a [`PostGenerationAnnotationHook`]. Borrowed for
/// the duration of the `annotate` call.
pub struct AnnotationContext<'a> {
    /// The fully assembled system prompt for this turn.
    pub system_prompt: &'a str,
    /// The current assistant message text (possibly already rewritten by an
    /// earlier hook). Annotation spans are relative to this.
    pub message_text: &'a str,
    /// The assembled conversation context sent to the model this turn. Feeds
    /// like `citation_retrieval` scan it for the tool-result citations they
    /// align to claim spans.
    pub messages: &'a [Message],
    /// Utility LLM service for model-backed alignment/verification. `None` when
    /// the deployment has no utility model configured.
    pub utility_llm_service: Option<&'a Arc<dyn crate::UtilityLlmService>>,
}

/// What a hook returns: annotations to attach, plus an optional replacement for
/// the message text.
#[derive(Debug, Default, Clone)]
pub struct AnnotationResult {
    pub annotations: Vec<TextAnnotation>,
    /// When set, replaces the message text before annotations are applied (e.g.
    /// to strip inline citation markers). Spans in `annotations` are relative
    /// to this rewritten text.
    pub rewritten_text: Option<String>,
}

impl AnnotationResult {
    /// An empty result (no annotations, no rewrite) — the fail-open value.
    pub fn none() -> Self {
        Self::default()
    }
}

/// An annotation hook paired with its contributing capability id.
pub struct AnnotationProvider {
    pub capability_id: String,
    pub provider: Arc<dyn PostGenerationAnnotationHook>,
}

/// Outcome of running all annotation providers over a message.
#[derive(Debug, Default, Clone)]
pub struct CollectedAnnotations {
    /// Final message text after any hook rewrites.
    pub text: String,
    /// All valid annotations, in provider-registration order.
    pub annotations: Vec<TextAnnotation>,
}

/// Run annotation providers in registration order, threading the (possibly
/// rewritten) text through each so a later hook sees an earlier hook's rewrite.
///
/// Spans that fall outside the current text's char bounds are dropped. When a
/// provider rewrites the text, previously-collected annotations are discarded
/// (their offsets are no longer valid against the new text) — in practice a
/// single citation feed is active per agent, so the common path is one
/// provider. Pure orchestration: each provider owns its own fail-open behavior.
pub async fn collect_annotations(
    providers: &[AnnotationProvider],
    system_prompt: &str,
    message_text: &str,
    messages: &[Message],
    utility_llm_service: Option<&Arc<dyn crate::UtilityLlmService>>,
) -> CollectedAnnotations {
    let mut text = message_text.to_string();
    let mut annotations: Vec<TextAnnotation> = Vec::new();

    for p in providers {
        let ctx = AnnotationContext {
            system_prompt,
            message_text: &text,
            messages,
            utility_llm_service,
        };
        let result = p.provider.annotate(&ctx).await;

        if let Some(rewritten) = result.rewritten_text {
            // A rewrite invalidates earlier annotations' offsets.
            if rewritten != text {
                annotations.clear();
            }
            text = rewritten;
        }

        let char_len = text.chars().count();
        for ann in result.annotations {
            if ann.start < ann.end && ann.end <= char_len {
                annotations.push(ann);
            }
        }
    }

    CollectedAnnotations { text, annotations }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::AnnotationSource;

    fn ann(start: usize, end: usize) -> TextAnnotation {
        TextAnnotation {
            start,
            end,
            origin: "test".to_string(),
            source: AnnotationSource {
                uri: "https://example.com".to_string(),
                title: None,
                snippet: None,
                location: None,
            },
            external_id: None,
            verified: None,
        }
    }

    struct FixedHook {
        annotations: Vec<TextAnnotation>,
        rewritten: Option<String>,
    }

    #[async_trait]
    impl PostGenerationAnnotationHook for FixedHook {
        fn id(&self) -> &str {
            "fixed"
        }
        async fn annotate(&self, _ctx: &AnnotationContext<'_>) -> AnnotationResult {
            AnnotationResult {
                annotations: self.annotations.clone(),
                rewritten_text: self.rewritten.clone(),
            }
        }
    }

    fn provider(hook: FixedHook) -> AnnotationProvider {
        AnnotationProvider {
            capability_id: "test".to_string(),
            provider: Arc::new(hook),
        }
    }

    #[tokio::test]
    async fn keeps_in_bounds_and_drops_out_of_bounds_spans() {
        let providers = vec![provider(FixedHook {
            annotations: vec![ann(0, 5), ann(3, 100)],
            rewritten: None,
        })];
        let out = collect_annotations(&providers, "", "hello world", &[], None).await;
        assert_eq!(out.text, "hello world");
        assert_eq!(out.annotations.len(), 1);
        assert_eq!((out.annotations[0].start, out.annotations[0].end), (0, 5));
    }

    #[tokio::test]
    async fn rewrite_replaces_text_and_resets_prior_annotations() {
        let providers = vec![
            provider(FixedHook {
                annotations: vec![ann(0, 4)],
                rewritten: None,
            }),
            provider(FixedHook {
                annotations: vec![ann(0, 2)],
                rewritten: Some("hi".to_string()),
            }),
        ];
        let out = collect_annotations(&providers, "", "hello", &[], None).await;
        assert_eq!(out.text, "hi");
        // First hook's (0,4) annotation is discarded by the rewrite; only the
        // rewriting hook's own (0,2) survives.
        assert_eq!(out.annotations.len(), 1);
        assert_eq!((out.annotations[0].start, out.annotations[0].end), (0, 2));
    }

    #[tokio::test]
    async fn drops_span_out_of_bounds_after_rewrite() {
        let providers = vec![provider(FixedHook {
            annotations: vec![ann(0, 5)],
            rewritten: Some("hi".to_string()),
        })];
        let out = collect_annotations(&providers, "", "hello", &[], None).await;
        assert_eq!(out.text, "hi");
        assert!(out.annotations.is_empty());
    }
}
