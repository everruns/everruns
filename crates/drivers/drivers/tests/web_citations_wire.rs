#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Wire tests for native web search citations on the Responses API drivers.
//
// OpenAI's hosted `web_search` and OpenRouter's `openrouter:web_search` both
// annotate the answer's `output_text` with `url_citation`s, indexed in chars
// of that output text. The driver reports them on `Done` as web citations
// whose spans are chars of the whole streamed answer, for the `citation_web`
// feed. Anthropic and Gemini have their own tests
// (`anthropic_web_search_wire.rs`, the Gemini driver's unit tests).

// Every mock answers with `Connection: close` (see anthropic_computer_toolset_wire.rs).

use everruns_contracts::BearerAuth;
use everruns_contracts::Provider;
use everruns_contracts::driver_registry::{LlmCallConfig, LlmStreamEvent, Message, MessageRole};
use everruns_contracts::web_citation::WebCitation;
use everruns_drivers::openai::OpenAIChatDriver;
use everruns_drivers::openrouter::OpenRouterChatDriver;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIRST: &str = "Grüße 😀! ";
const SECOND: &str = "Rust 1.95 ist da ([blog.rust-lang.org](https://blog.rust-lang.org/)).";

fn data(frame: Value) -> String {
    format!("data: {frame}\n\n")
}

/// A streamed answer in two deltas, then `response.completed` with `output`.
fn body(output: Value) -> String {
    [
        data(
            json!({ "type": "response.output_text.delta", "item_id": "msg_1",
                     "output_index": 1, "content_index": 0, "delta": FIRST }),
        ),
        data(
            json!({ "type": "response.output_text.delta", "item_id": "msg_1",
                     "output_index": 1, "content_index": 0, "delta": SECOND }),
        ),
        data(
            json!({ "type": "response.completed", "sequence_number": 9, "response": {
            "id": "resp_1", "object": "response", "created_at": 0, "model": "m",
            "status": "completed", "output": output,
            "usage": { "input_tokens": 10, "output_tokens": 5, "total_tokens": 15 } } }),
        ),
    ]
    .concat()
}

fn message(annotations: Value) -> Value {
    json!({ "type": "message", "id": "msg_1", "status": "completed", "role": "assistant",
            "content": [{ "type": "output_text", "text": format!("{FIRST}{SECOND}"),
                          "annotations": annotations }] })
}

async fn citations(
    provider: impl FnOnce(String) -> Provider,
    body: String,
) -> (String, Vec<WebCitation>) {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_raw(body, "text/event-stream"),
        )
        .mount(&server)
        .await;
    let mut stream = provider(format!("{}/v1", server.uri()))
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "what is new in Rust?")],
            &LlmCallConfig::new("m"),
        )
        .await
        .expect("stream should start");
    let mut text = String::new();
    let mut done = None;
    while let Some(event) = stream.next().await {
        match event.expect("no transport error") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::Done(metadata) => done = Some(metadata.web_citations),
            _ => {}
        }
    }
    (text, done.expect("stream ends with done"))
}

fn covered(text: &str, citation: &WebCitation) -> String {
    let span = citation.span.as_ref().expect("citation has a span");
    let covered: String = text
        .chars()
        .skip(span.start)
        .take(span.end - span.start)
        .collect();
    assert_eq!(covered, span.text);
    covered
}

/// `start` and `end` of `needle` in chars of the answer.
fn char_range(needle: &str) -> (usize, usize) {
    let answer = format!("{FIRST}{SECOND}");
    let byte = answer.find(needle).unwrap();
    let start = answer[..byte].chars().count();
    (start, start + needle.chars().count())
}

#[tokio::test]
async fn openai_url_citations_become_web_citations_in_streamed_chars() {
    let link = "([blog.rust-lang.org](https://blog.rust-lang.org/))";
    let (start, end) = char_range(link);
    // A search call item in the output, as OpenAI returns with web search.
    let output = json!([
        { "type": "web_search_call", "id": "ws_1", "status": "completed",
          "action": { "type": "search", "query": "rust 1.95" } },
        message(json!([{ "type": "url_citation", "url": "https://blog.rust-lang.org/",
                         "title": "Rust Blog", "start_index": start, "end_index": end }]))
    ]);
    let (text, cites) = citations(
        |url| {
            Provider::new("openai-test", OpenAIChatDriver::new())
                .base_url(url)
                .auth(BearerAuth::new("k"))
        },
        body(output),
    )
    .await;
    assert_eq!(text, format!("{FIRST}{SECOND}"));
    let [citation] = cites.as_slice() else {
        panic!("expected one citation: {cites:?}");
    };
    assert_eq!(citation.url, "https://blog.rust-lang.org/");
    assert_eq!(citation.title.as_deref(), Some("Rust Blog"));
    assert_eq!(covered(&text, citation), link);
}

#[tokio::test]
async fn openrouter_url_citations_become_web_citations() {
    let claim = "Rust 1.95 ist da";
    let (start, end) = char_range(claim);
    // OpenRouter's sparser shape: no title, and an index past the text.
    let output = json!([message(json!([
        { "type": "url_citation", "url": "https://blog.rust-lang.org/",
          "start_index": start, "end_index": end },
        { "type": "url_citation", "url": "https://example.com/x",
          "start_index": 0, "end_index": 999 }
    ]))]);
    let (text, cites) = citations(
        |url| {
            Provider::new("openrouter-test", OpenRouterChatDriver::new())
                .base_url(url)
                .auth(BearerAuth::new("k"))
        },
        body(output),
    )
    .await;
    assert_eq!(cites.len(), 2, "{cites:?}");
    assert_eq!(covered(&text, &cites[0]), claim);
    assert_eq!(cites[0].title, None);
    // A range that does not fit is not guessed: no span.
    assert_eq!(cites[1].url, "https://example.com/x");
    assert_eq!(cites[1].span, None);
}
