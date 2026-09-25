use super::*;

#[derive(Clone, Debug)]
struct RejectingProviderManagedDriver {
    native_available: Arc<AtomicBool>,
    calls: Arc<Mutex<Vec<CapturedLlmCall>>>,
}

#[async_trait]
impl everruns_provider::driver_registry::ChatDriver for RejectingProviderManagedDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        messages: Vec<everruns_provider::Message>,
        config: &everruns_provider::LlmCallConfig,
    ) -> everruns_provider::error::Result<everruns_provider::LlmResponseStream> {
        self.calls.lock().await.push((messages, config.clone()));
        if config
            .driver_options
            .contains_key("anthropic/server_compaction")
        {
            self.native_available.store(false, Ordering::SeqCst);
            return Err(
                everruns_provider::error::AgentLoopError::provider_capability_rejected(
                    everruns_provider::RejectedProviderCapability::AnthropicServerCompaction,
                    400,
                    r#"{"error":{"type":"invalid_request_error"}}"#,
                    "server compaction beta rejected",
                ),
            );
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(everruns_provider::LlmStreamEvent::TextDelta(
                "legacy fallback succeeded".to_string(),
            )),
            Ok(everruns_provider::LlmStreamEvent::Done(Box::default())),
        ])))
    }

    fn provider_managed_reduction_option(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        _model: &str,
        budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        self.native_available.load(Ordering::SeqCst).then(|| {
            (
                "anthropic/server_compaction".to_string(),
                json!({"trigger_tokens":budget_tokens}),
            )
        })
    }
}

#[tokio::test]
async fn pre_stream_capability_rejection_reassembles_legacy_history_once() {
    use everruns_builtins::{INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability};
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
    let provider_type = DriverId::Anthropic;
    set_default_test_model(
        &provider_store,
        provider_type.clone(),
        "eligible-model",
        Some("fake-api-key"),
    )
    .await;
    let mut session = session_store
        .get_session(session_id.into())
        .await
        .unwrap()
        .unwrap();
    session.capabilities = vec![AgentCapabilityConfig::with_config(
        INFINITY_CONTEXT_CAPABILITY_ID,
        json!({
            "context_budget_tokens": 120_000,
            "min_recent_messages": 1,
            "max_recent_messages": 2
        }),
    )];
    session_store.add_session(session).await;
    message_retriever
        .seed(
            session_id.into(),
            (0..6)
                .map(|index| RuntimeMessage::user(format!("raw message {index}")))
                .collect(),
        )
        .await;

    let native_available = Arc::new(AtomicBool::new(true));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let driver = RejectingProviderManagedDriver {
        native_available: native_available.clone(),
        calls: calls.clone(),
    };
    let mut drivers = DriverRegistry::new();
    drivers.register(provider_type.clone(), move |_| Box::new(driver.clone()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    let events = InMemoryEventEmitter::new();
    let atom = reason_atom_with_stores(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capabilities,
        drivers,
        events.clone(),
    );
    let input = || ReasonInput {
        context: create_context(session_id),
        harness_id,
        agent_id: Some(agent_id.into()),
        org_id: 0,
        mcp_tool_definitions: vec![],
        previous_response_id: None,
        iteration: 1,
    };

    assert!(atom.execute(input()).await.unwrap().success);
    assert!(atom.execute(input()).await.unwrap().success);

    let calls = calls.lock().await;
    assert_eq!(calls.len(), 3);
    assert!(
        calls[0]
            .1
            .driver_options
            .contains_key("anthropic/server_compaction")
    );
    for (messages, config) in &calls[1..] {
        assert!(
            !config
                .driver_options
                .contains_key("anthropic/server_compaction")
        );
        assert!(messages.iter().any(|message| {
            message
                .content
                .to_text()
                .contains("earlier messages are not in this context")
        }));
    }
    assert!(!native_available.load(Ordering::SeqCst));
    drop(calls);

    let events = events.events().await;
    let options = events
        .iter()
        .find_map(|event| match &event.data {
            everruns_core::EventData::LlmGeneration(data) => data.metadata.request_options.as_ref(),
            _ => None,
        })
        .expect("fallback generation should record request options");
    assert_eq!(
        options.provider_options["anthropic"]["reduction_mode"],
        "legacy"
    );
    assert_eq!(
        options.provider_options["anthropic"]["fallback_reason"],
        "provider_capability_rejected"
    );
}

#[derive(Clone, Debug)]
struct OutputBudgetFallbackDriver {
    calls: Arc<Mutex<Vec<CapturedLlmCall>>>,
}

#[async_trait]
impl everruns_provider::ChatDriver for OutputBudgetFallbackDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        messages: Vec<everruns_provider::Message>,
        config: &everruns_provider::LlmCallConfig,
    ) -> everruns_provider::Result<everruns_provider::LlmResponseStream> {
        self.calls.lock().await.push((messages, config.clone()));
        Ok(Box::pin(stream::iter(vec![
            Ok(everruns_provider::LlmStreamEvent::TextDelta(
                "legacy output-budget fallback".to_string(),
            )),
            Ok(everruns_provider::LlmStreamEvent::Done(Box::default())),
        ])))
    }

    fn provider_managed_reduction_option(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        _model: &str,
        budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        Some((
            "anthropic/server_compaction".to_string(),
            json!({"trigger_tokens":budget_tokens}),
        ))
    }

    fn provider_managed_reduction_fallback_reason(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        config: &everruns_provider::LlmCallConfig,
    ) -> Option<&'static str> {
        (config.max_tokens == Some(160_000)).then_some("configured_output_budget")
    }
}

#[tokio::test]
async fn configured_max_tokens_reassembles_legacy_history_before_the_call() {
    use everruns_builtins::{INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability};
    use everruns_capability::CapabilityRef as AgentCapabilityConfig;
    use everruns_core::execution_loading::SessionStore;
    use everruns_core::runtime_context::{TurnContextRequest, TurnContextResolver};
    use everruns_engine::ReasonAtom;
    use everruns_host::StoreTurnContextResolver;

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
    let provider_type = DriverId::Anthropic;
    set_default_test_model(
        &provider_store,
        provider_type.clone(),
        "eligible-model",
        Some("fake-api-key"),
    )
    .await;
    let mut session = session_store
        .get_session(session_id.into())
        .await
        .unwrap()
        .unwrap();
    session.capabilities = vec![AgentCapabilityConfig::with_config(
        INFINITY_CONTEXT_CAPABILITY_ID,
        json!({
            "context_budget_tokens": 120_000,
            "min_recent_messages": 1,
            "max_recent_messages": 2
        }),
    )];
    session_store.add_session(session).await;
    message_retriever
        .seed(
            session_id.into(),
            (0..6)
                .map(|index| RuntimeMessage::user(format!("raw message {index}")))
                .collect(),
        )
        .await;

    let calls = Arc::new(Mutex::new(Vec::new()));
    let driver = OutputBudgetFallbackDriver {
        calls: calls.clone(),
    };
    let mut drivers = DriverRegistry::new();
    drivers.register(provider_type.clone(), move |_| Box::new(driver.clone()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    let resolver = StoreTurnContextResolver::new(
        Arc::new(harness_store),
        Arc::new(agent_store),
        Arc::new(session_store),
        Arc::new(message_retriever.clone()),
        Arc::new(provider_store),
        capabilities.clone(),
        drivers,
    );
    let request = TurnContextRequest {
        session_id: session_id.into(),
        harness_id,
        agent_id: Some(agent_id.into()),
        mcp_tool_definitions: vec![],
        allow_provider_managed_reduction: true,
    };
    let mut assembled = resolver
        .resolve_turn_context(request)
        .await
        .expect("native context should assemble before request preflight");
    assembled.runtime_agent.max_tokens = Some(160_000);
    let events = InMemoryEventEmitter::new();
    let atom = ReasonAtom::new(resolver, message_retriever, capabilities, events.clone());

    let result = atom
        .execute_with_assembled_context(
            ReasonInput {
                context: create_context(session_id),
                harness_id,
                agent_id: Some(agent_id.into()),
                org_id: 0,
                mcp_tool_definitions: vec![],
                previous_response_id: None,
                iteration: 1,
            },
            assembled,
        )
        .await
        .unwrap();

    assert!(result.success);
    let calls = calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].0.iter().any(|message| {
        message
            .content
            .to_text()
            .contains("earlier messages are not in this context")
    }));
    assert!(
        !calls[0]
            .1
            .driver_options
            .contains_key("anthropic/server_compaction")
    );
    assert_eq!(
        calls[0].1.driver_options["everruns/provider_managed_reduction_fallback"]["reason"],
        "configured_output_budget"
    );
    drop(calls);
    let options = events
        .events()
        .await
        .iter()
        .find_map(|event| match &event.data {
            everruns_core::EventData::LlmGeneration(data) => data.metadata.request_options.as_ref(),
            _ => None,
        })
        .cloned()
        .expect("fallback generation should record request options");
    assert_eq!(
        options.provider_options["anthropic"]["fallback_reason"],
        "configured_output_budget"
    );
}
