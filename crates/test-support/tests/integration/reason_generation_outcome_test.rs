//! How a generation ended reaches `llm.generation` end to end: the driver's
//! raw stop reason, output tokens, and the tool calls it discarded because the
//! response was cut off.

use crate::reason_atom_test::{create_context, setup_test_environment};
use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, DriverId, DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmResponseStream,
    LlmStreamEvent, Message,
};
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_core::RuntimeMessage;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::engine::ReasonInput;
use everruns_core::events::LlmGenerationMetadata;
use everruns_test_support::{InMemoryEventEmitter, reason_atom_with_stores};
use futures::stream;

/// A driver whose stream ends with the given completion metadata.
#[derive(Clone, Debug)]
struct EndingDriver {
    metadata: LlmCompletionMetadata,
}

#[async_trait]
impl ChatDriver for EndingDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> everruns_contracts::error::Result<LlmResponseStream> {
        Ok(Box::pin(stream::iter(vec![
            Ok(LlmStreamEvent::TextDelta("partial answer".to_string())),
            Ok(LlmStreamEvent::Done(Box::new(self.metadata.clone()))),
        ])))
    }
}

async fn generation_for(metadata: LlmCompletionMetadata) -> LlmGenerationMetadata {
    let (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    ) = setup_test_environment().await;
    message_retriever
        .seed(
            session_id.into(),
            vec![RuntimeMessage::user("write it all")],
        )
        .await;
    let driver = EndingDriver { metadata };
    let mut drivers = DriverRegistry::new();
    drivers.register(DriverId::LlmSim, move |_| Box::new(driver.clone()));
    let event_emitter = InMemoryEventEmitter::new();
    let atom = reason_atom_with_stores(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        CapabilityRegistry::new(),
        drivers,
        event_emitter.clone(),
    );
    let result = atom
        .execute(ReasonInput {
            context: create_context(session_id),
            harness_id,
            agent_id: Some(agent_id.into()),
            org_id: 0,
            mcp_tool_definitions: vec![],
            previous_response_id: None,
            iteration: 1,
        })
        .await
        .expect("reason succeeds");
    assert!(result.success);
    event_emitter
        .events()
        .await
        .into_iter()
        .find_map(|event| match event.data {
            everruns_core::EventData::LlmGeneration(data) => Some(data.metadata),
            _ => None,
        })
        .expect("llm.generation emitted")
}

#[tokio::test]
async fn truncated_generation_reports_raw_reason_and_dropped_tool_calls() {
    let mut metadata = LlmCompletionMetadata::default();
    metadata.finish_reason = Some("length".to_string());
    metadata.provider_finish_reason = Some("max_tokens".to_string());
    metadata.prompt_tokens = Some(10);
    metadata.completion_tokens = Some(77);
    metadata.total_tokens = Some(87);
    metadata.tool_calls_dropped = 2;

    let generation = generation_for(metadata).await;
    assert_eq!(generation.finish_reasons, Some(vec!["length".to_string()]));
    assert_eq!(
        generation.provider_finish_reason.as_deref(),
        Some("max_tokens")
    );
    assert_eq!(generation.tool_calls_dropped, 2);
    assert_eq!(generation.tool_calls_truncated_executed, 0);
    assert_eq!(generation.usage.map(|usage| usage.output_tokens), Some(77));
}

#[tokio::test]
async fn clean_generation_reports_no_truncation() {
    let generation = generation_for(LlmCompletionMetadata::default()).await;
    // No provider reason: inferred from the output, and nothing extra recorded.
    assert_eq!(generation.finish_reasons, Some(vec!["stop".to_string()]));
    assert_eq!(generation.provider_finish_reason, None);
    assert_eq!(generation.tool_calls_dropped, 0);
    assert_eq!(generation.tool_calls_truncated_executed, 0);
}
