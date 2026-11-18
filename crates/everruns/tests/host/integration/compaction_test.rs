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
