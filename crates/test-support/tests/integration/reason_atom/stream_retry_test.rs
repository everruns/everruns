use super::*;

struct LongThinkingDriver {
    idle_timeout: Option<std::time::Duration>,
}

#[async_trait]
impl everruns_contracts::driver_registry::ChatDriver for LongThinkingDriver {
    fn stream_stall_timeout(
        &self,
        _config: &everruns_contracts::LlmCallConfig,
    ) -> Option<std::time::Duration> {
        self.idle_timeout
    }

    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_contracts::ProviderEndpoint,
        _messages: Vec<everruns_contracts::Message>,
        _config: &everruns_contracts::LlmCallConfig,
    ) -> everruns_contracts::error::Result<everruns_contracts::LlmResponseStream> {
        use everruns_contracts::LlmStreamEvent;
        Ok(Box::pin(stream::unfold(0, |phase| async move {
            let event = match phase {
                0 => LlmStreamEvent::TextDelta("before ".into()),
                1 => {
                    tokio::time::sleep(std::time::Duration::from_secs(180)).await;
                    LlmStreamEvent::TextDelta("after".into())
                }
                2 => {
                    let mut metadata = LlmCompletionMetadata::default();
                    metadata.finish_reason = Some("stop".into());
                    LlmStreamEvent::Done(Box::new(metadata))
                }
                _ => return None,
            };
            Some((Ok(event), phase + 1))
        })))
    }
}

#[tokio::test(start_paused = true)]
async fn provider_idle_hint_allows_long_thinking_and_preserves_host_deadlines() {
    use std::time::Duration;
    for (hint, host_timeout, succeeds) in [
        (Some(Duration::from_secs(1_200)), None, true),
        (
            Some(Duration::from_secs(1_200)),
            Some(Duration::from_secs(60)),
            false,
        ),
        (None, None, false),
    ] {
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
                vec![RuntimeMessage::user("Think carefully")],
            )
            .await;
        let mut registry = DriverRegistry::new();
        registry.register(DriverId::LlmSim, move |_config| {
            everruns_contracts::Provider::new(
                "long-thinking",
                LongThinkingDriver { idle_timeout: hint },
            )
            .into_boxed_driver()
        });
        let emitter = InMemoryEventEmitter::new();
        let mut atom = reason_atom_with_stores(
            harness_store,
            agent_store,
            session_store,
            message_retriever,
            provider_store,
            CapabilityRegistry::new(),
            registry,
            emitter,
        )
        .with_provider_retry_config(everruns_contracts::LlmRetryConfig::no_retry());
        if let Some(timeout) = host_timeout {
            atom = atom.with_provider_stall_timeout(timeout);
        }
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
        assert_eq!(
            result.success, succeeds,
            "hint={hint:?}, host={host_timeout:?}, error={:?}",
            result.error
        );
        if succeeds {
            assert_eq!(result.text, "before after");
        } else {
            assert!(
                result
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("provider stream stall")
            );
        }
    }
}

/// First attempt streams part of an answer and then breaks; the retry streams
/// the whole answer.
#[derive(Clone, Debug)]
struct BrokenThenWholeDriver {
    attempts: Arc<AtomicUsize>,
}

#[async_trait]
impl everruns_contracts::driver_registry::ChatDriver for BrokenThenWholeDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_contracts::runtime_provider::ProviderEndpoint,
        _messages: Vec<everruns_contracts::driver_registry::Message>,
        _config: &everruns_contracts::driver_registry::LlmCallConfig,
    ) -> everruns_contracts::error::Result<everruns_contracts::driver_registry::LlmResponseStream>
    {
        use everruns_contracts::driver_registry::LlmStreamEvent;
        let events = if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            vec![
                Ok(LlmStreamEvent::TextDelta("Here are the".to_string())),
                Ok(LlmStreamEvent::Error(
                    "server_error: internal failure".into(),
                )),
            ]
        } else {
            vec![
                Ok(LlmStreamEvent::TextDelta("Here are the links.".to_string())),
                Ok(LlmStreamEvent::Done(Box::default())),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

/// A stream that breaks after answer text (and no tool call) is retried under
/// the same message id instead of keeping the cut-off text.
#[tokio::test(start_paused = true)]
async fn a_stream_cut_after_text_is_retried_under_the_same_message() {
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
            vec![RuntimeMessage::user("Give me links")],
        )
        .await;

    let attempts = Arc::new(AtomicUsize::new(0));
    let driver = BrokenThenWholeDriver {
        attempts: Arc::clone(&attempts),
    };
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
        .expect("the retry should answer");

    assert!(result.success);
    assert_eq!(result.text, "Here are the links.");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);

    // Both attempts stream under one message id. The retry announces itself
    // with a second `output.message.started` and restarts `accumulated`.
    let stream: Vec<_> = event_emitter
        .events()
        .await
        .into_iter()
        .filter_map(|event| match event.data {
            everruns_core::EventData::OutputMessageStarted(data) => {
                Some((data.message_id, "started".to_string()))
            }
            everruns_core::EventData::OutputMessageDelta(data) => {
                Some((data.message_id, data.accumulated))
            }
            _ => None,
        })
        .collect();
    let texts: Vec<_> = stream.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(
        texts,
        ["started", "Here are the", "started", "Here are the links."]
    );
    assert!(
        stream.iter().all(|(id, _)| *id == stream[0].0),
        "the retry keeps the message id"
    );
}
