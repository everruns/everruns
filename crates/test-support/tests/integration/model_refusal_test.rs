//! Model refusal survives runtime classification, tracing and partial output.
use crate::reason_atom_test::{create_context, setup_test_environment};
use async_trait::async_trait;
use everruns_contracts::{
    ChatDriver, DriverId, DriverRegistry, LlmCallConfig, LlmResponseStream, LlmStreamError,
    LlmStreamEvent, Message, ProviderEndpoint,
};
use everruns_core::{
    EventData, RuntimeMessage, capabilities::CapabilityRegistry, engine::ReasonInput,
};
use everruns_test_support::{InMemoryEventEmitter, reason_atom_with_stores};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct RefusingDriver {
    attempts: Arc<AtomicUsize>,
    partial: bool,
    before_stream: bool,
}

#[async_trait]
impl ChatDriver for RefusingDriver {
    async fn chat_completion_stream(
        &self,
        _: &ProviderEndpoint,
        _: Vec<Message>,
        _: &LlmCallConfig,
    ) -> everruns_contracts::error::Result<LlmResponseStream> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let error = LlmStreamError::provider(
            Some("refusal"),
            None,
            "Cannot comply, even after a timeout.",
        );
        if self.before_stream {
            return Err(error.into_agent_error());
        }
        let mut events = vec![];
        if self.partial {
            events.push(Ok(LlmStreamEvent::TextDelta("Partial preface".into())));
        }
        events.push(Ok(LlmStreamEvent::Error(error)));
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

#[tokio::test]
async fn model_refusal_is_terminal_and_traced_even_after_partial_output() {
    for (partial, before_stream) in [(false, false), (true, false), (false, true)] {
        let (
            harness_store,
            agent_store,
            session_store,
            messages,
            provider_store,
            harness_id,
            agent_id,
            session_id,
        ) = setup_test_environment().await;
        messages
            .seed(session_id.into(), vec![RuntimeMessage::user("Hello")])
            .await;
        let attempts = Arc::new(AtomicUsize::new(0));
        let driver = RefusingDriver {
            attempts: attempts.clone(),
            partial,
            before_stream,
        };
        let mut drivers = DriverRegistry::new();
        drivers.register(DriverId::LlmSim, move |_| Box::new(driver.clone()));
        let emitter = InMemoryEventEmitter::new();
        let atom = reason_atom_with_stores(
            harness_store,
            agent_store,
            session_store,
            messages,
            provider_store,
            CapabilityRegistry::new(),
            drivers,
            emitter.clone(),
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
            .unwrap();
        assert!(!result.success);
        assert!(!result.has_tool_calls);
        assert_eq!(result.finish_reason.as_deref(), Some("refusal"));
        assert_eq!(result.user_facing_error.unwrap().code, "model_refusal");
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        let events = emitter.events().await;
        let generations: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.data {
                EventData::LlmGeneration(data) => Some(data),
                _ => None,
            })
            .collect();
        assert_eq!(generations.len(), 1);
        assert!(!generations[0].metadata.success);
        assert_eq!(
            generations[0].metadata.finish_reasons.as_deref(),
            Some(["refusal".to_string()].as_slice())
        );
        assert!(
            generations[0]
                .metadata
                .error
                .as_deref()
                .unwrap()
                .contains("Cannot comply")
        );
        assert!(generations[0].metadata.retry.is_none());
    }
}
