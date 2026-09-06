//! End-of-message citation annotation seam.
//!
//! A mutating sibling of the post-generation guardrail family in
//! [`crate::output_guardrail`]. Where a guardrail runs once on the finalized
//! assistant message and returns a block/allow decision, an annotation hook
//! runs at the same point and returns citation [`TextAnnotation`]s to attach to
//! the message text — optionally rewriting the text first (e.g. to strip
//! citation markers the model emitted). Contributed by citation capabilities
//! via `Capability::post_output_annotation_hooks_with_config()`. See
//! `knowledge/runtime-resources/citations.md`.
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

// ---------------------------------------------------------------------------
// Citation verification seam
// ---------------------------------------------------------------------------

/// Async verifier that stamps `verified` verdicts onto citations produced by
/// any feed.
///
/// Contributed by the `citation_verification` capability via
/// `Capability::citation_verifier_with_config`. Runs once after all annotation
/// feeds, over the collected annotations — decoupled from the feeds so any feed
/// can be paired with any verifier (see `knowledge/runtime-resources/citations.md`).
///
/// Contract: **fail open** — on any error, return the annotations unchanged
/// (unverified) rather than dropping them. Implementations must preserve order
/// and count.
#[async_trait]
pub trait CitationVerifier: Send + Sync {
    /// Stable identifier, usually the contributing capability id.
    fn id(&self) -> &str;

    /// Return `annotations` with `verified` stamped where a verdict could be
    /// produced.
    async fn verify(
        &self,
        ctx: &VerificationContext<'_>,
        annotations: Vec<TextAnnotation>,
    ) -> Vec<TextAnnotation>;
}

/// Runtime context handed to a [`CitationVerifier`].
pub struct VerificationContext<'a> {
    /// The finalized assistant message text; annotation spans index into it.
    pub message_text: &'a str,
    /// Utility LLM service for model-backed verification. `None` when the
    /// deployment has no utility model configured.
    pub utility_llm_service: Option<&'a Arc<dyn crate::UtilityLlmService>>,
}

/// A verifier paired with its contributing capability id.
pub struct VerifierProvider {
    pub capability_id: String,
    pub provider: Arc<dyn CitationVerifier>,
}

/// Run the configured verifiers, in registration order, over the annotations.
/// Typically zero or one verifier is active. Pure orchestration; each verifier
/// owns its fail-open behavior.
pub async fn verify_annotations(
    verifiers: &[VerifierProvider],
    message_text: &str,
    utility_llm_service: Option<&Arc<dyn crate::UtilityLlmService>>,
    mut annotations: Vec<TextAnnotation>,
) -> Vec<TextAnnotation> {
    for v in verifiers {
        let ctx = VerificationContext {
            message_text,
            utility_llm_service,
        };
        annotations = v.provider.verify(&ctx, annotations).await;
    }
    annotations
}

// ---------------------------------------------------------------------------
// Shared citation text helpers (used by feeds and verifiers)
// ---------------------------------------------------------------------------

/// Lowercase word tokens of length ≥ 3, dropping a small stopword set so
/// lexical overlap reflects distinctive content rather than filler.
pub fn citation_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .filter(|w| !is_stopword(w))
        .collect()
}

fn is_stopword(word: &str) -> bool {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "are", "was", "were", "this", "that", "with", "from", "have", "has",
        "not", "but", "you", "your", "its", "their", "they", "them", "then", "than", "which",
        "into", "onto", "over", "under", "about", "there", "here", "what", "when", "where",
    ];
    STOPWORDS.contains(&word)
}

/// Fraction of `needle`'s distinct tokens that also appear in `haystack`.
/// Returns 0.0 when `needle` is empty.
pub fn token_overlap_ratio(needle: &[String], haystack: &[String]) -> f32 {
    if needle.is_empty() {
        return 0.0;
    }
    let hay: std::collections::HashSet<&String> = haystack.iter().collect();
    let distinct: std::collections::HashSet<&String> = needle.iter().collect();
    let shared = distinct.iter().filter(|t| hay.contains(**t)).count();
    shared as f32 / distinct.len() as f32
}

/// The substring of `text` covered by a char span `[start, end)`.
pub fn span_text(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
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
        let valid = ann(0, 3);
        let providers = vec![provider(FixedHook {
            annotations: vec![
                valid.clone(),
                ann(0, 0),
                ann(2, 1),
                ann(1, 4),
                ann(3, 4),
                ann(usize::MAX, usize::MAX),
            ],
            rewritten: None,
        })];
        let out = collect_annotations(&providers, "", "αβγ", &[], None).await;
        assert_eq!(out.text, "αβγ");
        assert_eq!(out.annotations, [valid]);
        let empty = collect_annotations(&[], "", "Unchanged α", &[], None).await;
        assert_eq!(empty.text, "Unchanged α");
        assert!(empty.annotations.is_empty());
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
                rewritten: Some("hi".into()),
            }),
        ];
        let out = collect_annotations(&providers, "", "hello", &[], None).await;
        assert_eq!(out.text, "hi");
        assert_eq!(out.annotations, [ann(0, 2)]);
    }

    #[tokio::test]
    async fn drops_span_out_of_bounds_after_rewrite() {
        let providers = vec![provider(FixedHook {
            annotations: vec![ann(0, 5)],
            rewritten: Some("hi".into()),
        })];
        let out = collect_annotations(&providers, "", "hello", &[], None).await;
        assert_eq!(out.text, "hi");
        assert!(out.annotations.is_empty());
    }

    #[tokio::test]
    async fn hooks_receive_rewritten_context_and_only_changed_text_invalidates_prior_spans() {
        struct Hook {
            expected: &'static str,
            result: AnnotationResult,
            service: Arc<dyn crate::UtilityLlmService>,
        }
        #[async_trait]
        impl PostGenerationAnnotationHook for Hook {
            fn id(&self) -> &str {
                "context"
            }
            async fn annotate(&self, ctx: &AnnotationContext<'_>) -> AnnotationResult {
                assert_eq!(ctx.system_prompt, "system");
                assert_eq!(ctx.message_text, self.expected);
                assert_eq!(ctx.messages.len(), 1);
                assert_eq!(ctx.messages[0].text(), Some("question"));
                assert!(Arc::ptr_eq(ctx.utility_llm_service.unwrap(), &self.service));
                self.result.clone()
            }
        }
        let service: Arc<dyn crate::UtilityLlmService> = Arc::new(crate::DisabledUtilityLlmService);
        let mut first = ann(0, 1);
        first.origin = "first".into();
        let mut second = ann(1, 3);
        second.origin = "second".into();
        let mut providers = vec![provider(FixedHook {
            annotations: vec![ann(0, 2)],
            rewritten: None,
        })];
        for (expected, rewrite, annotation) in [
            ("αβγ", "δεζ", first.clone()),
            ("δεζ", "δεζ", second.clone()),
        ] {
            providers.push(AnnotationProvider {
                capability_id: "context".into(),
                provider: Arc::new(Hook {
                    expected,
                    result: AnnotationResult {
                        annotations: vec![annotation],
                        rewritten_text: Some(rewrite.into()),
                    },
                    service: service.clone(),
                }),
            });
        }
        let out = collect_annotations(
            &providers,
            "system",
            "αβγ",
            &[Message::user("question")],
            Some(&service),
        )
        .await;
        assert_eq!(out.text, "δεζ");
        assert_eq!(out.annotations, [first, second]);
    }

    #[tokio::test]
    async fn verifier_chain_preserves_annotations_and_threads_prior_verdicts() {
        use crate::message::{VerificationStatus, VerificationVerdict};
        struct Verifier {
            expected: Option<VerificationVerdict>,
            next: VerificationVerdict,
        }
        #[async_trait]
        impl CitationVerifier for Verifier {
            fn id(&self) -> &str {
                "verifier"
            }
            async fn verify(
                &self,
                ctx: &VerificationContext<'_>,
                mut annotations: Vec<TextAnnotation>,
            ) -> Vec<TextAnnotation> {
                assert_eq!(ctx.message_text, "αβγ");
                assert!(ctx.utility_llm_service.is_some());
                for annotation in &mut annotations {
                    assert_eq!(annotation.verified, self.expected);
                    annotation.verified = Some(self.next.clone());
                }
                annotations
            }
        }
        let original = vec![ann(0, 1), ann(1, 3)];
        assert_eq!(
            verify_annotations(&[], "αβγ", None, original.clone()).await,
            original
        );
        let first = VerificationVerdict {
            status: VerificationStatus::Uncertain,
            score: Some(0.5),
        };
        let final_verdict = VerificationVerdict {
            status: VerificationStatus::Entailed,
            score: Some(1.0),
        };
        let providers = vec![
            VerifierProvider {
                capability_id: "first".into(),
                provider: Arc::new(Verifier {
                    expected: None,
                    next: first.clone(),
                }),
            },
            VerifierProvider {
                capability_id: "second".into(),
                provider: Arc::new(Verifier {
                    expected: Some(first),
                    next: final_verdict.clone(),
                }),
            },
        ];
        let service: Arc<dyn crate::UtilityLlmService> = Arc::new(crate::DisabledUtilityLlmService);
        let out = verify_annotations(&providers, "αβγ", Some(&service), original.clone()).await;
        let expected = original
            .into_iter()
            .map(|mut a| {
                a.verified = Some(final_verdict.clone());
                a
            })
            .collect::<Vec<_>>();
        assert_eq!(out, expected);
    }

    #[test]
    fn citation_text_helpers_preserve_unicode_spans_and_distinct_overlap() {
        assert_eq!(
            citation_tokens("THE café Rust RUST, a xy 42 abc123"),
            ["café", "rust", "rust", "abc123"]
        );
        let words = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(token_overlap_ratio(&[], &words(&["a"])), 0.0);
        assert_eq!(token_overlap_ratio(&words(&["a"]), &[]), 0.0);
        assert_eq!(
            token_overlap_ratio(&words(&["a", "a", "b"]), &words(&["a", "a", "c"])),
            0.5
        );
        assert_eq!(
            token_overlap_ratio(&words(&["a", "b"]), &words(&["b", "a"])),
            1.0
        );
        assert_eq!(span_text("α😀z", 1, 2), "😀");
        assert_eq!(span_text("α😀z", 0, 3), "α😀z");
        assert_eq!(span_text("α😀z", 2, 1), "");
        assert_eq!(span_text("α😀z", 9, 12), "");
        assert_eq!(span_text("α😀z", 2, 12), "z");
    }
}
