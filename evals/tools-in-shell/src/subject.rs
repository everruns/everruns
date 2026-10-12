//! A Mira `Subject` that runs each case through an in-process Framework agent
//! with the shell and Tools in Shell, over the fake registry.
//!
//! Everything between the model and the fake tools is the shipped code: the
//! capability's prompt addition, the hook that hides the registry from the
//! model's tool list and names the sources on `bash`, the `tools` builtin's
//! help, search, input parsing and errors, and the per-call policy the turn
//! engine installs. Only the tools behind it are fake, so a run needs a model
//! key and nothing else.

use std::time::Instant;

use everruns::{Agent, EventStreamError, InMemoryEngine, Provider, Session, SessionEvent};
use mira::subject::summarize_events;
use mira::{ErrorKind, RunCx, Sample, Subject, Target, Transcript};
use serde_json::Value;

use crate::registry::{self, CallLog};

/// Transcript metadata key: the calls the fake tools received, `{tool, input}`.
pub const CALLS_KEY: &str = "tool_calls_made";

/// Neutral on purpose: it says the data is behind tools and nothing about how
/// to reach them. Discovery is the capability's prompt and the `tools` help,
/// which is what the study measures.
const INSTRUCTIONS: &str = "You are an operations assistant for a small company. Answer from the \
company's tools rather than from memory, and keep answers short.";

/// Ceiling on model steps per turn, so a model that never stops calling tools
/// does not burn a key.
const MAX_ITERATIONS: usize = 12;

pub struct ToolsInShellSubject;

#[async_trait::async_trait]
impl Subject for ToolsInShellSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let started = Instant::now();
        let provider = match provider(&cx.target) {
            Ok(provider) => provider,
            Err(error) => return Transcript::infra_error(error),
        };
        let log = CallLog::default();
        let session = match build_session(sample, &cx.target, provider, &log) {
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
) -> Result<Session, String> {
    let mut builder = Agent::builder()
        .name(format!("tools-in-shell-eval-{}", sample.id))
        .instructions(INSTRUCTIONS)
        .provider(provider)
        .model(target.model.clone())
        .max_iterations(MAX_ITERATIONS)
        .capability("bashkit_shell")
        .capability("tools_in_shell");
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

/// Usage, the model's own tool calls, the fake tools' call log and friction.
pub fn finish(transcript: &mut Transcript, log: &CallLog) {
    let (usage, _) = summarize_events(&transcript.events);
    transcript.usage = usage;
    transcript.tool_calls = transcript
        .events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool.completed"))
        .filter_map(|event| event.pointer("/data/tool_name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    transcript.tool_calls_count = transcript.tool_calls.len();
    transcript
        .metadata
        .insert(CALLS_KEY.into(), Value::Array(log.calls()));
    crate::scorers::record_friction(transcript);
}

fn mark_event_stream_error(transcript: &mut Transcript, error: EventStreamError) {
    transcript.error_kind = ErrorKind::Infra;
    transcript.error = Some(format!("Framework event stream incomplete: {error}"));
}

/// The canonical payload, as the generic study records it: scorers read tool
/// arguments and results, so the recorder wants the full envelope.
fn event_value(event: SessionEvent) -> Value {
    serde_json::json!({
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
