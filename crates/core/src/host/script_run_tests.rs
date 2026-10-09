use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use everruns_contracts::driver_registry::{LlmStreamEvent, Message, MessageRole};
use everruns_contracts::runtime::saved_scripts::SCRIPT_RUN_METADATA_KEY;
use everruns_contracts::tool_types::ToolDefinition;
use futures::StreamExt;
use serde_json::json;

use super::*;

/// The agent's real model: counts calls and answers "model".
#[derive(Default)]
struct RealModel {
    calls: AtomicUsize,
}

#[async_trait]
impl ChatDriver for RealModel {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(futures::stream::iter([Ok(
            LlmStreamEvent::TextDelta("model".into()),
        )])))
    }
}

const CALL: &str = "script_run_1";

fn run(wake: bool) -> ScriptRun {
    ScriptRun {
        script: "triage".into(),
        input: Some(json!({"repo": "x"})),
        wake_agent_on_failure: wake,
    }
}

fn config(with_bash: bool) -> LlmCallConfig {
    let mut config = LlmCallConfig::new("m");
    if with_bash {
        config.tools = vec![ToolDefinition::function("bash", "shell", json!({}))];
    }
    config
}

fn assistant_call() -> Message {
    let mut message = Message::text(MessageRole::Assistant, "");
    message.tool_calls = Some(vec![ToolCall {
        id: CALL.into(),
        name: "bash".into(),
        arguments: json!({}),
    }]);
    message
}

fn tool_result(result: serde_json::Value) -> Message {
    let mut message = Message::text(MessageRole::Tool, result.to_string());
    message.tool_call_id = Some(CALL.into());
    message
}

async fn answer(
    driver: &ScriptRunDriver,
    messages: Vec<Message>,
    config: &LlmCallConfig,
) -> Vec<LlmStreamEvent> {
    driver
        .chat_completion_stream(&ProviderEndpoint::default(), messages, config)
        .await
        .unwrap()
        .map(|event| event.unwrap())
        .collect()
        .await
}

fn driver(wake: bool) -> (ScriptRunDriver, Arc<RealModel>) {
    let model = Arc::new(RealModel::default());
    let driver = ScriptRunDriver::new(run(wake), CALL.into(), model.clone());
    (driver, model)
}

fn text(events: &[LlmStreamEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            LlmStreamEvent::TextDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn the_first_answer_is_one_bash_call_that_runs_the_script() {
    let (driver, model) = driver(false);
    let events = answer(
        &driver,
        vec![Message::text(MessageRole::User, "Run triage")],
        &config(true),
    )
    .await;
    let LlmStreamEvent::ToolCalls(calls) = &events[0] else {
        panic!("expected a tool call, got {events:?}");
    };
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, CALL);
    assert_eq!(calls[0].name, "bash");
    assert_eq!(calls[0].arguments["commands"], run(false).command());
    assert!(matches!(events[1], LlmStreamEvent::Done(_)));
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_clean_run_ends_the_turn_without_the_model() {
    let (driver, model) = driver(true);
    let events = answer(
        &driver,
        vec![
            Message::text(MessageRole::User, "Run triage"),
            assistant_call(),
            tool_result(json!({"stdout": "ok", "exit_code": 0, "success": true})),
        ],
        &config(true),
    )
    .await;
    assert_eq!(text(&events), "Ran saved script `triage`.");
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failed_run_is_recorded_or_wakes_the_agent() {
    let failed = || {
        vec![
            Message::text(MessageRole::User, "Run triage"),
            assistant_call(),
            tool_result(json!({"exit_code": 0, "success": false, "tools": {"stopped": {}}})),
        ]
    };

    let (quiet, model) = driver(false);
    let events = answer(&quiet, failed(), &config(true)).await;
    assert!(text(&events).contains("did not finish"), "{events:?}");
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);

    let (waking, model) = driver(true);
    let events = answer(&waking, failed(), &config(true)).await;
    assert_eq!(text(&events), "model");
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_woken_agent_keeps_the_turn() {
    let (driver, model) = driver(true);
    let mut messages = vec![
        Message::text(MessageRole::User, "Run triage"),
        assistant_call(),
        tool_result(json!({"exit_code": 1})),
    ];
    messages.push(Message::text(MessageRole::Assistant, "looking"));
    answer(&driver, messages, &config(true)).await;
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_earlier_runs_call_does_not_count() {
    let (driver, _) = driver(false);
    let mut earlier = assistant_call();
    earlier.tool_calls.as_mut().unwrap()[0].id = "script_run_0".into();
    let events = answer(
        &driver,
        vec![
            Message::text(MessageRole::User, "Run triage"),
            earlier,
            Message::text(MessageRole::Assistant, "Ran saved script `triage`."),
            Message::text(MessageRole::User, "Run triage"),
        ],
        &config(true),
    )
    .await;
    assert!(
        matches!(events[0], LlmStreamEvent::ToolCalls(_)),
        "{events:?}"
    );
}

#[tokio::test]
async fn without_bash_the_run_says_why_it_cannot_start() {
    let (driver, _) = driver(false);
    let events = answer(
        &driver,
        vec![Message::text(MessageRole::User, "Run triage")],
        &config(false),
    )
    .await;
    assert!(text(&events).contains("no bash tool"), "{events:?}");
}

#[test]
fn only_the_latest_user_message_asks_for_a_run() {
    let mut asking = RuntimeMessage::user("Run triage");
    asking.metadata = Some(HashMap::from([(
        SCRIPT_RUN_METADATA_KEY.to_string(),
        serde_json::to_value(run(false)).unwrap(),
    )]));
    let (found, call_id) = requested(std::slice::from_ref(&asking)).unwrap();
    assert_eq!(found, run(false));
    assert_eq!(call_id, format!("script_run_{}", asking.id.uuid().simple()));

    let later = RuntimeMessage::user("hello");
    assert!(requested(&[asking, later]).is_none());
}
