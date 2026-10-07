//! The output-truncation gate end to end: a generation that lost tool calls
//! to the output limit never runs them, and the `output_truncation` policy
//! decides whether the turn retries (telling the model), fails, or carries on.
//!
//! Run with: cargo test -p everruns-test-support --all-features --test integration truncation_gate_test::

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, DriverId, DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream,
    LlmStreamEvent, Message, MessageContent,
};
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::tool_types::ToolCall;
use everruns_core::builtins::OutputTruncationCapability;
use everruns_core::events::EventData;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::turn::TurnStopReason;
use everruns_test_support::InMemoryAgenticLoop;
use futures::stream;
use serde_json::{Value, json};

/// One scripted generation: text, the calls that survived, how it ended.
#[derive(Clone, Debug)]
struct Reply {
    text: &'static str,
    calls: Vec<ToolCall>,
    finish: &'static str,
    dropped: u32,
}

/// A cut-off generation: one call lost to the output limit.
fn cut(text: &'static str) -> Reply {
    Reply {
        text,
        calls: vec![],
        finish: "length",
        dropped: 1,
    }
}

fn answer(text: &'static str) -> Reply {
    Reply {
        text,
        calls: vec![],
        finish: "stop",
        dropped: 0,
    }
}

/// Plays the script in order and records every request it received.
#[derive(Clone, Debug, Default)]
struct ScriptedDriver {
    script: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Vec<Message>>>>,
}

#[async_trait]
impl ChatDriver for ScriptedDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> everruns_contracts::error::Result<LlmResponseStream> {
        self.requests.lock().unwrap().push(messages);
        let reply = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .expect("script has a reply for every call");
        let mut metadata = LlmCompletionMetadata::default();
        metadata.finish_reason = Some(reply.finish.to_string());
        metadata.tool_calls_dropped = reply.dropped;
        metadata.completion_tokens = Some(64);
        let mut events = vec![Ok(LlmStreamEvent::TextDelta(reply.text.to_string()))];
        if !reply.calls.is_empty() {
            events.push(Ok(LlmStreamEvent::ToolCalls(reply.calls)));
        }
        events.push(Ok(LlmStreamEvent::Done(Box::new(metadata))));
        Ok(Box::pin(stream::iter(events)))
    }
}

/// Counts how often it ran.
#[derive(Clone, Default)]
struct ReadTool {
    runs: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read a file."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}})
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.runs.lock().unwrap().push(arguments);
        ToolExecutionResult::success(json!({"content": "hello"}))
    }
}

struct Run {
    result: everruns_test_support::in_memory_loop::TurnResult,
    requests: Vec<Vec<Message>>,
    gate_labels: Vec<Option<String>>,
    tool_runs: Vec<Value>,
}

async fn run(script: Vec<Reply>, policy: Option<Value>) -> Run {
    let driver = ScriptedDriver {
        script: Arc::new(Mutex::new(script.into())),
        ..Default::default()
    };
    let requests = Arc::clone(&driver.requests);
    let mut registry = DriverRegistry::new();
    registry.register(DriverId::LlmSim, move |_| Box::new(driver.clone()));
    let tool = ReadTool::default();
    let runs = Arc::clone(&tool.runs);
    let mut builder = InMemoryAgenticLoop::builder()
        .model(ModelSpec::on(DriverId::LlmSim.as_str(), "scripted"))
        .driver_registry(registry)
        .tool(tool)
        .max_iterations(10);
    if let Some(config) = policy {
        builder = builder.capability_with_config(OutputTruncationCapability, config);
    }
    let runner = builder.build().await.expect("build loop");
    let result = runner
        .run_turn("write the report")
        .await
        .expect("turn runs");
    let gate_labels = runner
        .events()
        .await
        .into_iter()
        .filter_map(|event| match event.data {
            EventData::LlmGeneration(data) => Some(data.metadata.truncation_gate),
            _ => None,
        })
        .collect();
    let requests = requests.lock().unwrap().clone();
    let tool_runs = runs.lock().unwrap().clone();
    Run {
        result,
        requests,
        gate_labels,
        tool_runs,
    }
}

fn last_text(request: &[Message]) -> String {
    match &request.last().expect("non-empty request").content {
        MessageContent::Text(text) => text.clone(),
        other => format!("{other:?}"),
    }
}

/// Default policy (no capability configured): the model is told its call was
/// cut off and gets another generation, which then answers.
#[tokio::test]
async fn continue_by_default_tells_the_model_and_retries() {
    let run = run(
        vec![cut("Writing the report"), answer("Done, in parts.")],
        None,
    )
    .await;
    assert!(run.result.success, "{:?}", run.result);
    assert_eq!(run.result.response, "Done, in parts.");
    assert_eq!(run.requests.len(), 2);
    let notice = last_text(&run.requests[1]);
    assert!(
        notice.contains("hit the output token limit") && notice.contains("NOT run"),
        "{notice}"
    );
    assert_eq!(
        run.gate_labels,
        [Some("retried".to_string()), None],
        "the gate acts on the cut-off generation only"
    );
}

/// After `max_retries` consecutive cut-offs the turn fails with a clear error.
#[tokio::test]
async fn continue_fails_the_turn_after_the_retry_limit() {
    let run = run(
        vec![cut("a"), cut("b"), cut("c")],
        Some(json!({"policy": "continue", "max_retries": 2})),
    )
    .await;
    assert!(!run.result.success, "{:?}", run.result);
    assert_eq!(run.result.stop_reason, TurnStopReason::Error);
    let error = run.result.error.unwrap_or_default();
    assert!(
        error.contains("cut off at its output limit in 3 consecutive responses"),
        "{error}"
    );
    assert_eq!(run.requests.len(), 3, "two retries, then stop");
    assert_eq!(
        run.gate_labels,
        [
            Some("retried".to_string()),
            Some("retried".to_string()),
            Some("failed".to_string())
        ]
    );
}

/// A clean generation in between resets the consecutive count.
#[tokio::test]
async fn a_clean_generation_resets_the_retry_count() {
    let read = ToolCall {
        id: "call_read".into(),
        name: "read_file".into(),
        arguments: json!({"path": "a.txt"}),
    };
    let tool_turn = Reply {
        text: "",
        calls: vec![read],
        finish: "tool_calls",
        dropped: 0,
    };
    let run = run(
        vec![cut("a"), tool_turn, cut("b"), answer("ok")],
        Some(json!({"max_retries": 1})),
    )
    .await;
    assert!(run.result.success, "{:?}", run.result);
    assert_eq!(run.requests.len(), 4);
}

/// `fail` ends the turn on the first cut-off, without another generation.
#[tokio::test]
async fn fail_policy_ends_the_turn_on_the_first_cut_off() {
    let run = run(vec![cut("partial")], Some(json!({"policy": "fail"}))).await;
    assert!(!run.result.success, "{:?}", run.result);
    let error = run.result.error.unwrap_or_default();
    assert!(error.contains("output_truncation policy: fail"), "{error}");
    assert_eq!(run.requests.len(), 1);
    assert_eq!(run.gate_labels, [Some("failed".to_string())]);
}

/// `off` keeps the previous behaviour: the turn completes on the cut-off
/// answer, the model is not told, and nothing is retried.
#[tokio::test]
async fn off_policy_keeps_the_previous_behaviour() {
    let run = run(vec![cut("partial")], Some(json!({"policy": "off"}))).await;
    assert!(run.result.success, "{:?}", run.result);
    assert_eq!(run.result.stop_reason, TurnStopReason::MaxTokens);
    assert_eq!(run.requests.len(), 1);
    assert_eq!(run.gate_labels, [None]);
}

/// Calls that arrived whole from a cut-off response still run (under every
/// policy that does not fail); the notice follows their results, so tool
/// call and result stay adjacent for the provider.
#[tokio::test]
async fn surviving_calls_run_and_the_notice_follows_their_results() {
    let read = ToolCall {
        id: "call_read".into(),
        name: "read_file".into(),
        arguments: json!({"path": "a.txt"}),
    };
    let mixed = Reply {
        text: "",
        calls: vec![read],
        finish: "length",
        dropped: 1,
    };
    let run = run(vec![mixed, answer("All read.")], None).await;
    assert!(run.result.success, "{:?}", run.result);
    assert_eq!(run.tool_runs, [json!({"path": "a.txt"})]);
    let second = &run.requests[1];
    let tail: Vec<_> = second
        .iter()
        .rev()
        .take(3)
        .map(|message| (message.role.clone(), message.tool_call_id.is_some()))
        .collect();
    // Newest first: the notice (user), the tool result, the assistant call.
    assert!(last_text(second).contains("NOT run"), "{second:?}");
    assert!(tail[1].1, "tool result precedes the notice: {tail:?}");
    assert_eq!(run.gate_labels, [Some("retried".to_string()), None]);
}

/// Calls dropped for a refusal or content filter are not the gate's business:
/// a retry does not fix them.
#[tokio::test]
async fn calls_dropped_by_a_content_filter_do_not_retry() {
    let filtered = Reply {
        text: "",
        calls: vec![],
        finish: "content_filter",
        dropped: 1,
    };
    let run = run(vec![filtered], None).await;
    assert_eq!(run.requests.len(), 1);
    assert_eq!(run.gate_labels, [None]);
}
