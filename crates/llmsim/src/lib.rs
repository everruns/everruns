#![deny(missing_docs)]

//! Deterministic, offline LLM simulation for
//! [Everruns](https://everruns.com) agents and runtimes.
//!
//! `everruns-llmsim` implements the provider contracts from
//! [`everruns-provider`](https://docs.rs/everruns-provider) with configurable
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
//! # let _ = driver;
//! ```

#[cfg(feature = "host")]
mod runtime_ext;

#[cfg(feature = "host")]
pub use runtime_ext::{LLMSIM_MODEL_ID, LLMSIM_PROVIDER, LlmSimRuntimeExt, llm_sim_provider};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use everruns_provider::driver_registry::{
    BoxedChatDriver, ChatDriver, DriverDescriptor, DriverId, DriverRegistry, LlmCallConfig,
    LlmCompletionMetadata, LlmMessage, LlmMessageRole, LlmResponseStream, LlmStreamEvent,
};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::tool_types::ToolCall;
use llmsim::generator::{LoremGenerator, ResponseGenerator};
use llmsim::latency::LatencyProfile;
use llmsim::openai::{ChatCompletionRequest, Message, Role, Usage};
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
    /// `LlmMessage` slice it received, in call order. Tests use this to assert
    /// which messages actually reach the provider after context assembly and
    /// message filtering (e.g. Infinity Context history trimming).
    pub message_capture: Option<Arc<std::sync::Mutex<Vec<Vec<LlmMessage>>>>>,
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
    /// Every `chat_completion_stream` call appends the exact `LlmMessage` slice
    /// it received, in call order.
    pub fn with_message_capture(
        mut self,
        capture: Arc<std::sync::Mutex<Vec<Vec<LlmMessage>>>>,
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
        use everruns_provider::error::LlmErrorKind;

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
        }
    }

    /// Create a driver with default configuration (fixed response)
    pub fn default_driver() -> Self {
        Self::new(LlmSimConfig::default())
    }

    /// Generate response text based on configuration
    fn generate_response(&self, messages: &[LlmMessage]) -> String {
        match &self.config.response {
            ResponseConfig::Fixed(text) => text.clone(),

            ResponseConfig::Echo => {
                // Find last user message and echo it
                let last_user = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == LlmMessageRole::User)
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
                let idx = self.response_counter.fetch_add(1, Ordering::SeqCst);
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
    fn get_tool_calls(&self, messages: &[LlmMessage]) -> Option<Vec<ToolCall>> {
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
                let idx = self.tool_call_counter.fetch_add(1, Ordering::SeqCst);
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
                    if message.role != LlmMessageRole::User {
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

    fn generate_turn(&self, messages: &[LlmMessage]) -> Result<GeneratedTurn> {
        if let ResponseConfig::Scripted {
            turns,
            on_exhausted,
        } = &self.config.response
        {
            return self.generate_scripted_turn(turns, *on_exhausted);
        }

        Ok(GeneratedTurn {
            text: self.generate_response(messages),
            tool_calls: self.get_tool_calls(messages),
            stream_stall: false,
        })
    }

    fn generate_scripted_turn(
        &self,
        turns: &[SimTurn],
        on_exhausted: OnExhausted,
    ) -> Result<GeneratedTurn> {
        if turns.is_empty() {
            return Err(AgentLoopError::config(
                "llmsim scripted config must contain at least one turn",
            ));
        }

        let turn_index = self.response_counter.fetch_add(1, Ordering::SeqCst);
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

    /// Convert LlmMessage to llmsim ChatCompletionRequest
    fn to_chat_request(&self, messages: &[LlmMessage]) -> ChatCompletionRequest {
        let sim_messages: Vec<Message> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    LlmMessageRole::System => Role::System,
                    LlmMessageRole::User => Role::User,
                    LlmMessageRole::Assistant => Role::Assistant,
                    LlmMessageRole::Tool => Role::Tool,
                };
                Message {
                    role,
                    content: Some(m.content_as_text()),
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
        }
    }

    /// Resolve latency profile for a request.
    /// Model names containing "-latency" enable realistic streaming simulation
    /// via LatencyProfile::fast(). The config flag `simulate_latency` also enables it.
    /// Returns LatencyProfile::instant() when neither is set.
    fn resolve_latency_profile(&self, model_name: &str) -> LatencyProfile {
        if self.config.simulate_latency || model_name.contains("-latency") {
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
        _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Record the per-call reasoning effort for tests (EVE-595). Captured
        // before any error short-circuit so even error turns are observable.
        if let Some(capture) = &self.config.effort_capture
            && let Ok(mut efforts) = capture.lock()
        {
            efforts.push(config.reasoning_effort.clone());
        }

        // Record the provider-visible messages for tests. Captured before any
        // error short-circuit so even error turns are observable.
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

        // Apply response delay if configured or if model name contains "-ttft-{ms}".
        // TTFT = Time To First Token. This simulates LLM "thinking" time.
        // Used for testing cancellation scenarios where we need a predictable
        // time window to cancel an active turn before the LLM completes.
        let delay = self
            .config
            .response_delay
            .or_else(|| parse_ttft_from_model_name(&config.model));
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }

        let generated_turn = self.generate_turn(&messages)?;
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

        // Use llmsim's TokenStreamBuilder for streaming with latency simulation.
        // It handles TTFT and inter-token delays natively via LatencyProfile.
        let usage = Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        };

        let chunk_stream = TokenStreamBuilder::new(&model_name, &response_text)
            .latency(latency_profile)
            .usage(usage)
            .build()
            .into_chunk_stream();

        // Map llmsim ChatCompletionChunk -> our LlmStreamEvent, then append
        // tool calls and metadata after the text stream completes.
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
            tail.push(Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                total_tokens: Some(prompt_tokens + completion_tokens),
                prompt_tokens: Some(prompt_tokens),
                completion_tokens: Some(completion_tokens),
                cache_read_tokens: None,
                cache_creation_tokens: None,
                provider_cost_usd: None,
                model: Some(model_name_done),
                finish_reason: Some("stop".to_string()),
                retry_metadata: None,
                response_id: response_id_for_done,
                phase: None,
            }))));
            tail
        };

        let full_stream = event_stream.chain(stream::iter(done_events));
        Ok(Box::pin(full_stream))
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
    let mut descriptor = DriverDescriptor::chat_only(DriverId::LlmSim, |_config| {
        // Default driver - tests can create custom drivers directly.
        Box::new(LlmSimDriver::default_driver()) as BoxedChatDriver
    });
    descriptor.display_name = "LLM Simulator".into();
    registry.register_descriptor_or_replace(descriptor);
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
    let mut descriptor = DriverDescriptor::chat_only(DriverId::LlmSim, move |_config| {
        Box::new(driver.clone()) as BoxedChatDriver
    });
    descriptor.display_name = "LLM Simulator".into();
    registry.register_descriptor_or_replace(descriptor);
}

/// Parse TTFT (time to first token) delay from model name if it contains "-ttft-{ms}" pattern.
/// For example: "llmsim-ttft-2000" returns Some(Duration::from_millis(2000))
///
/// This allows tests to opt-in to response delays by using specific model names,
/// which is useful for testing cancellation of active turns.
fn parse_ttft_from_model_name(model_name: &str) -> Option<std::time::Duration> {
    if let Some(idx) = model_name.find("-ttft-") {
        let after_ttft = &model_name[idx + 6..]; // skip "-ttft-"
        let ms_str: String = after_ttft
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(ms) = ms_str.parse::<u64>()
            && ms > 0
        {
            return Some(std::time::Duration::from_millis(ms));
        }
    }
    None
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
// Pre-baked demo scripts
// ============================================================================

/// Scripted multi-turn config that drives the Cloud Cost & Security Auditor
/// example agent through a small, deterministic AWS audit using only the
/// `fake_aws` capability's tools. Useful for the `user_hooks` end-to-end
/// demo (and any operator who wants to exercise the auditor without an LLM
/// API key).
///
/// Sequence of assistant turns:
///   1. Call `aws_list_ec2_instances`.
///   2. Call `aws_list_s3_buckets`.
///   3. Write a short audit summary as plain text.
///
/// After turn 3 the script repeats the final turn, matching the default
/// `OnExhausted::RepeatLast` behavior.
pub fn auditor_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Starting the audit. Listing EC2 instances first.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "aws_list_ec2_instances".to_string(),
                arguments: serde_json::json!({}),
                id: Some("call_demo_ec2".to_string()),
            }],
        },
        SimTurn::Mixed {
            text: "EC2 inventory captured. Listing S3 buckets next.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "aws_list_s3_buckets".to_string(),
                arguments: serde_json::json!({}),
                id: Some("call_demo_s3".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Audit complete: inventoried EC2 instances and S3 buckets. \
             See /workspace/.audit.log for the per-tool-call audit trail \
             written by the post_tool_use hook bundle."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}

/// Scripted scenario that exercises the `pre_tool_use` block path. The
/// scripted agent first issues a destructive `bash` call (`rm -rf /`)
/// followed by a benign one (`ls -la`). When combined with a `pre_tool_use`
/// hook bundle that denies `rm -rf` patterns, the first tool call gets
/// blocked (the tool is not invoked) and the second succeeds — the agent
/// observes the difference in tool results.
///
/// Used by `LLMSIM_DEMO=guarded` to demonstrate `pre_tool_use` without an
/// LLM API key.
pub fn guarded_bash_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Step 1: attempting a destructive command.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "bash".to_string(),
                arguments: serde_json::json!({ "commands": "rm -rf /" }),
                id: Some("call_demo_rm".to_string()),
            }],
        },
        SimTurn::Mixed {
            text: "Step 2: trying a safe command.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "bash".to_string(),
                arguments: serde_json::json!({ "commands": "ls -la /workspace" }),
                id: Some("call_demo_ls".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Guarded-bash demo complete. The first tool call should be \
             blocked by the pre_tool_use hook; the second should succeed."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}

/// Scripted scenario that exercises the session task registry end-to-end
/// without an LLM API key. The scripted agent starts a background bash run
/// via `spawn_background` (which creates a `background_tool` session task),
/// then inspects the registry with `list_tasks`.
///
/// Used by `LLMSIM_DEMO=tasks`. Requires an agent with the `bashkit_shell`
/// and `session_tasks` capabilities.
pub fn session_tasks_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Kicking off a background bash run.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "spawn_background".to_string(),
                arguments: serde_json::json!({
                    "tool": "bash",
                    "args": { "commands": "echo task demo start; echo task demo done" },
                    "title": "Demo background run",
                    "signal_on_completion": false,
                }),
                id: Some("call_demo_spawn".to_string()),
            }],
        },
        SimTurn::Mixed {
            text: "Checking the session task registry.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "list_tasks".to_string(),
                arguments: serde_json::json!({}),
                id: Some("call_demo_list".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Session tasks demo complete: a background run was started and \
             tracked as a session task. Inspect it via \
             GET /v1/sessions/{session_id}/tasks."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}

/// Scripted scenario for the monitor task kind: spawns a recurring scheduled
/// monitor (cron fires at second 0 every minute) so the session scheduler
/// creates the schedule and a linked `monitor` task. Used by
/// `LLMSIM_DEMO=monitor` for end-to-end verification without an LLM API key.
pub fn monitor_demo_script() -> LlmSimConfig {
    let turns = vec![
        SimTurn::Mixed {
            text: "Setting up a recurring monitor.".to_string(),
            tool_calls: vec![SimToolCall {
                name: "spawn_background".to_string(),
                arguments: serde_json::json!({
                    "tool": "bash",
                    "args": { "commands": "echo monitor check" },
                    "title": "Demo monitor",
                    "signal_on_completion": false,
                    "schedule": { "cron_expression": "0 * * * * * *", "timezone": "UTC" },
                }),
                id: Some("call_demo_monitor".to_string()),
            }],
        },
        SimTurn::Assistant(
            "Monitor demo complete: a recurring monitor was scheduled and tracked as a session task. Inspect it via GET /v1/sessions/{session_id}/tasks."
                .to_string(),
        ),
    ];
    LlmSimConfig::scripted(turns)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    impl LlmSimDriver {
        async fn chat_completion(
            &self,
            messages: Vec<LlmMessage>,
            config: &LlmCallConfig,
        ) -> Result<everruns_provider::driver_registry::LlmResponse> {
            ChatDriver::chat_completion(
                self,
                &everruns_provider::runtime_provider::ProviderEndpoint::default(),
                messages,
                config,
            )
            .await
        }

        async fn chat_completion_stream(
            &self,
            messages: Vec<LlmMessage>,
            config: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            ChatDriver::chat_completion_stream(
                self,
                &everruns_provider::runtime_provider::ProviderEndpoint::default(),
                messages,
                config,
            )
            .await
        }
    }

    #[test]
    fn auditor_demo_script_calls_ec2_then_s3_then_summarises() {
        let config = auditor_demo_script();
        let turns = match &config.response {
            ResponseConfig::Scripted { turns, .. } => turns,
            other => panic!("expected Scripted, got {other:?}"),
        };
        assert_eq!(turns.len(), 3, "script has three turns");
        match &turns[0] {
            SimTurn::Mixed { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "aws_list_ec2_instances");
            }
            other => panic!("turn 0 should be Mixed, got {other:?}"),
        }
        match &turns[1] {
            SimTurn::Mixed { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].name, "aws_list_s3_buckets");
            }
            other => panic!("turn 1 should be Mixed, got {other:?}"),
        }
        match &turns[2] {
            SimTurn::Assistant(text) => {
                assert!(
                    text.contains("/workspace/.audit.log"),
                    "summary mentions the audit log: {text:?}"
                );
            }
            other => panic!("turn 2 should be Assistant, got {other:?}"),
        }
    }

    fn make_config() -> LlmCallConfig {
        LlmCallConfig {
            speed: None,
            verbosity: None,
            model: "test-model".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
        }
    }

    fn user_message(content: &str) -> LlmMessage {
        LlmMessage::text(LlmMessageRole::User, content)
    }

    fn system_message(content: &str) -> LlmMessage {
        LlmMessage::text(LlmMessageRole::System, content)
    }

    #[tokio::test]
    async fn test_fixed_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello, world!"));
        let messages = vec![user_message("Hi there")];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert_eq!(response.text, "Hello, world!");
        assert!(response.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_echo_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::echo());
        let messages = vec![
            system_message("You are a helpful assistant"),
            user_message("What is 2+2?"),
        ];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert_eq!(response.text, "Echo: What is 2+2?");
    }

    #[tokio::test]
    async fn test_sequence_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::sequence(vec![
            "First".to_string(),
            "Second".to_string(),
            "Third".to_string(),
        ]));

        let messages = vec![user_message("test")];

        // First call
        let r1 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r1.text, "First");

        // Second call
        let r2 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r2.text, "Second");

        // Third call
        let r3 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r3.text, "Third");

        // Fourth call - cycles back to first
        let r4 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(r4.text, "First");
    }

    #[tokio::test]
    async fn test_lorem_response() {
        let driver = LlmSimDriver::new(LlmSimConfig::lorem(50));
        let messages = vec![user_message("Generate text")];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        // Lorem response should have content
        assert!(!response.text.is_empty());
        // Should have multiple words
        assert!(response.text.split_whitespace().count() > 5);
    }

    #[tokio::test]
    async fn test_fixed_tool_calls() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let driver = LlmSimDriver::new(
            LlmSimConfig::fixed("Let me check the weather.")
                .with_tool_calls(vec![tool_call.clone()]),
        );

        let messages = vec![user_message("What's the weather?")];
        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert_eq!(response.text, "Let me check the weather.");
        let calls = response.tool_calls.expect("Expected tool calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].id, "call_123");
    }

    #[tokio::test]
    async fn test_tool_call_sequence() {
        let call1 = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
        };
        let call2 = ToolCall {
            id: "call_2".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
        };

        let driver = LlmSimDriver::new(
            LlmSimConfig::fixed("Processing...").with_tool_call_sequence(vec![
                vec![call1.clone()],
                vec![call2.clone()],
                vec![],
            ]),
        );

        let messages = vec![user_message("test")];

        // First call - should get search
        let r1 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        let calls1 = r1.tool_calls.expect("Expected tool calls");
        assert_eq!(calls1[0].name, "search");

        // Second call - should get fetch
        let r2 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        let calls2 = r2.tool_calls.expect("Expected tool calls");
        assert_eq!(calls2[0].name, "fetch");

        // Third call - no tool calls
        let r3 = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert!(r3.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_scripted_multi_turn_tool_call_agent_sequence() {
        let driver = LlmSimDriver::new(
            LlmSimConfig::scripted(vec![
                SimTurn::ToolCalls(vec![SimToolCall {
                    name: "bash".to_string(),
                    arguments: serde_json::json!({"command": "echo hello > /tmp/x.txt"}),
                    id: None,
                }]),
                SimTurn::ToolCalls(vec![SimToolCall {
                    name: "bash".to_string(),
                    arguments: serde_json::json!({"command": "sed -i s/hello/world/ /tmp/x.txt"}),
                    id: None,
                }]),
                SimTurn::Assistant("done".to_string()),
            ])
            .with_on_exhausted(OnExhausted::Error),
        );

        let messages = vec![user_message("create /tmp/x.txt then change hello to world")];

        let first = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        let first_calls = first.tool_calls.expect("first turn should call bash");
        assert_eq!(first.text, "");
        assert_eq!(first_calls[0].name, "bash");
        assert_eq!(first_calls[0].id, "call_llmsim_0_0");

        let second = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        let second_calls = second.tool_calls.expect("second turn should call bash");
        assert_eq!(second_calls[0].name, "bash");
        assert_eq!(second_calls[0].id, "call_llmsim_1_0");

        let final_response = driver
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap();
        assert_eq!(final_response.text, "done");
        assert!(final_response.tool_calls.is_none());

        let exhausted = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap_err();
        assert!(matches!(exhausted, AgentLoopError::Configuration(_)));
    }

    #[tokio::test]
    async fn test_scripted_mixed_turn_streams_text_and_tool_calls() {
        let driver = LlmSimDriver::new(LlmSimConfig::scripted(vec![SimTurn::Mixed {
            text: "Let me check".to_string(),
            tool_calls: vec![SimToolCall {
                name: "search".to_string(),
                arguments: serde_json::json!({"q": "rust"}),
                id: Some("call_search".to_string()),
            }],
        }]));

        let mut stream = driver
            .chat_completion_stream(vec![user_message("find rust")], &make_config())
            .await
            .unwrap();

        let mut text_parts = Vec::new();
        let mut tool_calls = None;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmStreamEvent::TextDelta(text) => text_parts.push(text),
                LlmStreamEvent::ToolCalls(calls) => tool_calls = Some(calls),
                LlmStreamEvent::Done(_) => {}
                _ => {}
            }
        }

        assert!(!text_parts.is_empty(), "scripted text should stream");
        assert_eq!(text_parts.join(""), "Let me check");
        let calls = tool_calls.expect("mixed turn should emit tool calls");
        assert_eq!(calls[0].id, "call_search");
        assert_eq!(calls[0].name, "search");
    }

    #[tokio::test]
    async fn test_scripted_on_exhausted_modes() {
        let repeat = LlmSimDriver::new(LlmSimConfig::scripted(vec![
            SimTurn::Assistant("one".to_string()),
            SimTurn::Assistant("two".to_string()),
        ]));
        let messages = vec![user_message("test")];
        assert_eq!(
            repeat
                .chat_completion(messages.clone(), &make_config())
                .await
                .unwrap()
                .text,
            "one"
        );
        assert_eq!(
            repeat
                .chat_completion(messages.clone(), &make_config())
                .await
                .unwrap()
                .text,
            "two"
        );
        assert_eq!(
            repeat
                .chat_completion(messages.clone(), &make_config())
                .await
                .unwrap()
                .text,
            "two"
        );

        let looping = LlmSimDriver::new(
            LlmSimConfig::scripted(vec![
                SimTurn::Assistant("a".to_string()),
                SimTurn::Assistant("b".to_string()),
            ])
            .with_on_exhausted(OnExhausted::Loop),
        );
        assert_eq!(
            looping
                .chat_completion(messages.clone(), &make_config())
                .await
                .unwrap()
                .text,
            "a"
        );
        assert_eq!(
            looping
                .chat_completion(messages.clone(), &make_config())
                .await
                .unwrap()
                .text,
            "b"
        );
        assert_eq!(
            looping
                .chat_completion(messages, &make_config())
                .await
                .unwrap()
                .text,
            "a"
        );
    }

    #[tokio::test]
    async fn test_scripted_error_turn() {
        let driver = LlmSimDriver::new(LlmSimConfig::scripted(vec![SimTurn::Error(
            SimError::RateLimit,
        )]));

        let err = driver
            .chat_completion(vec![user_message("test")], &make_config())
            .await
            .unwrap_err();

        assert!(err.is_rate_limited());
    }

    #[tokio::test]
    async fn test_conditional_tool_calls() {
        let weather_call = ToolCall {
            id: "call_w".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({}),
        };
        let search_call = ToolCall {
            id: "call_s".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({}),
        };

        let config = LlmSimConfig {
            response: ResponseConfig::Fixed("Response".to_string()),
            tool_calls: Some(ToolCallConfig::Conditional {
                patterns: vec![
                    ToolCallPattern::new("weather", vec![weather_call]),
                    ToolCallPattern::new("search", vec![search_call]),
                ],
            }),
            simulate_latency: false,
            model_name: "test".to_string(),
            response_delay: None,
            response_id: None,
            effort_capture: None,
            message_capture: None,
        };

        let driver = LlmSimDriver::new(config);

        // Weather query - should trigger weather tool
        let r1 = driver
            .chat_completion(vec![user_message("What's the weather?")], &make_config())
            .await
            .unwrap();
        let calls1 = r1.tool_calls.expect("Expected weather tool");
        assert_eq!(calls1[0].name, "get_weather");

        // Search query - should trigger search tool
        let r2 = driver
            .chat_completion(vec![user_message("search for rust")], &make_config())
            .await
            .unwrap();
        let calls2 = r2.tool_calls.expect("Expected search tool");
        assert_eq!(calls2[0].name, "search");

        // No matching pattern - no tool calls
        let r3 = driver
            .chat_completion(vec![user_message("hello world")], &make_config())
            .await
            .unwrap();
        assert!(r3.tool_calls.is_none());
    }

    #[tokio::test]
    async fn test_streaming() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello world test"));
        let messages = vec![user_message("test")];

        let mut stream = driver
            .chat_completion_stream(messages, &make_config())
            .await
            .unwrap();

        let mut text_parts = Vec::new();
        let mut got_done = false;

        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmStreamEvent::TextDelta(text) => text_parts.push(text),
                LlmStreamEvent::Done(meta) => {
                    got_done = true;
                    assert!(meta.total_tokens.is_some());
                    assert!(meta.model.is_some());
                }
                _ => {}
            }
        }

        assert!(got_done);
        // llmsim's TokenStream handles chunking; verify full text is correct
        assert!(!text_parts.is_empty());
        assert_eq!(text_parts.join(""), "Hello world test");
    }

    #[tokio::test]
    async fn test_metadata() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hi").with_model("custom-model"));
        let messages = vec![user_message("test")];

        let mut config = make_config();
        config.model = "request-model".to_string();

        let response = driver.chat_completion(messages, &config).await.unwrap();

        // Model should come from the request config
        assert_eq!(response.metadata.model, Some("request-model".to_string()));
        assert!(response.metadata.prompt_tokens.is_some());
        assert!(response.metadata.completion_tokens.is_some());
    }

    #[tokio::test]
    async fn test_register_driver() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        assert!(registry.has_driver(&DriverId::LlmSim));

        // Creating a driver should work (with any API key since it's simulated)
        let config = everruns_provider::driver_registry::ProviderConfig::new(DriverId::LlmSim)
            .with_api_key("fake-key");
        let driver = registry.create_chat_driver(&config);
        assert!(driver.is_ok());
    }

    #[tokio::test]
    async fn test_empty_response() {
        let config = LlmSimConfig {
            response: ResponseConfig::Empty,
            tool_calls: None,
            simulate_latency: false,
            model_name: "test".to_string(),
            response_delay: None,
            response_id: None,
            effort_capture: None,
            message_capture: None,
        };

        let driver = LlmSimDriver::new(config);
        let messages = vec![user_message("test")];

        let response = driver
            .chat_completion(messages, &make_config())
            .await
            .unwrap();

        assert!(response.text.is_empty());
    }

    #[test]
    fn test_driver_debug() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("test").with_latency());
        let debug = format!("{:?}", driver);

        assert!(debug.contains("LlmSimDriver"));
        assert!(debug.contains("simulate_latency"));
    }

    #[test]
    fn test_default_config() {
        let config = LlmSimConfig::default();
        assert!(matches!(config.response, ResponseConfig::Fixed(_)));
        assert!(config.tool_calls.is_none());
        assert!(!config.simulate_latency);
    }

    #[test]
    fn test_config_builder() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let config = LlmSimConfig::fixed("Result")
            .with_tool_calls(vec![tool_call.clone()])
            .with_latency()
            .with_model("gpt-4")
            .with_response_delay(std::time::Duration::from_secs(2));

        assert!(config.tool_calls.is_some());
        assert!(config.simulate_latency);
        assert_eq!(config.model_name, "gpt-4");
        assert_eq!(
            config.response_delay,
            Some(std::time::Duration::from_secs(2))
        );
    }

    #[test]
    fn test_parse_ttft_from_model_name() {
        use super::parse_ttft_from_model_name;

        // Valid patterns
        assert_eq!(
            parse_ttft_from_model_name("llmsim-ttft-2000"),
            Some(std::time::Duration::from_millis(2000))
        );
        assert_eq!(
            parse_ttft_from_model_name("test-ttft-500-extra"),
            Some(std::time::Duration::from_millis(500))
        );

        // No TTFT patterns
        assert_eq!(parse_ttft_from_model_name("llmsim-model"), None);
        assert_eq!(parse_ttft_from_model_name("llmsim-ttft-0"), None);
        assert_eq!(parse_ttft_from_model_name("llmsim-ttft-abc"), None);
    }

    #[test]
    fn test_resolve_latency_profile_from_model_name() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("test"));

        // "-latency" in model name -> fast profile (non-instant)
        let profile = driver.resolve_latency_profile("llmsim-latency");
        assert!(profile.sample_ttft().as_nanos() > 0);

        // default model name -> instant profile
        let profile = driver.resolve_latency_profile("llmsim-default");
        assert_eq!(profile.sample_ttft().as_nanos(), 0);

        // config flag also enables fast profile
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("test").with_latency());
        let profile = driver.resolve_latency_profile("llmsim-default");
        assert!(profile.sample_ttft().as_nanos() > 0);
    }

    #[tokio::test]
    async fn test_latency_streaming_from_model_name() {
        // Default driver (simulate_latency=false) but model name triggers latency
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello world"));
        let messages = vec![user_message("test")];

        let mut config = make_config();
        config.model = "llmsim-latency".to_string();

        let start = std::time::Instant::now();
        let mut stream = driver
            .chat_completion_stream(messages, &config)
            .await
            .unwrap();

        let mut text_parts = Vec::new();
        let mut got_done = false;

        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmStreamEvent::TextDelta(text) => text_parts.push(text),
                LlmStreamEvent::Done(meta) => {
                    got_done = true;
                    assert_eq!(meta.model, Some("llmsim-latency".to_string()));
                }
                _ => {}
            }
        }

        assert!(got_done);
        assert_eq!(text_parts.join(""), "Hello world");
        // With latency simulation, streaming should take non-zero time
        // (TTFT + inter-token delays)
        assert!(
            start.elapsed().as_millis() > 0,
            "latency simulation should introduce delays"
        );
    }

    #[tokio::test]
    async fn test_no_latency_streaming_is_instant() {
        let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello world"));
        let messages = vec![user_message("test")];

        let mut config = make_config();
        config.model = "llmsim-default".to_string();

        let start = std::time::Instant::now();
        let response = driver.chat_completion(messages, &config).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(response.text, "Hello world");
        // Without latency, should complete nearly instantly (under 50ms)
        assert!(
            elapsed.as_millis() < 50,
            "instant mode should have no delays, took {}ms",
            elapsed.as_millis()
        );
    }
}
