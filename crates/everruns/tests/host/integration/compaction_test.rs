//! Managed continuity through the public reason/act host, including a fresh runtime.
use everruns_contracts::{CapabilityRef, tool_types::ToolCall};
use everruns_core::compaction_checkpoint::{
    PORTABLE_COMPACTION_FORMAT_VERSION as FORMAT, PORTABLE_COMPACTION_MODEL as MODEL,
    PORTABLE_COMPACTION_PROVIDER as PROVIDER,
};
use everruns_core::host::HostBackends;
use everruns_core::{
    EventData, RuntimeMessage,
    events::{EventContext, EventRequest, InputMessageData},
};
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use serde_json::json;

const CURRENT_INPUT: &str = "Inspect the original evidence; never delete anything.";

struct ParallelHistoryDriver(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[async_trait::async_trait]
impl everruns_contracts::driver_registry::ChatDriver for ParallelHistoryDriver {
    fn effective_context_window(&self, _: &str) -> Option<usize> {
        Some(128_000)
    }

    async fn chat_completion_stream(
        &self,
        _: &everruns_contracts::runtime_provider::ProviderEndpoint,
        messages: Vec<everruns_contracts::driver_registry::Message>,
        config: &everruns_contracts::driver_registry::LlmCallConfig,
    ) -> everruns_contracts::error::Result<everruns_contracts::driver_registry::LlmResponseStream>
    {
        use everruns_contracts::driver_registry::{
            LlmCompletionMetadata, LlmResponse, MessageContent, MessageRole,
        };
        let summary = config
            .metadata
            .get("purpose")
            .is_some_and(|p| p == "continuity_summary");
        let calls = if summary {
            None
        } else {
            assert!(messages.iter().any(|m| m.role == MessageRole::User && matches!(&m.content, MessageContent::Text(text) if text.contains(CURRENT_INPUT))), "the real current input must reach every main-model request");
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Some(
                    (0..12)
                        .map(|i| ToolCall {
                            id: format!("parallel-{i}"),
                            name: "query_history".into(),
                            arguments: json!({"message_range":{"from":0,"to":20},"limit":20}),
                        })
                        .collect(),
                )
            } else {
                None
            }
        };
        let mut metadata = LlmCompletionMetadata::default();
        metadata.finish_reason = Some(
            if calls.is_some() {
                "tool_calls"
            } else {
                "stop"
            }
            .into(),
        );
        Ok(LlmResponse {
            text: if summary {
                "Earlier evidence remains reference data; inspect only, no deletion approved."
            } else {
                "Evidence checked; nothing deleted."
            }
            .into(),
            reasoning: vec![],
            tool_calls: calls,
            metadata,
        }
        .into_stream())
    }
}

#[tokio::test]
async fn a_large_parallel_history_batch_compacts_without_losing_the_current_instruction() {
    use everruns_contracts::runtime_provider::Provider;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let backends = HostBackends::in_memory();
    let runtime = everruns::batteries::runtime_builder()
        .backends(backends.clone())
        .provider_with_default_model(
            Provider::new("test", ParallelHistoryDriver(calls.clone())),
            "test",
        )
        .single_session(|s| {
            s.harness(
                "read-only",
                "Only query_history is available; inspect only.",
            )
            .harness_capability(CapabilityRef::with_config(
                "compaction",
                json!({"context_budget_tokens":4096}),
            ))
            .agent("worker", "Inspect evidence; preserve user constraints.")
        })
        .build()
        .await
        .unwrap();
    let session = runtime.default_session_id().unwrap();
    for i in 0..20 {
        backends
            .event_log
            .append(EventRequest::new(
                session,
                EventContext::empty(),
                InputMessageData::new(RuntimeMessage::user(format!(
                    "original evidence {i}: {}",
                    "retained evidence ".repeat(100)
                ))),
            ))
            .await
            .unwrap();
    }
    let result = runtime.run_text_turn(session, CURRENT_INPUT).await.unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let messages = runtime.messages(session).await.unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.text().is_some_and(|t| t.starts_with("original evidence")))
            .count(),
        20
    );
    assert_eq!(
        messages
            .iter()
            .filter(|m| m
                .tool_call_id()
                .is_some_and(|id| id.starts_with("parallel-")))
            .count(),
        12
    );
    let checkpoint = backends
        .compaction_checkpoint_store
        .get_latest_format(session, PROVIDER, MODEL, FORMAT)
        .await
        .unwrap()
        .unwrap();
    let events = runtime.events().await.unwrap();
    let input_sequence = events.iter().find(|e| matches!(&e.data, EventData::InputMessage(d) if d.message.text() == Some(CURRENT_INPUT))).unwrap().sequence.unwrap();
    assert!(
        checkpoint.source_sequence > i64::from(input_sequence),
        "the input is preserved even when the checkpoint covers it"
    );
}

#[tokio::test]
async fn compaction_alone_restores_continuity_and_retrieves_original_history_after_restart() {
    let backends = HostBackends::in_memory();
    let runtime = everruns::batteries::runtime_builder()
        .backends(backends.clone())
        .llm_sim_as_default(
            LlmSimConfig::fixed("continuity retained").with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "history".into(),
                    name: "query_history".into(),
                    arguments: json!({"query":"deployment-code-amber"}),
                }],
                vec![],
            ]),
        )
        .single_session(|s| {
            s.harness("continuity", "Only retrieve history; do not deploy.")
                .harness_capability(CapabilityRef::with_config(
                    "compaction",
                    json!({"context_budget_tokens":4096}),
                ))
                .agent("worker", "Use the retained journal to verify details.")
        })
        .build()
        .await
        .unwrap();
    let session = runtime.default_session_id().unwrap();
    for index in 0..20 {
        let text = if index == 0 {
            "deployment-code-amber".to_string()
        } else {
            format!("historical {index}: {}", "evidence ".repeat(125))
        };
        backends
            .event_log
            .append(EventRequest::new(
                session,
                EventContext::empty(),
                InputMessageData::new(RuntimeMessage::user(text)),
            ))
            .await
            .unwrap();
    }
    let result = runtime
        .run_text_turn(session, "Find the original deployment code.")
        .await
        .unwrap();
    assert!(result.success, "{:?}", result.error);
    let checkpoint = backends
        .compaction_checkpoint_store
        .get_latest_format(session, PROVIDER, MODEL, FORMAT)
        .await
        .unwrap()
        .expect("durable portable continuity");
    assert!(checkpoint.is_portable());
    let messages = runtime.messages(session).await.unwrap();
    assert!(messages.iter().any(|m| m.tool_call_id() == Some("history")
        && m.content_to_llm_string().contains("deployment-code-amber")));
    assert!(
        messages
            .iter()
            .any(|m| m.text() == Some("deployment-code-amber")),
        "the journal stays lossless"
    );

    let before = runtime.events().await.unwrap();
    let summaries = before.iter().filter(|e| matches!(&e.data, EventData::LlmGeneration(d)
        if d.metadata.request_options.as_ref().is_some_and(|o| o.metadata.get("purpose").is_some_and(|p| p == "continuity_summary")))).count();
    assert_eq!(summaries, 1);
    let restarted = everruns::batteries::runtime_builder()
        .backends(backends.clone())
        .llm_sim_as_default(LlmSimConfig::fixed("resumed"))
        .single_session(|s| {
            s.session_id(session)
                .harness("continuity", "Only retrieve history; do not deploy.")
                .harness_capability(CapabilityRef::with_config(
                    "compaction",
                    json!({"context_budget_tokens":4096}),
                ))
                .agent("worker", "Continue with preserved constraints.")
        })
        .build()
        .await
        .unwrap();
    let result = restarted
        .run_text_turn(session, "Continue without repeating work.")
        .await
        .unwrap();
    assert!(result.success, "{:?}", result.error);
    let restored = backends
        .compaction_checkpoint_store
        .get_latest_format(session, PROVIDER, MODEL, FORMAT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restored.id, checkpoint.id,
        "unchanged working context reuses the checkpoint"
    );
}
