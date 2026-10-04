//! OpenAI Agents API backend: Everruns policy at the tool and output
//! boundaries (EVE-1124).
//!
//! The stateful fake from `agents_api_support` holds the provider's required
//! action open across an approval, a denial, an expiry, a client-side answer,
//! and a provider timeout; the remote loop stops on an output guardrail or a
//! budget; and MCP credentials stay out of everything sent to the provider.
#![cfg(feature = "openai-agents-api")]

use super::agents_api_support::*;

// ---------------------------------------------------------------------------
// Policy at the tool and output boundaries (EVE-1124)
// ---------------------------------------------------------------------------

const APPROVAL_REQUEST: &str = "tool_approval_call_1";

fn approval_park() -> Script {
    Script::Park(ParkReason::Approval {
        request_call_id: APPROVAL_REQUEST.to_string(),
    })
}

/// The answer surface completes the engine-authored request, as
/// `POST /v1/sessions/{id}/tool-approvals` or the deadline sweep does.
async fn answer_approval(ledger: &TestLedger, approved: bool, outcome: &str) {
    let summary = json!({"outcome": outcome, "approved": approved,
        "tool": "lookup_customer", "tool_call_id": "call_1"});
    ledger
        .emit(EventRequest::new(
            SessionId::from_seed(1),
            EventContext::empty(),
            ToolCompletedData::success(
                APPROVAL_REQUEST.to_string(),
                "approve_tool_call".to_string(),
                vec![ContentPart::text(summary.to_string())],
                None,
            ),
        ))
        .await
        .unwrap();
}

/// The same Everruns turn, resumed after its pause was answered.
fn resumed(request: &AgentsApiTurnRequest) -> AgentsApiTurnRequest {
    AgentsApiTurnRequest {
        iteration: request.iteration + 1,
        ..request.clone()
    }
}

fn parked_reason(h: &Harness) -> Option<ParkReason> {
    match &h.checkpoint().turn?.tool_results.get("call_1")?.state {
        ToolResultState::Parked { reason, .. } => Some(reason.clone()),
        _ => None,
    }
}

/// Park turn 1 on an approval and check that nothing reached the provider.
async fn park_on_approval(h: &Harness) -> AgentsApiTurnRequest {
    h.executor.then(approval_park());
    let request = request(1, "Who is customer 123?");
    let (outcome, _) = h.run(&request).await;
    assert!(
        matches!(outcome, AgentsApiTurnOutcome::Paused),
        "{outcome:?}"
    );
    assert_eq!(
        h.fake.with(|s| s.tool_result_posts),
        0,
        "nothing is submitted while a person decides"
    );
    assert_eq!(
        h.fake.with(|s| s.sessions[0].required_actions.len()),
        1,
        "the provider's required action stays open"
    );
    assert_eq!(
        h.executor.calls.load(Ordering::SeqCst),
        0,
        "the gated call did not run"
    );
    assert!(matches!(
        parked_reason(h),
        Some(ParkReason::Approval { .. })
    ));
    request
}

#[tokio::test]
async fn an_approval_holds_the_required_action_open_and_runs_the_call_once_approved() {
    let h = Harness::new().await;
    let request = park_on_approval(&h).await;

    // A replay of the activity that parked stays parked and runs nothing.
    let (replayed, _) = h.run(&request).await;
    assert!(matches!(replayed, AgentsApiTurnOutcome::Paused));
    assert_eq!(h.executor.batches.load(Ordering::SeqCst), 1);
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 0);

    answer_approval(&h.ledger, true, "approved_once").await;
    let (outcome, _) = h.run(&resumed(&request)).await;
    assert_completed_with(&outcome, 3);
    // The approved call ran once, as a fresh local attempt the gate checks again.
    assert_eq!(
        *h.executor.ran.lock().unwrap(),
        vec!["call_1-retry1".to_string()]
    );
    let submitted = h.fake.with(|s| s.tool_results.clone());
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["call_id"], "call_1");
    assert_eq!(submitted[0]["success"], true);
    h.ledger.assert_each_record_once();
    // The retry is a new assistant tool call, as when a native model retries.
    let retry_calls = h
        .ledger
        .of_type("output.message.completed")
        .into_iter()
        .filter(|m| m["message"]["content"][0]["id"] == "call_1-retry1")
        .count();
    assert_eq!(retry_calls, 1);
}

#[tokio::test]
async fn a_denied_approval_submits_a_failed_result_without_running_the_call() {
    let h = Harness::new().await;
    let request = park_on_approval(&h).await;
    answer_approval(&h.ledger, false, "rejected_once").await;
    let (outcome, _) = h.run(&resumed(&request)).await;
    assert!(
        matches!(outcome, AgentsApiTurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        h.executor.calls.load(Ordering::SeqCst),
        0,
        "a denial never runs the call"
    );
    assert_eq!(h.executor.batches.load(Ordering::SeqCst), 1);
    let submitted = h.fake.with(|s| s.tool_results.clone());
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["success"], false);
    assert!(
        submitted[0]["error"]
            .as_str()
            .unwrap()
            .contains("not approved (rejected_once)"),
        "{submitted:?}"
    );
    h.ledger.assert_each_record_once();
}

#[tokio::test]
async fn an_expired_approval_is_not_an_approval() {
    let h = Harness::new().await;
    let request = park_on_approval(&h).await;
    // The deadline sweep resolves the request as not approved.
    answer_approval(&h.ledger, false, "expired").await;
    let (_, _) = h.run(&resumed(&request)).await;
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    let submitted = h.fake.with(|s| s.tool_results.clone());
    assert!(submitted[0]["error"].as_str().unwrap().contains("expired"));
}

#[tokio::test]
async fn a_resume_without_a_recorded_decision_fails_closed() {
    let h = Harness::new().await;
    let request = park_on_approval(&h).await;
    let (_, _) = h.run(&resumed(&request)).await;
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    let submitted = h.fake.with(|s| s.tool_results.clone());
    assert_eq!(submitted[0]["success"], false);
}

#[tokio::test]
async fn a_provider_timeout_during_the_pause_fails_the_turn_canonically() {
    let h = Harness::new().await;
    let request = park_on_approval(&h).await;
    h.fake.with(|s| s.expire_turn(0));
    answer_approval(&h.ledger, true, "approved_once").await;
    let (outcome, _) = h.run(&resumed(&request)).await;
    match outcome {
        AgentsApiTurnOutcome::Failed {
            code,
            message,
            policy,
        } => {
            assert_eq!(code.as_deref(), Some(PARKED_CALL_EXPIRED));
            assert!(policy, "Everruns-authored, user-facing as is");
            assert!(message.contains("timed out"), "{message}");
        }
        other => panic!("expected a failed turn, got {other:?}"),
    }
    assert_eq!(
        h.executor.calls.load(Ordering::SeqCst),
        0,
        "nothing runs for a turn the provider abandoned"
    );
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 0);
}

#[tokio::test]
async fn a_crash_while_resuming_does_not_run_the_approved_call_twice() {
    let h = Harness::new().await;
    let request = park_on_approval(&h).await;
    answer_approval(&h.ledger, true, "approved_once").await;
    h.executor.crash_after_record.store(true, Ordering::SeqCst);
    let (outcome, crashes) = h.run(&resumed(&request)).await;
    assert_eq!(crashes, 1);
    assert_completed_with(&outcome, 3);
    assert_eq!(
        h.executor.calls.load(Ordering::SeqCst),
        1,
        "recovery reused the recorded result of the approved run"
    );
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 1);
    h.ledger.assert_each_record_once();
}

#[tokio::test]
async fn a_client_side_call_submits_the_clients_answer() {
    let h = Harness::new().await;
    h.executor.then(Script::Park(ParkReason::ClientResult));
    let request = request(1, "Who is customer 123?");
    let (outcome, _) = h.run(&request).await;
    assert!(matches!(outcome, AgentsApiTurnOutcome::Paused));
    // The client answers through the tool-results API.
    h.ledger
        .emit(EventRequest::new(
            SessionId::from_seed(1),
            EventContext::empty(),
            ToolCompletedData::success(
                "call_1".to_string(),
                "lookup_customer".to_string(),
                vec![ContentPart::text("client says Ada")],
                None,
            ),
        ))
        .await
        .unwrap();
    let (outcome, _) = h.run(&resumed(&request)).await;
    assert!(matches!(outcome, AgentsApiTurnOutcome::Completed { .. }));
    let submitted = h.fake.with(|s| s.tool_results.clone());
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["output"], "client says Ada");
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_new_message_cancels_the_abandoned_parked_provider_turn() {
    let h = Harness::new().await;
    park_on_approval(&h).await;
    // The person sends a new message instead of answering.
    let (outcome, _) = h.run(&request(2, "Never mind, who is 456?")).await;
    assert_completed_with(&outcome, 2);
    assert_eq!(h.fake.with(|s| s.cancel_posts), 1);
    assert_eq!(
        h.fake.with(|s| s.sessions[0].turns[0].status.clone()),
        "cancelled"
    );
}

#[tokio::test]
async fn a_budget_halt_stops_the_remote_turn_before_more_tools_run() {
    let h = Harness::new().await;
    h.executor.then(Script::Halt(
        "budget_exhausted",
        "Budget exhausted. Increase the budget to continue.",
    ));
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    match outcome {
        AgentsApiTurnOutcome::Failed {
            code,
            message,
            policy,
        } => {
            assert_eq!(code.as_deref(), Some("budget_exhausted"));
            assert!(policy);
            assert!(message.starts_with("Budget exhausted."));
        }
        other => panic!("expected a failed turn, got {other:?}"),
    }
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        h.fake.with(|s| (s.tool_result_posts, s.cancel_posts)),
        (0, 1)
    );
    // The denied call has a canonical failed result.
    let results = h.ledger.of_type("tool.completed");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["tool_call_id"], "call_1");
    assert!(
        results[0]["error"]
            .as_str()
            .unwrap()
            .starts_with("Budget exhausted.")
    );
}

/// Blocks any message containing `needle`.
struct BlockingPolicy {
    needle: &'static str,
    withhold: bool,
    message_checks: AtomicUsize,
}

impl BlockingPolicy {
    fn new(needle: &'static str, withhold: bool) -> Arc<Self> {
        Arc::new(Self {
            needle,
            withhold,
            message_checks: AtomicUsize::new(0),
        })
    }

    fn trip(&self) -> TrippedGuardrail {
        TrippedGuardrail {
            capability_id: "guardrails".into(),
            guardrail_id: "blocklist".into(),
            block: GuardrailBlock {
                reason_code: "blocked_term".into(),
                replacement: "[withheld by policy]".into(),
            },
        }
    }
}

#[async_trait]
impl AgentsApiOutputPolicy for BlockingPolicy {
    fn withholds_deltas(&self) -> bool {
        self.withhold
    }

    fn check_delta(&self, _: &str, accumulated: &str, _: &str) -> Option<TrippedGuardrail> {
        accumulated.contains(self.needle).then(|| self.trip())
    }

    async fn check_message(&self, _: &str, text: &str) -> Option<TrippedGuardrail> {
        self.message_checks.fetch_add(1, Ordering::SeqCst);
        text.contains(self.needle).then(|| self.trip())
    }
}

fn leaked(ledger: &TestLedger, needle: &str) -> bool {
    ledger
        .data()
        .iter()
        .filter(|(kind, _)| kind.starts_with("output.message"))
        .any(|(_, data)| data.to_string().contains(needle))
}

#[tokio::test]
async fn an_output_guardrail_replaces_a_remote_final_answer_and_stops_the_turn() {
    let h = Harness::new().await;
    *h.policy.lock().unwrap() = Some(BlockingPolicy::new("sessions are durable", false));
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    match &outcome {
        AgentsApiTurnOutcome::Completed {
            final_text,
            final_message_id,
            ..
        } => {
            assert_eq!(final_text, "[withheld by policy]");
            assert!(final_message_id.is_some());
        }
        other => panic!("expected a completed turn, got {other:?}"),
    }
    assert!(
        !leaked(&h.ledger, "sessions are durable"),
        "{:#?}",
        h.ledger.data()
    );
    let replaced = h.ledger.of_type("output.message.replaced");
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0]["guardrail_id"], "blocklist");
    assert_eq!(replaced[0]["reason_code"], "blocked_term");
    assert_eq!(h.fake.with(|s| s.cancel_posts), 1);
    h.ledger.assert_each_record_once();
}

#[tokio::test]
async fn a_tripped_commentary_stops_the_turn_before_any_tool_runs() {
    let h = Harness::new().await;
    let policy = BlockingPolicy::new("Looking the customer", false);
    *h.policy.lock().unwrap() = Some(policy.clone());
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert!(
        matches!(outcome, AgentsApiTurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    // The live stream tripped first; the completed item reused that verdict.
    assert_eq!(policy.message_checks.load(Ordering::SeqCst), 0);
    assert_eq!(h.executor.batches.load(Ordering::SeqCst), 0);
    assert_eq!(
        h.fake.with(|s| (s.tool_result_posts, s.cancel_posts)),
        (0, 1)
    );
    assert!(!leaked(&h.ledger, "Looking the customer"));
}

#[tokio::test]
async fn an_end_of_message_guardrail_withholds_live_text() {
    let h = Harness::new().await;
    *h.policy.lock().unwrap() = Some(BlockingPolicy::new("nothing matches this", true));
    let (outcome, _) = h.run(&request(1, "Who is customer 123?")).await;
    assert_completed(&outcome);
    assert!(h.ledger.of_type("output.message.delta").is_empty());
    assert_turn_record(&h.ledger, 1);
}

#[tokio::test]
async fn a_restart_after_a_guardrail_trip_finishes_the_stop_without_rejudging() {
    let h = Harness::new().await;
    let policy = BlockingPolicy::new("sessions are durable", false);
    *h.policy.lock().unwrap() = Some(policy.clone());
    // Dies after the replacement was recorded, before the outcome was saved.
    h.store
        .crash_when(|cp| cp.turn.as_ref().is_some_and(|turn| turn.outcome.is_some()));
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(crashes, 1);
    assert!(matches!(outcome, AgentsApiTurnOutcome::Completed { .. }));
    assert_eq!(h.ledger.of_type("output.message.replaced").len(), 1);
    assert_eq!(
        policy.message_checks.load(Ordering::SeqCst),
        2,
        "the commentary and the final answer are each judged once; the saved stop is finished, not judged again"
    );
    h.ledger.assert_each_record_once();
    assert!(!leaked(&h.ledger, "sessions are durable"));
}

#[tokio::test]
async fn mcp_credentials_never_reach_the_provider() {
    let h = Harness::new().await;
    // A scoped MCP tool reaches the provider as a client function; its server
    // and credentials stay with Everruns' session-scoped MCP client.
    let mut agent = agent();
    agent.tools.push(ToolDefinition::function(
        "mcp_crm__lookup",
        "Look up a CRM record",
        json!({"type": "object", "properties": {}}),
    ));
    let mut request = request(1, "Who is customer 123?");
    request.config = build_session_config(&agent, "", None).unwrap();
    request.config.ensure_enforceable().unwrap();
    let (outcome, _) = h.run(&request).await;
    assert_completed(&outcome);
    let body = h.fake.with(|s| s.create_bodies[0].clone());
    assert!(
        body["agent"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["type"] == "function"),
        "{body}"
    );
    let text = body.to_string();
    assert!(
        !text.contains("headers") && !text.contains("Authorization"),
        "{text}"
    );

    // A configuration that would hand a server's credentials to the provider
    // is refused before anything is saved or sent.
    let h = Harness::new().await;
    let request = request_with_mcp_credentials();
    request.config.validate_direct_mcp().unwrap_err();
    let error = h.driver().run(&request).await.unwrap_err();
    assert!(
        matches!(error, AgentsApiError::PolicyViolation(_)),
        "{error}"
    );
    assert!(!format!("{error} {error:?} {request:?}").contains("sk-mcp-secret"));
    assert_eq!(h.fake.with(|s| s.requests), 0);
}

fn request_with_mcp_credentials() -> AgentsApiTurnRequest {
    use everruns_core::host::openai_agents_api::AgentsApiTool;
    let mut request = request(1, "hi");
    request.config = build_session_config(&agent(), "", None)
        .unwrap()
        .with_direct_mcp("crm", "https://crm.example.com/mcp", &["lookup"])
        .unwrap();
    if let Some(AgentsApiTool::Mcp { transport, .. }) = request.config.agent.tools.last_mut() {
        transport
            .headers
            .insert("Authorization".into(), "Bearer sk-mcp-secret".into());
    }
    request
}

fn assert_completed_with(outcome: &AgentsApiTurnOutcome, tool_calls: u32) {
    match outcome {
        AgentsApiTurnOutcome::Completed {
            final_text,
            tool_calls: count,
            ..
        } => {
            assert_eq!(final_text, "Customer 123 is Ada; sessions are durable.");
            assert_eq!(*count, tool_calls);
        }
        other => panic!("expected a completed turn, got {other:?}"),
    }
}

// Observed provider work at the production runtime boundary.
fn inventory(name: &str) -> Value {
    let key = if name == "list_mcp_resources" {
        "resources"
    } else {
        "resourceTemplates"
    };
    json!({"type":"mcp_call", "id":name, "turn_id":"turn_1", "server_label":"codex", "name":name, "arguments":{}, "status":"completed", "error":null,
        "output":{"_meta":null, "content":[{"type":"text", "text":json!({key:[]}).to_string()}], "structuredContent":null}})
}

fn assert_refused(outcome: AgentsApiTurnOutcome) {
    assert!(
        matches!(outcome, AgentsApiTurnOutcome::Failed { code: Some(ref code), policy: true, .. } if code == "provider_tool_policy_violation"),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn unconfigured_provider_mcp_is_refused_before_client_execution() {
    let h = Harness::new().await;
    h.fake.with(|s| s.initial_provider_items.push(json!({"type":"mcp_call", "id":"unconfigured", "turn_id":"turn_1", "server_label":"unknown", "name":"steal", "arguments":{}, "status":"in_progress"})));
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.fake.with(|s| s.cancel_posts), 1);
}

#[tokio::test]
async fn exact_empty_provider_inventories_complete_and_remain_outside_act() {
    for name in ["list_mcp_resources", "list_mcp_resource_templates"] {
        let h = Harness::new().await;
        let mut pending = inventory(name);
        pending["status"] = json!("in_progress");
        pending["output"] = Value::Null;
        h.fake.with(|s| {
            s.initial_provider_items.push(pending);
            s.final_provider_items = Some(vec![inventory(name)]);
            s.duplicate = true;
        });
        assert_completed(
            &h.driver()
                .with_runtime_policy()
                .run(&request(1, "hi"))
                .await
                .unwrap(),
        );
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
        let hosted = h.ledger.of_type("tool.hosted_call");
        assert_eq!(hosted.len(), 2, "{hosted:?}");
        assert_eq!(hosted[0]["tool_name"], format!("mcp_codex__{name}"));
        assert_eq!(h.ledger.of_type("tool.completed").len(), 1);
        h.ledger.assert_each_record_once();
    }
}

#[tokio::test]
async fn provider_call_and_inventory_payload_matrix_fails_closed_without_leaking() {
    let base = inventory("list_mcp_resources");
    let mut cases = vec![
        json!({"type":"web_search_call", "id":"hosted", "status":"in_progress", "action":{"query":"sk-forbidden-payload"}}),
        json!({"type":"unknown_call", "arguments":{"key":"sk-forbidden-payload"}}),
        json!({"type":"new_executable_provider_kind"}),
        json!({"type":"mcp_call", "name":"sk-forbidden-payload"}),
        json!({"type":"function_call", "id":"unknown", "name":"sk-forbidden-payload", "arguments":{}}),
    ];
    for (key, value) in [
        ("server_label", json!("sk-forbidden-payload")),
        ("name", json!("read_mcp_resource")),
        ("arguments", json!({"server":"sk-forbidden-payload"})),
        ("arguments", Value::Null),
        ("arguments", json!("{}")),
        ("arguments", json!([])),
        ("id", Value::Null),
        ("status", json!("failed")),
        ("status", json!("cancelled")),
        ("status", json!("incomplete")),
        ("error", json!({"message":"sk-forbidden-payload"})),
        ("output", json!("sk-forbidden-payload")),
        (
            "output",
            json!({"resources":[{"uri":"sk-forbidden-payload"}]}),
        ),
        ("output", json!({"resourceTemplates":[]})),
        (
            "output",
            json!({"resources":[], "nextCursor":"sk-forbidden-payload"}),
        ),
        (
            "output",
            json!({"content":[{"type":"text", "text":"{\"resources\":[]}"}], "isError":true}),
        ),
        (
            "output",
            json!({"content":[{"type":"text", "text":"{\"resources\":[]}"}], "structuredContent":{"resources":["sk-forbidden-payload"]}}),
        ),
        (
            "output",
            json!({"content":[{"type":"text", "text":"{\"resources\":[]}"}, {"type":"text", "text":"sk-forbidden-payload"}]}),
        ),
        (
            "output",
            json!({"content":[{"type":"text", "text":"{\"resources\":[]}"}], "_meta":{"key":"sk-forbidden-payload"}}),
        ),
    ] {
        let mut case = base.clone();
        case[key] = value;
        cases.push(case);
    }
    let mut missing_args = base.clone();
    missing_args.as_object_mut().unwrap().remove("arguments");
    cases.push(missing_args);
    for item in cases {
        let h = Harness::new().await;
        h.fake.with(|s| s.initial_provider_items.push(item.clone()));
        let outcome = h
            .driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap();
        assert!(
            !format!("{outcome:?}").contains("sk-forbidden-payload"),
            "{item}"
        );
        assert_refused(outcome);
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0, "{item}");
        assert_eq!(h.fake.with(|s| s.tool_result_posts), 0, "{item}");
        assert!(
            !format!("{:?}", h.ledger.data()).contains("sk-forbidden-payload"),
            "{item}"
        );
    }
}

#[tokio::test]
async fn a_missing_provider_item_stream_is_checked_before_client_functions() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.initial_provider_items
            .push(json!({"type":"shell_call", "id":"shell", "status":"in_progress"}));
        s.drop = vec![
            "agent.session.turn.item.added",
            "agent.session.turn.item.done",
        ];
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn final_saved_inventory_is_validated_when_terminal_items_were_not_streamed() {
    for item in [
        json!({"type":"mcp_call", "id":"unknown", "name":"unknown", "status":"completed"}),
        {
            let mut item = inventory("list_mcp_resources");
            item["output"] = json!({"resources":["unexpected"]});
            item
        },
        {
            let mut item = inventory("list_mcp_resources");
            item["status"] = json!("in_progress");
            item["output"] = Value::Null;
            item
        },
    ] {
        let h = Harness::new().await;
        h.fake.with(|s| {
            s.final_provider_items = Some(vec![item]);
            s.drop = vec!["agent.session.turn.item.done"];
        });
        assert_refused(
            h.driver()
                .with_runtime_policy()
                .run(&request(1, "hi"))
                .await
                .unwrap(),
        );
        assert_eq!(h.fake.with(|s| s.cancel_posts), 1);
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
        assert!(h.checkpoint().turn.unwrap().policy_stop.is_some());
    }
}

#[tokio::test]
async fn a_pending_inventory_missing_from_the_terminal_snapshot_cannot_complete() {
    let h = Harness::new().await;
    let mut pending = inventory("list_mcp_resources");
    pending["status"] = json!("in_progress");
    pending["output"] = Value::Null;
    h.fake.with(|s| {
        s.stream_only_provider_items.push(pending);
        s.final_provider_items = Some(vec![]);
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    assert_eq!(h.fake.with(|s| s.cancel_posts), 1);
}

#[tokio::test]
async fn hidden_subagents_and_malformed_actions_are_refused_before_act() {
    let good = json!({"type":"function_call", "turn_id":"turn_1", "call_id":"call_1", "name":"lookup_customer", "arguments":{}});
    let mut cases = vec![json!({"type":"environment_connection"}), Value::Null];
    for (field, value) in [
        ("subagent_id", json!("child")),
        ("subagent_turn_id", json!("child")),
        ("turn_id", json!("unknown_child_turn")),
        ("turn_id", Value::Null),
        ("call_id", Value::Null),
        ("name", json!("unknown")),
        ("arguments", json!("{malformed")),
    ] {
        let mut case = good.clone();
        case[field] = value;
        cases.push(case);
    }
    for action in cases {
        let h = Harness::new().await;
        h.fake
            .with(|s| s.initial_required_actions = Some(vec![action]));
        assert_refused(
            h.driver()
                .with_runtime_policy()
                .run(&request(1, "hi"))
                .await
                .unwrap(),
        );
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    }
    let h = Harness::new().await;
    h.fake.with(|s| s.hidden_subagent = true);
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn strict_config_is_refused_before_acquiring_or_reusing_a_provider_session() {
    let h = Harness::new().await;
    h.fake.with(|s| s.final_provider_items = Some(vec![]));
    let first = request(1, "hi");
    assert!(matches!(
        h.driver().with_runtime_policy().run(&first).await.unwrap(),
        AgentsApiTurnOutcome::Completed { tool_calls: 1, .. }
    ));
    let checkpoint = h.checkpoint();
    let requests = h.fake.with(|s| s.requests);
    let mut next = request(2, "hi again");
    next.config = next
        .config
        .with_direct_mcp("docs", "https://docs.example.com/mcp", &["search"])
        .unwrap();
    assert!(matches!(
        h.driver().with_runtime_policy().run(&next).await,
        Err(AgentsApiError::PolicyViolation(_))
    ));
    assert_eq!(h.fake.with(|s| s.requests), requests);
    assert_eq!(
        h.checkpoint().turn.unwrap().turn_id,
        checkpoint.turn.unwrap().turn_id
    );
    let empty = Harness::new().await;
    assert!(
        empty
            .driver()
            .with_runtime_policy()
            .run(&next)
            .await
            .is_err()
    );
    assert_eq!(empty.fake.with(|s| s.requests), 0);
    assert!(empty.store.inner.snapshot(first.session_id).is_none());
}

#[tokio::test]
async fn saved_provider_stop_survives_crash_and_replay_without_more_execution() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.initial_provider_items
            .push(json!({"type":"web_search_call", "id":"search", "status":"in_progress"}))
    });
    // Fail while saving the terminal outcome, after the stop and cancel are durable.
    h.store
        .crash_when(|cp| cp.turn.as_ref().is_some_and(|t| t.outcome.is_some()));
    let request = request(1, "hi");
    assert!(
        h.driver()
            .with_runtime_policy()
            .run(&request)
            .await
            .is_err()
    );
    assert!(h.checkpoint().turn.unwrap().policy_stop.is_some());
    h.store.inner.expire(request.session_id);
    // The provider no longer reports the evidence; the persisted stop wins.
    h.fake.with(|s| {
        s.sessions[0].turns[0]
            .items
            .retain(|item| item["id"] != "search")
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request)
            .await
            .unwrap(),
    );
    let requests = h.fake.with(|s| s.requests);
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request)
            .await
            .unwrap(),
    );
    assert_eq!(
        h.fake.with(|s| s.requests),
        requests,
        "terminal replay contacts no provider"
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 0);
    assert_eq!(h.ledger.of_type("llm.generation").len(), 1);
}

#[tokio::test]
async fn custom_driver_remains_explicitly_permissive() {
    let h = Harness::new().await;
    h.fake.with(|s| s.managed_extras = true);
    assert!(matches!(
        h.driver().run(&request(1, "hi")).await.unwrap(),
        AgentsApiTurnOutcome::Completed { .. }
    ));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
    assert!(!h.ledger.of_type("tool.hosted_call").is_empty());
}

#[tokio::test]
async fn parent_provenance_cannot_masquerade_as_a_root_client_action() {
    for provenance in [
        json!({"parent_turn_id":"turn_1"}),
        json!({"root_turn_id":"other_root"}),
        json!({"turn":{"id":"child", "subagent_id":null}}),
    ] {
        let h = Harness::new().await;
        let mut action = json!({"type":"function_call", "turn_id":"turn_1", "call_id":"call_1", "name":"lookup_customer", "arguments":{}});
        action
            .as_object_mut()
            .unwrap()
            .extend(provenance.as_object().unwrap().clone());
        h.fake
            .with(|s| s.initial_required_actions = Some(vec![action]));
        assert_refused(
            h.driver()
                .with_runtime_policy()
                .run(&request(1, "hi"))
                .await
                .unwrap(),
        );
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn a_missing_root_created_event_reconciles_before_a_legitimate_client_action() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.drop = vec!["agent.session.turn.created"];
        s.final_provider_items = Some(vec![inventory("list_mcp_resources")]);
    });
    assert_completed(
        &h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn old_mcp_checkpoint_entries_require_authoritative_allowed_items_before_resuming() {
    use everruns_core::agents_api_store::{ItemCorrelation, ItemKind, ItemState};
    for state in [ItemState::Open, ItemState::Completed] {
        let h = Harness::new().await;
        h.executor.then(Script::Park(ParkReason::ClientResult));
        let initial = request(1, "hi");
        assert!(matches!(
            h.driver().run(&initial).await.unwrap(),
            AgentsApiTurnOutcome::Paused
        ));
        let lease = AgentsApiLease {
            org_id: initial.org_id,
            session_id: initial.session_id,
            owner: uuid::Uuid::new_v4(),
        };
        let mut checkpoint = h.store.acquire(lease).await.unwrap();
        checkpoint.turn.as_mut().unwrap().items.insert(
            "mcp:legacy_missing".into(),
            ItemCorrelation {
                kind: ItemKind::McpCall,
                local_id: MessageId::new().to_string(),
                state,
            },
        );
        h.store.save(lease, &checkpoint).await.unwrap();
        h.store.release(lease).await.unwrap();
        assert_refused(
            h.driver()
                .with_runtime_policy()
                .run(&resumed(&initial))
                .await
                .unwrap(),
        );
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
        assert_eq!(h.fake.with(|s| s.tool_result_posts), 0);
    }
}

#[tokio::test]
async fn known_earlier_roots_do_not_trip_the_current_turn_policy() {
    let h = Harness::new().await;
    h.fake
        .with(|s| s.final_provider_items = Some(vec![inventory("list_mcp_resources")]));
    assert_completed(
        &h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    h.fake.with(|s| {
        s.initial_provider_items =
            vec![json!({"type":"web_search_call", "id":"old_search", "turn_id":"turn_1"})];
        let mut next = inventory("list_mcp_resource_templates");
        next["turn_id"] = json!("turn_2");
        s.final_provider_items = Some(vec![next]);
    });
    assert!(matches!(
        h.driver()
            .with_runtime_policy()
            .run(&request(2, "next"))
            .await
            .unwrap(),
        AgentsApiTurnOutcome::Completed { .. }
    ));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 2);
    assert_eq!(h.fake.with(|s| s.cancel_posts), 0);
}

#[tokio::test]
async fn refused_inventory_completion_closes_its_known_hosted_lifecycle_once() {
    let h = Harness::new().await;
    let mut pending = inventory("list_mcp_resources");
    pending["status"] = json!("in_progress");
    pending["output"] = Value::Null;
    let mut nonempty = inventory("list_mcp_resources");
    nonempty["output"] = json!({"resources":["sk-forbidden-payload"]});
    h.fake.with(|s| {
        s.initial_provider_items.push(pending);
        s.final_provider_items = Some(vec![nonempty]);
    });
    let initial = request(1, "hi");
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    let hosted = h.ledger.of_type("tool.hosted_call");
    assert_eq!(hosted.len(), 2, "{hosted:?}");
    assert_eq!(hosted[0]["status"], "in_progress");
    assert_eq!(hosted[1]["status"], "failed");
    assert_eq!(hosted[0]["call_id"], hosted[1]["call_id"]);
    assert!(!format!("{:?}", h.ledger.data()).contains("sk-forbidden-payload"));
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    assert_eq!(h.ledger.of_type("tool.hosted_call").len(), 2);
    h.ledger.assert_each_record_once();
}

#[tokio::test]
async fn a_crash_immediately_after_saving_the_policy_stop_preserves_rejected_call_cost() {
    use everruns_core::agents_api_store::ItemState;
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.initial_provider_items.push(
            json!({"type":"web_search_call", "id":"sk-forbidden-payload", "status":"in_progress"}),
        )
    });
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.policy_stop.is_some()
                && turn.items.iter().any(|(key, item)| {
                    key.starts_with("hosted-done:rejected-") && item.state == ItemState::Completing
                })
        })
    });
    let initial = request(1, "hi");
    assert!(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .is_err()
    );
    let persisted = h.checkpoint().turn.unwrap();
    assert!(persisted.policy_stop.is_some());
    assert!(persisted.items.iter().any(
        |(key, item)| key.starts_with("hosted-done:rejected-") && item.state == ItemState::Open
    ));
    h.store.inner.expire(initial.session_id);
    h.fake.with(|s| {
        s.sessions[0].turns[0]
            .items
            .retain(|item| item["type"] != "web_search_call")
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    let generations = h.ledger.of_type("llm.generation");
    assert_eq!(generations.len(), 1);
    assert!(
        generations[0].to_string().contains("web_search_call"),
        "{generations:?}"
    );
    assert_eq!(h.ledger.of_type("tool.hosted_call").len(), 1);
    assert!(!format!("{:?}", h.ledger.data()).contains("sk-forbidden-payload"));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_unknown_additional_root_is_refused_even_without_its_stream() {
    let h = Harness::new().await;
    h.executor.then(Script::Park(ParkReason::ClientResult));
    let initial = request(1, "hi");
    assert!(matches!(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
        AgentsApiTurnOutcome::Paused
    ));
    h.fake.with(|s| {
        s.sessions[0].turns.push(FakeTurn {
            id: "unknown_root".into(),
            status: "in_progress".into(),
            created_at: 2,
            ..FakeTurn::default()
        })
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&resumed(&initial))
            .await
            .unwrap(),
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 0);
}

#[tokio::test]
async fn policy_accounting_never_copies_poisoned_root_or_hidden_child_errors() {
    let h = Harness::new().await;
    h.executor.then(Script::Park(ParkReason::ClientResult));
    let initial = request(1, "hi");
    assert!(matches!(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
        AgentsApiTurnOutcome::Paused
    ));
    h.fake.with(|s| {
        s.sessions[0].turns[0].error = json!({"message":"sk-forbidden-payload-root"});
        s.sessions[0].turns.push(FakeTurn {
            id: "child_turn".into(),
            subagent_id: Some("sk-forbidden-payload-label".into()),
            status: "failed".into(),
            created_at: 2,
            error: json!({"message":"sk-forbidden-payload-child"}),
            ..FakeTurn::default()
        });
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&resumed(&initial))
            .await
            .unwrap(),
    );
    let generations = h.ledger.of_type("llm.generation");
    assert_eq!(generations.len(), 2, "orphan child and root both accounted");
    assert!(!format!("{:?}", h.ledger.data()).contains("sk-forbidden-payload"));
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.ledger.of_type("tool.hosted_call").len(), 1);
}

#[tokio::test]
async fn a_created_provider_environment_is_refused_before_client_actions() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.provider_environment = Some(json!({"type":"hosted", "secret":"sk-forbidden-payload"}))
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.fake.with(|s| s.tool_result_posts), 0);
    assert!(!format!("{:?}", h.ledger.data()).contains("sk-forbidden-payload"));
}

#[tokio::test]
async fn permissive_hosted_checkpoints_are_refused_before_client_execution() {
    use everruns_core::agents_api_store::{ItemCorrelation, ItemKind, ItemState};
    for (key, local_id) in [
        ("hosted:web", "web"),
        ("hosted-done:web", "web_search_call"),
        ("hosted:missing_inventory", "missing_inventory"),
    ] {
        let h = Harness::new().await;
        h.executor.then(Script::Park(ParkReason::ClientResult));
        let initial = request(1, "hi");
        assert!(matches!(
            h.driver().run(&initial).await.unwrap(),
            AgentsApiTurnOutcome::Paused
        ));
        let lease = AgentsApiLease {
            org_id: initial.org_id,
            session_id: initial.session_id,
            owner: uuid::Uuid::new_v4(),
        };
        let mut checkpoint = h.store.acquire(lease).await.unwrap();
        checkpoint.turn.as_mut().unwrap().items.insert(
            key.into(),
            ItemCorrelation {
                kind: ItemKind::HostedCall,
                local_id: local_id.into(),
                state: ItemState::Completed,
            },
        );
        h.store.save(lease, &checkpoint).await.unwrap();
        h.store.release(lease).await.unwrap();
        assert_refused(
            h.driver()
                .with_runtime_policy()
                .run(&resumed(&initial))
                .await
                .unwrap(),
        );
        assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
        assert_eq!(h.fake.with(|s| s.tool_result_posts), 0);
    }
}

#[tokio::test]
async fn contradictory_earlier_root_provenance_cannot_hide_current_work() {
    let h = Harness::new().await;
    h.fake
        .with(|s| s.final_provider_items = Some(vec![inventory("list_mcp_resources")]));
    assert_completed(
        &h.driver()
            .with_runtime_policy()
            .run(&request(1, "hi"))
            .await
            .unwrap(),
    );
    h.fake.with(|s| {
        s.initial_required_actions = Some(vec![json!({"type":"function_call", "turn_id":"turn_1", "parent_turn_id":"turn_1", "turn":{"root_turn_id":"turn_2"}, "call_id":"call_2", "name":"lookup_customer", "arguments":{}})]);
    });
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&request(2, "next"))
            .await
            .unwrap(),
    );
    assert_eq!(
        h.executor.calls.load(Ordering::SeqCst),
        1,
        "only first legitimate turn executed"
    );
}

#[tokio::test]
async fn a_subagent_before_root_adoption_is_refused_and_its_unknown_spend_accounted() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.hidden_subagent = true;
        s.pre_root_events = vec![json!({"type":"agent.session.turn.created", "turn_id":"hidden_child", "subagent_id":"child", "turn":{"id":"hidden_child", "parent_turn_id":"turn_1", "subagent_id":"child"}})];
    });
    let initial = request(1, "hi");
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    let generations = h.ledger.of_type("llm.generation");
    assert_eq!(
        generations.len(),
        2,
        "root recovered for accounting plus child"
    );
    assert_eq!(generations[0]["metadata"]["usage"], Value::Null);
    assert!(
        !generations[0]["metadata"]["cost_components"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    assert_eq!(h.ledger.of_type("llm.generation").len(), 2);
}

#[tokio::test]
async fn a_durable_rejected_child_missing_on_recovery_keeps_its_spend_unknown() {
    use everruns_core::agents_api_store::ItemState;
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.hidden_subagent = true;
        s.usage_on_failure = true;
    });
    h.store.crash_when(|cp| {
        cp.turn.as_ref().is_some_and(|turn| {
            turn.policy_stop.is_some()
                && turn
                    .items
                    .get("usage:hidden_child")
                    .is_some_and(|item| item.state == ItemState::Completing)
        })
    });
    let initial = request(1, "hi");
    assert!(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .is_err()
    );
    assert!(
        h.checkpoint()
            .turn
            .unwrap()
            .items
            .contains_key("subagent:hidden_child")
    );
    h.store.inner.expire(initial.session_id);
    h.fake
        .with(|s| s.sessions[0].turns.retain(|turn| turn.id != "hidden_child"));
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    let generations = h.ledger.of_type("llm.generation");
    assert_eq!(generations.len(), 2);
    assert_eq!(generations[0]["metadata"]["response_id"], "hidden_child");
    assert_eq!(generations[0]["metadata"]["usage"], Value::Null);
    assert_eq!(
        generations[0]["metadata"]["cost_components"][0]["cost_usd"],
        Value::Null
    );
    assert!(!generations[1]["metadata"]["usage"].is_null());
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rejected_hosted_work_with_no_recoverable_root_retains_unknown_tokens_and_known_calls() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.hide_turns = true;
        s.pre_root_events = vec![json!({"type":"agent.session.turn.item.added", "item":{"type":"web_search_call", "id":"search", "status":"in_progress"}})];
    });
    let initial = request(1, "hi");
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    let generations = h.ledger.of_type("llm.generation");
    assert_eq!(generations.len(), 1);
    assert_eq!(generations[0]["metadata"]["usage"], Value::Null);
    let components = generations[0]["metadata"]["cost_components"]
        .as_array()
        .unwrap();
    assert_eq!(components.len(), 2);
    assert_eq!(components[0]["cost_usd"], Value::Null);
    assert_eq!(components[1]["name"], "web_search_call");
    assert_eq!(components[1]["quantity"], 1);
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    assert_refused(
        h.driver()
            .with_runtime_policy()
            .run(&initial)
            .await
            .unwrap(),
    );
    assert_eq!(h.ledger.of_type("llm.generation").len(), 1);
}
