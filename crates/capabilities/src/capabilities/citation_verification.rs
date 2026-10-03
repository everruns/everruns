//! `citation_verification` capability — stamps faithfulness verdicts on
//! citations produced by any feed.
//!
//! A standalone guardrail capability, decoupled from the citation feeds: it
//! consumes the [`TextAnnotation`]s collected during a turn and, for each,
//! decides whether the cited source actually supports the claim span, writing a
//! [`VerificationVerdict`]. Because it is separate from the feeds, any feed
//! (`citation_retrieval`, a native provider feed, …) can be paired with it, and
//! evals can hold the feed fixed while varying the verifier. See
//! `knowledge/runtime-resources/citations.md`.
//!
//! Two modes:
//! * `heuristic` (default) — lexical entailment (token overlap between the claim
//!   span and the source snippet). Deterministic, free, model-agnostic; a weak
//!   but honest NLI baseline, strongest on verbatim/native citations.
//! * `llm` — a utility-model judgement per claim/source pair. More accurate;
//!   falls back to heuristic when no utility model is configured.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use everruns_core::annotation_hook::{
    CitationVerifier, VerificationContext, citation_tokens, span_text, token_overlap_ratio,
};
use everruns_core::capabilities::Capability;
use everruns_core::capability_types::CapabilityStatus;
use everruns_core::message::{TextAnnotation, VerificationStatus, VerificationVerdict};
use everruns_core::utility_llm::UtilityLlmRequest;

/// Canonical capability id.
pub const CITATION_VERIFICATION_CAPABILITY_ID: &str = "citation_verification";

/// Default entailment threshold: a claim whose distinctive tokens are at least
/// half-covered by the source is treated as entailed.
const DEFAULT_THRESHOLD: f32 = 0.5;

/// Verification strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    /// Deterministic lexical entailment. No model call.
    #[default]
    Heuristic,
    /// Utility-model judgement, with heuristic fallback.
    Llm,
}

/// Per-agent config for `citation_verification`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationVerificationConfig {
    #[serde(default)]
    pub mode: VerificationMode,
    #[serde(default = "default_threshold")]
    pub threshold: f32,
}

fn default_threshold() -> f32 {
    DEFAULT_THRESHOLD
}

impl Default for CitationVerificationConfig {
    fn default() -> Self {
        Self {
            mode: VerificationMode::default(),
            threshold: DEFAULT_THRESHOLD,
        }
    }
}

impl CitationVerificationConfig {
    fn from_value(config: &serde_json::Value) -> Self {
        if config.is_null() {
            return Self::default();
        }
        serde_json::from_value(config.clone()).unwrap_or_default()
    }
}

/// The verification guardrail capability.
pub struct CitationVerificationCapability;

impl Capability for CitationVerificationCapability {
    fn id(&self) -> &str {
        CITATION_VERIFICATION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Citation verification"
    }

    fn description(&self) -> &str {
        "Verify that each cited source actually supports the claim it is \
         attached to, stamping a faithfulness verdict on every citation."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("shield-check")
    }

    fn category(&self) -> Option<&str> {
        Some("Knowledge")
    }

    /// A constraint on citations rather than a new ability.
    fn is_guardrail(&self) -> bool {
        true
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["citations"]
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["heuristic", "llm"],
                    "default": "heuristic",
                    "description": "heuristic = lexical overlap (no model call); llm = utility-model judgement with heuristic fallback."
                },
                "threshold": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "default": DEFAULT_THRESHOLD,
                    "description": "Entailment threshold for the heuristic verdict."
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let cfg: CitationVerificationConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid citation_verification config: {e}"))?;
        if !(0.0..=1.0).contains(&cfg.threshold) {
            return Err("threshold must be between 0.0 and 1.0".to_string());
        }
        Ok(())
    }

    fn citation_verifier_with_config(
        &self,
        config: &serde_json::Value,
    ) -> Option<Arc<dyn CitationVerifier>> {
        let cfg = CitationVerificationConfig::from_value(config);
        Some(Arc::new(CitationVerificationVerifier {
            mode: cfg.mode,
            threshold: cfg.threshold,
        }))
    }
}

struct CitationVerificationVerifier {
    mode: VerificationMode,
    threshold: f32,
}

#[async_trait]
impl CitationVerifier for CitationVerificationVerifier {
    fn id(&self) -> &str {
        CITATION_VERIFICATION_CAPABILITY_ID
    }

    async fn verify(
        &self,
        ctx: &VerificationContext<'_>,
        mut annotations: Vec<TextAnnotation>,
    ) -> Vec<TextAnnotation> {
        if annotations.is_empty() {
            return annotations;
        }

        // Try the LLM path first when requested and available; on any failure
        // fall through to the deterministic heuristic (fail-open).
        if self.mode == VerificationMode::Llm
            && let Some(svc) = ctx.utility_llm_service
            && let Some(verdicts) = llm_verdicts(svc, ctx.message_text, &annotations).await
            && verdicts.len() == annotations.len()
        {
            for (ann, verdict) in annotations.iter_mut().zip(verdicts) {
                ann.verified = Some(verdict);
            }
            return annotations;
        }

        for ann in annotations.iter_mut() {
            ann.verified = Some(heuristic_verdict(ctx.message_text, ann, self.threshold));
        }
        annotations
    }
}

/// Lexical entailment: fraction of the claim span's distinctive tokens present
/// in the cited source snippet.
fn heuristic_verdict(text: &str, ann: &TextAnnotation, threshold: f32) -> VerificationVerdict {
    let claim = span_text(text, ann.start, ann.end);
    let snippet = ann.source.snippet.as_deref().unwrap_or("");
    let claim_tokens = citation_tokens(&claim);
    let snippet_tokens = citation_tokens(snippet);
    let ratio = token_overlap_ratio(&claim_tokens, &snippet_tokens);
    let status = if ratio >= threshold {
        VerificationStatus::Entailed
    } else if ratio >= threshold * 0.5 {
        VerificationStatus::Uncertain
    } else {
        VerificationStatus::Unsupported
    };
    VerificationVerdict {
        status,
        score: Some(ratio),
    }
}

/// Ask the utility model to judge every claim/source pair in one call. Returns
/// `None` on any transport/parse failure so the caller can fall back.
async fn llm_verdicts(
    svc: &Arc<dyn everruns_core::UtilityLlmService>,
    text: &str,
    annotations: &[TextAnnotation],
) -> Option<Vec<VerificationVerdict>> {
    let prompt = build_verification_prompt(text, annotations);
    let request = UtilityLlmRequest::user_text(prompt)
        .with_temperature(0.0)
        .with_max_tokens(700)
        .with_metadata("purpose", "citation_verification");
    let response = svc.chat_completion(request).await.ok()?;
    parse_verification_response(&response.text, annotations.len())
}

/// Build a compact JSON-in/JSON-out verification prompt.
fn build_verification_prompt(text: &str, annotations: &[TextAnnotation]) -> String {
    let mut pairs = String::new();
    for (i, ann) in annotations.iter().enumerate() {
        let claim = span_text(text, ann.start, ann.end);
        let snippet = ann.source.snippet.as_deref().unwrap_or("");
        pairs.push_str(&format!(
            "[{i}]\nCLAIM: {}\nSOURCE: {}\n\n",
            claim.trim(),
            snippet.trim()
        ));
    }
    format!(
        "You verify citations. For each numbered pair, decide whether the SOURCE \
         supports the CLAIM.\n\
         Reply with ONLY a JSON array; one object per pair, in order:\n\
         [{{\"index\": 0, \"status\": \"entailed|unsupported|uncertain\", \"score\": 0.0}}]\n\
         `entailed` = the source clearly supports the claim; `unsupported` = it does \
         not; `uncertain` = cannot tell. `score` is your confidence in [0,1].\n\n\
         {pairs}"
    )
}

/// Parse the model's JSON array back into per-annotation verdicts, tolerating
/// code fences and surrounding prose. Returns `None` unless exactly `expected`
/// verdicts are recovered in index order.
fn parse_verification_response(raw: &str, expected: usize) -> Option<Vec<VerificationVerdict>> {
    let json = extract_json_array(raw)?;
    let array = json.as_array()?;
    if array.len() != expected {
        return None;
    }
    let mut out = Vec::with_capacity(expected);
    for entry in array {
        let status = match entry.get("status").and_then(|s| s.as_str())? {
            "entailed" => VerificationStatus::Entailed,
            "unsupported" => VerificationStatus::Unsupported,
            _ => VerificationStatus::Uncertain,
        };
        let score = entry
            .get("score")
            .and_then(|s| s.as_f64())
            .map(|s| s.clamp(0.0, 1.0) as f32);
        out.push(VerificationVerdict { status, score });
    }
    Some(out)
}

/// Extract the first top-level JSON array from a model response.
fn extract_json_array(raw: &str) -> Option<serde_json::Value> {
    let start = raw.find('[')?;
    let end = raw.rfind(']')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&raw[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::message::AnnotationSource;

    fn annotation(start: usize, end: usize, snippet: &str) -> TextAnnotation {
        TextAnnotation {
            start,
            end,
            origin: "citation_retrieval".to_string(),
            source: AnnotationSource {
                uri: "u".to_string(),
                title: None,
                snippet: Some(snippet.to_string()),
                location: None,
            },
            external_id: None,
            verified: None,
        }
    }

    #[test]
    fn heuristic_marks_supported_claim_entailed() {
        let text = "Photosynthesis converts sunlight into chemical energy.";
        let ann = annotation(
            0,
            text.chars().count(),
            "Photosynthesis converts sunlight into chemical energy in plants.",
        );
        let verdict = heuristic_verdict(text, &ann, 0.5);
        assert_eq!(verdict.status, VerificationStatus::Entailed);
    }

    #[test]
    fn heuristic_marks_unrelated_claim_unsupported() {
        let text = "The stock market rallied on strong earnings today.";
        let ann = annotation(
            0,
            text.chars().count(),
            "Mitochondria produce ATP through cellular respiration.",
        );
        let verdict = heuristic_verdict(text, &ann, 0.5);
        assert_eq!(verdict.status, VerificationStatus::Unsupported);
    }

    #[test]
    fn parses_verdict_array_with_code_fence() {
        let raw = "```json\n[{\"index\":0,\"status\":\"entailed\",\"score\":0.9},\
                   {\"index\":1,\"status\":\"unsupported\",\"score\":0.1}]\n```";
        let verdicts = parse_verification_response(raw, 2).expect("parsed");
        assert_eq!(verdicts[0].status, VerificationStatus::Entailed);
        assert_eq!(verdicts[1].status, VerificationStatus::Unsupported);
        assert!((verdicts[0].score.unwrap() - 0.9).abs() < 1e-6);
    }

    #[test]
    fn parse_rejects_wrong_count() {
        let raw = "[{\"index\":0,\"status\":\"entailed\"}]";
        assert!(parse_verification_response(raw, 2).is_none());
    }

    #[test]
    fn config_validation_rejects_bad_threshold() {
        let cap = CitationVerificationCapability;
        assert!(cap.validate_config(&json!({"threshold": 2.0})).is_err());
        assert!(
            cap.validate_config(&json!({"mode": "llm", "threshold": 0.4}))
                .is_ok()
        );
    }

    #[tokio::test]
    async fn verifier_stamps_all_annotations_heuristically() {
        let cap = CitationVerificationCapability;
        let verifier = cap
            .citation_verifier_with_config(&json!({"mode": "heuristic"}))
            .expect("verifier");
        let text = "Water boils at 100 degrees Celsius at sea level.";
        let anns = vec![annotation(
            0,
            text.chars().count(),
            "At sea level water boils at 100 degrees Celsius.",
        )];
        let ctx = VerificationContext {
            message_text: text,
            utility_llm_service: None,
        };
        let out = verifier.verify(&ctx, anns).await;
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].verified.as_ref().unwrap().status,
            VerificationStatus::Entailed
        );
    }
}
