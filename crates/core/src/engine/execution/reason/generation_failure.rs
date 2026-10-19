//! Failure records for calls that terminate before a completion is available.

use super::*;
use crate::engine::events::LlmRetryInfo;

/// One logical generation covers provider retries and reactive compaction.
pub(super) struct FailedGeneration<'a> {
    pub emitter: &'a PhaseEffectEmitter<dyn PhaseEffectSink>,
    pub session_id: SessionId,
    pub context: EventContext,
    pub trace_id: &'a str,
    pub reason_span_id: &'a str,
    pub agent: &'a crate::runtime_agent::RuntimeAgent,
    pub model: &'a crate::runtime_context::ResolvedModelExecution,
    pub messages: &'a [RuntimeMessage],
    pub request_options: Option<crate::engine::events::LlmRequestOptions>,
    pub started: Instant,
}

impl FailedGeneration<'_> {
    pub async fn terminal_start_failure(
        &self,
        error: &AgentLoopError,
        retry: &RetryMetadata,
        config: &LlmRetryConfig,
    ) {
        if !error.rejected_provider_capability(
            everruns_contracts::RejectedProviderCapability::AnthropicServerCompaction,
        ) {
            self.emit(&error.to_string(), None, retry).await;
        }
        generation_outcome::observe_terminal(self.model, error, retry.attempts, config);
    }

    pub async fn result<T>(&self, result: Result<T>, retry: &RetryMetadata) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => Err(self.error(error, retry).await),
        }
    }

    pub async fn error(&self, error: AgentLoopError, retry: &RetryMetadata) -> AgentLoopError {
        self.emit(&error.to_string(), None, retry).await;
        error
    }

    pub async fn emit(&self, error: &str, first_token_ms: Option<u64>, retry: &RetryMetadata) {
        let mut data = LlmGenerationData::failure(
            self.messages.to_vec(),
            self.agent.tools.iter().map(Into::into).collect(),
            self.agent.model.clone(),
            Some(self.model.provider_type.to_string()),
            error.to_string(),
            Some(self.started.elapsed().as_millis() as u64),
            first_token_ms,
        )
        .with_provider_id(Some(self.model.provider.to_string()));
        data.metadata.finish_reasons = Some(vec!["error".to_string()]);
        if let Some(options) = &self.request_options {
            data = data.with_request_options(options.clone());
        }
        if retry.had_retries() {
            data = data.with_retry(LlmRetryInfo {
                attempts: retry.attempts,
                total_wait_ms: retry.total_retry_wait.as_millis() as u64,
            });
        }
        let _ = self
            .emitter
            .emit(EventRequest::new(
                self.session_id,
                self.context.clone().with_span(
                    self.trace_id.to_string(),
                    Uuid::now_v7().to_string(),
                    Some(self.reason_span_id.to_string()),
                ),
                data,
            ))
            .await;
    }
}
