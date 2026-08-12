//! `citation_retrieval` capability — claim-level citations from retrieval tools.
//!
//! Turns the sources surfaced by `search_index` / `search_knowledge` into
//! claim-level provenance on the assistant's answer. It contributes no tools of
//! its own; instead it registers a post-generation annotation hook (see
//! [`everruns_core::annotation_hook`]) that, once the model has answered, scans the
//! turn's retrieval tool results, aligns each retrieved passage to the sentence
//! it best supports, and attaches a [`TextAnnotation`] there. Alignment is
//! deterministic token overlap — no extra model call, no text rewrite — so it is
//! model- and provider-agnostic and never changes the streamed answer.
//!
//! See `knowledge/runtime-resources/citations.md`. This is the retrieval *feed*; verification is a
//! separate `citation_verification` capability.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use everruns_core::annotation_hook::{
    AnnotationContext, AnnotationResult, PostGenerationAnnotationHook, citation_tokens,
    token_overlap_ratio,
};
use everruns_core::capabilities::Capability;
use everruns_core::capability_types::CapabilityStatus;
use everruns_core::message::{AnnotationSource, ContentPart, Message, TextAnnotation};

/// Canonical capability id.
pub const CITATION_RETRIEVAL_CAPABILITY_ID: &str = "citation_retrieval";

/// Tool names whose results carry retrieval citations.
const RETRIEVAL_TOOLS: &[&str] = &["search_index", "search_knowledge"];

/// Default minimum token-overlap ratio for a passage to be attached to a
/// sentence. Overlap is `|shared tokens| / |passage tokens|`, so 0.5 means at
/// least half the passage's distinctive words appear in the sentence.
const DEFAULT_MIN_OVERLAP: f32 = 0.5;

/// Per-agent config for `citation_retrieval`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationRetrievalConfig {
    /// Minimum overlap ratio in `[0, 1]` for a citation to attach.
    #[serde(default = "default_min_overlap")]
    pub min_overlap: f32,
}

fn default_min_overlap() -> f32 {
    DEFAULT_MIN_OVERLAP
}

impl Default for CitationRetrievalConfig {
    fn default() -> Self {
        Self {
            min_overlap: DEFAULT_MIN_OVERLAP,
        }
    }
}

impl CitationRetrievalConfig {
    fn from_value(config: &serde_json::Value) -> Self {
        if config.is_null() {
            return Self::default();
        }
        serde_json::from_value(config.clone()).unwrap_or_default()
    }
}

/// The retrieval citation feed.
pub struct CitationRetrievalCapability;

impl Capability for CitationRetrievalCapability {
    fn id(&self) -> &str {
        CITATION_RETRIEVAL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Retrieval citations"
    }

    fn description(&self) -> &str {
        "Attach claim-level citations to the answer from search_index / \
         search_knowledge results, so each grounded sentence links to its source."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("quote")
    }

    fn category(&self) -> Option<&str> {
        Some("Knowledge")
    }

    /// Surfaces the citation UI when any citation feed is active.
    fn features(&self) -> Vec<&'static str> {
        vec!["citations"]
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "min_overlap": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "default": DEFAULT_MIN_OVERLAP,
                    "description": "Minimum token-overlap ratio for a retrieved passage to be cited on a sentence."
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let cfg: CitationRetrievalConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid citation_retrieval config: {e}"))?;
        if !(0.0..=1.0).contains(&cfg.min_overlap) {
            return Err("min_overlap must be between 0.0 and 1.0".to_string());
        }
        Ok(())
    }

    fn post_output_annotation_hooks_with_config(
        &self,
        config: &serde_json::Value,
    ) -> Vec<Arc<dyn PostGenerationAnnotationHook>> {
        let cfg = CitationRetrievalConfig::from_value(config);
        vec![Arc::new(RetrievalAnnotationHook {
            min_overlap: cfg.min_overlap,
        })]
    }
}

/// The annotation hook: extract retrieval citations from the turn's tool
/// results, then align each to the sentence it best supports.
struct RetrievalAnnotationHook {
    min_overlap: f32,
}

#[async_trait]
impl PostGenerationAnnotationHook for RetrievalAnnotationHook {
    fn id(&self) -> &str {
        CITATION_RETRIEVAL_CAPABILITY_ID
    }

    async fn annotate(&self, ctx: &AnnotationContext<'_>) -> AnnotationResult {
        let citations = extract_retrieval_citations(ctx.messages);
        if citations.is_empty() {
            return AnnotationResult::none();
        }
        let annotations = align_citations(ctx.message_text, &citations, self.min_overlap);
        AnnotationResult {
            annotations,
            rewritten_text: None,
        }
    }
}

/// A retrieval citation extracted from a tool result, normalized across the
/// `search_index` and `search_knowledge` result shapes.
#[derive(Debug, Clone)]
struct RetrievalCitation {
    external_id: Option<String>,
    uri: String,
    title: Option<String>,
    snippet: String,
    location: Option<serde_json::Value>,
}

/// Scan the assembled context for retrieval tool results and normalize their
/// entries. Tool results carry only a `tool_call_id`, so we first map call ids
/// to tool names from the assistant `ToolCall` parts.
fn extract_retrieval_citations(messages: &[Message]) -> Vec<RetrievalCitation> {
    let mut tool_names: HashMap<&str, &str> = HashMap::new();
    for msg in messages {
        for part in &msg.content {
            if let ContentPart::ToolCall(call) = part {
                tool_names.insert(call.id.as_str(), call.name.as_str());
            }
        }
    }

    let mut out = Vec::new();
    for msg in messages {
        for part in &msg.content {
            let ContentPart::ToolResult(result) = part else {
                continue;
            };
            let Some(name) = tool_names.get(result.tool_call_id.as_str()) else {
                continue;
            };
            if !RETRIEVAL_TOOLS.contains(name) {
                continue;
            }
            let Some(value) = &result.result else {
                continue;
            };
            let Some(results) = value.get("results").and_then(|r| r.as_array()) else {
                continue;
            };
            for entry in results {
                if let Some(citation) = normalize_entry(entry) {
                    out.push(citation);
                }
            }
        }
    }
    out
}

/// Normalize one result object, tolerating either the `KnowledgeIndexCitation`
/// (`source_uri` / `document_title`) or `KnowledgeSearchHit` (`resource` /
/// `title`) shape. Entries without a usable snippet are skipped — there is
/// nothing to align.
fn normalize_entry(entry: &serde_json::Value) -> Option<RetrievalCitation> {
    let snippet = entry.get("snippet").and_then(|s| s.as_str())?.trim();
    if snippet.is_empty() {
        return None;
    }
    let external_id = entry.get("id").and_then(|v| v.as_str()).map(str::to_string);
    let title = entry
        .get("document_title")
        .or_else(|| entry.get("title"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let uri = entry
        .get("source_uri")
        .or_else(|| entry.get("resource"))
        .or_else(|| entry.get("uri"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            external_id
                .as_ref()
                .map(|id| format!("everruns://citation/{id}"))
        })
        .unwrap_or_else(|| "everruns://citation".to_string());
    Some(RetrievalCitation {
        external_id,
        uri,
        title,
        snippet: snippet.to_string(),
        location: entry.get("location").cloned(),
    })
}

/// Attach each citation to the single sentence in `text` with the highest token
/// overlap against the citation's snippet, when that overlap clears
/// `min_overlap`.
fn align_citations(
    text: &str,
    citations: &[RetrievalCitation],
    min_overlap: f32,
) -> Vec<TextAnnotation> {
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return Vec::new();
    }
    let sentence_tokens: Vec<Vec<String>> =
        sentences.iter().map(|s| citation_tokens(&s.text)).collect();

    let mut annotations = Vec::new();
    for citation in citations {
        let needle = citation_tokens(&citation.snippet);
        if needle.is_empty() {
            continue;
        }
        let mut best: Option<(usize, f32)> = None;
        for (idx, tokens) in sentence_tokens.iter().enumerate() {
            let ratio = token_overlap_ratio(&needle, tokens);
            if best.map(|(_, b)| ratio > b).unwrap_or(true) {
                best = Some((idx, ratio));
            }
        }
        if let Some((idx, ratio)) = best
            && ratio >= min_overlap
        {
            let sentence = &sentences[idx];
            annotations.push(TextAnnotation {
                start: sentence.start,
                end: sentence.end,
                origin: CITATION_RETRIEVAL_CAPABILITY_ID.to_string(),
                source: AnnotationSource {
                    uri: citation.uri.clone(),
                    title: citation.title.clone(),
                    snippet: Some(citation.snippet.clone()),
                    location: citation.location.clone(),
                },
                external_id: citation.external_id.clone(),
                verified: None,
            });
        }
    }
    annotations
}

/// A sentence and its char span (0-indexed, exclusive end) into the source text.
struct Sentence {
    start: usize,
    end: usize,
    text: String,
}

/// Split into sentences on `.`, `!`, `?`, and newlines, tracking char offsets.
/// Approximate (markdown-agnostic) but sufficient for span attachment.
fn split_sentences(text: &str) -> Vec<Sentence> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let is_break = matches!(c, '.' | '!' | '?' | '\n');
        let at_end = i + 1 == chars.len();
        if is_break || at_end {
            let end = i + 1;
            let segment: String = chars[start..end].iter().collect();
            if !segment.trim().is_empty() {
                // Trim leading whitespace from the span so chips sit on words.
                let lead_ws = segment.chars().take_while(|c| c.is_whitespace()).count();
                out.push(Sentence {
                    start: start + lead_ws,
                    end,
                    text: segment,
                });
            }
            start = end;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::message::{MessageRole, ToolCallContentPart, ToolResultContentPart};

    fn tool_call_msg(call_id: &str, tool: &str) -> Message {
        let mut m = Message::assistant("");
        m.role = MessageRole::Agent;
        m.content = vec![ContentPart::ToolCall(ToolCallContentPart::new(
            call_id,
            tool,
            json!({}),
        ))];
        m
    }

    fn tool_result_msg(call_id: &str, result: serde_json::Value) -> Message {
        let mut m = Message::assistant("");
        m.role = MessageRole::ToolResult;
        m.content = vec![ContentPart::ToolResult(ToolResultContentPart::new(
            call_id,
            Some(result),
            None,
        ))];
        m
    }

    #[test]
    fn extracts_index_and_knowledge_shapes() {
        let messages = vec![
            tool_call_msg("c1", "search_index"),
            tool_result_msg(
                "c1",
                json!({"results": [{
                    "id": "kchk_1",
                    "source_uri": "github://o/r@main/a.md",
                    "document_title": "A",
                    "snippet": "Photosynthesis converts sunlight into chemical energy.",
                    "location": {"lines": [1, 4]},
                    "score": 0.9
                }]}),
            ),
            tool_call_msg("c2", "search_knowledge"),
            tool_result_msg(
                "c2",
                json!({"count": 1, "results": [{
                    "id": "kbe_2",
                    "kb_id": "kb_x",
                    "title": "B",
                    "kind": "note",
                    "tags": [],
                    "snippet": "The mitochondria is the powerhouse of the cell."
                }]}),
            ),
        ];
        let cites = extract_retrieval_citations(&messages);
        assert_eq!(cites.len(), 2);
        assert_eq!(cites[0].external_id.as_deref(), Some("kchk_1"));
        assert_eq!(cites[0].uri, "github://o/r@main/a.md");
        assert_eq!(cites[1].external_id.as_deref(), Some("kbe_2"));
        // KnowledgeSearchHit with no `resource` falls back to a synthetic uri.
        assert_eq!(cites[1].uri, "everruns://citation/kbe_2");
    }

    #[test]
    fn ignores_non_retrieval_tools() {
        let messages = vec![
            tool_call_msg("c1", "bash"),
            tool_result_msg(
                "c1",
                json!({"results": [{"id": "x", "snippet": "hi there"}]}),
            ),
        ];
        assert!(extract_retrieval_citations(&messages).is_empty());
    }

    #[test]
    fn aligns_citation_to_best_sentence() {
        let text = "Plants are green. Photosynthesis converts sunlight into chemical energy. \
                    Cells divide.";
        let cites = vec![RetrievalCitation {
            external_id: Some("kchk_1".to_string()),
            uri: "github://o/r@main/a.md".to_string(),
            title: Some("A".to_string()),
            snippet: "Photosynthesis converts sunlight into chemical energy.".to_string(),
            location: None,
        }];
        let anns = align_citations(text, &cites, 0.5);
        assert_eq!(anns.len(), 1);
        let ann = &anns[0];
        let cited: String = text
            .chars()
            .skip(ann.start)
            .take(ann.end - ann.start)
            .collect();
        assert!(cited.contains("Photosynthesis converts sunlight"));
        assert_eq!(ann.external_id.as_deref(), Some("kchk_1"));
        assert_eq!(ann.origin, CITATION_RETRIEVAL_CAPABILITY_ID);
    }

    #[test]
    fn drops_low_overlap_citations() {
        let text = "The weather today is sunny and warm.";
        let cites = vec![RetrievalCitation {
            external_id: None,
            uri: "u".to_string(),
            title: None,
            snippet: "Quantum chromodynamics describes the strong interaction.".to_string(),
            location: None,
        }];
        assert!(align_citations(text, &cites, 0.5).is_empty());
    }

    #[test]
    fn config_validation_rejects_out_of_range() {
        let cap = CitationRetrievalCapability;
        assert!(cap.validate_config(&json!({"min_overlap": 1.5})).is_err());
        assert!(cap.validate_config(&json!({"min_overlap": 0.3})).is_ok());
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
    }
}
