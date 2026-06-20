//! [`Subject`]: the thing under evaluation. The prototype ships the
//! [`RuntimeSubject`] adapter (drives a real `everruns-runtime` session). The
//! spec also defines `ToolSubject` (a bashkit `Tool` in a minimal agent loop)
//! and `CliSubject` (an external binary — the polyglot / other-language path).
//! All three produce the same [`Transcript`], so scorers and reporting are
//! shared.

use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use everruns_core::provider::DriverId;
use everruns_core::typed_id::SessionId;
use everruns_core::{InputMessage, ResolvedModel};
use everruns_runtime::InProcessRuntime;

use crate::{RunCx, Sample, Transcript, Usage};

/// One cell of the model matrix: a label plus the resolved provider/model.
#[derive(Clone, Debug)]
pub struct ModelSpec {
    pub label: String,
    pub model: ResolvedModel,
    /// The offline `llmsim` model. The runtime factory must register a matching
    /// `llm_sim` driver for this to run without an API key.
    pub is_sim: bool,
}

impl ModelSpec {
    /// Offline simulator — runs end-to-end with no API key. Default matrix cell.
    pub fn sim() -> Self {
        Self {
            label: "sim".into(),
            model: ResolvedModel {
                model: "llmsim-eval".into(),
                provider_type: DriverId::LlmSim,
                api_key: Some("fake-key".into()),
                base_url: None,
                provider_metadata: None,
            },
            is_sim: true,
        }
    }

    /// An Anthropic model; API key read from `ANTHROPIC_API_KEY` at build time.
    pub fn anthropic(model: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            label: format!("anthropic/{model}"),
            model: ResolvedModel {
                model,
                provider_type: DriverId::Anthropic,
                api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
                base_url: None,
                provider_metadata: None,
            },
            is_sim: false,
        }
    }

    /// An OpenAI model; API key read from `OPENAI_API_KEY` at build time.
    pub fn openai(model: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            label: format!("openai/{model}"),
            model: ResolvedModel {
                model,
                provider_type: DriverId::OpenAI,
                api_key: std::env::var("OPENAI_API_KEY").ok(),
                base_url: None,
                provider_metadata: None,
            },
            is_sim: false,
        }
    }

    /// True if a non-sim cell is missing its API key (skip rather than fail).
    pub fn missing_key(&self) -> bool {
        !self.is_sim && self.model.api_key.as_deref().unwrap_or("").is_empty()
    }
}

/// The thing being evaluated. Implementors turn a [`Sample`] into a
/// [`Transcript`]. Each call gets a fresh subject instance (isolation), so
/// state from one sample cannot leak into another.
#[async_trait]
pub trait Subject: Send + Sync {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript;
}

/// Result of a runtime factory: a built runtime and the session to drive.
pub type RuntimeHandle = (InProcessRuntime, SessionId);

/// Builds a fresh [`InProcessRuntime`] for a given matrix cell. Embedders own
/// platform/capability/tool wiring here — the framework stays agnostic to it.
pub type RuntimeFactory = Box<
    dyn Fn(ModelSpec) -> Pin<Box<dyn Future<Output = Result<RuntimeHandle, String>> + Send>>
        + Send
        + Sync,
>;

/// Drives a real `everruns-runtime` session: sends each input turn, then
/// normalizes `TurnResult` + the event stream into a [`Transcript`].
pub struct RuntimeSubject {
    factory: RuntimeFactory,
}

impl RuntimeSubject {
    pub fn new(factory: RuntimeFactory) -> Self {
        Self { factory }
    }
}

#[async_trait]
impl Subject for RuntimeSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let (runtime, session_id) = match (self.factory)(cx.model.clone()).await {
            Ok(handle) => handle,
            Err(e) => {
                return Transcript {
                    error: Some(format!("runtime build failed: {e}")),
                    ..Default::default()
                };
            }
        };

        let mut transcript = Transcript::default();
        for prompt in &sample.input {
            match runtime
                .run_turn(session_id, InputMessage::user(prompt.clone()))
                .await
            {
                Ok(result) => {
                    transcript.final_response = result.response;
                    transcript.iterations += result.iterations;
                    transcript.tool_calls_count += result.tool_calls_count;
                    if !result.success {
                        transcript.error = result.error;
                        break;
                    }
                }
                Err(e) => {
                    transcript.error = Some(e.to_string());
                    break;
                }
            }
        }

        if let Ok(events) = runtime.events().await {
            for event in &events {
                if let Ok(value) = serde_json::to_value(event) {
                    transcript.events.push(value);
                }
            }
        }
        let (usage, tools) = summarize_events(&transcript.events);
        transcript.usage = usage;
        transcript.tool_calls = tools;
        transcript
    }
}

/// Extract token/cost usage and tool-call names from the serialized event
/// stream. Walking the JSON keeps this robust to internal struct churn; the
/// production version would read the typed `Event` enum directly.
fn summarize_events(events: &[serde_json::Value]) -> (Usage, Vec<String>) {
    let mut usage = Usage::default();
    let mut tools = Vec::new();
    for event in events {
        walk(event, &mut usage, &mut tools);
    }
    (usage, tools)
}

fn walk(value: &serde_json::Value, usage: &mut Usage, tools: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            // A usage block carries token counts (appears on completed messages).
            if let Some(input) = map.get("input_tokens").and_then(|v| v.as_u64()) {
                usage.input_tokens += input;
                usage.output_tokens += map
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if let Some(cost) = map.get("cost").and_then(|v| v.as_f64()) {
                    usage.cost_usd += cost;
                }
            }
            // A tool call object carries both a `name` and an `input` payload.
            if map.contains_key("input")
                && let Some(name) = map.get("name").and_then(|v| v.as_str())
            {
                tools.push(name.to_string());
            }
            for child in map.values() {
                walk(child, usage, tools);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk(item, usage, tools);
            }
        }
        _ => {}
    }
}
