use super::*;

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
