//! The budget gate driven through the in-process native loop: a counting
//! model and a scripted checker show which provider calls the gate allows.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmResponseStream, LlmStreamEvent, Message,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime_provider::{Provider, ProviderEndpoint};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::user_facing_error::codes;

use crate::budget::{BudgetSummary, BudgetToolResponse};
use crate::events::EventData;
use crate::host::{InProcessRuntime, TurnStopReason};
use crate::tool_execution::BudgetChecker;

/// Counts provider calls. Asks for a tool once when `tool_first`, then
/// answers with text.
struct CountingModel {
    calls: Arc<AtomicUsize>,
    tool_first: bool,
}

#[async_trait]
impl ChatDriver for CountingModel {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let event = if self.tool_first && call == 0 {
            // A tool no capability provides: the act records a failed
            // result and the loop schedules another reason.
            LlmStreamEvent::ToolCalls(vec![ToolCall {
                id: "call_1".into(),
                name: "missing_tool".into(),
                arguments: serde_json::json!({}),
            }])
        } else {
            LlmStreamEvent::TextDelta("model answer".into())
        };
        Ok(Box::pin(futures::stream::iter([Ok(event)])))
    }
}

/// Answers each check with the next scripted reading (the last one repeats).
struct ScriptedChecker {
    readings: Mutex<VecDeque<Result<BudgetToolResponse>>>,
    calls: AtomicUsize,
}

impl ScriptedChecker {
    fn new(readings: Vec<Result<BudgetToolResponse>>) -> Arc<Self> {
        Arc::new(Self {
            readings: Mutex::new(readings.into()),
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl BudgetChecker for ScriptedChecker {
    async fn check_budgets(&self, _session_id: &str) -> Result<BudgetToolResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut readings = self.readings.lock().unwrap();
        if readings.len() > 1 {
            readings.pop_front().unwrap()
        } else {
            match readings.front().unwrap() {
                Ok(response) => Ok(response.clone()),
                Err(_) => Err(AgentLoopError::store("budget store unreachable")),
            }
        }
    }
}

fn reading(status: &str, balance: f64) -> Result<BudgetToolResponse> {
    Ok(BudgetToolResponse {
        status: status.into(),
        budgets: vec![BudgetSummary {
            budget_id: Some("bdgt_test".into()),
            currency: "tokens".into(),
            limit: 100.0,
            balance,
            soft_limit: None,
            percent_remaining: balance,
            status: if status == "exhausted" {
                "exhausted".into()
            } else {
                "active".into()
            },
        }],
        hint: None,
    })
}

async fn runtime(
    checker: Arc<ScriptedChecker>,
    tool_first: bool,
) -> (InProcessRuntime, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let runtime = InProcessRuntime::builder()
        .provider_with_default_model(
            Provider::new(
                "counting",
                CountingModel {
                    calls: calls.clone(),
                    tool_first,
                },
            ),
            "counting-model",
        )
        .budget_checker(checker)
        .single_session(|session| {
            session
                .harness("budgeted", "You are concise.")
                .agent("budgeted-agent", "Reply once.")
        })
        .build()
        .await
        .expect("runtime builds");
    (runtime, calls)
}

async fn run(runtime: &InProcessRuntime) -> crate::host::TurnResult {
    let session_id = runtime.default_session_id().expect("default session");
    runtime
        .run_text_turn(session_id, "hello")
        .await
        .expect("turn runs")
}

async fn events(runtime: &InProcessRuntime) -> Vec<EventData> {
    runtime
        .events()
        .await
        .expect("events")
        .into_iter()
        .map(|event| event.data)
        .collect()
}

fn turn_failed_code(events: &[EventData]) -> Option<String> {
    events.iter().find_map(|event| match event {
        EventData::TurnFailed(data) => data.error_code.clone(),
        _ => None,
    })
}

#[tokio::test]
async fn exhausted_budget_stops_the_turn_before_the_model_runs() {
    let checker = ScriptedChecker::new(vec![reading("exhausted", 0.0)]);
    let (runtime, model_calls) = runtime(checker.clone(), false).await;

    let result = run(&runtime).await;

    assert!(!result.success);
    assert_eq!(result.stop_reason, TurnStopReason::Error);
    assert_eq!(model_calls.load(Ordering::SeqCst), 0, "model must not run");
    assert_eq!(checker.calls.load(Ordering::SeqCst), 1);
    let events = events(&runtime).await;
    let exhausted = events
        .iter()
        .find_map(|event| match event {
            EventData::BudgetExhausted(data) => Some(data.clone()),
            _ => None,
        })
        .expect("budget.exhausted emitted");
    assert_eq!(exhausted.budget_id, "bdgt_test");
    assert_eq!(exhausted.balance, 0.0);
    assert_eq!(
        turn_failed_code(&events).as_deref(),
        Some(codes::BUDGET_EXHAUSTED)
    );
    let failures = events
        .iter()
        .filter(|event| matches!(event, EventData::TurnFailed(_)))
        .count();
    assert_eq!(failures, 1, "exactly one turn.failed");
    // The user sees the canonical copy in the conversation.
    let messages = runtime
        .messages(runtime.default_session_id().unwrap())
        .await
        .unwrap();
    assert!(messages.iter().any(|message| {
        message
            .content_to_llm_string()
            .starts_with("Budget exhausted. 100.00 tokens spent reached")
    }));
}

#[tokio::test]
async fn paused_budget_stops_the_turn_with_the_paused_code() {
    let checker = ScriptedChecker::new(vec![reading("paused", 30.0)]);
    let (runtime, model_calls) = runtime(checker, false).await;

    let result = run(&runtime).await;

    assert!(!result.success);
    assert_eq!(model_calls.load(Ordering::SeqCst), 0);
    let events = events(&runtime).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EventData::BudgetPaused(_)))
    );
    assert_eq!(
        turn_failed_code(&events).as_deref(),
        Some(codes::BUDGET_PAUSED)
    );
}

#[tokio::test]
async fn budget_running_out_mid_turn_stops_before_the_next_provider_call() {
    let checker = ScriptedChecker::new(vec![reading("active", 50.0), reading("exhausted", 0.0)]);
    let (runtime, model_calls) = runtime(checker.clone(), true).await;

    let result = run(&runtime).await;

    assert!(!result.success);
    assert_eq!(
        model_calls.load(Ordering::SeqCst),
        1,
        "the second generation must not run"
    );
    assert_eq!(
        checker.calls.load(Ordering::SeqCst),
        2,
        "one check per reason"
    );
    assert_eq!(
        turn_failed_code(&events(&runtime).await).as_deref(),
        Some(codes::BUDGET_EXHAUSTED)
    );
}

#[tokio::test]
async fn checker_error_fails_open() {
    let checker = ScriptedChecker::new(vec![Err(AgentLoopError::store("unreachable"))]);
    let (runtime, model_calls) = runtime(checker.clone(), false).await;

    let result = run(&runtime).await;

    assert!(result.success, "a checker error must not stop the turn");
    assert_eq!(result.response, "model answer");
    assert_eq!(model_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn active_budget_runs_normally_without_budget_events() {
    let checker = ScriptedChecker::new(vec![reading("active", 90.0)]);
    let (runtime, model_calls) = runtime(checker, false).await;

    let result = run(&runtime).await;

    assert!(result.success);
    assert_eq!(model_calls.load(Ordering::SeqCst), 1);
    let events = events(&runtime).await;
    assert!(!events.iter().any(|event| matches!(
        event,
        EventData::BudgetWarning(_) | EventData::BudgetPaused(_) | EventData::BudgetExhausted(_)
    )));
}

#[tokio::test]
async fn low_budget_warns_once_per_turn_and_keeps_running() {
    let checker = ScriptedChecker::new(vec![reading("warning", 10.0)]);
    let (runtime, model_calls) = runtime(checker.clone(), true).await;

    let result = run(&runtime).await;

    assert!(result.success);
    assert_eq!(model_calls.load(Ordering::SeqCst), 2);
    assert_eq!(checker.calls.load(Ordering::SeqCst), 2);
    let warnings = events(&runtime)
        .await
        .into_iter()
        .filter(|event| matches!(event, EventData::BudgetWarning(_)))
        .count();
    assert_eq!(warnings, 1, "one budget.warning per turn");
}
