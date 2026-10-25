//! Live conformance of the durable OpenAI Agents API backend (EVE-1123)
//! against api.openai.com. The fake-API suite is in
//! `crates/everruns/tests/host/openai_agents_api.rs`.

use crate::agents_api_support::*;

// ---------------------------------------------------------------------------
// Live conformance
// ---------------------------------------------------------------------------

/// Answers the client function with fixed data and records it like Act.
struct LiveExecutor {
    ledger: Arc<TestLedger>,
    calls: AtomicUsize,
}

#[async_trait]
impl AgentsApiFunctionExecutor for LiveExecutor {
    async fn execute(&self, calls: &[ToolCall]) -> Result<FunctionBatch, AgentsApiError> {
        let output = r#"{"customer_id":"123","name":"Ada Lovelace"}"#.to_string();
        for call in calls {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(call.name, "lookup_customer");
            self.ledger
                .emit(EventRequest::new(
                    SessionId::from_seed(1),
                    EventContext::empty(),
                    ToolCompletedData::success(
                        call.id.clone(),
                        call.name.clone(),
                        vec![ContentPart::text(output.clone())],
                        None,
                    ),
                ))
                .await?;
        }
        Ok(FunctionBatch::Outcomes(
            calls
                .iter()
                .map(|_| FunctionOutcome::Done(Ok(output.clone())))
                .collect(),
        ))
    }
}

/// Credentialed conformance: one client function and one allowed MCP tool,
/// end to end through the durable driver against api.openai.com.
#[tokio::test]
async fn live_conformance_one_client_function_and_one_allowed_mcp_tool() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required");
    let mut req = request(
        1,
        "Look up customer 123, then search the OpenAI docs for 'Agents API sessions'. Answer in one sentence.",
    );
    req.session_id = SessionId::new();
    req.config = build_session_config(&agent(), "", None)
        .unwrap()
        .with_direct_mcp(
            "docs",
            "https://developers.openai.com/mcp",
            &["search_openai_docs"],
        )
        .unwrap();
    let ledger = Arc::new(TestLedger::default());
    let executor = Arc::new(LiveExecutor {
        ledger: ledger.clone(),
        calls: AtomicUsize::new(0),
    });
    let client = AgentsApiClient::new(api_key);
    let store = Arc::new(InMemoryAgentsApiStore::new());
    let driver = AgentsApiTurnDriver::new(
        client.clone(),
        store.clone(),
        ledger.clone(),
        executor.clone(),
    );
    let outcome = driver.run(&req).await;
    for (kind, data) in ledger.data() {
        eprintln!("{kind} {data}");
    }
    // Do not leave the provider session behind.
    if let Some(provider_session) = store
        .snapshot(req.session_id)
        .and_then(|checkpoint| checkpoint.provider_session_id)
    {
        client.delete_session(&provider_session).await.unwrap();
    }
    let outcome = outcome.expect("live turn reached a terminal state");
    let AgentsApiTurnOutcome::Completed { final_text, .. } = &outcome else {
        panic!("live turn did not complete: {outcome:?}");
    };
    assert!(!final_text.is_empty());
    assert_eq!(
        executor.calls.load(Ordering::SeqCst),
        1,
        "one client function call"
    );
    let mcp_results: Vec<_> = ledger
        .of_type("tool.hosted_call")
        .into_iter()
        .filter(|result| {
            result["tool_name"] == "mcp_docs__search_openai_docs" && result["status"] == "completed"
        })
        .collect();
    assert!(!mcp_results.is_empty(), "the allowed MCP tool ran");
    assert!(
        ledger
            .of_type("tool.hosted_call")
            .iter()
            .all(|result| result["tool_name"] != "mcp_docs__fetch_openai_doc"),
        "a tool outside the allowlist ran"
    );
    ledger.assert_each_record_once();
}

/// Credentialed seeding (EVE-1146): a new provider session created from the
/// Everruns record recalls the earlier exchange, a tool result included.
#[tokio::test]
async fn live_seeded_session_recalls_the_earlier_conversation() {
    use everruns_core::RuntimeMessage;
    use everruns_core::host::openai_agents_api::seed::seed_transcript;
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required");
    let call = ToolCall {
        id: "toolu_01SeedProbe".into(),
        name: "lookup_customer".into(),
        arguments: json!({"customer_id": "4711"}),
    };
    let input = RuntimeMessage::user(
        "Without calling any tool: what is our project's code word, and what is customer 4711's name? Answer in one sentence.",
    );
    let record = vec![
        RuntimeMessage::user("Our project's code word is PERIWINKLE-42. Look up customer 4711."),
        RuntimeMessage::assistant_with_tools("Looking it up.", vec![call]),
        RuntimeMessage::tool_result(
            "toolu_01SeedProbe",
            Some(json!({"name": "Ada Lovelace"})),
            None,
        ),
        RuntimeMessage::assistant("Customer 4711 is Ada Lovelace."),
        input.clone(),
    ];
    let mut req = request(1, input.text().unwrap());
    req.session_id = SessionId::new();
    req.seed = seed_transcript(&record, input.id);
    assert!(req.seed.is_some());
    let ledger = Arc::new(TestLedger::default());
    let executor = Arc::new(LiveExecutor {
        ledger: ledger.clone(),
        calls: AtomicUsize::new(0),
    });
    let client = AgentsApiClient::new(api_key);
    let store = Arc::new(InMemoryAgentsApiStore::new());
    let driver = AgentsApiTurnDriver::new(
        client.clone(),
        store.clone(),
        ledger.clone(),
        executor.clone(),
    );
    let outcome = driver.run(&req).await;
    for (kind, data) in ledger.data() {
        eprintln!("{kind} {data}");
    }
    // Do not leave the provider session behind.
    if let Some(provider_session) = store
        .snapshot(req.session_id)
        .and_then(|checkpoint| checkpoint.provider_session_id)
    {
        client.delete_session(&provider_session).await.unwrap();
    }
    let outcome = outcome.expect("live turn reached a terminal state");
    let AgentsApiTurnOutcome::Completed { final_text, .. } = &outcome else {
        panic!("live turn did not complete: {outcome:?}");
    };
    assert!(final_text.contains("PERIWINKLE-42"), "{final_text}");
    assert!(final_text.contains("Ada"), "{final_text}");
    // Whether the model looks the customer up again is its choice (it did on
    // 2026-10-02 despite the prompt); recall is the contract.
    eprintln!(
        "client function calls: {}",
        executor.calls.load(Ordering::SeqCst)
    );
    ledger.assert_each_record_once();
}

/// Production-policy conformance: no offered tools or provider environment,
/// but the managed harness may still run its own empty inventory helpers.
#[tokio::test]
async fn live_runtime_policy_accepts_only_empty_codex_inventory() {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("credentialed conformance requires OPENAI_API_KEY");
    let mut req = request(
        1,
        "Call both codex.list_mcp_resources and codex.list_mcp_resource_templates with empty arguments, then report whether each inventory is empty. Do not call any other tool or delegate work.",
    );
    req.session_id = SessionId::new();
    let runtime_agent = RuntimeAgent::new(
        "Report the two MCP inventories using only the named inventory helpers.",
        "gpt-6-astra",
    );
    req.config = build_session_config(&runtime_agent, "", None).unwrap();
    req.tools.clear();
    req.config.ensure_enforceable().unwrap();
    let ledger = Arc::new(TestLedger::default());
    let executor = Arc::new(LiveExecutor {
        ledger: ledger.clone(),
        calls: AtomicUsize::new(0),
    });
    let client = AgentsApiClient::new(api_key);
    let store = Arc::new(InMemoryAgentsApiStore::new());
    let driver = AgentsApiTurnDriver::new(
        client.clone(),
        store.clone(),
        ledger.clone(),
        executor.clone(),
    )
    .with_runtime_policy();
    let outcome = tokio::time::timeout(Duration::from_secs(240), driver.run(&req)).await;
    // Clean up on both a policy failure and a conformance assertion failure.
    if let Some(provider_session) = store
        .snapshot(req.session_id)
        .and_then(|checkpoint| checkpoint.provider_session_id)
    {
        client.delete_session(&provider_session).await.unwrap();
    }
    let outcome = outcome
        .expect("live turn finished within four minutes")
        .expect("live turn reached a terminal state");
    assert!(
        matches!(&outcome, AgentsApiTurnOutcome::Completed { final_text, .. } if !final_text.is_empty()),
        "{outcome:?}"
    );
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(ledger.of_type("tool.started").is_empty());
    assert!(ledger.of_type("tool.completed").is_empty());
    let hosted = ledger.of_type("tool.hosted_call");
    for name in [
        "mcp_codex__list_mcp_resources",
        "mcp_codex__list_mcp_resource_templates",
    ] {
        assert!(
            hosted
                .iter()
                .any(|event| event["tool_name"] == name && event["status"] == "completed"),
            "missing completed inventory: {name}"
        );
    }
    assert!(hosted.iter().all(|event| {
        matches!(
            event["tool_name"].as_str(),
            Some("mcp_codex__list_mcp_resources" | "mcp_codex__list_mcp_resource_templates")
        )
    }));
    assert!(matches!(
        outcome,
        AgentsApiTurnOutcome::Completed { tool_calls: 2, .. }
    ));
    ledger.assert_each_record_once();
}
