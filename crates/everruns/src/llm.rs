//! Stability: stable — no breaking change without a major bump; see [`stability`](crate::stability).
//!
//! Direct model calls, without an agent.
//!
//! Some work is one prompt and one answer: classify a string, draft a summary,
//! extract a field. That needs the provider edge — drivers, endpoints,
//! credentials, retries — but none of the agent loop around it. This module is
//! the value-first surface for exactly that, over the same [`Provider`] an
//! [`Agent`](crate::Agent) would use.
//!
//! Start from a [`Model`]: [`Model::complete`] for the one-line case,
//! [`Model::completion`] when the call needs a system message, more turns, or
//! per-call controls.
//!
//! A completion can also offer tools ([`Completion::tool`]) without becoming an
//! agent: the schemas go to the model and the calls come back to the caller,
//! which is what an application owning its own loop needs. And because nothing
//! bounds a provider that stops mid-answer from outside, the per-call limits
//! live here too — [`Completion::timeout`],
//! [`Completion::first_token_timeout`], [`Completion::max_response_bytes`].
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::Model;
//!
//! let answer = Model::simulated("4").complete("What is 2 + 2?").await?;
//! assert_eq!(answer, "4");
//! # Ok(())
//! # }
//! ```
//!
//! Applications that own a wire protocol call [`Provider`] directly; see
//! [Custom providers](https://docs.everruns.com/framework/custom-providers/).
//! Conversation state, tool *execution*, workspaces, and durability stay with
//! [`Agent`](crate::Agent) — a completion here keeps no history of its own.

use std::fmt;

use std::time::Duration;

use everruns_provider::driver_registry::{
    LlmCallConfig, LlmResponse, LlmResponseStream, Message, MessageRole,
};
use everruns_provider::error::AgentLoopError;
use everruns_provider::model::ReasoningEffort;
use everruns_provider::runtime_provider::Provider;
use everruns_provider::tool_types::ToolDefinition;
use serde_json::Value;

use crate::Model;

/// Why a direct completion could not be made.
///
/// [`MissingProvider`](Self::MissingProvider) and
/// [`NoMessages`](Self::NoMessages) are configuration mistakes, caught before
/// any request leaves the process. [`Call`](Self::Call) carries the provider
/// failure verbatim, so callers keep the full
/// [`LlmError`](crate::LlmError) classification.
#[derive(Debug)]
#[non_exhaustive]
pub enum CompletionError {
    /// The model carries no provider, and none was attached to the completion.
    ///
    /// A bare model id (`Model::from("gpt-5-mini")`) names a model without
    /// saying how to reach it. Use [`Model::new`] or
    /// [`Completion::provider`].
    MissingProvider,
    /// The completion was sent with no messages.
    NoMessages,
    /// The provider call failed.
    Call(AgentLoopError),
}

impl fmt::Display for CompletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompletionError::MissingProvider => write!(
                f,
                "model has no provider; use Model::new(id, provider) or Completion::provider"
            ),
            CompletionError::NoMessages => {
                write!(f, "completion has no messages; add at least one")
            }
            CompletionError::Call(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CompletionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CompletionError::Call(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AgentLoopError> for CompletionError {
    fn from(error: AgentLoopError) -> Self {
        CompletionError::Call(error)
    }
}

/// One direct model call, described before it is sent.
///
/// Built with [`Model::completion`]. Messages append in call order; the
/// per-call controls each map to one provider request field and stay unset
/// unless assigned, so the provider keeps its own defaults.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::Model;
///
/// let response = Model::simulated("Paris.")
///     .completion()
///     .system("Answer with a single word.")
///     .user("What is the capital of France?")
///     .max_tokens(16)
///     .send()
///     .await?;
/// assert_eq!(response.text, "Paris.");
/// # Ok(())
/// # }
/// ```
pub struct Completion {
    provider: Option<Provider>,
    config: LlmCallConfig,
    messages: Vec<Message>,
}

impl Completion {
    fn new(model: &Model) -> Self {
        Self {
            provider: model.bundled_provider().cloned(),
            config: LlmCallConfig::new(model.id()),
            messages: Vec::new(),
        }
    }

    /// Reach the model through `provider`, replacing any the model bundled.
    ///
    /// Needed when the model is a bare provider-visible id, and useful when one
    /// provider value is shared across many calls.
    pub fn provider(mut self, provider: impl Into<Provider>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Append a system message.
    pub fn system(self, content: impl Into<String>) -> Self {
        self.message(MessageRole::System, content)
    }

    /// Append a user message.
    pub fn user(self, content: impl Into<String>) -> Self {
        self.message(MessageRole::User, content)
    }

    /// Append an assistant message, replaying a prior answer as context.
    pub fn assistant(self, content: impl Into<String>) -> Self {
        self.message(MessageRole::Assistant, content)
    }

    fn message(mut self, role: MessageRole, content: impl Into<String>) -> Self {
        self.messages.push(Message::text(role, content));
        self
    }

    /// Set the sampling temperature.
    pub fn temperature(mut self, temperature: f32) -> Self {
        self.config.temperature = Some(temperature);
        self
    }

    /// Cap the tokens the model may generate.
    pub fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.config.max_tokens = Some(max_tokens);
        self
    }

    /// Set the reasoning effort, on models that support it.
    pub fn reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.config.reasoning_effort = Some(effort);
        self
    }

    /// Offer the model a tool it may call.
    ///
    /// The schema only: nothing here executes. A direct completion has no
    /// loop, so a tool call comes back to the caller on
    /// [`LlmResponse::tool_calls`], to run and answer however it likes — which
    /// is the point for an application that owns its own loop. An agent that
    /// should call tools *and* run them is [`Agent`](crate::Agent), which
    /// takes executable [`Tool`](crate::Tool)s instead.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::Model;
    /// use serde_json::json;
    ///
    /// let response = Model::simulated("checking")
    ///     .completion()
    ///     .user("what is the weather in Kyiv?")
    ///     .tool(
    ///         "get_weather",
    ///         "Current weather for a city",
    ///         json!({"type": "object", "properties": {"city": {"type": "string"}}}),
    ///     )
    ///     .send()
    ///     .await?;
    /// for call in response.tool_calls.unwrap_or_default() {
    ///     println!("{} {}", call.name, call.arguments);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn tool(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        json_schema: Value,
    ) -> Self {
        self.config
            .tools
            .push(ToolDefinition::function(name, description, json_schema));
        self
    }

    /// Offer the model several tools at once.
    ///
    /// For callers that already hold definitions — read from their own
    /// configuration, or converted from OpenAI-shaped JSON with
    /// [`openai_wire`](everruns_provider::openai_wire).
    pub fn tools(mut self, tools: impl IntoIterator<Item = ToolDefinition>) -> Self {
        self.config.tools.extend(tools);
        self
    }

    /// Let the model issue several tool calls in one turn, or forbid it.
    ///
    /// Unset by default, leaving the provider's own behavior.
    pub fn parallel_tool_calls(mut self, allowed: bool) -> Self {
        self.config.parallel_tool_calls = Some(allowed);
        self
    }

    /// Give up on the call if it has not finished within `timeout`.
    ///
    /// Covers the whole turn, streaming included. Without one a provider that
    /// stops sending mid-answer holds the caller indefinitely.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.limits.total = Some(timeout);
        self
    }

    /// Give up if the provider has not started answering within `timeout`.
    pub fn first_token_timeout(mut self, timeout: Duration) -> Self {
        self.config.limits.first_event = Some(timeout);
        self
    }

    /// Record the exact request body the driver sends, on
    /// [`LlmCompletionMetadata::request_body`].
    ///
    /// Off by default. What a driver puts on the wire is its own — which
    /// fields, how tools and reasoning are shaped — so a caller storing or
    /// showing what was asked otherwise has to approximate it. The body
    /// carries the whole prompt, which is why turning it on is deliberate;
    /// credentials travel in headers and are never captured.
    ///
    /// [`LlmCompletionMetadata::request_body`]: crate::LlmCompletionMetadata::request_body
    pub fn capture_request(mut self, capture: bool) -> Self {
        self.config.capture_request = capture;
        self
    }

    /// Refuse an answer that grows past `bytes`.
    ///
    /// Counted across answer text and readable reasoning, and checked as they
    /// accumulate, so a runaway generation is cut off rather than buffered.
    pub fn max_response_bytes(mut self, bytes: u64) -> Self {
        self.config.limits.max_response_bytes = Some(bytes);
        self
    }

    /// Send the call and wait for the whole answer.
    ///
    /// Non-streaming: drivers with a native non-streaming endpoint use it,
    /// the rest collect their own stream.
    pub async fn send(self) -> Result<LlmResponse, CompletionError> {
        let (provider, messages, config) = self.into_request()?;
        Ok(provider
            .chat_completion_non_streaming(messages, &config)
            .await?)
    }

    /// Send the call and return the answer text, discarding everything else.
    ///
    /// Including any tool calls: a completion carrying tools wants
    /// [`send`](Self::send) instead.
    pub async fn text(self) -> Result<String, CompletionError> {
        Ok(self.send().await?.text)
    }

    /// Send the call and observe it as provider events as they arrive.
    ///
    /// The stream yields [`LlmStreamEvent`](crate::LlmStreamEvent) values and
    /// ends with a `Done` carrying the call's metadata.
    pub async fn stream(self) -> Result<LlmResponseStream, CompletionError> {
        let (provider, messages, config) = self.into_request()?;
        Ok(provider.chat_completion_stream(messages, &config).await?)
    }

    /// Validate the described call, keeping both configuration mistakes off the
    /// wire.
    fn into_request(self) -> Result<(Provider, Vec<Message>, LlmCallConfig), CompletionError> {
        let provider = self.provider.ok_or(CompletionError::MissingProvider)?;
        if self.messages.is_empty() {
            return Err(CompletionError::NoMessages);
        }
        Ok((provider, self.messages, self.config))
    }
}

impl fmt::Debug for Completion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Completion")
            .field("provider", &self.provider)
            .field("model", &self.config.model)
            .field("messages", &self.messages.len())
            .field("tools", &self.config.tools.len())
            .finish()
    }
}

impl Model {
    /// Ask this model one question and take its answer as text.
    ///
    /// The whole one-shot path: no agent, no session, no history. Use
    /// [`completion`](Self::completion) when the call needs a system message,
    /// more than one turn of context, streaming, or per-call controls.
    ///
    /// ```
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::Model;
    ///
    /// let answer = Model::simulated("4").complete("What is 2 + 2?").await?;
    /// assert_eq!(answer, "4");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn complete(&self, prompt: impl Into<String>) -> Result<String, CompletionError> {
        self.completion().user(prompt).text().await
    }

    /// Describe a direct call to this model.
    pub fn completion(&self) -> Completion {
        Completion::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_llmsim::LlmSimConfig;
    use futures::StreamExt;

    #[tokio::test]
    async fn complete_returns_the_models_answer() {
        let answer = Model::simulated("4")
            .complete("What is 2 + 2?")
            .await
            .expect("simulated completion succeeds");
        assert_eq!(answer, "4");
    }

    #[tokio::test]
    async fn completion_sends_every_message_in_order() {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut sim = LlmSimConfig::fixed("ack");
        sim.message_capture = Some(captured.clone());
        let model = Model::simulated_with_config(sim);

        let response = model
            .completion()
            .system("Be terse.")
            .user("First.")
            .assistant("Noted.")
            .user("Second.")
            .send()
            .await
            .expect("simulated completion succeeds");

        assert_eq!(response.text, "ack");
        let calls = captured.lock().expect("capture lock");
        let sent = calls.last().expect("one recorded call");
        let roles: Vec<_> = sent.iter().map(|message| message.role.clone()).collect();
        assert_eq!(
            roles,
            vec![
                MessageRole::System,
                MessageRole::User,
                MessageRole::Assistant,
                MessageRole::User,
            ]
        );
        assert_eq!(
            sent.last().expect("last message").content.to_text(),
            "Second."
        );
    }

    #[tokio::test]
    async fn stream_yields_the_answer_as_events() {
        let mut text = String::new();
        let mut stream = Model::simulated("streamed")
            .completion()
            .user("go")
            .stream()
            .await
            .expect("simulated stream starts");
        while let Some(event) = stream.next().await {
            if let crate::LlmStreamEvent::TextDelta(delta) = event.expect("no stream error") {
                text.push_str(&delta);
            }
        }
        assert_eq!(text, "streamed");
    }

    #[test]
    fn tools_are_declared_as_schemas_the_caller_will_answer_for() {
        let completion = Model::simulated("ok")
            .completion()
            .user("weather?")
            .tool(
                "get_weather",
                "Current weather for a city",
                serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}}),
            )
            .tools([ToolDefinition::function(
                "get_time",
                "Current time",
                serde_json::json!({"type": "object"}),
            )]);

        let names: Vec<_> = completion
            .config
            .tools
            .iter()
            .map(|tool| tool.name())
            .collect();
        assert_eq!(names, vec!["get_weather", "get_time"]);
        // Schema only: a direct completion runs nothing, so the definitions
        // are client-side and the calls come back to the caller.
        assert!(
            completion
                .config
                .tools
                .iter()
                .all(|tool| matches!(tool, ToolDefinition::ClientSide(_)))
        );
    }

    #[tokio::test]
    async fn a_completion_with_tools_still_answers_in_text_when_the_model_does() {
        let response = Model::simulated("no tool needed")
            .completion()
            .user("hi")
            .tool(
                "noop",
                "does nothing",
                serde_json::json!({"type": "object"}),
            )
            .send()
            .await
            .expect("simulated completion succeeds");
        assert_eq!(response.text, "no tool needed");
        assert!(response.tool_calls.is_none());
    }

    #[test]
    fn per_call_bounds_reach_the_call_configuration() {
        let completion = Model::simulated("ok")
            .completion()
            .user("go")
            .timeout(Duration::from_secs(30))
            .first_token_timeout(Duration::from_secs(5))
            .max_response_bytes(1024)
            .parallel_tool_calls(false);
        assert_eq!(
            completion.config.limits.total,
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            completion.config.limits.first_event,
            Some(Duration::from_secs(5))
        );
        assert_eq!(completion.config.limits.max_response_bytes, Some(1024));
        assert_eq!(completion.config.parallel_tool_calls, Some(false));
        assert!(
            !completion.config.capture_request,
            "the prompt is not recorded unless asked for"
        );
        assert!(
            Model::simulated("ok")
                .completion()
                .capture_request(true)
                .config
                .capture_request
        );
    }

    #[tokio::test]
    async fn a_bare_model_id_has_no_provider() {
        let error = Model::from("gpt-5-mini")
            .complete("anything")
            .await
            .expect_err("a bare id cannot be reached");
        assert!(matches!(error, CompletionError::MissingProvider));
    }

    #[tokio::test]
    async fn an_empty_completion_never_reaches_the_provider() {
        let error = Model::simulated("unused")
            .completion()
            .send()
            .await
            .expect_err("no messages is a configuration error");
        assert!(matches!(error, CompletionError::NoMessages));
    }

    #[tokio::test]
    async fn an_explicit_provider_answers_for_a_bare_model_id() {
        let provider = Provider::new(
            "llmsim",
            everruns_llmsim::LlmSimDriver::new(LlmSimConfig::fixed("attached")),
        );
        let answer = Model::from("any-model")
            .completion()
            .provider(provider)
            .user("go")
            .text()
            .await
            .expect("explicit provider answers");
        assert_eq!(answer, "attached");
    }
}
