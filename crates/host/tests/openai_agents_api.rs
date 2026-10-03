//! Durable OpenAI Agents API backend (EVE-1123).
//!
//! A stateful fake of the Agents API drives the turn driver through every
//! restart boundary: session create, follow-up input, function result, stream
//! disconnect, and terminal recovery. Every fake stream ends after the events
//! available so far, so each run also exercises reconnect and reconciliation.
//! A crash is a dependency failing mid-run: the in-memory driver state is
//! dropped, the lease expires, and a new driver resumes from the checkpoint.
//!
//! Policy at the tool and output boundaries (EVE-1124) is in
//! `openai_agents_api_policy.rs`.
//!
//! The live conformance test talks to OpenAI only when `OPENAI_API_KEY` is set
//! and the test is run with `--ignored`.
#![cfg(feature = "openai-agents-api")]

mod agents_api_support;

use agents_api_support::*;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_turn_records_each_item_once_across_stream_disconnects() {
    let h = Harness::new().await;
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 0);
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.fake.with(|s| (s.creates, s.tool_result_posts)), (1, 1));
    // The session metadata lets a replacement worker adopt the session.
    let metadata = h.fake.with(|s| s.sessions[0].metadata.clone());
    assert_eq!(
        metadata["everruns_session_id"],
        SessionId::from_seed(1).to_string()
    );
    assert!(metadata["everruns_create_attempt"].is_string());
    let checkpoint = h.checkpoint();
    assert_eq!(checkpoint.provider_session_id.as_deref(), Some("sess_1"));
    let turn = checkpoint.turn.unwrap();
    assert_eq!(turn.provider_turn_id.as_deref(), Some("turn_1"));
    assert!(turn.last_event_id.is_some(), "stream cursor is persisted");
    assert!(matches!(
        turn.tool_results["call_1"].state,
        ToolResultState::Submitted { success: true, .. }
    ));
    let mcp = h.ledger.of_type("tool.completed");
    assert!(
        mcp.iter()
            .any(|r| r["tool_name"] == "mcp_docs__search_openai_docs")
    );
}

#[tokio::test]
async fn duplicate_provider_events_cannot_duplicate_messages_or_tool_effects() {
    let h = Harness::new().await;
    h.fake.with(|s| s.duplicate = true);
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 1);
}

#[tokio::test]
async fn missing_provider_events_are_recovered_from_saved_items() {
    let h = Harness::new().await;
    // No item completions and no required-action events ever reach the stream.
    h.fake.with(|s| {
        s.drop = vec![
            "agent.session.turn.item.done",
            "agent.session.requires_action",
            "agent.session.turn.completed",
        ]
    });
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn restart_after_uncertain_create_adopts_the_session_instead_of_creating_another() {
    let h = Harness::new().await;
    // The worker dies after the provider created the session, before saving its id.
    h.store.crash_when(|cp| cp.provider_session_id.is_some());
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_eq!(
        h.fake.with(|s| s.creates),
        1,
        "the uncertain create was adopted"
    );
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn restart_after_input_send_does_not_start_a_second_provider_turn() {
    let h = Harness::new().await;
    let (first, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&first);
    // Second Everruns turn on the same provider session: the worker dies after
    // the input reached the provider, before recording that it did.
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.turn_id == TurnId::from_seed(102)
                && turn
                    .input
                    .as_ref()
                    .is_some_and(|input| input.state == OutboxState::Delivered)
        })
    });
    let (second, crashes) = h.run(&request(2, "And again?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&second);
    let (creates, turns, input_posts) = h
        .fake
        .with(|s| (s.creates, s.sessions[0].turns.len(), s.input_posts));
    assert_eq!(creates, 1);
    assert_eq!(
        input_posts, 2,
        "the input was retried with its idempotency key"
    );
    assert_eq!(turns, 2, "the retry did not start another provider turn");
    assert_turn_record(&h.ledger, 2);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn restart_after_tool_execution_reuses_the_recorded_result() {
    let h = Harness::new().await;
    h.executor.crash_after_record.store(true, Ordering::SeqCst);
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_eq!(
        h.executor.calls.load(Ordering::SeqCst),
        1,
        "the tool ran once; recovery reused its recorded result"
    );
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 1);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn restart_after_tool_result_submission_does_not_resubmit_a_second_result() {
    let h = Harness::new().await;
    // Dies after the provider accepted the result, before recording it.
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.tool_results
                .values()
                .any(|entry| matches!(entry.state, ToolResultState::Submitted { .. }))
        })
    });
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    // Recovery saw the provider's saved output item, so it never resubmitted.
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 1);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn restart_while_recording_a_message_checks_the_log_before_emitting_again() {
    let h = Harness::new().await;
    // Dies after emitting the final answer, before marking it completed.
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.items.get("msg:msg_final_1").is_some_and(|item| {
                item.state == everruns_core::agents_api_store::ItemState::Completed
            })
        })
    });
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn terminal_recovery_returns_the_saved_outcome_without_provider_calls() {
    let h = Harness::new().await;
    // Dies after the provider turn completed, before the outcome was saved.
    h.store
        .crash_when(|cp| cp.turn.as_ref().is_some_and(|turn| turn.outcome.is_some()));
    let request = request(1, "Who is customer 123?");
    let (outcome, crashes) = h.run(&request).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);

    // A replayed activity for the finished turn makes no provider call.
    let before = h.fake.with(|s| s.requests);
    let (replayed, _) = h.run(&request).await;
    assert_completed(&replayed);
    assert_eq!(h.fake.with(|s| s.requests), before);
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn failed_provider_turn_reports_the_cause() {
    let h = Harness::new().await;
    h.fake.with(|s| s.fail_turns = true);
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    match outcome {
        AgentsApiTurnOutcome::Failed { code, message, .. } => {
            assert_eq!(code.as_deref(), Some("usage_limit_exceeded"));
            assert!(message.contains("billing limit"));
        }
        other => panic!("expected a failed turn, got {other:?}"),
    }
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_competing_worker_cannot_drive_a_leased_session() {
    let h = Harness::new().await;
    let lease = AgentsApiLease {
        org_id: 1,
        session_id: SessionId::from_seed(1),
        owner: uuid::Uuid::new_v4(),
    };
    h.store.acquire(lease).await.unwrap();
    let error = h.driver().run(&request(1, "hi")).await.unwrap_err();
    assert!(matches!(error, AgentsApiError::Store(_)), "{error}");
    assert_eq!(
        h.fake.with(|s| s.requests),
        0,
        "no provider call without the lease"
    );
}

#[tokio::test]
async fn transient_http_errors_surface_status_and_body() {
    // Permanent failures (401, 403, 404, a retired model) end the turn with a
    // stable code instead; see openai_agents_api_lifecycle.rs.
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(503).set_body_string(r#"{"error":{"code":"overloaded"}}"#),
        )
        .mount(&server)
        .await;
    let ledger = Arc::new(TestLedger::default());
    let driver = AgentsApiTurnDriver::new(
        AgentsApiClient::new("test-key").with_base_url(server.uri()),
        Arc::new(InMemoryAgentsApiStore::new()),
        ledger.clone(),
        TestExecutor::new(ledger),
    );
    let error = driver.run(&request(1, "hi")).await.unwrap_err();
    assert!(
        matches!(error, AgentsApiError::Api { status: 503, ref body } if body.contains("overloaded")),
        "{error}"
    );
    let auth = server.received_requests().await.unwrap()[0].clone();
    assert_eq!(
        auth.headers.get("authorization").unwrap(),
        "Bearer test-key"
    );
    assert_eq!(auth.headers.get("openai-beta").unwrap(), "agents=v1");
}

// ---------------------------------------------------------------------------
// Recorded live traffic
// ---------------------------------------------------------------------------

/// Serves a stream recorded from the live API: the create stream up to the
/// first required action, the rest on the next event stream, and saved items
/// and the turn resource derived from the recording.
#[derive(Clone)]
struct Replay {
    first: Vec<Value>,
    rest: Arc<Mutex<Option<Vec<Value>>>>,
    items: Vec<Value>,
    turn: Value,
    submitted: Arc<Mutex<Vec<Value>>>,
}

impl Replay {
    fn new(recording: &str) -> Self {
        let events: Vec<Value> = serde_json::from_str(recording).unwrap();
        let split = events
            .iter()
            .position(|e| e["type"] == "agent.session.requires_action")
            .map_or(events.len(), |i| i + 1);
        let mut items: Vec<Value> = Vec::new();
        for event in &events {
            if let Some(item) = event.get("item") {
                match items.iter_mut().find(|known| known["id"] == item["id"]) {
                    Some(known) => *known = item.clone(),
                    None => items.push(item.clone()),
                }
            }
        }
        let turn = events
            .iter()
            .rev()
            .find(|e| {
                e["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("agent.session.turn."))
                    && e["turn"]["status"].is_string()
            })
            .map(|e| e["turn"].clone())
            .unwrap();
        Self {
            first: events[..split].to_vec(),
            rest: Arc::new(Mutex::new(Some(events[split..].to_vec()))),
            items,
            turn,
            submitted: Arc::default(),
        }
    }
}

impl Respond for Replay {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let path = request.url.path().to_string();
        match (request.method.as_str(), path.as_str()) {
            ("POST", "/agents/sessions") => sse(&self.first),
            ("POST", p) if p.ends_with("/events") => {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                self.submitted.lock().unwrap().push(body);
                ResponseTemplate::new(202)
            }
            ("GET", p) if p.ends_with("/events") => {
                sse(&self.rest.lock().unwrap().take().unwrap_or_default())
            }
            ("GET", p) if p.ends_with("/items") => ResponseTemplate::new(200)
                .set_body_json(json!({"data": self.items, "has_more": false})),
            ("GET", p) if p.contains("/turns/") => {
                ResponseTemplate::new(200).set_body_json(self.turn.clone())
            }
            ("GET", p) if p.ends_with("/turns") => ResponseTemplate::new(200)
                .set_body_json(json!({"data": [self.turn], "has_more": false})),
            ("GET", _) => ResponseTemplate::new(200).set_body_json(
                json!({"id": "sess_replay", "status": "idle", "required_actions": []}),
            ),
            _ => ResponseTemplate::new(404),
        }
    }
}

async fn replay_run(
    recording: &str,
    text: &str,
) -> (
    Replay,
    AgentsApiTurnOutcome,
    Arc<TestLedger>,
    Arc<TestExecutor>,
) {
    let replay = Replay::new(recording);
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(replay.clone())
        .mount(&server)
        .await;
    let ledger = Arc::new(TestLedger::default());
    let executor = TestExecutor::new(ledger.clone());
    let driver = AgentsApiTurnDriver::new(
        AgentsApiClient::new("test-key").with_base_url(server.uri()),
        Arc::new(InMemoryAgentsApiStore::new()),
        ledger.clone(),
        executor.clone(),
    )
    .with_reconnect_policy(4, Duration::from_millis(1))
    .with_usage_poll(2, Duration::from_millis(1));
    let outcome = driver.run(&request(1, text)).await.unwrap();
    (replay, outcome, ledger, executor)
}

#[tokio::test]
async fn recorded_live_round_trip_projects_one_function_and_two_mcp_calls() {
    // Recorded from api.openai.com on 2026-09-30 (MCP outputs trimmed).
    let (replay, outcome, ledger, executor) = replay_run(
        include_str!("fixtures/agents_api_live_round_trip.json"),
        "Who is customer 123?",
    )
    .await;
    let AgentsApiTurnOutcome::Completed {
        final_text,
        usage,
        tool_calls,
        ..
    } = outcome
    else {
        panic!("expected completion, got {outcome:?}");
    };
    assert!(
        final_text.starts_with("Customer 123 is Ada Lovelace"),
        "{final_text}"
    );
    assert!(
        usage.is_none(),
        "usage was still null when the turn completed"
    );
    assert_eq!(tool_calls, 3);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let submitted = replay.submitted.lock().unwrap().clone();
    assert_eq!(submitted.len(), 1);
    let result = &submitted[0]["events"][0];
    assert_eq!(result["type"], "agent.session.input.tool_result");
    assert_eq!(
        result["call_id"],
        "exec_63e3dc63df0ba2a4da7f253c9825b12d4ea2deca00f59aab39"
    );
    assert_eq!(result["success"], true);

    ledger.assert_each_record_once();
    let messages = ledger.of_type("output.message.completed");
    let phases: Vec<_> = messages
        .iter()
        .map(|m| m["message"]["phase"].clone())
        .collect();
    assert_eq!(phases.first(), Some(&json!("commentary")));
    assert_eq!(phases.last(), Some(&json!("final_answer")));
    // Commentary, the function call, two MCP calls, and the final answer.
    assert_eq!(messages.len(), 5);
    let results = ledger.of_type("tool.completed");
    assert_eq!(results.len(), 3);
    assert!(
        results.iter().any(|r| {
            r["tool_name"] == "mcp_docs__search_openai_docs" && r["status"] == "success"
        })
    );
    assert!(
        results
            .iter()
            .any(|r| { r["tool_name"] == "mcp_docs__fetch_openai_doc" && r["status"] == "error" })
    );
}

#[tokio::test]
async fn recorded_live_failed_turn_reports_the_billing_cause() {
    // Recorded from api.openai.com on 2026-09-30, when the org had no credits.
    let (_, outcome, ledger, executor) = replay_run(
        include_str!("fixtures/agents_api_live_failed_turn.json"),
        "hi",
    )
    .await;
    assert!(
        matches!(&outcome, AgentsApiTurnOutcome::Failed { code: Some(code), .. } if code == "usage_limit_exceeded"),
        "{outcome:?}"
    );
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(ledger.of_type("output.message.completed").is_empty());
}

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
///
/// `doppler run -- cargo test -p everruns-host --features openai-agents-api \
///   --test openai_agents_api -- --ignored live_`
#[tokio::test]
#[ignore = "calls the paid OpenAI Agents API; needs OPENAI_API_KEY"]
async fn live_conformance_one_client_function_and_one_allowed_mcp_tool() {
    let Ok(api_key) = std::env::var("OPENAI_API_KEY") else {
        eprintln!("SKIP: OPENAI_API_KEY is not set");
        return;
    };
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
        .of_type("tool.completed")
        .into_iter()
        .filter(|result| result["tool_name"] == "mcp_docs__search_openai_docs")
        .collect();
    assert!(!mcp_results.is_empty(), "the allowed MCP tool ran");
    assert!(
        ledger
            .of_type("tool.completed")
            .iter()
            .all(|result| result["tool_name"] != "mcp_docs__fetch_openai_doc"),
        "a tool outside the allowlist ran"
    );
    ledger.assert_each_record_once();
}

/// Credentialed seeding (EVE-1146): a new provider session created from the
/// Everruns record recalls the earlier exchange, a tool result included.
///
/// `doppler run -- cargo test -p everruns-host --features openai-agents-api \
///   --test openai_agents_api -- --ignored live_`
#[tokio::test]
#[ignore = "calls the paid OpenAI Agents API; needs OPENAI_API_KEY"]
async fn live_seeded_session_recalls_the_earlier_conversation() {
    use everruns_core::RuntimeMessage;
    use everruns_host::openai_agents_api::seed::seed_transcript;
    let Ok(api_key) = std::env::var("OPENAI_API_KEY") else {
        eprintln!("SKIP: OPENAI_API_KEY is not set");
        return;
    };
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
