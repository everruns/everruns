//! OpenAI Agents API backend: Everruns policy at the tool and output
//! boundaries (EVE-1124).
//!
//! The stateful fake from `agents_api_support` holds the provider's required
//! action open across an approval, a denial, an expiry, a client-side answer,
//! and a provider timeout; the remote loop stops on an output guardrail or a
//! budget; and MCP credentials stay out of everything sent to the provider.
#![cfg(feature = "openai-agents-api")]

mod agents_api_support;

use agents_api_support::*;

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
    use everruns_host::openai_agents_api::AgentsApiTool;
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
