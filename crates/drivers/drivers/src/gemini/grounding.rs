//! Grounding with Google Search citations as [`WebCitation`]s for the
//! `citation_web` feed (see `everruns_contracts::web_citation`).
//!
//! A grounded answer's `groundingMetadata` lists the sources
//! (`groundingChunks[].web`) and which parts of the answer they support
//! (`groundingSupports[]`: a `segment` and the chunk indices). Segment offsets
//! count UTF-8 bytes. Whether they count from the start of the request's answer or of
//! one streamed part is not something a stream makes certain, so a segment is
//! kept only where its own `text` sits: at its byte range in that request's
//! streamed answer, or else at that text's single occurrence in it. A segment
//! that matches neither is dropped, never attached to a guess.

use everruns_contracts::web_citation::{CitedSpan, WebCitation};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct GroundingChunk {
    #[serde(default)]
    web: Option<WebChunk>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WebChunk {
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GroundingSupport {
    #[serde(default)]
    segment: Option<Segment>,
    #[serde(default)]
    grounding_chunk_indices: Vec<usize>,
}

/// Proto3 JSON omits zero values, so an absent `startIndex` is 0.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Segment {
    #[serde(default)]
    start_index: usize,
    #[serde(default)]
    end_index: usize,
    #[serde(default)]
    text: String,
}

/// One citation per (supported segment, web source) pair whose segment can be
/// found in `answer`, the streamed answer text.
pub(super) fn web_citations(
    answer: &str,
    chunks: &[GroundingChunk],
    supports: &[GroundingSupport],
) -> Vec<WebCitation> {
    let mut out = Vec::new();
    for support in supports {
        let Some(span) = support
            .segment
            .as_ref()
            .and_then(|segment| locate(answer, segment))
        else {
            continue;
        };
        for index in &support.grounding_chunk_indices {
            let Some(web) = chunks.get(*index).and_then(|chunk| chunk.web.as_ref()) else {
                continue;
            };
            let Some(uri) = web.uri.as_deref().filter(|uri| !uri.trim().is_empty()) else {
                continue;
            };
            out.push(WebCitation {
                url: uri.to_string(),
                title: web.title.clone().filter(|title| !title.trim().is_empty()),
                snippet: None,
                span: Some(span.clone()),
            });
        }
    }
    out
}

fn locate(answer: &str, segment: &Segment) -> Option<CitedSpan> {
    if segment.text.trim().is_empty() {
        return None;
    }
    if let Some(span) = CitedSpan::from_bytes(answer, segment.start_index, segment.end_index)
        && span.text == segment.text
    {
        return Some(span);
    }
    let mut matches = answer.match_indices(&segment.text);
    let (start, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    CitedSpan::from_bytes(answer, start, start + segment.text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(chunks: serde_json::Value, supports: serde_json::Value) -> Vec<WebCitation> {
        let chunks: Vec<GroundingChunk> = serde_json::from_value(chunks).unwrap();
        let supports: Vec<GroundingSupport> = serde_json::from_value(supports).unwrap();
        web_citations(ANSWER, &chunks, &supports)
    }

    // "Grüße 😀! " is 9 chars and 14 bytes.
    const ANSWER: &str = "Grüße 😀! Rust 1.95 ist da. Tokio folgt.";

    fn chunks() -> serde_json::Value {
        json!([
            { "web": { "uri": "https://vertexaisearch.example/a", "title": "rust-lang.org" } },
            { "web": { "uri": "https://vertexaisearch.example/b", "title": "tokio.rs" } },
            { "retrievedContext": { "uri": "gs://x" } }
        ])
    }

    #[test]
    fn byte_segments_become_char_spans() {
        let rust = ANSWER.find("Rust").unwrap();
        assert_eq!(rust, 14);
        let cites = parse(
            chunks(),
            json!([{ "segment": { "startIndex": rust, "endIndex": rust + 17,
                                   "text": "Rust 1.95 ist da." },
                     "groundingChunkIndices": [0, 2, 9] }]),
        );
        assert_eq!(cites.len(), 1, "non-web and missing chunks are skipped");
        let span = cites[0].span.as_ref().unwrap();
        assert_eq!((span.start, span.end), (9, 26));
        assert_eq!(span.text, "Rust 1.95 ist da.");
        assert_eq!(cites[0].url, "https://vertexaisearch.example/a");
        assert_eq!(cites[0].title.as_deref(), Some("rust-lang.org"));
    }

    #[test]
    fn segments_with_other_offsets_are_found_by_text_or_dropped() {
        // Offsets relative to a streamed part, not the answer: found by text.
        let cites = parse(
            chunks(),
            json!([
                { "segment": { "endIndex": 12, "text": "Tokio folgt." },
                  "groundingChunkIndices": [1] },
                // Offsets splitting "ü", and text not in the answer: dropped.
                { "segment": { "startIndex": 1, "endIndex": 2, "text": "x" },
                  "groundingChunkIndices": [0] },
                { "segment": { "startIndex": 0, "endIndex": 4, "text": "Nope" },
                  "groundingChunkIndices": [0] },
                { "groundingChunkIndices": [0] }
            ]),
        );
        assert_eq!(cites.len(), 1);
        let span = cites[0].span.as_ref().unwrap();
        let covered: String = ANSWER
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        assert_eq!(covered, "Tokio folgt.");
        assert_eq!(cites[0].title.as_deref(), Some("tokio.rs"));
    }
}
