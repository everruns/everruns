use super::*;
use everruns_contracts::web_citation::CitedSpan;
use everruns_core::message::{RuntimeMessageRole, ToolCallContentPart, ToolResultContentPart};
use serde_json::json;

const TOKIO: &str = "https://tokio.rs/blog/2026-10-tokio-1-50";

fn tool_call_msg(call_id: &str, tool: &str) -> RuntimeMessage {
    let mut m = RuntimeMessage::assistant("");
    m.role = RuntimeMessageRole::Agent;
    m.content = vec![ContentPart::ToolCall(ToolCallContentPart::new(
        call_id,
        tool,
        json!({"query": "tokio"}),
    ))];
    m
}

fn tool_result_msg(call_id: &str, result: Value) -> RuntimeMessage {
    let mut m = RuntimeMessage::assistant("");
    m.role = RuntimeMessageRole::ToolResult;
    m.content = vec![ContentPart::ToolResult(ToolResultContentPart::new(
        call_id,
        Some(result),
        None,
    ))];
    m
}

/// A turn in which the model ran `web_search` and got two results.
fn search_context() -> Vec<RuntimeMessage> {
    vec![
        tool_call_msg("c1", "web_search"),
        tool_result_msg(
            "c1",
            json!({"query": "tokio", "results": [
                {"id": web_result_id(TOKIO), "title": "Announcing Tokio 1.50", "url": TOKIO,
                 "site": "tokio.rs", "snippet": "Tokio 1.50 adds io_uring support."},
                {"id": web_result_id("https://docs.rs/tokio"), "title": "tokio - Rust",
                 "url": "https://docs.rs/tokio", "site": "docs.rs",
                 "excerpt": "An event-driven, non-blocking I/O platform."}
            ]}),
        ),
    ]
}

async fn annotate(
    text: &str,
    messages: &[RuntimeMessage],
    native: &[WebCitation],
) -> Vec<TextAnnotation> {
    let ctx = AnnotationContext {
        system_prompt: "",
        message_text: text,
        messages,
        web_citations: native,
        utility_llm_service: None,
    };
    WebAnnotationHook.annotate(&ctx).await.annotations
}

fn cited(text: &str, annotation: &TextAnnotation) -> String {
    span_text(text, annotation.start, annotation.end)
}

#[tokio::test]
async fn cites_the_sentence_holding_a_result_url() {
    let text = "Rust is fast. Tokio 1.50 adds io_uring ([announcement](https://TOKIO.rs/blog/2026-10-tokio-1-50/#io)). Done.";
    let anns = annotate(text, &search_context(), &[]).await;
    assert_eq!(anns.len(), 1);
    let ann = &anns[0];
    assert_eq!(
        cited(text, ann),
        "Tokio 1.50 adds io_uring ([announcement](https://TOKIO.rs/blog/2026-10-tokio-1-50/#io))."
    );
    assert_eq!(ann.origin, CITATION_WEB_CAPABILITY_ID);
    assert_eq!(ann.source.uri, TOKIO);
    assert_eq!(ann.source.title.as_deref(), Some("Announcing Tokio 1.50"));
    assert_eq!(
        ann.source.snippet.as_deref(),
        Some("Tokio 1.50 adds io_uring support.")
    );
    assert_eq!(ann.external_id, Some(web_result_id(TOKIO)));
}

#[tokio::test]
async fn cites_a_result_id_with_non_ascii_text_around_it() {
    let id = web_result_id("https://docs.rs/tokio");
    let text = format!("Zürich 😀 first. Tokio ist eine Laufzeit für I/O [{id}]. Ende.");
    let anns = annotate(&text, &search_context(), &[]).await;
    assert_eq!(anns.len(), 1);
    assert_eq!(
        cited(&text, &anns[0]),
        format!("Tokio ist eine Laufzeit für I/O [{id}].")
    );
    // The span is in chars: the emoji and umlauts before it count once each.
    assert_eq!(anns[0].start, "Zürich 😀 first. ".chars().count());
    // The excerpt stands in for a missing snippet.
    assert_eq!(
        anns[0].source.snippet.as_deref(),
        Some("An event-driven, non-blocking I/O platform.")
    );
}

#[tokio::test]
async fn a_trailing_citation_attaches_to_the_claim_before_it() {
    let text = format!("Tokio 1.50 adds io_uring.\nSource: {TOKIO}");
    let anns = annotate(&text, &search_context(), &[]).await;
    assert_eq!(anns.len(), 1);
    assert_eq!(cited(&text, &anns[0]), "Tokio 1.50 adds io_uring.");
}

#[tokio::test]
async fn matching_is_exact_not_fuzzy() {
    let messages = search_context();
    for text in [
        // Another page on the same site.
        "See https://tokio.rs/blog/2026-10-tokio-1-49 for details.".to_string(),
        // A different query string is a different page.
        format!("See {TOKIO}?page=2 for details."),
        // An id that is no result's, and one glued into a word.
        "See w000000 for details.".to_string(),
        format!("See x{} for details.", web_result_id(TOKIO)),
        // No URL at all.
        "Tokio 1.50 adds io_uring support.".to_string(),
    ] {
        assert!(annotate(&text, &messages, &[]).await.is_empty(), "{text}");
    }
}

#[tokio::test]
async fn ignores_other_tools_and_handles_brave_results_by_url() {
    let url = "https://example.com/brave";
    let text = format!("Example is cited ({url}).");
    let other = vec![
        tool_call_msg("c1", "search_index"),
        tool_result_msg(
            "c1",
            json!({"results": [{"id": "x", "url": url, "snippet": "s"}]}),
        ),
    ];
    assert!(annotate(&text, &other, &[]).await.is_empty());

    let brave = vec![
        tool_call_msg("c1", "brave_web_search"),
        tool_result_msg(
            "c1",
            json!({"query": "q", "count": 1, "results": [
                {"title": "Brave hit", "url": url, "description": "From Brave."}
            ]}),
        ),
    ];
    let anns = annotate(&text, &brave, &[]).await;
    assert_eq!(anns.len(), 1);
    assert_eq!(anns[0].external_id, Some(web_result_id(url)));
    assert_eq!(anns[0].source.snippet.as_deref(), Some("From Brave."));
}

#[tokio::test]
async fn native_citations_keep_their_span_when_the_text_is_unchanged() {
    let text = "Größe zählt 😀. Tokio 1.50 adds io_uring. Bye.";
    let claim = "Tokio 1.50 adds io_uring.";
    let start = text.find(claim).unwrap();
    let span = CitedSpan::from_bytes(text, start, start + claim.len()).unwrap();
    let native = [WebCitation {
        url: TOKIO.to_string(),
        title: None,
        snippet: Some("Tokio 1.50 ships io_uring.".to_string()),
        span: Some(span.clone()),
    }];
    let anns = annotate(text, &[], &native).await;
    assert_eq!(anns.len(), 1);
    assert_eq!((anns[0].start, anns[0].end), (span.start, span.end));
    assert_eq!(cited(text, &anns[0]), claim);
    assert_eq!(anns[0].origin, CITATION_WEB_CAPABILITY_ID);
    assert_eq!(anns[0].external_id, Some(web_result_id(TOKIO)));
    assert_eq!(
        anns[0].source.snippet.as_deref(),
        Some("Tokio 1.50 ships io_uring.")
    );

    // With the search in context, the result fills in a missing title.
    let anns = annotate(text, &search_context(), &native).await;
    assert_eq!(
        anns[0].source.title.as_deref(),
        Some("Announcing Tokio 1.50")
    );
}

#[tokio::test]
async fn native_spans_follow_moved_text_or_are_dropped() {
    let claim = "Tokio 1.50 adds io_uring.";
    let streamed = format!("Intro. {claim}");
    let span = CitedSpan::from_chars(&streamed, 7, 7 + claim.chars().count()).unwrap();
    let native = [WebCitation {
        url: TOKIO.to_string(),
        span: Some(span),
        ..Default::default()
    }];
    // A filter shortened the text: the claim moved, once.
    let moved = format!("Ü. {claim}");
    let anns = annotate(&moved, &[], &native).await;
    assert_eq!(cited(&moved, &anns[0]), claim);
    // The claim is gone, or occurs twice: no citation rather than a wrong one.
    assert!(
        annotate("Intro. Something else.", &[], &native)
            .await
            .is_empty()
    );
    let twice = format!("{claim} {claim}");
    assert!(annotate(&twice, &[], &native).await.is_empty());
}

#[tokio::test]
async fn native_citations_without_a_span_are_placed_by_url() {
    let text = format!("Tokio 1.50 adds io_uring ({TOKIO}).");
    let native = [WebCitation {
        url: format!("{TOKIO}/"),
        title: Some("Tokio".to_string()),
        ..Default::default()
    }];
    let anns = annotate(&text, &[], &native).await;
    assert_eq!(anns.len(), 1);
    assert_eq!(cited(&text, &anns[0]), text);
    assert!(annotate("No link here.", &[], &native).await.is_empty());
}

#[tokio::test]
async fn the_same_source_on_the_same_span_is_cited_once() {
    let text = format!("Tokio 1.50 adds io_uring ({TOKIO}).");
    let native = [WebCitation {
        url: TOKIO.to_string(),
        ..Default::default()
    }];
    let anns = annotate(&text, &search_context(), &native).await;
    assert_eq!(anns.len(), 1);
}

#[test]
fn url_normalization() {
    let norm = |url: &str| normalize_url(url);
    assert_eq!(
        norm("HTTPS://Tokio.RS/Blog/#x"),
        Some("https://tokio.rs/Blog".into())
    );
    assert_eq!(norm("https://tokio.rs"), norm("https://tokio.rs/"));
    assert_eq!(
        norm("https://a.io/x?b=1#c"),
        Some("https://a.io/x?b=1".into())
    );
    assert_ne!(norm("https://a.io/x"), norm("http://a.io/x"));
    assert_eq!(norm("ftp://a.io/x"), None);
    assert_eq!(norm("https:///x"), None);
    assert_eq!(norm("not a url"), None);
}

#[test]
fn url_extraction() {
    let urls = |text: &str| {
        let chars: Vec<char> = text.chars().collect();
        find_urls(&chars)
            .into_iter()
            .map(|(start, end, url)| {
                assert_eq!(chars[start..end].iter().collect::<String>(), url);
                url
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        urls(
            "See [x](https://a.io/b). And <https://c.io/d>, https://en.wikipedia.org/wiki/Rust_(language)."
        ),
        [
            "https://a.io/b",
            "https://c.io/d",
            "https://en.wikipedia.org/wiki/Rust_(language)"
        ]
    );
    assert_eq!(urls("Ü https://ä.de/ö?x=1!"), ["https://ä.de/ö?x=1"]);
    assert!(urls("xhttps://a.io and https:// alone").is_empty());
}

#[test]
fn capability_shape() {
    let cap = CitationWebCapability;
    assert_eq!(cap.id(), CITATION_WEB_CAPABILITY_ID);
    assert_eq!(cap.features(), vec!["citations"]);
    assert_eq!(
        cap.post_output_annotation_hooks_with_config(&Value::Null)
            .len(),
        1
    );
    assert!(cap.validate_config(&Value::Null).is_ok());
}
