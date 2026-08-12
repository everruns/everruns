//! End-to-end proof for the citation pipeline (see `knowledge/runtime-resources/citations.md`).
//!
//! Drives the *real* capabilities — `citation_retrieval` (feed) and
//! `citation_verification` (verifier) — through the public annotation seam,
//! exactly as the reason atom does, and proves the chain discriminates:
//!  - a grounded answer gets a citation, verified `entailed`;
//!  - a hallucinated answer gets no (false) citation;
//!  - swapping in the verifier is what adds verdicts — the "compare options" axis.
//!
//! No live LLM: the model's answer and the retrieval tool results are supplied
//! directly, so the test is deterministic and fast.

use everruns_core::annotation_hook::{
    AnnotationProvider, VerifierProvider, collect_annotations, verify_annotations,
};
use everruns_core::capabilities::Capability;
use everruns_core::message::{
    ContentPart, Message, MessageRole, ToolCallContentPart, ToolResultContentPart,
    VerificationStatus,
};
use everruns_platform::capabilities::{
    CitationRetrievalCapability, CitationVerificationCapability,
};
use serde_json::json;

/// A turn context in which the model called `search_index` and got two passages
/// back — one about photosynthesis, one about mitochondria.
fn context_with_search_results() -> Vec<Message> {
    let mut call = Message::assistant("");
    call.role = MessageRole::Agent;
    call.content = vec![ContentPart::ToolCall(ToolCallContentPart::new(
        "call_1",
        "search_index",
        json!({ "query": "photosynthesis" }),
    ))];

    let mut result = Message::assistant("");
    result.role = MessageRole::ToolResult;
    result.content = vec![ContentPart::ToolResult(ToolResultContentPart::new(
        "call_1",
        Some(json!({
            "results": [
                {
                    "id": "kchk_photo",
                    "source_uri": "github://o/r@main/bio.md",
                    "document_title": "Biology",
                    "snippet": "Photosynthesis converts sunlight into chemical energy stored in glucose.",
                    "location": { "lines": [10, 12] },
                    "score": 0.94
                },
                {
                    "id": "kchk_mito",
                    "source_uri": "github://o/r@main/cell.md",
                    "document_title": "Cells",
                    "snippet": "Mitochondria produce ATP through cellular respiration.",
                    "score": 0.71
                }
            ]
        })),
        None,
    ))];

    vec![call, result]
}

fn retrieval_providers() -> Vec<AnnotationProvider> {
    let cap = CitationRetrievalCapability;
    cap.post_output_annotation_hooks_with_config(&json!({}))
        .into_iter()
        .map(|provider| AnnotationProvider {
            capability_id: "citation_retrieval".to_string(),
            provider,
        })
        .collect()
}

fn verifier_providers(mode: &str) -> Vec<VerifierProvider> {
    let cap = CitationVerificationCapability;
    vec![VerifierProvider {
        capability_id: "citation_verification".to_string(),
        provider: cap
            .citation_verifier_with_config(&json!({ "mode": mode }))
            .expect("verifier"),
    }]
}

#[tokio::test]
async fn grounded_answer_is_cited_and_verified() {
    let messages = context_with_search_results();
    let answer = "Photosynthesis converts sunlight into chemical energy.";

    // Feed: retrieval attaches a citation to the grounded sentence.
    let collected = collect_annotations(&retrieval_providers(), "", answer, &messages, None).await;
    assert_eq!(
        collected.annotations.len(),
        1,
        "grounded answer should get exactly one citation"
    );
    assert_eq!(
        collected.annotations[0].external_id.as_deref(),
        Some("kchk_photo")
    );
    assert_eq!(
        collected.annotations[0].source.uri,
        "github://o/r@main/bio.md"
    );
    // Before verification there is no verdict.
    assert!(collected.annotations[0].verified.is_none());

    // Verifier (heuristic): stamps the citation entailed.
    let verified = verify_annotations(
        &verifier_providers("heuristic"),
        answer,
        None,
        collected.annotations,
    )
    .await;
    assert_eq!(
        verified[0].verified.as_ref().unwrap().status,
        VerificationStatus::Entailed,
        "the cited source supports the claim, so it verifies entailed"
    );
}

#[tokio::test]
async fn hallucinated_answer_is_not_falsely_cited() {
    let messages = context_with_search_results();
    // An answer unrelated to either retrieved passage.
    let answer = "The stock market rose three percent on Tuesday afternoon.";

    let collected = collect_annotations(&retrieval_providers(), "", answer, &messages, None).await;
    assert!(
        collected.annotations.is_empty(),
        "an ungrounded answer must not be decorated with false citations"
    );
}

#[tokio::test]
async fn verifier_is_the_axis_that_adds_verdicts() {
    // Same feed + same answer; the only difference is whether the verifier runs.
    // This is the "compare options" property: feeds and verifiers compose
    // independently, so an eval can vary one axis at a time.
    let messages = context_with_search_results();
    let answer = "Photosynthesis converts sunlight into chemical energy.";

    let without = collect_annotations(&retrieval_providers(), "", answer, &messages, None).await;
    assert!(without.annotations.iter().all(|a| a.verified.is_none()));

    let with = verify_annotations(
        &verifier_providers("heuristic"),
        answer,
        None,
        without.annotations.clone(),
    )
    .await;
    assert!(with.iter().all(|a| a.verified.is_some()));
    // Same citations either way — verification only annotates, never drops them.
    assert_eq!(without.annotations.len(), with.len());
}
