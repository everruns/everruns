use super::*;

#[derive(Clone, Debug)]
struct ProviderManagedCheckpointDriver {
    calls: Arc<Mutex<Vec<CapturedLlmCall>>>,
    fail_stream: Arc<AtomicBool>,
    exhaust_stream: Arc<AtomicBool>,
}

struct FailingProviderInstallStore {
    install_attempts: Arc<AtomicUsize>,
}

#[async_trait]
impl everruns_core::CompactionCheckpointStore for FailingProviderInstallStore {
    async fn get_latest(
        &self,
        _session_id: SessionId,
        _provider_type: &str,
        _model: &str,
    ) -> everruns_provider::error::Result<Option<everruns_core::CompactionCheckpoint>> {
        Ok(None)
    }

    async fn install(
        &self,
        _checkpoint: everruns_core::CompactionCheckpoint,
    ) -> everruns_provider::error::Result<bool> {
        self.install_attempts.fetch_add(1, Ordering::SeqCst);
        Err(everruns_provider::error::AgentLoopError::store(
            "checkpoint payload rejected",
        ))
    }

    async fn get_proactive_attempt(
        &self,
        _session_id: SessionId,
        _provider_type: &str,
        _model: &str,
    ) -> everruns_provider::error::Result<Option<everruns_core::ProactiveCompactionAttempt>> {
        Ok(None)
    }

    async fn record_proactive_attempt(
        &self,
        _session_id: SessionId,
        _provider_type: &str,
        _model: &str,
        _attempt: everruns_core::ProactiveCompactionAttempt,
    ) -> everruns_provider::error::Result<()> {
        Ok(())
    }
}

struct ReplayProjectionCapability;

impl everruns_core::Capability for ReplayProjectionCapability {
    fn id(&self) -> &str {
        "replay_projection_test"
    }

    fn name(&self) -> &str {
        "Replay projection test"
    }

    fn description(&self) -> &str {
        "Transforms public output while provider replay state remains internal."
    }

    fn filter_response_text(&self, text: String, _config: &serde_json::Value) -> String {
        format!("filtered: {text}")
    }

    fn finalized_tool_calls_hook(
        &self,
        _config: &serde_json::Value,
    ) -> Option<Arc<dyn everruns_core::finalized_tool_calls::FinalizedToolCallsHook>> {
        Some(Arc::new(NormalizeReplayToolCall))
    }
}

struct NormalizeReplayToolCall;

#[async_trait]
impl everruns_core::finalized_tool_calls::FinalizedToolCallsHook for NormalizeReplayToolCall {
    async fn apply(
        &self,
        _context: &everruns_core::finalized_tool_calls::FinalizedToolCallsContext<'_>,
        calls: &mut [ToolCall],
    ) {
        for call in calls {
            call.arguments = json!({"normalized":true});
        }
    }
}

#[derive(Clone, Debug)]
struct ReplayProjectionDriver;

#[async_trait]
impl everruns_provider::ChatDriver for ReplayProjectionDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        _messages: Vec<everruns_provider::Message>,
        config: &everruns_provider::LlmCallConfig,
    ) -> everruns_provider::Result<everruns_provider::LlmResponseStream> {
        let candidate = everruns_provider::driver_registry::ProviderCheckpointCandidate {
            format_version: everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
            context: everruns_provider::ProviderOpaqueContext::AnthropicMessagesPrefix {
                messages_json: serde_json::to_string(&json!([
                    {"role":"user","content":[{"type":"text","text":"request"}]},
                    {"role":"assistant","content":[
                        {"type":"compaction","content":"summary","encrypted_content":"cipher"},
                        {"type":"text","text":"provider text"},
                        {"type":"tool_use","id":"call_1","name":"lookup","input":{"raw":true}}
                    ]}
                ]))
                .unwrap(),
            },
        };
        let mut metadata = LlmCompletionMetadata::default();
        metadata.provider_opaque_content = Some(everruns_provider::ProviderOpaqueContent::new(
            "anthropic",
            json!([
                {"type":"text","text":"provider text"},
                {"type":"tool_use","id":"call_1","name":"lookup","input":{"raw":true}}
            ]),
        ));
        metadata.provider_checkpoint_candidate = Some(candidate);
        metadata.model = Some(config.model.clone());
        Ok(Box::pin(stream::iter(vec![
            Ok(everruns_provider::LlmStreamEvent::ProviderCompactionStarted),
            Ok(everruns_provider::LlmStreamEvent::TextDelta(
                "provider text".to_string(),
            )),
            Ok(everruns_provider::LlmStreamEvent::ToolCalls(vec![
                ToolCall {
                    id: "call_1".to_string(),
                    name: "lookup".to_string(),
                    arguments: json!({"raw":true}),
                },
            ])),
            Ok(everruns_provider::LlmStreamEvent::Done(Box::new(metadata))),
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
}

#[tokio::test]
async fn response_and_tool_projections_preserve_native_replay_state() {
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
    let provider_type = DriverId::external("replay-projection-test");
    let model = "eligible-model";
    set_default_test_model(&provider_store, provider_type.clone(), model, None).await;
    let mut session = session_store
        .get_session(session_id.into())
        .await
        .unwrap()
        .unwrap();
    session.capabilities = vec![
        AgentCapabilityConfig::with_config(
            INFINITY_CONTEXT_CAPABILITY_ID,
            json!({"context_budget_tokens":120_000}),
        ),
        AgentCapabilityConfig::new("replay_projection_test"),
    ];
    session_store.add_session(session).await;
    message_retriever
        .seed(session_id.into(), vec![RuntimeMessage::user("request")])
        .await;

    let mut drivers = DriverRegistry::new();
    drivers.register_external(provider_type.as_str(), |_| Box::new(ReplayProjectionDriver));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    capabilities.register(ReplayProjectionCapability);
    let events = InMemoryEventEmitter::new();
    let checkpoint_store = Arc::new(everruns_host::InMemoryCompactionCheckpointStore::default());
    let result = reason_atom_with_stores(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capabilities,
        drivers,
        events.clone(),
    )
    .with_compaction_checkpoint_store(checkpoint_store.clone())
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

    assert_eq!(result.text, "filtered: provider text");
    assert_eq!(result.tool_calls[0].arguments, json!({"normalized":true}));
    let completed = events
        .events()
        .await
        .into_iter()
        .find_map(|event| match event.data {
            everruns_core::EventData::OutputMessageCompleted(data) => Some(data),
            _ => None,
        })
        .expect("completed output should be durable");
    assert!(completed.message.content.iter().any(|part| {
        matches!(
            part,
            everruns_core::ContentPart::ProviderOpaque(content)
                if content.content[0]["text"] == "provider text"
                    && content.content[1]["input"] == json!({"raw":true})
        )
    }));
    let checkpoint = checkpoint_store
        .get_latest_format(
            session_id.into(),
            provider_type.as_str(),
            model,
            everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
        )
        .await
        .unwrap()
        .expect("projection changes must not discard the native checkpoint");
    let everruns_core::CompactionCheckpointPayload::ProviderOpaque {
        context: everruns_provider::ProviderOpaqueContext::AnthropicMessagesPrefix { messages_json },
    } = checkpoint.payload
    else {
        panic!("expected Anthropic checkpoint");
    };
    assert!(messages_json.contains("\"raw\":true"));
    assert!(!messages_json.contains("\"normalized\":true"));
}

#[async_trait]
impl everruns_provider::driver_registry::ChatDriver for ProviderManagedCheckpointDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
        messages: Vec<everruns_provider::driver_registry::Message>,
        config: &everruns_provider::driver_registry::LlmCallConfig,
    ) -> everruns_provider::error::Result<everruns_provider::driver_registry::LlmResponseStream>
    {
        self.calls.lock().await.push((messages, config.clone()));
        if self.fail_stream.load(Ordering::SeqCst) {
            return Ok(Box::pin(stream::iter(vec![
                Ok(everruns_provider::driver_registry::LlmStreamEvent::ProviderCompactionStarted),
                Ok(
                    everruns_provider::driver_registry::LlmStreamEvent::TextDelta(
                        "partial output after secret compaction content".to_string(),
                    ),
                ),
                Ok(everruns_provider::driver_registry::LlmStreamEvent::Error(
                    everruns_provider::driver_registry::LlmStreamError::new(
                        "invalid compaction block with secret content",
                    ),
                )),
            ])));
        }
        if self.exhaust_stream.load(Ordering::SeqCst) {
            return Ok(Box::pin(stream::iter(vec![
                Ok(everruns_provider::driver_registry::LlmStreamEvent::ProviderCompactionStarted),
                Ok(
                    everruns_provider::driver_registry::LlmStreamEvent::TextDelta(
                        "incomplete provider-managed output".to_string(),
                    ),
                ),
            ])));
        }
        let prefix = config
            .provider_opaque_context
            .as_ref()
            .and_then(|context| {
                match context {
                everruns_provider::driver_registry::ProviderOpaqueContext::AnthropicMessagesPrefix {
                    messages_json,
                } => Some(messages_json.clone()),
                _ => None,
            }
            })
            .unwrap_or_else(|| {
                serde_json::to_string(&json!([
                    {
                        "role":"user",
                        "content":[{"type":"text","text":"first request"}]
                    },
                    {
                        "role":"assistant",
                        "content":[{
                            "type":"compaction",
                            "content":"durable summary",
                            "encrypted_content":"encrypted-summary"
                        }]
                    }
                ]))
                .unwrap()
            });
        Ok(Box::pin(stream::iter(vec![
            Ok(everruns_provider::driver_registry::LlmStreamEvent::ProviderCompactionStarted),
            Ok(
                everruns_provider::driver_registry::LlmStreamEvent::TextDelta(
                    "provider-managed response".to_string(),
                ),
            ),
            Ok(everruns_provider::driver_registry::LlmStreamEvent::Done(
                Box::new({
                    let mut metadata = LlmCompletionMetadata::default();
                    metadata.model = Some(config.model.clone());
                    metadata.finish_reason = Some("stop".to_string());
                    metadata.prompt_tokens = Some(61_000);
                    metadata.completion_tokens = Some(120);
                    metadata.cache_read_tokens = Some(40_000);
                    metadata.cache_creation_tokens = Some(2_000);
                    metadata.provider_checkpoint_candidate =
                        Some(everruns_provider::driver_registry::ProviderCheckpointCandidate {
                            format_version:
                                everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
                            context: everruns_provider::driver_registry::ProviderOpaqueContext::AnthropicMessagesPrefix {
                                messages_json: prefix,
                            },
                        });
                    metadata
                }),
            )),
        ])))
    }

    fn provider_managed_reduction_option(
        &self,
        _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
        _model: &str,
        budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        Some((
            "anthropic/server_compaction".to_string(),
            json!({"trigger_tokens":budget_tokens}),
        ))
    }

    fn validate_provider_opaque_context(
        &self,
        context: &everruns_provider::driver_registry::ProviderOpaqueContext,
    ) -> bool {
        match context {
            everruns_provider::driver_registry::ProviderOpaqueContext::AnthropicMessagesPrefix {
                messages_json,
            } => serde_json::from_str::<Vec<serde_json::Value>>(messages_json).is_ok(),
            _ => true,
        }
    }
}

#[tokio::test]
async fn provider_managed_checkpoint_installs_after_completion_and_restores_on_restart() {
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
    let model = "eligible-model";
    set_default_test_model(
        &provider_store,
        provider_type.clone(),
        model,
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
        json!({"context_budget_tokens":120_000}),
    )];
    session_store.add_session(session).await;
    message_retriever
        .seed(
            session_id.into(),
            vec![RuntimeMessage::user("first request")],
        )
        .await;

    let calls = Arc::new(Mutex::new(Vec::new()));
    let fail_stream = Arc::new(AtomicBool::new(false));
    let exhaust_stream = Arc::new(AtomicBool::new(false));
    let driver = ProviderManagedCheckpointDriver {
        calls: calls.clone(),
        fail_stream: fail_stream.clone(),
        exhaust_stream: exhaust_stream.clone(),
    };
    let mut drivers = DriverRegistry::new();
    drivers.register(provider_type.clone(), move |_| Box::new(driver.clone()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    let checkpoint_store = Arc::new(everruns_host::InMemoryCompactionCheckpointStore::default());
    let execute = |event_emitter| {
        reason_atom_with_stores(
            harness_store.clone(),
            agent_store.clone(),
            session_store.clone(),
            message_retriever.clone(),
            provider_store.clone(),
            capabilities.clone(),
            drivers.clone(),
            event_emitter,
        )
        .with_compaction_checkpoint_store(checkpoint_store.clone())
    };

    let first_events = InMemoryEventEmitter::new();
    let first = execute(first_events.clone())
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
        .expect("completed output should install its checkpoint");
    assert!(first.success);
    let events = first_events.events().await;
    let compacting_index = events
        .iter()
        .position(|event| matches!(event.data, everruns_core::EventData::ContextCompacting(_)))
        .expect("provider compaction block should start the lifecycle");
    let output_index = events
        .iter()
        .position(|event| {
            matches!(
                event.data,
                everruns_core::EventData::OutputMessageCompleted(_)
            )
        })
        .expect("completed output should be durable");
    let compacted_index = events
        .iter()
        .position(|event| matches!(event.data, everruns_core::EventData::ContextCompacted(_)))
        .expect("durable output should close the compaction lifecycle");
    assert!(compacting_index < output_index);
    assert!(output_index < compacted_index);
    let everruns_core::EventData::ContextCompacting(compacting) = &events[compacting_index].data
    else {
        unreachable!()
    };
    assert_eq!(compacting.tokens_before, Some(120_000));
    let everruns_core::EventData::ContextCompacted(compacted) = &events[compacted_index].data
    else {
        unreachable!()
    };
    assert!(compacted.checkpoint_id.is_some());
    assert!(compacted.checkpoint_bytes.is_some_and(|bytes| bytes > 0));
    assert_eq!(compacted.replay_source.as_deref(), Some("raw"));
    assert_eq!(compacted.tokens_before, Some(61_000));
    assert_eq!(compacted.tokens_after, Some(120));
    assert_eq!(compacted.cache_read_tokens, Some(40_000));
    assert_eq!(compacted.cache_creation_tokens, Some(2_000));
    let options = events
        .iter()
        .find_map(|event| match &event.data {
            everruns_core::EventData::LlmGeneration(data) => data.metadata.request_options.as_ref(),
            _ => None,
        })
        .expect("generation should record sanitized compaction options");
    assert_eq!(
        options.provider_options["anthropic"]["reduction_mode"],
        "server_compaction"
    );
    assert_eq!(
        options.provider_options["anthropic"]["replay_source"],
        "raw"
    );
    assert_eq!(
        options.provider_options["anthropic"]["compaction_block_observed"],
        true
    );
    let lifecycle_json =
        serde_json::to_string(&[&events[compacting_index], &events[compacted_index]]).unwrap();
    assert!(!lifecycle_json.contains("first request"));
    assert!(!lifecycle_json.contains("durable summary"));
    assert!(!lifecycle_json.contains("encrypted-summary"));
    let checkpoint = checkpoint_store
        .get_latest_format(
            session_id.into(),
            provider_type.as_str(),
            model,
            everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
        )
        .await
        .unwrap()
        .expect("format-2 checkpoint should be durable");
    assert!(checkpoint.source_sequence > 0);
    let checkpoint_source_sequence = checkpoint.source_sequence;
    assert!(matches!(
        checkpoint.payload,
        everruns_core::CompactionCheckpointPayload::ProviderOpaque {
            context:
                everruns_provider::driver_registry::ProviderOpaqueContext::AnthropicMessagesPrefix { .. }
        }
    ));
    let mut canonical_history = vec![RuntimeMessage::user("first request")];
    canonical_history.resize_with(checkpoint_source_sequence as usize, || {
        RuntimeMessage::assistant("persisted event before checkpoint boundary")
    });
    canonical_history.push(RuntimeMessage::user("second request"));

    message_retriever
        .seed(session_id.into(), canonical_history)
        .await;
    let second = execute(InMemoryEventEmitter::new())
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
        .expect("a fresh reason atom should restore the checkpoint");
    assert!(second.success);

    let captured_calls = calls.lock().await;
    assert_eq!(captured_calls.len(), 2);
    assert!(captured_calls[0].1.provider_opaque_context.is_none());
    let restored = captured_calls[1]
        .1
        .provider_opaque_context
        .as_ref()
        .expect("restart should restore provider-owned context");
    let everruns_provider::driver_registry::ProviderOpaqueContext::AnthropicMessagesPrefix {
        messages_json,
    } = restored
    else {
        panic!("expected Anthropic prefix");
    };
    assert!(messages_json.contains("encrypted-summary"));
    assert!(captured_calls[1].0.iter().any(|message| {
        matches!(
            &message.content,
            everruns_provider::driver_registry::MessageContent::Text(text)
                if text == "second request"
        )
    }));
    assert!(!captured_calls[1].0.iter().any(|message| {
        matches!(
            &message.content,
            everruns_provider::driver_registry::MessageContent::Text(text)
                if text == "first request"
        )
    }));
    drop(captured_calls);

    fail_stream.store(true, Ordering::SeqCst);
    let failed_events = InMemoryEventEmitter::new();
    let failed = execute(failed_events.clone())
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
        .expect("terminal native stream failure should be reported as a reason result");
    assert!(!failed.success);
    let events = failed_events.events().await;
    let compacting_index = events
        .iter()
        .position(|event| matches!(event.data, everruns_core::EventData::ContextCompacting(_)))
        .expect("compaction block should start the lifecycle");
    let failed_index = events
        .iter()
        .position(|event| {
            matches!(
                event.data,
                everruns_core::EventData::ContextCompactionFailed(_)
            )
        })
        .expect("terminal native stream error should fail the lifecycle");
    assert!(compacting_index < failed_index);
    assert!(
        !events
            .iter()
            .any(|event| { matches!(event.data, everruns_core::EventData::ContextCompacted(_)) })
    );
    let failed_json = serde_json::to_string(&events[failed_index]).unwrap();
    assert!(!failed_json.contains("secret"));
    let retained_checkpoint = checkpoint_store
        .get_latest_format(
            session_id.into(),
            provider_type.as_str(),
            model,
            everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained_checkpoint.id, checkpoint.id);
    assert_eq!(calls.lock().await.len(), 3);
    fail_stream.store(false, Ordering::SeqCst);
    exhaust_stream.store(true, Ordering::SeqCst);
    let exhausted_events = InMemoryEventEmitter::new();
    let exhausted = execute(exhausted_events.clone())
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
        .expect("incomplete native stream should be reported as a reason result");
    assert!(!exhausted.success);
    let events = exhausted_events.events().await;
    assert!(events.iter().any(|event| matches!(
        event.data,
        everruns_core::EventData::ContextCompactionFailed(_)
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.data, everruns_core::EventData::ContextCompacted(_)))
    );
    let completed_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.data,
                everruns_core::EventData::OutputMessageCompleted(_)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(completed_events.len(), 1);
    assert!(
        !serde_json::to_string(completed_events[0])
            .unwrap()
            .contains("incomplete provider-managed output")
    );
    let retained_checkpoint = checkpoint_store
        .get_latest_format(
            session_id.into(),
            provider_type.as_str(),
            model,
            everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained_checkpoint.id, checkpoint.id);
    exhaust_stream.store(false, Ordering::SeqCst);
    let install_attempts = Arc::new(AtomicUsize::new(0));
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
    .with_compaction_checkpoint_store(Arc::new(FailingProviderInstallStore {
        install_attempts: install_attempts.clone(),
    }));
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
        .expect("checkpoint installation failure must preserve completed output");
    assert!(result.success);
    assert_eq!(result.text, "provider-managed response");
    assert_eq!(install_attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn corrupt_provider_checkpoint_rebuilds_from_full_raw_history() {
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
    let provider_type = DriverId::external("provider-managed-corrupt-checkpoint-test");
    let model = "eligible-model";
    set_default_test_model(
        &provider_store,
        provider_type.clone(),
        model,
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
        json!({"context_budget_tokens":120_000}),
    )];
    session_store.add_session(session).await;
    message_retriever
        .seed(
            session_id.into(),
            vec![
                RuntimeMessage::user("first raw message"),
                RuntimeMessage::assistant("second raw message"),
                RuntimeMessage::user("third raw message"),
            ],
        )
        .await;

    let calls = Arc::new(Mutex::new(Vec::new()));
    let fail_stream = Arc::new(AtomicBool::new(false));
    let driver = ProviderManagedCheckpointDriver {
        calls: calls.clone(),
        fail_stream,
        exhaust_stream: Arc::new(AtomicBool::new(false)),
    };
    let mut drivers = DriverRegistry::new();
    drivers.register_external(provider_type.as_str(), move |_| Box::new(driver.clone()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    let checkpoint_store = Arc::new(everruns_host::InMemoryCompactionCheckpointStore::default());
    checkpoint_store
        .install(everruns_core::CompactionCheckpoint {
            id: Uuid::now_v7(),
            session_id: session_id.into(),
            source_sequence: 2,
            provider_type: provider_type.to_string(),
            model: model.to_string(),
            format_version: everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
            payload: everruns_core::CompactionCheckpointPayload::ProviderOpaque {
                context: everruns_provider::ProviderOpaqueContext::AnthropicMessagesPrefix {
                    messages_json: "{not-json".to_string(),
                },
            },
        })
        .await
        .unwrap();

    let result = reason_atom_with_stores(
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capabilities,
        drivers,
        InMemoryEventEmitter::new(),
    )
    .with_compaction_checkpoint_store(checkpoint_store)
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
    .expect("corrupt checkpoint should fall back to raw history");

    assert!(result.success);
    let calls = calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.provider_opaque_context.is_none());
    assert!(calls[0].0.iter().any(|message| {
        matches!(
            &message.content,
            everruns_provider::MessageContent::Text(text) if text == "first raw message"
        )
    }));
}

#[derive(Clone, Debug)]
struct DynamicContextFallbackDriver {
    calls: Arc<Mutex<Vec<CapturedLlmCall>>>,
}

#[async_trait]
impl everruns_provider::ChatDriver for DynamicContextFallbackDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        messages: Vec<everruns_provider::Message>,
        config: &everruns_provider::LlmCallConfig,
    ) -> everruns_provider::Result<everruns_provider::LlmResponseStream> {
        self.calls.lock().await.push((messages, config.clone()));
        Ok(Box::pin(stream::iter(vec![
            Ok(everruns_provider::LlmStreamEvent::TextDelta(
                "legacy context response".to_string(),
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
}

#[tokio::test]
async fn changing_agents_and_channel_context_bypass_a_native_checkpoint() {
    use everruns_builtins::{
        AGENT_INSTRUCTIONS_CAPABILITY_ID, AgentInstructionsCapability,
        CHANNEL_CONTEXT_CAPABILITY_ID, ChannelContextCapability, INFINITY_CONTEXT_CAPABILITY_ID,
        InfinityContextCapability,
    };
    use everruns_capability::CapabilityRef as AgentCapabilityConfig;
    use everruns_core::channel::{ChannelViewContext, ThreadContext, save_thread_context};
    use everruns_core::execution_loading::SessionStore;
    use everruns_core::message::ExternalActor;
    use everruns_core::session_file::InitialFile;
    use everruns_engine::ReasonAtom;
    use everruns_host::{
        InMemorySessionFileStore, InMemorySessionStorageStore, StoreTurnContextResolver,
    };

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
    let provider_type = DriverId::external("dynamic-context-checkpoint-test");
    let model = "eligible-model";
    set_default_test_model(&provider_store, provider_type.clone(), model, None).await;
    let mut session = session_store
        .get_session(session_id.into())
        .await
        .unwrap()
        .unwrap();
    session.capabilities = vec![
        AgentCapabilityConfig::with_config(
            INFINITY_CONTEXT_CAPABILITY_ID,
            json!({"context_budget_tokens":120_000}),
        ),
        AgentCapabilityConfig::new(AGENT_INSTRUCTIONS_CAPABILITY_ID),
        AgentCapabilityConfig::new(CHANNEL_CONTEXT_CAPABILITY_ID),
    ];
    session_store.add_session(session).await;
    message_retriever
        .seed(
            session_id.into(),
            vec![
                RuntimeMessage::user("first raw message"),
                RuntimeMessage::assistant("first raw response"),
                RuntimeMessage::user("latest raw message"),
            ],
        )
        .await;

    let file_store = InMemorySessionFileStore::new();
    let agents_file = |content: &str| InitialFile {
        path: "/AGENTS.md".to_string(),
        content: content.to_string(),
        encoding: "text".to_string(),
        is_readonly: false,
    };
    file_store
        .seed_initial_file(session_id.into(), &agents_file("INSTRUCTION-V1"))
        .await
        .unwrap();
    let storage = InMemorySessionStorageStore::new();
    let mut thread = ThreadContext::new("thread-1", "slack");
    thread.track_participant(&ExternalActor {
        actor_id: "U1".to_string(),
        actor_name: Some("Alice".to_string()),
        source: "slack".to_string(),
        metadata: None,
    });
    thread.set_current_view(ChannelViewContext {
        channel_id: Some("C1".to_string()),
        ..Default::default()
    });
    save_thread_context(&storage, session_id.into(), &thread)
        .await
        .unwrap();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let driver = DynamicContextFallbackDriver {
        calls: calls.clone(),
    };
    let mut drivers = DriverRegistry::new();
    drivers.register_external(provider_type.as_str(), move |_| Box::new(driver.clone()));
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(InfinityContextCapability);
    capabilities.register(AgentInstructionsCapability);
    capabilities.register(ChannelContextCapability);
    let checkpoint_store = Arc::new(everruns_host::InMemoryCompactionCheckpointStore::default());
    checkpoint_store
        .install(everruns_core::CompactionCheckpoint {
            id: Uuid::now_v7(),
            session_id: session_id.into(),
            source_sequence: 2,
            provider_type: provider_type.to_string(),
            model: model.to_string(),
            format_version: everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
            payload: everruns_core::CompactionCheckpointPayload::ProviderOpaque {
                context: everruns_provider::ProviderOpaqueContext::AnthropicMessagesPrefix {
                    messages_json: serde_json::to_string(&json!([
                        {"role":"user","content":[{"type":"text","text":"checkpoint prefix"}]}
                    ]))
                    .unwrap(),
                },
            },
        })
        .await
        .unwrap();
    let resolver = StoreTurnContextResolver::new(
        Arc::new(harness_store),
        Arc::new(agent_store),
        Arc::new(session_store),
        Arc::new(message_retriever.clone()),
        Arc::new(provider_store),
        capabilities.clone(),
        drivers,
    )
    .with_file_store(Arc::new(file_store.clone()))
    .with_session_storage(Arc::new(storage.clone()));
    let atom = ReasonAtom::new(
        resolver,
        message_retriever,
        capabilities,
        InMemoryEventEmitter::new(),
    )
    .with_compaction_checkpoint_store(checkpoint_store);
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
    file_store
        .seed_initial_file(session_id.into(), &agents_file("INSTRUCTION-V2"))
        .await
        .unwrap();
    let mut changed_thread = ThreadContext::new("thread-1", "slack");
    changed_thread.track_participant(&ExternalActor {
        actor_id: "U2".to_string(),
        actor_name: Some("Bob".to_string()),
        source: "slack".to_string(),
        metadata: None,
    });
    changed_thread.set_current_view(ChannelViewContext {
        channel_id: Some("C2".to_string()),
        ..Default::default()
    });
    save_thread_context(&storage, session_id.into(), &changed_thread)
        .await
        .unwrap();
    assert!(atom.execute(input()).await.unwrap().success);

    let calls = calls.lock().await;
    assert_eq!(calls.len(), 2);
    for (_, config) in calls.iter() {
        assert!(config.provider_opaque_context.is_none());
        assert!(
            !config
                .driver_options
                .contains_key("anthropic/server_compaction")
        );
        assert_eq!(
            config.driver_options["everruns/provider_managed_reduction_fallback"]["reason"],
            "dynamic_conversation_context"
        );
    }
    let request_text = |index: usize| {
        calls[index]
            .0
            .iter()
            .map(|message| message.content.to_text())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let context_messages = |index: usize, marker: &str| {
        calls[index]
            .0
            .iter()
            .filter(|message| message.content.to_text().contains(marker))
            .count()
    };
    let first = request_text(0);
    assert_eq!(context_messages(0, "INSTRUCTION-V1"), 1);
    assert!(first.contains("Alice"));
    assert!(first.contains("C1"));
    let second = request_text(1);
    assert_eq!(context_messages(1, "INSTRUCTION-V2"), 1);
    assert!(second.contains("Bob"));
    assert!(second.contains("C2"));
    for stale in ["INSTRUCTION-V1", "Alice", "C1", "checkpoint prefix"] {
        assert!(
            !second.contains(stale),
            "{stale} leaked into changed context"
        );
    }
}
