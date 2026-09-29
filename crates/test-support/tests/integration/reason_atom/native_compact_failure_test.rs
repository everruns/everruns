use super::*;

#[derive(Clone, Debug)]
struct NativeCompactFailureDriver {
    attempts: Arc<AtomicUsize>,
}

#[async_trait]
impl everruns_provider::driver_registry::ChatDriver for NativeCompactFailureDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
        _messages: Vec<everruns_provider::driver_registry::Message>,
        _config: &everruns_provider::driver_registry::LlmCallConfig,
    ) -> everruns_provider::error::Result<everruns_provider::driver_registry::LlmResponseStream>
    {
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(everruns_provider::error::AgentLoopError::request_too_large(
                "force compact",
            ));
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(
                everruns_provider::driver_registry::LlmStreamEvent::TextDelta(
                    "fallback succeeded".to_string(),
                ),
            ),
            Ok(everruns_provider::driver_registry::LlmStreamEvent::Done(
                Box::default(),
            )),
        ])))
    }

    fn supports_compact(&self) -> bool {
        true
    }

    async fn compact(
        &self,
        _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
        _request: everruns_provider::compact::CompactRequest,
    ) -> everruns_provider::error::Result<Option<everruns_provider::compact::CompactResponse>> {
        Err(everruns_provider::error::AgentLoopError::llm(
            "compact failed",
        ))
    }
}

#[tokio::test]
async fn native_compact_failure_does_not_install_checkpoint() {
    use everruns_builtins::{COMPACTION_CAPABILITY_ID, CompactionCapability};
    use everruns_capability::CapabilityRef as AgentCapabilityConfig;
    use everruns_core::execution_loading::SessionStore;

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
    set_default_test_model(
        &provider_store,
        DriverId::OpenAI,
        "gpt-5.4",
        Some("fake-api-key"),
    )
    .await;
    let mut session = session_store
        .get_session(session_id.into())
        .await
        .unwrap()
        .unwrap();
    session.capabilities = vec![AgentCapabilityConfig::with_config(
        COMPACTION_CAPABILITY_ID,
        json!({ "strategy": "native", "proactive": false }),
    )];
    session_store.add_session(session).await;
    message_retriever
        .seed(session_id.into(), vec![RuntimeMessage::user("raw history")])
        .await;

    let driver = NativeCompactFailureDriver {
        attempts: Arc::new(AtomicUsize::new(0)),
    };
    let mut drivers = DriverRegistry::new();
    drivers.register(DriverId::OpenAI, move |_| Box::new(driver.clone()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(CompactionCapability);
    let checkpoint_store = Arc::new(everruns_host::InMemoryCompactionCheckpointStore::default());
    let atom = reason_atom_with_stores(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capabilities,
        drivers,
        InMemoryEventEmitter::new(),
    )
    .with_compaction_checkpoint_store(checkpoint_store.clone());

    atom.execute(ReasonInput {
        context: create_context(session_id),
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    })
    .await
    .expect("fallback retry should succeed");

    assert!(
        checkpoint_store
            .get_latest(session_id.into(), "openai", "gpt-5.4")
            .await
            .unwrap()
            .is_none()
    );
}
