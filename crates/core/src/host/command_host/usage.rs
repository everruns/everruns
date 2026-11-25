//! Accounting for tool-less calls, never their private prompts or answers.

use std::sync::Arc;

use crate::event_emitter::EventEmitter;
use crate::events::{
    EventContext, EventRequest, LlmGenerationData, LlmRequestOptions, LlmRetryInfo,
};
use everruns_contracts::driver_registry::{
    LlmCompletionMetadata, LlmResponseStream, LlmStreamEvent,
};
use everruns_contracts::{DriverId, error::Result, typed_id::SessionId};
use futures::StreamExt;

pub(super) struct CompletionUsage {
    pub emitter: Arc<dyn EventEmitter>,
    pub session: SessionId,
    pub context: EventContext,
    pub model: String,
    pub provider: DriverId,
    pub provider_id: String,
    pub purpose: String,
    pub started: everruns_contracts::rt::Instant,
    pub operation: uuid::Uuid,
}

impl CompletionUsage {
    pub async fn record(
        &self,
        metadata: Option<&LlmCompletionMetadata>,
        stream: bool,
    ) -> Result<()> {
        let duration = Some(self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        let mut generation = match metadata {
            Some(meta) => LlmGenerationData::success_with_metadata(
                vec![],
                vec![],
                None,
                vec![],
                self.model.clone(),
                Some(self.provider.to_string()),
                crate::llm_usage::completion_usage(meta, &self.provider, &self.model),
                duration,
                None,
                meta.finish_reason.clone().map(|reason| vec![reason]),
                meta.response_id.clone(),
            )
            .with_response_model(meta.response_model.clone())
            .with_stop_details(
                meta.provider_finish_reason.clone(),
                meta.tool_calls_dropped,
                meta.tool_calls_truncated_executed,
            ),
            None => LlmGenerationData::failure(
                vec![],
                vec![],
                self.model.clone(),
                Some(self.provider.to_string()),
                "Auxiliary completion failed before final usage was reported".into(),
                duration,
                None,
            ),
        };
        if let Some(retry) = metadata
            .and_then(|meta| meta.retry_metadata.as_ref())
            .filter(|retry| retry.attempts > 0)
        {
            generation = generation.with_retry(LlmRetryInfo {
                attempts: retry.attempts,
                total_wait_ms: retry.total_retry_wait.as_millis().min(u128::from(u64::MAX)) as u64,
            });
        }
        generation = generation
            .with_provider_id(Some(self.provider_id.clone()))
            .with_request_options(LlmRequestOptions {
                stream: Some(stream),
                metadata: [("purpose".into(), self.purpose.clone())].into(),
                ..Default::default()
            });
        self.emitter
            .emit(
                EventRequest::new(self.session, self.context.clone(), generation).with_metadata(
                    serde_json::json!({"completion_operation_id":self.operation.to_string()}),
                ),
            )
            .await?;
        Ok(())
    }

    pub fn stream(self, events: LlmResponseStream) -> LlmResponseStream {
        Box::pin(futures::stream::unfold(
            (events, self, false),
            |(mut events, usage, ended)| async move {
                if ended {
                    return None;
                }
                let Some(item) = events.next().await else {
                    return match usage.record(None, true).await {
                        Ok(()) => None,
                        Err(error) => Some((Err(error), (events, usage, true))),
                    };
                };
                let final_usage = match &item {
                    Ok(LlmStreamEvent::Done(metadata)) => Some(Some(metadata.as_ref())),
                    Ok(LlmStreamEvent::Error(_)) | Err(_) => Some(None),
                    _ => None,
                };
                let ended = final_usage.is_some();
                if let Some(metadata) = final_usage
                    && let Err(error) = usage.record(metadata, true).await
                {
                    return Some((Err(error), (events, usage, true)));
                }
                Some((item, (events, usage, ended)))
            },
        ))
    }
}
