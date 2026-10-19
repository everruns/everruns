#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]

//! Deterministic, offline LLM simulation for
//! [Everruns](https://everruns.com) agents and runtimes.
//!
//! `everruns-llmsim` implements the provider contracts from
//! [`everruns-contracts`](https://docs.rs/everruns-contracts) with configurable
//! fixed, echo, sequence, and scripted responses. It runs in process without
//! credentials or network access and supports deterministic tool calls,
//! injected failures, latency controls, and request capture.
//!
//! Framework applications can use `everruns::Model::simulated` without naming
//! this crate. Advanced hosts and tests can configure the driver directly:
//!
//! ```
//! use everruns_llmsim::{LlmSimConfig, LlmSimDriver};
//!
//! let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello."));
//! ```

mod demo_scripts;
mod model_names;
pub mod realtime;
mod scenarios;

pub use demo_scripts::{
    auditor_demo_script, guarded_bash_demo_script, monitor_demo_script, session_tasks_demo_script,
};
pub use scenarios::builtin_scenario_names;
#[cfg(feature = "host")]
mod runtime_ext;
pub mod speech;

#[cfg(feature = "host")]
pub use runtime_ext::{LLMSIM_MODEL_ID, LLMSIM_PROVIDER, LlmSimRuntimeExt, llm_sim_provider};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
type SessionSequenceCounters =
    std::collections::HashMap<String, (Arc<AtomicUsize>, Arc<AtomicUsize>)>;

use everruns_contracts::driver_registry::{
    BoxedChatDriver, ChatDriver, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent, Message, MessageRole,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::tool_types::ToolCall;
use llmsim::generator::{LoremGenerator, ResponseGenerator};
use llmsim::latency::LatencyProfile;
use llmsim::openai::{ChatCompletionRequest, Message as SimMessage, Role, Usage};
use llmsim::script::auto_tool_call_id;
use llmsim::stream::TokenStreamBuilder;

// ============================================================================
// Configuration Types
// ============================================================================

/// Configuration for the LlmSim driver.
#[derive(Debug, Clone)]
pub struct LlmSimConfig {
    /// Response generation configuration
    pub response: ResponseConfig,
    /// Optional tool calls to include in responses
    pub tool_calls: Option<ToolCallConfig>,
    /// Enable latency simulation (default: false for fast tests)
    pub simulate_latency: bool,
    /// Model name to report in metadata
    pub model_name: String,
    /// Optional delay before responding (TTFT - time to first token).
    /// This is useful for testing cancellation scenarios where we need a
    /// predictable time window to cancel an active turn before completion.
    pub response_delay: Option<std::time::Duration>,
    /// Optional response ID to include in completion metadata.
    /// Enables testing `previous_response_id` chaining.
    pub response_id: Option<String>,
    /// Optional capture sink for the per-call `reasoning_effort` (EVE-595).
    /// When set, every `chat_completion_stream` call appends the effort it saw
    /// in `LlmCallConfig`, in call order. Tests use this to assert that a
    /// mid-turn effort change is observed by subsequent LLM steps.
    pub effort_capture: Option<Arc<std::sync::Mutex<Vec<Option<String>>>>>,
    /// Optional capture sink for the provider-visible messages of each call.
    /// When set, every `chat_completion_stream` call appends the exact
    /// `Message` slice it received, in call order. Tests use this to assert
    /// which messages actually reach the provider after context assembly and
    /// message filtering (e.g. Infinity Context history trimming).
    pub message_capture: Option<Arc<std::sync::Mutex<Vec<Vec<Message>>>>>,
}

impl Default for LlmSimConfig {
    fn default() -> Self {
        Self {
            response: ResponseConfig::Fixed("Hello! I'm a simulated LLM response.".to_string()),
            tool_calls: None,
            simulate_latency: false,
            model_name: "llmsim-model".to_string(),
            response_delay: None,
            response_id: None,
            effort_capture: None,
            message_capture: None,
        }
    }
}

impl LlmSimConfig {
    /// Create a new config with a fixed response
    pub fn fixed(response: impl Into<String>) -> Self {
        Self {
            response: ResponseConfig::Fixed(response.into()),
            ..Default::default()
        }
    }

    /// Create a new config that echoes user input
    pub fn echo() -> Self {
        Self {
            response: ResponseConfig::Echo,
            ..Default::default()
        }
    }

    /// Create a new config with lorem ipsum text
    pub fn lorem(target_tokens: usize) -> Self {
        Self {
            response: ResponseConfig::Lorem { target_tokens },
            ..Default::default()
        }
    }

    /// Create a new config with a sequence of responses
    pub fn sequence(responses: Vec<String>) -> Self {
        Self {
            response: ResponseConfig::Sequence(responses),
            ..Default::default()
        }
    }

    /// Create a new config that replays scripted turns in order.
    pub fn scripted(turns: Vec<SimTurn>) -> Self {
        Self {
            response: ResponseConfig::Scripted {
                turns,
                on_exhausted: OnExhausted::default(),
            },
            ..Default::default()
        }
    }

    /// Set the behavior when a scripted response config exhausts its turns.
    pub fn with_on_exhausted(mut self, mode: OnExhausted) -> Self {
        if let ResponseConfig::Scripted { on_exhausted, .. } = &mut self.response {
            *on_exhausted = mode;
        }
        self
    }

    /// Add tool calls to the response.
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(ToolCallConfig::Fixed(tool_calls));
        self
    }

    /// Add a sequence of tool calls (different per call).
    pub fn with_tool_call_sequence(mut self, sequences: Vec<Vec<ToolCall>>) -> Self {
        self.tool_calls = Some(ToolCallConfig::Sequence(sequences));
        self
    }

    /// Enable latency simulation.
    pub fn with_latency(mut self) -> Self {
        self.simulate_latency = true;
        self
    }

    /// Set model name for metadata.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model_name = model.into();
        self
    }

    /// Set a delay before responding (TTFT - time to first token).
    /// This creates a predictable time window for testing cancellation scenarios.
    pub fn with_response_delay(mut self, delay: std::time::Duration) -> Self {
        self.response_delay = Some(delay);
        self
    }

    /// Set a response ID to include in completion metadata (for testing chaining)
    pub fn with_response_id(mut self, id: impl Into<String>) -> Self {
        self.response_id = Some(id.into());
        self
    }

    /// Set a shared capture sink for the per-call `reasoning_effort` (EVE-595).
    /// Every `chat_completion_stream` call appends the effort it observed in
    /// `LlmCallConfig`, in call order.
    pub fn with_effort_capture(
        mut self,
        capture: Arc<std::sync::Mutex<Vec<Option<String>>>>,
    ) -> Self {
        self.effort_capture = Some(capture);
        self
    }

    /// Set a shared capture sink for the provider-visible messages of each call.
    /// Every `chat_completion_stream` call appends the exact `Message` slice
    /// it received, in call order.
    pub fn with_message_capture(
        mut self,
        capture: Arc<std::sync::Mutex<Vec<Vec<Message>>>>,
    ) -> Self {
        self.message_capture = Some(capture);
        self
    }

    /// Create a new config that returns an error (for testing error handling)
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            response: ResponseConfig::Error(message.into()),
            ..Default::default()
        }
    }

    /// Create a new config that returns a model-not-available error
    pub fn model_not_available() -> Self {
        Self {
            response: ResponseConfig::ModelNotAvailable,
            ..Default::default()
        }
    }
}

/// Response generation configuration.
#[derive(Debug, Clone)]
pub enum ResponseConfig {
    /// Return a fixed response
    Fixed(String),
    /// Echo back the last user message with a prefix
    Echo,
    /// Generate lorem ipsum text with target token count
    Lorem {
        /// Approximate number of generated tokens.
        target_tokens: usize,
    },
    /// Return responses from a sequence (cycles when exhausted)
    Sequence(Vec<String>),
    /// Replay scripted assistant turns for multi-turn agent scenario tests.
    Scripted {
        /// Ordered turns to replay.
        turns: Vec<SimTurn>,
        /// Behavior after all turns have been consumed.
        on_exhausted: OnExhausted,
    },
    /// Empty response (useful for tool-only responses)
    Empty,
    /// Simulate an error (useful for testing error handling)
    Error(String),
    /// Simulate a model-not-available error
    ModelNotAvailable,
}

/// A single scripted assistant turn.
#[derive(Debug, Clone, PartialEq)]
pub enum SimTurn {
    /// Plain assistant text response.
    Assistant(String),
    /// One or more tool calls in a single assistant turn.
    ToolCalls(Vec<SimToolCall>),
    /// Mixed assistant text and tool calls in the same turn.
    Mixed {
        /// Assistant text emitted before the tool calls.
        text: String,
        /// Tool calls emitted in the same assistant turn.
        tool_calls: Vec<SimToolCall>,
    },
    /// Simulate an API/transport error on this turn.
    Error(SimError),
    /// Return a stream that never produces an event.
    StreamStall,
}

/// A single tool call inside a scripted turn.
#[derive(Debug, Clone, PartialEq)]
pub struct SimToolCall {
    /// Tool name.
    pub name: String,
    /// JSON arguments passed to the tool.
    pub arguments: serde_json::Value,
    /// Optional stable tool-call id; generated deterministically when absent.
    pub id: Option<String>,
}

/// Error to inject for a scripted turn.
#[derive(Debug, Clone, PartialEq)]
pub enum SimError {
    /// Provider rate limit.
    RateLimit,
    /// Provider timeout.
    Timeout,
    /// Transport failure.
    Transport,
    /// Provider overload.
    Overloaded,
    /// Authentication failure.
    Authentication,
    /// Provider quota exhaustion.
    QuotaExhausted,
    /// Unsupported model id.
    UnsupportedModel(String),
    /// Invalid provider response.
    InvalidResponse(String),
    /// Other injected LLM error.
    Other(String),
}

impl SimError {
    fn message(&self) -> String {
        match self {
            SimError::RateLimit => "Rate limit exceeded. Please retry after some time.".to_string(),
            SimError::Timeout => "Request timed out".to_string(),
            SimError::Transport => "Transport connection failed".to_string(),
            SimError::Overloaded => "Provider overloaded".to_string(),
            SimError::Authentication => "Invalid provider credentials".to_string(),
            SimError::QuotaExhausted => "Provider quota exhausted".to_string(),
            SimError::UnsupportedModel(model) => format!("Model not available: {model}"),
            SimError::InvalidResponse(message) | SimError::Other(message) => message.clone(),
        }
    }

    fn agent_error(&self) -> AgentLoopError {
        use everruns_contracts::error::LlmErrorKind;

        match self {
            SimError::RateLimit => {
                AgentLoopError::llm_kind(LlmErrorKind::RateLimited, self.message())
            }
            SimError::Timeout | SimError::Transport | SimError::Overloaded => {
                AgentLoopError::llm_kind(LlmErrorKind::Unavailable, self.message())
            }
            SimError::Other(_) => AgentLoopError::llm_kind(LlmErrorKind::Other, self.message()),
            SimError::Authentication => {
                AgentLoopError::llm_kind(LlmErrorKind::Authentication, self.message())
            }
            SimError::QuotaExhausted => {
                AgentLoopError::llm_kind(LlmErrorKind::QuotaExhausted, self.message())
            }
            SimError::UnsupportedModel(model) => AgentLoopError::model_not_available(model),
            SimError::InvalidResponse(_) => {
                AgentLoopError::llm_kind(LlmErrorKind::InvalidRequest, self.message())
            }
        }
    }
}

/// Behavior when a scripted config has consumed all turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnExhausted {
    /// Repeat the last turn forever.
    #[default]
    RepeatLast,
    /// Return an error when the script is exhausted.
    Error,
    /// Cycle through the script from the start.
    Loop,
}

/// Tool call configuration.
#[derive(Debug, Clone)]
pub enum ToolCallConfig {
    /// Always return these tool calls
    Fixed(Vec<ToolCall>),
    /// Return tool calls from a sequence (cycles when exhausted)
    Sequence(Vec<Vec<ToolCall>>),
    /// Conditionally return tool calls based on message content
    Conditional {
        /// Patterns to match against user message
        patterns: Vec<ToolCallPattern>,
    },
}

/// Pattern for conditional tool calls
#[derive(Debug, Clone)]
pub struct ToolCallPattern {
    /// Substring to match in user message
    pub contains: String,
    /// Tool calls to return when pattern matches
    pub tool_calls: Vec<ToolCall>,
}

impl ToolCallPattern {
    /// Match user messages containing `contains` and return `tool_calls`.
    pub fn new(contains: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            contains: contains.into(),
            tool_calls,
        }
    }
}

fn materialize_scripted_tool_calls(
    turn_index: usize,
    calls: Vec<SimToolCall>,
) -> Option<Vec<ToolCall>> {
    if calls.is_empty() {
        return None;
    }

    Some(
        calls
            .into_iter()
            .enumerate()
            .map(|(call_index, call)| ToolCall {
                id: call
                    .id
                    .unwrap_or_else(|| auto_tool_call_id(turn_index, call_index)),
                name: call.name,
                arguments: call.arguments,
            })
            .collect(),
    )
}

// ============================================================================
// Driver Implementation
// ============================================================================

/// Deterministic LLM simulator driver.
///
/// This driver generates simulated responses based on configuration.
/// It's intended for unit and integration tests where you need
/// deterministic or configurable LLM behavior.
///
/// # Example
///
/// ```ignore
/// use everruns_llmsim::{LlmSimDriver, LlmSimConfig};
///
/// // Simple fixed response
/// let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello!"));
///
/// // With tool calls
/// let driver = LlmSimDriver::new(
///     LlmSimConfig::fixed("Let me check that for you.")
///         .with_tool_calls(vec![ToolCall { ... }])
/// );
///
/// // Sequence of responses for multi-turn tests
/// let driver = LlmSimDriver::new(
///     LlmSimConfig::sequence(vec![
///         "First response".to_string(),
///         "Second response".to_string(),
///     ])
/// );
/// ```
#[derive(Clone)]
pub struct LlmSimDriver {
    config: LlmSimConfig,
    /// Counter for sequence-based responses
    response_counter: Arc<AtomicUsize>,
    /// Counter for sequence-based tool calls
    tool_call_counter: Arc<AtomicUsize>,
    /// Attempt counts for scenario steps that fail first
    scenario_attempts: scenarios::Attempts,
    // Registered Agents share one driver; sequence state belongs to each session.
    session_counters: Arc<std::sync::Mutex<SessionSequenceCounters>>,
}

struct GeneratedTurn {
    text: String,
    tool_calls: Option<Vec<ToolCall>>,
    stream_stall: bool,
}

impl LlmSimDriver {
    /// Create a new driver with the given configuration
    pub fn new(config: LlmSimConfig) -> Self {
        Self {
            config,
            response_counter: Arc::new(AtomicUsize::new(0)),
            tool_call_counter: Arc::new(AtomicUsize::new(0)),
            scenario_attempts: scenarios::Attempts::default(),
            session_counters: Arc::default(),
        }
    }

    /// Create a driver with default configuration (fixed response)
    pub fn default_driver() -> Self {
        Self::new(LlmSimConfig::default())
    }

    /// Generate response text based on configuration
    fn generate_response(&self, messages: &[Message], counter: &AtomicUsize) -> String {
        match &self.config.response {
            ResponseConfig::Fixed(text) => text.clone(),

            ResponseConfig::Echo => {
                // Find last user message and echo it
                let last_user = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == MessageRole::User)
                    .map(|m| m.content_as_text())
                    .unwrap_or_default();
                format!("Echo: {}", last_user)
            }

            ResponseConfig::Lorem { target_tokens } => {
                let generator = LoremGenerator::new(*target_tokens);
                let request = self.to_chat_request(messages);
                generator.generate(&request)
            }

            ResponseConfig::Sequence(responses) => {
                if responses.is_empty() {
                    return String::new();
                }
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                responses[idx % responses.len()].clone()
            }

            ResponseConfig::Empty => String::new(),

            // Error/ModelNotAvailable cases should never be reached; checked in chat_completion_stream
            ResponseConfig::Error(_)
            | ResponseConfig::ModelNotAvailable
            | ResponseConfig::Scripted { .. } => {
                unreachable!("Special configs handled in chat_completion_stream")
            }
        }
    }

    /// Get tool calls based on configuration
    fn get_tool_calls(&self, messages: &[Message], counter: &AtomicUsize) -> Option<Vec<ToolCall>> {
        match &self.config.tool_calls {
            None => None,

            Some(ToolCallConfig::Fixed(calls)) => {
                if calls.is_empty() {
                    None
                } else {
                    Some(calls.clone())
                }
            }

            Some(ToolCallConfig::Sequence(sequences)) => {
                if sequences.is_empty() {
                    return None;
                }
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                let calls = &sequences[idx % sequences.len()];
                if calls.is_empty() {
                    None
                } else {
                    Some(calls.clone())
                }
            }

            Some(ToolCallConfig::Conditional { patterns }) => {
                // Scan user messages newest-first and use the first one that
                // matches a pattern. Looking only at the very last user message
                // is brittle: an injected user-role notification (e.g. a
                // background task's terminal wake-up) can land after the
                // triggering prompt and mask it, even though content-keyed
                // patterns are meant to make scheduling order irrelevant.
                // Newest-first honours the most recent matching intent while
                // skipping interleaved non-matching notifications.
                for message in messages.iter().rev() {
                    if message.role != MessageRole::User {
                        continue;
                    }
                    let text = message.content_as_text();
                    if let Some(pattern) = patterns.iter().find(|p| text.contains(&p.contains)) {
                        return if pattern.tool_calls.is_empty() {
                            None
                        } else {
                            Some(pattern.tool_calls.clone())
                        };
                    }
                }
                None
            }
        }
    }

    fn generate_turn(
        &self,
        messages: &[Message],
        response: &AtomicUsize,
        tools: &AtomicUsize,
    ) -> Result<GeneratedTurn> {
        if let ResponseConfig::Scripted {
            turns,
            on_exhausted,
        } = &self.config.response
        {
            return self.generate_scripted_turn(turns, *on_exhausted, response);
        }

        Ok(GeneratedTurn {
            text: self.generate_response(messages, response),
            tool_calls: self.get_tool_calls(messages, tools),
            stream_stall: false,
        })
    }

    fn generate_scripted_turn(
        &self,
        turns: &[SimTurn],
        on_exhausted: OnExhausted,
        counter: &AtomicUsize,
    ) -> Result<GeneratedTurn> {
        if turns.is_empty() {
            return Err(AgentLoopError::config(
                "llmsim scripted config must contain at least one turn",
            ));
        }

        let turn_index = counter.fetch_add(1, Ordering::SeqCst);
        let turn = if turn_index < turns.len() {
            turns[turn_index].clone()
        } else {
            match on_exhausted {
                OnExhausted::RepeatLast => turns[turns.len() - 1].clone(),
                OnExhausted::Loop => turns[turn_index % turns.len()].clone(),
                OnExhausted::Error => {
                    return Err(AgentLoopError::config("llmsim scripted config exhausted"));
                }
            }
        };

        match turn {
            SimTurn::Assistant(text) => Ok(GeneratedTurn {
                text,
                tool_calls: None,
                stream_stall: false,
            }),
            SimTurn::ToolCalls(calls) => Ok(GeneratedTurn {
                text: String::new(),
                tool_calls: materialize_scripted_tool_calls(turn_index, calls),
                stream_stall: false,
            }),
            SimTurn::Mixed { text, tool_calls } => Ok(GeneratedTurn {
                text,
                tool_calls: materialize_scripted_tool_calls(turn_index, tool_calls),
                stream_stall: false,
            }),
            SimTurn::Error(error) => Err(error.agent_error()),
            SimTurn::StreamStall => Ok(GeneratedTurn {
                text: String::new(),
                tool_calls: None,
                stream_stall: true,
            }),
        }
    }

    /// Convert Message to llmsim ChatCompletionRequest
    fn to_chat_request(&self, messages: &[Message]) -> ChatCompletionRequest {
        let sim_messages: Vec<SimMessage> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => Role::System,
                    MessageRole::User => Role::User,
                    MessageRole::Assistant => Role::Assistant,
                    MessageRole::Tool => Role::Tool,
                };
                SimMessage {
                    role,
                    content: Some(m.content_as_text().into()),
                    name: None,
                    tool_calls: None,
                    tool_call_id: m.tool_call_id.clone(),
                }
            })
            .collect();

        ChatCompletionRequest {
            model: self.config.model_name.clone(),
            messages: sim_messages,
            temperature: None,
            top_p: None,
            n: None,
            max_tokens: None,
            max_completion_tokens: None,
            stream: true,
            stop: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            tools: None,
            tool_choice: None,
            seed: None,
            response_format: None,
            reasoning_effort: None,
        }
    }

    /// Latency for a request: `-realistic` names (see [`model_names`]), else
    /// `fast()` for `-latency` names or `simulate_latency`, else `instant()`.
    fn resolve_latency_profile(&self, model_name: &str) -> LatencyProfile {
        if let Some(profile) = model_names::realistic_latency(model_name) {
            profile
        } else if self.config.simulate_latency || model_name.contains("-latency") {
            LatencyProfile::fast()
        } else {
            LatencyProfile::instant()
        }
    }

    /// Estimate token count for text
    fn estimate_tokens(text: &str) -> u32 {
        // Simple estimation: ~4 chars per token
        (text.len() / 4).max(1) as u32
    }
}

#[async_trait]
impl ChatDriver for LlmSimDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_contracts::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Record per-call effort (EVE-595) before any error, so error turns show.
        if let Some(capture) = &self.config.effort_capture
            && let Ok(mut efforts) = capture.lock()
        {
            efforts.push(config.reasoning_effort.map(|e| e.as_str().to_string()));
        }

        // Record the provider-visible messages for tests, also before any error.
        if let Some(capture) = &self.config.message_capture
            && let Ok(mut calls) = capture.lock()
        {
            calls.push(messages.clone());
        }

        // Check for error configs first
        if let ResponseConfig::Error(error_msg) = &self.config.response {
            return Err(anyhow::anyhow!("LLM error: {}", error_msg).into());
        }
        if matches!(self.config.response, ResponseConfig::ModelNotAvailable) {
            return Err(AgentLoopError::model_not_available(config.model.clone()));
        }

        // Response delay (config or "-ttft-{ms}" in the model name) simulates time to
        // first token, giving cancellation tests a predictable window.
        let delay = self
            .config
            .response_delay
            .or_else(|| model_names::ttft(&config.model));
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }

        // A scenario marker in the user message (or an `llmsim-scenario-*`
        // model) takes over the whole call, pacing included.
        if let Some(played) = scenarios::play(
            &messages,
            &config.tools,
            &config.model,
            self.resolve_latency_profile(&config.model),
            &self.scenario_attempts,
        ) {
            return played;
        }

        let counters = if let Some(session) = config.metadata.get("session_id") {
            let mut sessions = self
                .session_counters
                .lock()
                .map_err(|_| AgentLoopError::config("simulator session counters unavailable"))?;
            if sessions.len() >= 100_000 && !sessions.contains_key(session) {
                return Err(AgentLoopError::config("simulator session capacity reached"));
            }
            sessions
                .entry(session.clone())
                .or_insert_with(|| (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))))
                .clone()
        } else {
            (
                self.response_counter.clone(),
                self.tool_call_counter.clone(),
            )
        };
        let generated_turn = model_names::realistic_turn(self, &messages, config).map_or_else(
            || self.generate_turn(&messages, &counters.0, &counters.1),
            Ok,
        )?;
        if generated_turn.stream_stall {
            return Ok(Box::pin(futures::stream::pending()));
        }
        let response_text = generated_turn.text;
        let tool_calls = generated_turn.tool_calls;
        let model_name = config.model.clone();
        let response_id_for_done = self.config.response_id.clone();
        let latency_profile = self.resolve_latency_profile(&model_name);

        // Calculate token estimates
        let prompt_tokens: u32 = messages
            .iter()
            .map(|m| Self::estimate_tokens(&m.content_as_text()))
            .sum();
        let completion_tokens = Self::estimate_tokens(&response_text);

        // TokenStreamBuilder handles TTFT and inter-token delays via LatencyProfile.
        let usage = Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            completion_tokens_details: None,
        };

        let chunk_stream = TokenStreamBuilder::new(&model_name, &response_text)
            .latency(latency_profile)
            .usage(usage)
            .build()
            .into_chunk_stream();

        // Reasoning artifacts, when the caller asked the model to reason.
        //
        // llmsim exists so the agent loop can be exercised without provider
        // credits, and reasoning was the one part of a turn it could not
        // produce. That left the whole reasoning path — persistence, the REST
        // projection, and the sanitization that keeps replay state out of the
        // API — testable only against live providers, so it went untested.
        //
        // The artifact deliberately carries both readable text and opaque
        // replay state (signature and encrypted payload, as Anthropic and
        // OpenAI respectively return), so a test can assert that the text
        // survives to the API while the replay state does not.
        let reasoning_events: Vec<Result<LlmStreamEvent>> = if config
            .reasoning_effort
            .is_some_and(|effort| effort.requests_reasoning())
        {
            let part = everruns_contracts::reasoning::ReasoningContentPart::opaque("llmsim")
                .with_item_id("rs_llmsim_0")
                .with_signature("llmsim-signature-opaque")
                .with_encrypted("llmsim-encrypted-opaque")
                .with_text(everruns_contracts::reasoning::ReasoningText::Plain {
                    text: LLMSIM_REASONING_TEXT.to_string(),
                })
                .with_tokens(8);
            vec![
                Ok(LlmStreamEvent::ReasoningDelta {
                    delta: LLMSIM_REASONING_TEXT.to_string(),
                    summary: false,
                }),
                Ok(LlmStreamEvent::ReasoningItem(part)),
            ]
        } else {
            Vec::new()
        };

        // Map simulated chunks to stream events, then append tool calls and metadata.
        let tool_calls_tail = tool_calls;
        let model_name_done = model_name.clone();
        let event_stream = chunk_stream.flat_map(move |chunk| {
            let mut events: Vec<Result<LlmStreamEvent>> = Vec::new();

            for choice in &chunk.choices {
                if let Some(content) = &choice.delta.content
                    && !content.is_empty()
                {
                    events.push(Ok(LlmStreamEvent::TextDelta(content.clone())));
                }
            }

            stream::iter(events)
        });

        // Append tool calls + done after the text stream
        let done_events: Vec<Result<LlmStreamEvent>> = {
            let mut tail = Vec::new();
            if let Some(calls) = tool_calls_tail {
                tail.push(Ok(LlmStreamEvent::ToolCalls(calls)));
            }
            tail.push(Ok(LlmStreamEvent::Done(Box::new({
                let mut metadata = LlmCompletionMetadata::default();
                metadata.total_tokens = Some(prompt_tokens + completion_tokens);
                metadata.prompt_tokens = Some(prompt_tokens);
                metadata.completion_tokens = Some(completion_tokens);
                metadata.model = Some(model_name_done);
                metadata.response_model = Some(self.config.model_name.clone());
                metadata.finish_reason = Some("stop".to_string());
                metadata.response_id = response_id_for_done;
                metadata
            }))));
            tail
        };

        // Reasoning precedes the answer, matching every real provider.
        let full_stream = stream::iter(reasoning_events)
            .chain(event_stream)
            .chain(stream::iter(done_events));
        Ok(Box::pin(full_stream))
    }

    /// Scripted answers stand in for a schema-enforcing provider.
    fn supports_response_format(&self, _model: &str) -> bool {
        true
    }
}

impl std::fmt::Debug for LlmSimDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmSimDriver")
            .field("model", &self.config.model_name)
            .field("simulate_latency", &self.config.simulate_latency)
            .finish()
    }
}

/// Readable reasoning text llmsim emits when a turn requests reasoning.
///
/// Distinctive so a test can assert it reached the API, and assert that the
/// opaque replay state alongside it did not.
pub const LLMSIM_REASONING_TEXT: &str = "llmsim deliberating about the request";

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the LlmSim driver with the driver registry
///
/// This registers a driver for the `LlmSim` provider type.
/// The driver is created with a default configuration; for custom configs,
/// create the driver directly using `LlmSimDriver::new()`.
///
/// # Example
///
/// ```ignore
/// use everruns_core::DriverRegistry;
/// use everruns_llmsim::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    let descriptor = DriverDescriptor::chat_only(DriverId::LlmSim, |_config| {
        // Default driver - tests can create custom drivers directly.
        Box::new(LlmSimDriver::default_driver()) as BoxedChatDriver
    });
    registry.register_descriptor_or_replace(realtime::with_simulated_realtime(descriptor));
}

/// Register the LlmSim driver with a custom configuration. Useful for
/// servers/workers that want to opt into a scripted scenario (e.g. the
/// `user_hooks` audit-log demo) without changing the default behaviour for
/// callers that don't.
///
/// The same `config` is cloned for every constructed driver in the
/// registry, so its `Arc`-backed counters (sequence index, etc.) are
/// shared across invocations.
pub fn register_driver_with_config(registry: &mut DriverRegistry, config: LlmSimConfig) {
    let driver = LlmSimDriver::new(config);
    let descriptor = DriverDescriptor::chat_only(DriverId::LlmSim, move |_config| {
        Box::new(driver.clone()) as BoxedChatDriver
    });
    registry.register_descriptor_or_replace(realtime::with_simulated_realtime(descriptor));
}

/// Create a LlmSim driver with custom configuration
///
/// This is the preferred way to create a driver in tests.
/// Unlike `register_driver`, this gives you full control over the config.
///
/// # Example
///
/// ```ignore
/// use everruns_llmsim::{create_chat_driver, LlmSimConfig};
///
/// let driver = create_chat_driver(
///     LlmSimConfig::fixed("I'll help you with that!")
///         .with_tool_calls(vec![...])
/// );
/// ```
pub fn create_chat_driver(config: LlmSimConfig) -> BoxedChatDriver {
    Box::new(LlmSimDriver::new(config))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests;
