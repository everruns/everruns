//! Named load-test scenarios: a `[[llmsim:<name>]]` marker in the user message
//! (or an `llmsim-scenario-<name>` model id) plays a scripted conversation
//! from llmsim's scenario engine (tool rounds, long answers, reasoning, slow
//! first tokens, stalls, a 429, a dropped stream).
//!
//! Decisions:
//! - Step selection is llmsim's pure `resolve`: it counts the assistant
//!   replies since the marked user message, so thousands of concurrent
//!   sessions share one driver with no cursor, and any worker or restart
//!   plays the same step.
//! - The built-in set is llmsim's starter library, copied into `scenarios/`
//!   because the published crate does not expose it. Timing comes from
//!   production observations (first token 0.8 to 1.3 s, about 65 tokens/s).
//! - A step that needs a tool, played on a call that offers none (titles,
//!   compaction and other side calls carry the user message but no tools),
//!   falls back to the driver's normal answer instead of failing that call.
//! - A cut stream ends with an in-stream `Error` event after the given number
//!   of tokens and no `Done`, the way the HTTP drivers report a connection
//!   that drops mid-stream, so the agent loop handles it as it would a real
//!   provider.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use everruns_contracts::driver_registry::{
    LlmCompletionMetadata, LlmStreamEvent, Message, MessageRole,
};
use everruns_contracts::error::{AgentLoopError, LlmErrorKind, Result};
use everruns_contracts::reasoning::{ReasoningContentPart, ReasoningText};
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use llmsim::latency::LatencyProfile;
use llmsim::scenario::{
    self, AttemptOutcome, AttemptTracker, ConversationMessage, ResolveError, ScenarioSet,
    StepError, ToolSpec,
};
use llmsim::scenario_stream::{Pacing, word_tokens};
use llmsim::{SimError, SimTurn};

const BUILTIN: &[&str] = &[
    include_str!("../scenarios/broken-stream.json"),
    include_str!("../scenarios/chat-short.json"),
    include_str!("../scenarios/flaky-429.json"),
    include_str!("../scenarios/long-answer.json"),
    include_str!("../scenarios/parallel-tools.json"),
    include_str!("../scenarios/research-3-tools.json"),
    include_str!("../scenarios/slow-first-token.json"),
    include_str!("../scenarios/stream-stall.json"),
    include_str!("../scenarios/thinking.json"),
];

// Invalid or duplicate entries are skipped here; a test keeps every built-in
// scenario loadable.
static BUILTIN_SET: LazyLock<ScenarioSet> = LazyLock::new(|| {
    let mut set = ScenarioSet::new();
    for file in BUILTIN
        .iter()
        .filter_map(|json| ScenarioSet::from_json(json).ok())
    {
        for name in file.names() {
            if let Some(scenario) = file.get(&name) {
                let _ = set.insert(scenario.clone());
            }
        }
    }
    set
});

/// Names of the built-in scenarios a marker can select.
pub fn builtin_scenario_names() -> Vec<String> {
    BUILTIN_SET.names()
}

/// Attempt counts for `fail_first` steps, shared by clones of one driver.
#[derive(Clone, Default)]
pub(crate) struct Attempts(Arc<AttemptTracker>);

/// The scenario stream for this call, or `None` when no scenario applies.
pub(crate) fn play(
    messages: &[Message],
    tools: &[ToolDefinition],
    model: &str,
    latency: LatencyProfile,
    attempts: &Attempts,
) -> Option<Result<BoxStream<'static, Result<LlmStreamEvent>>>> {
    let conversation: Vec<ConversationMessage> = messages.iter().map(conversation).collect();
    let specs: Vec<ToolSpec> = tools
        .iter()
        .map(|tool| ToolSpec {
            name: tool.name().to_string(),
            parameters: Some(tool.parameters().clone()),
        })
        .collect();
    let resolution = match scenario::resolve(&conversation, &specs, Some(model), &BUILTIN_SET) {
        Ok(resolution) => resolution?,
        Err(ResolveError::NoToolsOffered { .. }) => return None,
        Err(err) => {
            return Some(Err(AgentLoopError::llm_kind(
                LlmErrorKind::InvalidRequest,
                err.to_string(),
            )));
        }
    };

    let cut_after = match resolution.outcome(attempts.0.next_attempt(resolution.fingerprint)) {
        AttemptOutcome::Fail(err) => return Some(Err(step_error(&err))),
        AttemptOutcome::Respond { cut_after_tokens } => cut_after_tokens,
    };
    let (text, calls) = match resolution.turn {
        SimTurn::Assistant { text } => (text, Vec::new()),
        SimTurn::ToolCalls { calls } => (String::new(), calls),
        SimTurn::Mixed { text, calls } => (text, calls),
        // Error steps always fail in `outcome`, so this is unreachable.
        SimTurn::Error(err) => return Some(Err(sim_error(&err, None))),
    };
    let tool_calls: Vec<ToolCall> = calls
        .into_iter()
        .enumerate()
        .map(|(i, call)| ToolCall {
            id: call
                .id
                .unwrap_or_else(|| format!("call_llmsim_{}_{i}", resolution.fingerprint)),
            name: call.name,
            arguments: call.arguments,
        })
        .collect();

    // Plan every event with the wait before it, then play the plan.
    let pacing = Pacing::new(resolution.timing, latency);
    let reasoning = resolution.reasoning.unwrap_or_default();
    let reasoning_tokens = word_tokens(&reasoning);
    let answer_tokens = word_tokens(&text);
    let completion_tokens = (reasoning_tokens.len() + answer_tokens.len()) as u32;
    let mut plan: Vec<(Duration, Result<LlmStreamEvent>)> = Vec::new();
    let tokens = reasoning_tokens
        .into_iter()
        .map(|delta| LlmStreamEvent::ReasoningDelta {
            delta,
            summary: false,
        })
        .chain(answer_tokens.into_iter().map(LlmStreamEvent::TextDelta));
    let mut reasoning_open = !reasoning.is_empty();
    for (emitted, event) in tokens.enumerate() {
        if cut_after.is_some_and(|cut| emitted >= cut) {
            plan.push((
                Duration::ZERO,
                Ok(LlmStreamEvent::Error(
                    "llmsim scenario cut the stream".to_string().into(),
                )),
            ));
            return Some(Ok(play_plan(plan)));
        }
        // Close the reasoning block before the first answer token.
        if reasoning_open && matches!(event, LlmStreamEvent::TextDelta(_)) {
            plan.push((Duration::ZERO, Ok(reasoning_item(&reasoning))));
            reasoning_open = false;
        }
        let wait = if emitted == 0 {
            pacing.ttft()
        } else {
            pacing.before_token(emitted)
        };
        plan.push((wait, Ok(event)));
    }
    if reasoning_open {
        plan.push((Duration::ZERO, Ok(reasoning_item(&reasoning))));
    }
    if !tool_calls.is_empty() {
        plan.push((Duration::ZERO, Ok(LlmStreamEvent::ToolCalls(tool_calls))));
    }
    let mut metadata = LlmCompletionMetadata::default();
    metadata.completion_tokens = Some(completion_tokens);
    metadata.total_tokens = Some(completion_tokens);
    metadata.model = Some(model.to_string());
    metadata.response_model = Some(model.to_string());
    metadata.finish_reason = Some("stop".to_string());
    plan.push((Duration::ZERO, Ok(LlmStreamEvent::Done(Box::new(metadata)))));
    Some(Ok(play_plan(plan)))
}

fn play_plan(
    plan: Vec<(Duration, Result<LlmStreamEvent>)>,
) -> BoxStream<'static, Result<LlmStreamEvent>> {
    stream::iter(plan)
        .then(|(wait, event)| async move {
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            event
        })
        .boxed()
}

fn conversation(message: &Message) -> ConversationMessage {
    let role = match message.role {
        MessageRole::System => scenario::MessageRole::System,
        MessageRole::User => scenario::MessageRole::User,
        MessageRole::Assistant => scenario::MessageRole::Assistant,
        MessageRole::Tool => scenario::MessageRole::Tool,
    };
    let ids = message
        .tool_calls
        .iter()
        .flatten()
        .map(|call| call.id.clone())
        .chain(message.tool_call_id.clone());
    ConversationMessage::new(role, message.content_as_text()).with_tool_call_ids(ids)
}

fn reasoning_item(text: &str) -> LlmStreamEvent {
    LlmStreamEvent::ReasoningItem(
        ReasoningContentPart::opaque("llmsim")
            .with_item_id("rs_llmsim_scenario")
            .with_text(ReasoningText::Plain {
                text: text.to_string(),
            }),
    )
}

fn step_error(err: &StepError) -> AgentLoopError {
    sim_error(&err.error, err.retry_after_secs())
}

fn sim_error(err: &SimError, retry_after_secs: Option<u64>) -> AgentLoopError {
    let error = match err {
        SimError::RateLimit => AgentLoopError::llm_kind(
            LlmErrorKind::RateLimited,
            "llmsim scenario: rate limit exceeded",
        ),
        SimError::Timeout => {
            AgentLoopError::llm_kind(LlmErrorKind::Unavailable, "llmsim scenario: timed out")
        }
        SimError::InvalidRequest { message } => {
            AgentLoopError::llm_kind(LlmErrorKind::InvalidRequest, message.clone())
        }
        SimError::Other {
            message,
            status_code,
        } => {
            let kind = if status_code.unwrap_or(500) >= 500 {
                LlmErrorKind::Unavailable
            } else {
                LlmErrorKind::Other
            };
            AgentLoopError::llm_kind(kind, message.clone())
        }
    };
    match retry_after_secs {
        Some(secs) => error.with_retry_after_secs(secs),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_tool() -> ToolDefinition {
        ToolDefinition::function(
            "read_file",
            "Read a file",
            serde_json::json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
            }),
        )
    }

    /// Play one call and collect its events; `None` when no scenario applies.
    async fn call(
        messages: &[Message],
        tools: &[ToolDefinition],
        attempts: &Attempts,
    ) -> Option<Result<Vec<Result<LlmStreamEvent>>>> {
        let played = play(
            messages,
            tools,
            "llmsim-default",
            LatencyProfile::instant(),
            attempts,
        )?;
        Some(match played {
            Ok(stream) => Ok(stream.collect().await),
            Err(err) => Err(err),
        })
    }

    fn text(events: &[Result<LlmStreamEvent>]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                Ok(LlmStreamEvent::TextDelta(t)) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    fn tool_calls(events: &[Result<LlmStreamEvent>]) -> Vec<ToolCall> {
        events
            .iter()
            .find_map(|e| match e {
                Ok(LlmStreamEvent::ToolCalls(calls)) => Some(calls.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn is_done(events: &[Result<LlmStreamEvent>]) -> bool {
        matches!(events.last(), Some(Ok(LlmStreamEvent::Done(_))))
    }

    /// Append the assistant reply and its tool results, as the loop would.
    fn record(messages: &mut Vec<Message>, events: &[Result<LlmStreamEvent>]) {
        let calls = tool_calls(events);
        let mut reply = Message::text(MessageRole::Assistant, text(events));
        if !calls.is_empty() {
            reply.tool_calls = Some(calls.clone());
        }
        messages.push(reply);
        for c in calls {
            let mut result = Message::text(MessageRole::Tool, "ok");
            result.tool_call_id = Some(c.id);
            messages.push(result);
        }
    }

    #[test]
    fn every_builtin_scenario_loads() {
        assert_eq!(
            builtin_scenario_names(),
            [
                "broken-stream",
                "chat-short",
                "flaky-429",
                "long-answer",
                "parallel-tools",
                "research-3-tools",
                "slow-first-token",
                "stream-stall",
                "thinking",
            ]
        );
    }

    #[tokio::test]
    async fn marker_plays_steps_counted_from_the_conversation() {
        let attempts = Attempts::default();
        let tools = [read_tool()];
        let mut messages = vec![Message::text(
            MessageRole::User,
            "Summarise the repo [[llmsim:research-3-tools speed=0.001]]",
        )];

        // Three tool rounds (1, 1, 2 calls), then the answer.
        for expected_calls in [1, 1, 2] {
            let events = call(&messages, &tools, &attempts).await.unwrap().unwrap();
            let calls = tool_calls(&events);
            assert_eq!(calls.len(), expected_calls);
            assert!(calls.iter().all(|c| c.name == "read_file"));
            assert_eq!(calls[0].arguments, serde_json::json!({"path": "."}));
            assert!(calls[0].id.starts_with("call_llmsim_research-3-tools_"));
            assert!(is_done(&events));
            record(&mut messages, &events);
        }
        let answer = call(&messages, &tools, &attempts).await.unwrap().unwrap();
        assert!(tool_calls(&answer).is_empty());
        assert_eq!(text(&answer).split_whitespace().count(), 600);

        // A fresh session replays the same plan: no shared cursor.
        let fresh = [messages[0].clone()];
        let replay = call(&fresh, &tools, &attempts).await.unwrap().unwrap();
        assert_eq!(tool_calls(&replay).len(), 1);
    }

    #[tokio::test]
    async fn no_marker_or_no_tools_leaves_the_call_to_the_driver() {
        let attempts = Attempts::default();
        let plain = [Message::text(MessageRole::User, "hello")];
        assert!(call(&plain, &[read_tool()], &attempts).await.is_none());

        // A side call (title, compaction) carries the marker but no tools.
        let marked = [Message::text(
            MessageRole::User,
            "hi [[llmsim:research-3-tools]]",
        )];
        assert!(call(&marked, &[], &attempts).await.is_none());
    }

    #[tokio::test]
    async fn unknown_scenario_is_an_invalid_request() {
        let messages = [Message::text(MessageRole::User, "[[llmsim:nope]]")];
        let err = call(&messages, &[], &Attempts::default())
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err.llm_error_kind(), Some(LlmErrorKind::InvalidRequest));
        assert!(err.to_string().contains("chat-short"), "{err}");
    }

    #[tokio::test]
    async fn fail_first_rate_limits_once_then_succeeds() {
        let attempts = Attempts::default();
        let tools = [read_tool()];
        let messages = [Message::text(
            MessageRole::User,
            "go [[llmsim:flaky-429 speed=0.001]]",
        )];
        let err = call(&messages, &tools, &attempts)
            .await
            .unwrap()
            .unwrap_err();
        assert!(err.is_rate_limited());
        assert_eq!(err.retry_after_secs(), Some(1));

        let retry = call(&messages, &tools, &attempts).await.unwrap().unwrap();
        assert_eq!(tool_calls(&retry).len(), 1);
    }

    #[tokio::test]
    async fn broken_stream_cuts_the_first_attempt_only() {
        let attempts = Attempts::default();
        let messages = [Message::text(
            MessageRole::User,
            "go [[llmsim:broken-stream speed=0.001]]",
        )];
        let cut = call(&messages, &[], &attempts).await.unwrap().unwrap();
        assert_eq!(text(&cut).split_whitespace().count(), 50);
        assert!(matches!(cut.last(), Some(Ok(LlmStreamEvent::Error(_)))));

        let retry = call(&messages, &[], &attempts).await.unwrap().unwrap();
        assert_eq!(text(&retry).split_whitespace().count(), 200);
        assert!(is_done(&retry));
    }

    #[tokio::test]
    async fn thinking_streams_reasoning_before_the_answer() {
        let messages = [Message::text(
            MessageRole::User,
            "go [[llmsim:thinking speed=0.001]]",
        )];
        let events = call(&messages, &[], &Attempts::default())
            .await
            .unwrap()
            .unwrap();
        let first_text = events
            .iter()
            .position(|e| matches!(e, Ok(LlmStreamEvent::TextDelta(_))))
            .unwrap();
        let item = events
            .iter()
            .position(|e| matches!(e, Ok(LlmStreamEvent::ReasoningItem(_))))
            .unwrap();
        let deltas = events[..item]
            .iter()
            .filter(|e| matches!(e, Ok(LlmStreamEvent::ReasoningDelta { .. })))
            .count();
        assert_eq!(deltas, 400);
        assert_eq!(item + 1, first_text);
    }
}
