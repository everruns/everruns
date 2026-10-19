//! `citation_web` end to end: a turn runs through the real reason atom, and
//! the answer the session stores carries web citations, from `web_search`
//! function results the model cited by URL or id, and from the citations a
//! provider returned with its own search (driver `web_citations` on `Done`).
//! Without the capability the same turns store no annotations.
//!
//! Run with: cargo test -p everruns-test-support --all-features --test integration web_citations_test::

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_capabilities::capabilities::{CITATION_WEB_CAPABILITY_ID, CitationWebCapability};
use everruns_contracts::driver_registry::{
    ChatDriver, DriverId, DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream,
    LlmStreamEvent, Message,
};
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime::web_search::web_result_id;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::web_citation::{CitedSpan, WebCitation};
use everruns_core::MessageRetriever;
use everruns_core::message::{ContentPart, RuntimeMessageRole, TextAnnotation};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_test_support::InMemoryAgenticLoop;
use futures::stream;
use serde_json::{Value, json};

const TOKIO: &str = "https://tokio.rs/blog/2026-10-tokio-1-50";

/// One scripted generation: streamed text, tool calls, provider citations.
#[derive(Clone, Default)]
struct Reply {
    text: String,
    calls: Vec<ToolCall>,
    citations: Vec<WebCitation>,
}

#[derive(Clone, Default)]
struct ScriptedDriver {
    script: Arc<Mutex<VecDeque<Reply>>>,
}

#[async_trait]
impl ChatDriver for ScriptedDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> everruns_contracts::error::Result<LlmResponseStream> {
        let reply = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .expect("script has a reply for every call");
        let mut metadata = LlmCompletionMetadata::default();
        metadata.finish_reason = Some(
            if reply.calls.is_empty() {
                "stop"
            } else {
                "tool_calls"
            }
            .into(),
        );
        metadata.web_citations = reply.citations;
        // Streamed in two deltas, split inside a multi-byte character's word.
        let split = reply.text.char_indices().nth(3).map_or(0, |(i, _)| i);
        let mut events = vec![
            Ok(LlmStreamEvent::TextDelta(reply.text[..split].to_string())),
            Ok(LlmStreamEvent::TextDelta(reply.text[split..].to_string())),
        ];
        if !reply.calls.is_empty() {
            events.push(Ok(LlmStreamEvent::ToolCalls(reply.calls)));
        }
        events.push(Ok(LlmStreamEvent::Done(Box::new(metadata))));
        Ok(Box::pin(stream::iter(events)))
    }
}

/// Stands in for the `web_search` tool: the result shape models cite from.
struct FakeWebSearch;

#[async_trait]
impl Tool for FakeWebSearch {
    fn name(&self) -> &str {
        "web_search"
    }
    fn description(&self) -> &str {
        "Search the web."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"query": {"type": "string"}}})
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({
            "query": arguments["query"],
            "results": [
                {"id": web_result_id(TOKIO), "title": "Announcing Tokio 1.50", "url": TOKIO,
                 "site": "tokio.rs", "snippet": "Tokio 1.50 adds io_uring support."},
                {"id": web_result_id("https://docs.rs/tokio"), "title": "tokio - Rust",
                 "url": "https://docs.rs/tokio", "site": "docs.rs",
                 "snippet": "An event-driven, non-blocking I/O platform."}
            ]
        }))
    }
}

/// Runs one turn and returns the stored answer's text and annotations.
async fn run(script: Vec<Reply>, cite: bool) -> (String, Vec<TextAnnotation>) {
    let driver = ScriptedDriver {
        script: Arc::new(Mutex::new(script.into())),
    };
    let mut registry = DriverRegistry::new();
    registry.register(DriverId::LlmSim, move |_| Box::new(driver.clone()));
    let mut builder = InMemoryAgenticLoop::builder()
        .model(ModelSpec::on(DriverId::LlmSim.as_str(), "scripted"))
        .driver_registry(registry)
        .tool(FakeWebSearch);
    if cite {
        builder = builder.capability(CitationWebCapability);
    }
    let runner = builder.build().await.expect("build loop");
    let result = runner
        .run_turn("what is new in tokio?")
        .await
        .expect("turn runs");
    assert!(result.success, "{result:?}");
    let messages = runner
        .message_retriever()
        .load(runner.session_id())
        .await
        .expect("messages load");
    let answer = messages
        .iter()
        .rev()
        .find(|m| m.role == RuntimeMessageRole::Agent)
        .expect("answer stored");
    let Some(ContentPart::Text(part)) = answer.content.first() else {
        panic!("answer is text: {answer:?}");
    };
    (part.text.clone(), part.annotations.clone())
}

fn search_then(answer: String) -> Vec<Reply> {
    vec![
        Reply {
            calls: vec![ToolCall {
                id: "call_1".into(),
                name: "web_search".into(),
                arguments: json!({"query": "tokio 1.50"}),
            }],
            ..Default::default()
        },
        Reply {
            text: answer,
            ..Default::default()
        },
    ]
}

fn cited(text: &str, annotation: &TextAnnotation) -> String {
    text.chars()
        .skip(annotation.start)
        .take(annotation.end - annotation.start)
        .collect()
}

#[tokio::test]
async fn function_results_cited_by_url_and_id_are_annotated() {
    let docs = web_result_id("https://docs.rs/tokio");
    let answer = format!(
        "Grüße 😀! Tokio 1.50 adds io_uring ([announcement]({TOKIO}/)). Tokio ist eine Laufzeit [{docs}]."
    );
    let (text, annotations) = run(search_then(answer.clone()), true).await;
    assert_eq!(text, answer, "citations never rewrite the answer");
    assert_eq!(annotations.len(), 2, "{annotations:?}");
    assert_eq!(
        cited(&text, &annotations[0]),
        format!("Tokio 1.50 adds io_uring ([announcement]({TOKIO}/)).")
    );
    assert_eq!(annotations[0].source.uri, TOKIO);
    assert_eq!(annotations[0].external_id, Some(web_result_id(TOKIO)));
    assert_eq!(
        cited(&text, &annotations[1]),
        format!("Tokio ist eine Laufzeit [{docs}].")
    );
    assert!(
        annotations
            .iter()
            .all(|a| a.origin == CITATION_WEB_CAPABILITY_ID)
    );

    let (_, without) = run(search_then(answer), false).await;
    assert!(without.is_empty(), "opt-in: {without:?}");
}

#[tokio::test]
async fn native_citations_from_the_driver_are_annotated() {
    let answer = "Grüße 😀! Tokio 1.50 adds io_uring. Bye.".to_string();
    let claim = "Tokio 1.50 adds io_uring.";
    let byte = answer.find(claim).unwrap();
    let reply = || Reply {
        text: answer.clone(),
        citations: vec![
            WebCitation {
                url: TOKIO.into(),
                title: Some("Tokio".into()),
                snippet: Some("io_uring support".into()),
                span: CitedSpan::from_bytes(&answer, byte, byte + claim.len()),
            },
            // A span whose text is not in the answer is dropped, not guessed.
            WebCitation {
                url: "https://example.com".into(),
                span: Some(CitedSpan {
                    start: 0,
                    end: 4,
                    text: "Nope".into(),
                }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let (text, annotations) = run(vec![reply()], true).await;
    assert_eq!(annotations.len(), 1, "{annotations:?}");
    assert_eq!(cited(&text, &annotations[0]), claim);
    assert_eq!(annotations[0].start, "Grüße 😀! ".chars().count());
    assert_eq!(annotations[0].origin, CITATION_WEB_CAPABILITY_ID);
    assert_eq!(annotations[0].source.title.as_deref(), Some("Tokio"));
    assert_eq!(
        annotations[0].source.snippet.as_deref(),
        Some("io_uring support")
    );
    assert_eq!(annotations[0].external_id, Some(web_result_id(TOKIO)));

    let (_, without) = run(vec![reply()], false).await;
    assert!(without.is_empty(), "opt-in: {without:?}");
}
