//! A Mira `Subject` that runs each case through an in-process Framework agent
//! with the generic `tool_search` capability over the fake registry.
//!
//! The `reveal` axis picks the arm: `redefine` puts a found tool's full schema
//! on its definition from the next step, `in_result` keeps the permissive stub
//! and leaves the model to call from the schema the search returned.
//! Everything between the model and the fake tools is the shipped code: the
//! capability's prompt addition, the deferral hook, the search tool and its
//! result message, and the turn engine. Only the tools are fake, so a run needs
//! one model key and no server.

use std::time::Instant;

use everruns::{
    Agent, CapabilityRef, EventStreamError, InMemoryEngine, Provider, Session, SessionEvent,
};
#[path = "../../common/everruns_usage.rs"]
mod event_usage;
use mira::{ErrorKind, RunCx, Sample, Subject, Target, Transcript};
use serde_json::{Value, json};

use crate::registry::{self, CallLog};

/// Transcript metadata key: every call the model made to a fake tool, in
/// order, `{tool, input, valid, rejected_by, errors}`. See [`attempts`].
pub const CALLS_KEY: &str = "tool_calls_made";

/// Transcript metadata key: the calls the fake tools' handlers received.
pub const TOOL_LOG_KEY: &str = "tool_log";

/// The two arms, as the capability's `reveal` config spells them.
pub const ARMS: [&str; 2] = ["redefine", "in_result"];

/// Neutral on purpose: nothing here about schemas or searching. That guidance
/// is the capability's own prompt addition, which is part of what is measured.
/// "Act, do not ask" keeps a careful model from ending the turn on a
/// confirmation question that no one will answer.
const INSTRUCTIONS: &str = "You are an operations assistant for a small company. Carry out \
requests with the company's tools. The person has given you everything you need: act on it \
without asking for confirmation, and use ids, codes and amounts exactly as they follow from the \
request. Reply with one short sentence when done.";

/// Ceiling on model steps per turn, so a model stuck in a rejection loop does
/// not burn a key.
const MAX_ITERATIONS: usize = 12;

pub struct ToolSearchRevealSubject;

#[async_trait::async_trait]
impl Subject for ToolSearchRevealSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let started = Instant::now();
        let reveal = cx.param("reveal").unwrap_or("redefine").to_string();
        let provider = match provider(&cx.target) {
            Ok(provider) => provider,
            Err(error) => return Transcript::infra_error(error),
        };
        let log = CallLog::default();
        let session = match build_session(sample, &cx.target, provider, &log, &reveal) {
            Ok(session) => session,
            Err(error) => {
                return Transcript::infra_error(format!("Framework build failed: {error}"));
            }
        };
        let mut transcript = run_turns(&session, sample).await;
        finish(&mut transcript, &log);
        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        transcript
    }
}

pub fn build_session(
    sample: &Sample,
    target: &Target,
    provider: Provider,
    log: &CallLog,
    reveal: &str,
) -> Result<Session, String> {
    if !ARMS.contains(&reveal) {
        return Err(format!("unknown reveal arm '{reveal}'"));
    }
    // Threshold 1: every fake tool is deferred, whatever the count.
    let tool_search =
        CapabilityRef::new("tool_search").config(json!({ "reveal": reveal, "threshold": 1 }));
    let mut builder = Agent::builder()
        .name(format!("tool-search-reveal-{}-{reveal}", sample.id))
        .instructions(INSTRUCTIONS)
        .provider(provider)
        .model(target.model.clone())
        .max_iterations(MAX_ITERATIONS)
        .capability(tool_search);
    for tool in registry::tools(log) {
        builder = builder.tool(tool);
    }
    let agent = builder.build().map_err(|error| error.to_string())?;
    Ok(InMemoryEngine::new().create(agent))
}

/// Run the sample's turns and record the event stream.
pub async fn run_turns(session: &Session, sample: &Sample) -> Transcript {
    let mut events = session.events();
    let mut transcript = Transcript::default();
    for turn in &sample.input {
        match session.run(turn.clone()).await {
            Ok(result) => {
                transcript.final_response = result.response;
                transcript.iterations += result.iterations;
                if !result.success {
                    let error = result.error.unwrap_or_else(|| "turn failed".into());
                    transcript.error_kind = classify(&error);
                    transcript.error = Some(error);
                    break;
                }
            }
            Err(error) => {
                let error = error.to_string();
                transcript.error_kind = classify(&error);
                transcript.error = Some(error);
                break;
            }
        }
    }
    loop {
        match events.try_recv() {
            Ok(Some(event)) => transcript.events.push(event_value(event)),
            Ok(None) => break,
            Err(error) => {
                mark_event_stream_error(&mut transcript, error);
                break;
            }
        }
    }
    transcript
}

/// Usage, the model's own tool calls, the fake tools' call log and metrics.
pub fn finish(transcript: &mut Transcript, log: &CallLog) {
    event_usage::summarize_events(&transcript.events).apply_to(transcript);
    transcript.tool_calls = transcript
        .events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool.completed"))
        .filter_map(|event| event.pointer("/data/tool_name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    transcript.tool_calls_count = transcript.tool_calls.len();
    let attempts = attempts(&transcript.events);
    transcript
        .metadata
        .insert(CALLS_KEY.into(), Value::Array(attempts));
    transcript
        .metadata
        .insert(TOOL_LOG_KEY.into(), Value::Array(log.calls()));
    crate::scorers::record_metrics(transcript);
}

/// Every call the model made to a fake tool, rejected or not.
///
/// Two gates can refuse a call. The engine's tool registry checks arguments
/// against the tool's *registered* schema before running it
/// (`validate_tool_arguments`, code `invalid_tool_arguments`); the deferral
/// hook only changes what the model is shown, so in both arms that check uses
/// the full schema. The fake tool's own validation is the second gate and
/// fires only if the first ever lets something through. A refused call never
/// reaches the handler, so the handler log alone would undercount; this reads
/// the arguments from `tool.started` and the outcome from `tool.completed`.
pub fn attempts(events: &[Value]) -> Vec<Value> {
    let names = registry::names();
    let mut started: std::collections::HashMap<String, Value> = Default::default();
    let mut out = Vec::new();
    for event in events {
        match event.get("type").and_then(Value::as_str) {
            Some("tool.started") => {
                let call = &event["data"]["tool_call"];
                if let Some(id) = call["id"].as_str() {
                    started.insert(id.to_string(), call["arguments"].clone());
                }
            }
            Some("tool.completed") => {
                let data = &event["data"];
                let tool = data["tool_name"].as_str().unwrap_or_default();
                if !names.contains(&tool) {
                    continue;
                }
                let input = data["tool_call_id"]
                    .as_str()
                    .and_then(|id| started.get(id).cloned())
                    .unwrap_or(Value::Null);
                let success = data["success"].as_bool().unwrap_or(false);
                let text = if success {
                    data["result"].to_string()
                } else {
                    data["error"].as_str().unwrap_or_default().to_string()
                };
                let rejected_by = if text.contains("invalid_tool_arguments") {
                    Some("engine")
                } else if text.contains("invalid_arguments for") {
                    Some("tool")
                } else if !success {
                    Some("other")
                } else {
                    None
                };
                out.push(json!({
                    "tool": tool,
                    "input": input,
                    "valid": rejected_by.is_none(),
                    "rejected_by": rejected_by,
                    "errors": if rejected_by.is_some() { Value::String(text) } else { Value::Null },
                }));
            }
            _ => {}
        }
    }
    out
}

fn mark_event_stream_error(transcript: &mut Transcript, error: EventStreamError) {
    transcript.error_kind = ErrorKind::Infra;
    transcript.error = Some(format!("Framework event stream incomplete: {error}"));
}

/// The canonical payload: scorers read tool names and results.
fn event_value(event: SessionEvent) -> Value {
    json!({
        "type": event.event_type(),
        "data": event.canonical_json().get("data").cloned().unwrap_or(Value::Null),
    })
}

/// Map the matrix target onto a Framework provider. Keys are read here, so
/// Mira targets stay key-free labels.
pub fn provider(target: &Target) -> Result<Provider, String> {
    let key = |name: &str| std::env::var(name).map_err(|_| format!("missing API key: {name}"));
    Ok(match target.provider.as_str() {
        "anthropic" => {
            everruns_drivers::anthropic::provider("anthropic", key("ANTHROPIC_API_KEY")?)
        }
        "openai" => everruns_drivers::openai::provider("openai", key("OPENAI_API_KEY")?),
        "openrouter" => {
            everruns_drivers::openrouter::provider("openrouter", key("OPENROUTER_API_KEY")?)
        }
        other => {
            return Err(format!(
                "unsupported provider '{other}' (supported: anthropic, openai, openrouter)"
            ));
        }
    })
}

/// Rate limits, outages and missing keys are infra (N/A, retried); anything
/// else is the subject's.
fn classify(message: &str) -> ErrorKind {
    if mira::is_rate_limited(message) {
        return ErrorKind::Infra;
    }
    let lower = message.to_ascii_lowercase();
    const INFRA: &[&str] = &[
        "402",
        "500",
        "502",
        "503",
        "quota",
        "credit",
        "timed out",
        "timeout",
        "connection",
        "service unavailable",
        "missing api key",
    ];
    if INFRA.iter().any(|signal| lower.contains(signal)) {
        ErrorKind::Infra
    } else {
        ErrorKind::Subject
    }
}
