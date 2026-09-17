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
//! Conversation state, tool execution, workspaces, and durability stay with
//! [`Agent`](crate::Agent) — a completion here keeps no history of its own.

use std::fmt;

use everruns_provider::driver_registry::{
    LlmCallConfig, LlmMessage, LlmMessageRole, LlmResponse, LlmResponseStream,
};
use everruns_provider::error::AgentLoopError;
use everruns_provider::model::ReasoningEffort;
use everruns_provider::runtime_provider::Provider;

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
    messages: Vec<LlmMessage>,
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
        self.message(LlmMessageRole::System, content)
    }

    /// Append a user message.
    pub fn user(self, content: impl Into<String>) -> Self {
        self.message(LlmMessageRole::User, content)
    }

    /// Append an assistant message, replaying a prior answer as context.
    pub fn assistant(self, content: impl Into<String>) -> Self {
        self.message(LlmMessageRole::Assistant, content)
    }

    fn message(mut self, role: LlmMessageRole, content: impl Into<String>) -> Self {
        self.messages.push(LlmMessage::text(role, content));
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
    fn into_request(self) -> Result<(Provider, Vec<LlmMessage>, LlmCallConfig), CompletionError> {
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
                LlmMessageRole::System,
                LlmMessageRole::User,
                LlmMessageRole::Assistant,
                LlmMessageRole::User,
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
