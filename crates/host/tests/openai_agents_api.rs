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
    let mcp = h.ledger.of_type("tool.hosted_call");
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
    replay_request(recording, &request(1, text)).await
}

async fn replay_request(
    recording: &str,
    req: &AgentsApiTurnRequest,
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
    let outcome = driver.run(req).await.unwrap();
    (replay, outcome, ledger, executor)
}

#[tokio::test]
async fn recorded_provider_mcp_inventory_is_hosted_work_without_act_records() {
    // Live 2026-10-02: tools: [], environment: none, yet the managed harness
    // called its own codex inventory tools. IDs are normalized in the fixture.
    let recording = include_str!("fixtures/agents_api_live_mcp_inventory.json");
    let events: Vec<Value> = serde_json::from_str(recording).unwrap();
    assert_eq!(events[0]["session"]["agent"]["tools"], json!([]));
    assert_eq!(events[0]["session"]["environment"]["type"], "none");
    let mut req = request(1, "List MCP resources.");
    req.config.agent.tools.clear();
    req.tools.clear();
    let (replay, outcome, ledger, executor) = replay_request(recording, &req).await;
    assert!(matches!(
        outcome,
        AgentsApiTurnOutcome::Completed { tool_calls: 2, .. }
    ));
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert!(replay.submitted.lock().unwrap().is_empty());
    assert!(ledger.of_type("tool.started").is_empty());
    assert!(ledger.of_type("tool.completed").is_empty());
    let hosted = ledger.of_type("tool.hosted_call");
    for name in ["list_mcp_resources", "list_mcp_resource_templates"] {
        let calls: Vec<_> = hosted
            .iter()
            .filter(|call| call["tool_name"] == format!("mcp_codex__{name}"))
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["status"], "in_progress");
        assert_eq!(calls[1]["status"], "completed");
        assert_eq!(calls[0]["call_id"], calls[1]["call_id"]);
    }
    let messages = ledger.of_type("output.message.completed");
    assert_eq!(
        messages.len(),
        2,
        "only provider commentary and final answer"
    );
    assert!(messages.iter().all(|message| {
        message["message"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|part| part["type"] != "tool_call")
    }));
    for event in ledger
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.event_type == "tool.hosted_call")
    {
        let metadata = event.metadata.as_ref().unwrap();
        assert_eq!(metadata["provider_session_id"], "sess_inventory_1");
        assert_eq!(metadata["provider_turn_id"], "turn_inventory_1");
        let data = serde_json::to_value(&event.data).unwrap();
        assert_eq!(metadata["provider_item_id"], data["call_id"]);
        assert_eq!(data["turn_id"], request(1, "").turn_id.to_string());
    }
    ledger.assert_each_record_once();
}

#[tokio::test]
async fn provider_mcp_payloads_do_not_enter_the_canonical_record() {
    let mut events: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/agents_api_live_round_trip.json")).unwrap();
    let secret = "private-mcp-payload".repeat(10_000);
    for event in &mut events {
        if event["item"]["type"] == "mcp_call" {
            let item = &mut event["item"];
            item["arguments"] = json!({"credential": secret});
            item["output"] = json!({"private_result": secret});
            if item["status"] == "failed" {
                // A provider may also put a failure in error even when its
                // status says completed; both are a failed hosted call.
                item["status"] = json!("completed");
                item["error"] = json!({"message": secret});
            }
        }
    }
    let (_, outcome, ledger, executor) = replay_run(
        &serde_json::to_string(&events).unwrap(),
        "Who is customer 123?",
    )
    .await;
    assert!(matches!(
        outcome,
        AgentsApiTurnOutcome::Completed { tool_calls: 3, .. }
    ));
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let hosted = ledger.of_type("tool.hosted_call");
    assert_eq!(hosted.len(), 4);
    assert!(hosted.iter().any(
        |call| call["tool_name"] == "mcp_docs__fetch_openai_doc" && call["status"] == "failed"
    ));
    for call in &hosted {
        assert_eq!(call["summary"], "Provider-run MCP call");
        assert!(call["summary"].as_str().unwrap().len() <= 256);
    }
    assert!(
        !serde_json::to_string(&ledger.data())
            .unwrap()
            .contains("private-mcp-payload")
    );
    assert_eq!(ledger.of_type("output.message.completed").len(), 3);
    assert_eq!(ledger.of_type("tool.completed").len(), 1);
    assert!(ledger.of_type("tool.started").is_empty());
}

#[tokio::test]
async fn restart_after_provider_mcp_completion_does_not_repeat_its_lifecycle_or_charge() {
    use everruns_core::agents_api_store::ItemState;
    let h = Harness::new().await;
    h.fake.with(|s| s.duplicate = true);
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.items
                .get("hosted-done:mcp_1")
                .is_some_and(|item| item.state == ItemState::Completed)
        })
    });
    let req = request(1, "Who is customer 123?");
    let (outcome, crashes) = h.run(&req).await;
    assert_eq!(crashes, 1);
    assert_completed(&outcome);
    assert_turn_record(&h.ledger, 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.ledger.of_type("tool.hosted_call").len(), 2);
    let generation = h.ledger.of_type("llm.generation");
    assert_eq!(generation.len(), 1);
    let components = generation[0]["metadata"]["cost_components"]
        .as_array()
        .unwrap();
    let mcp = components
        .iter()
        .find(|component| component["name"] == "mcp_call")
        .unwrap();
    assert_eq!(mcp["kind"], "hosted_tool");
    assert_eq!(mcp["quantity"], 1);
    assert!(mcp["cost_usd"].is_null(), "unknown price stays explicit");
    let before = h.ledger.data().len();
    let (replayed, _) = h.run(&req).await;
    assert_completed(&replayed);
    assert_eq!(h.ledger.data().len(), before);
}

#[tokio::test]
async fn legacy_provider_mcp_checkpoints_reconcile_without_double_counting() {
    use everruns_core::agents_api_store::{ItemCorrelation, ItemKind, ItemState};
    for completed in [false, true] {
        let h = Harness::new().await;
        let req = request(1, "Who is customer 123?");
        let (outcome, _) = h.run(&req).await;
        assert_completed(&outcome);
        let mut checkpoint = h.checkpoint();
        let turn = checkpoint.turn.as_mut().unwrap();
        turn.outcome = None;
        turn.items.remove("usage:turn_1");
        turn.items.remove("hosted:mcp_1");
        turn.items.remove("hosted-done:mcp_1");
        turn.items.insert(
            "mcp:mcp_1".to_string(),
            ItemCorrelation {
                kind: ItemKind::McpCall,
                local_id: MessageId::new().to_string(),
                state: ItemState::Completed,
            },
        );
        if completed {
            turn.items.insert(
                "mcp-result:mcp_1".to_string(),
                ItemCorrelation {
                    kind: ItemKind::McpCall,
                    local_id: "mcp_1".to_string(),
                    state: ItemState::Completed,
                },
            );
        }
        h.ledger.events.lock().unwrap().retain(|event| {
            !matches!(
                event.event_type.as_str(),
                "tool.hosted_call" | "llm.generation"
            )
        });
        // Replace only the persisted representation, as an older worker's
        // checkpoint would be encountered after the deployment.
        h.store.inner.expire(req.session_id);
        let lease = AgentsApiLease {
            org_id: req.org_id,
            session_id: req.session_id,
            owner: uuid::Uuid::new_v4(),
        };
        h.store.inner.acquire(lease).await.unwrap();
        h.store.inner.save(lease, &checkpoint).await.unwrap();
        h.store.inner.release(lease).await.unwrap();
        let (outcome, crashes) = h.run(&req).await;
        assert_eq!(crashes, 0);
        assert_completed(&outcome);
        assert_eq!(
            h.ledger.of_type("tool.hosted_call").len(),
            if completed { 0 } else { 2 }
        );
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
        let generation = h.ledger.of_type("llm.generation");
        let components = generation[0]["metadata"]["cost_components"]
            .as_array()
            .unwrap();
        let mcp = components
            .iter()
            .find(|component| component["name"] == "mcp_call")
            .unwrap();
        assert_eq!(mcp["quantity"], 1);
    }
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
    // Commentary, the client function call, and the final answer.
    assert_eq!(messages.len(), 3);
    assert_eq!(ledger.of_type("tool.completed").len(), 1);
    let results = ledger.of_type("tool.hosted_call");
    assert_eq!(results.len(), 4);
    assert!(results.iter().any(|r| {
        r["tool_name"] == "mcp_docs__search_openai_docs" && r["status"] == "completed"
    }));
    assert!(
        results
            .iter()
            .any(|r| { r["tool_name"] == "mcp_docs__fetch_openai_doc" && r["status"] == "failed" })
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

/// Production-policy conformance: no offered tools or provider environment,
/// but the managed harness may still run its own empty inventory helpers.
#[tokio::test]
#[ignore = "calls the paid OpenAI Agents API; needs OPENAI_API_KEY"]
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
