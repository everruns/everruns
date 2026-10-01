//! OpenAI Agents API backend: events, usage, and cost in Everruns
//! observability (EVE-1125).
//!
//! Contract tests against the stateful fake from `agents_api_support`: a
//! managed turn that delegates to a subagent, runs a hosted web search,
//! reasons, and compacts its context projects only canonical events, in
//! order; root and subagent terminal events end only their own turn; unknown
//! provider events are ignored; a reconnect, a duplicate stream, or a restart
//! bills each provider turn once; and a cancelled turn still reports what it
//! spent, or says it does not know.
#![cfg(feature = "openai-agents-api")]

mod agents_api_support;

use agents_api_support::*;
use everruns_core::events::EventData;
use tokio::sync::watch;

/// Event types the SSE stream and UI already render for native turns.
const CANONICAL: &[&str] = &[
    "output.message.started",
    "output.message.delta",
    "output.message.completed",
    "tool.started",
    "tool.completed",
    "tool.hosted_call",
    "reason.item",
    "context.compacted",
    "llm.generation",
];

const FINAL_TEXT: &str = "Customer 123 is Ada; sessions are durable.";

async fn managed_turn(configure: impl FnOnce(&mut FakeState)) -> (Harness, AgentsApiTurnOutcome) {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.managed_extras = true;
        configure(s);
    });
    let (outcome, crashes) = h.run(&request(1, "Who is customer 123?")).await;
    assert_eq!(
        crashes, 0,
        "a misattributed terminal would end the turn early"
    );
    (h, outcome)
}

fn events(h: &Harness) -> Vec<EventRequest> {
    h.ledger.events.lock().unwrap().clone()
}

fn position(types: &[String], kind: &str) -> Vec<usize> {
    types
        .iter()
        .enumerate()
        .filter(|(_, t)| *t == kind)
        .map(|(i, _)| i)
        .collect()
}

fn generations(h: &Harness) -> Vec<Value> {
    h.ledger.of_type("llm.generation")
}

/// The driver's own events; the emulated tool pipeline's carry no metadata.
fn projected(h: &Harness) -> Vec<EventRequest> {
    events(h)
        .into_iter()
        .filter(|event| event.metadata.is_some())
        .collect()
}

#[tokio::test]
async fn a_managed_turn_projects_only_canonical_events_in_order() {
    let (h, outcome) = managed_turn(|_| {}).await;
    match &outcome {
        AgentsApiTurnOutcome::Completed { final_text, .. } => assert_eq!(final_text, FINAL_TEXT),
        other => panic!("expected a completed turn, got {other:?}"),
    }
    h.ledger.assert_each_record_once();

    // Every event is one an existing consumer renders, and survives the SSE
    // wire round trip as a typed event, not an unsupported one.
    let all = events(&h);
    for event in &all {
        assert!(
            CANONICAL.contains(&event.event_type.as_str()),
            "non-canonical event {}",
            event.event_type
        );
        let wire = serde_json::to_string(event).unwrap();
        let parsed: EventRequest = serde_json::from_str(&wire).unwrap();
        assert!(!parsed.data.is_unsupported(), "{wire}");
    }

    let types: Vec<String> = all.iter().map(|e| e.event_type.clone()).collect();
    let completed = position(&types, "output.message.completed");
    let generation = position(&types, "llm.generation");
    assert_eq!(
        generation.len(),
        2,
        "one per provider turn: subagent and root"
    );
    assert!(
        generation.iter().all(|g| completed.iter().all(|c| c < g)),
        "accounting follows every message of the turn: {types:?}"
    );
    // Every record precedes the accounting. (A reconnect reconciles the
    // root's saved items before the live tail, so the final answer may land
    // before a subagent's lifecycle; both precede the billing.)
    for kind in ["tool.hosted_call", "reason.item", "context.compacted"] {
        let at = position(&types, kind);
        assert!(!at.is_empty(), "{kind} missing");
        assert!(at.iter().all(|i| i < &generation[0]), "{kind} {types:?}");
    }
    // Each message starts before it completes.
    let started = position(&types, "output.message.started");
    assert!(started.first() < completed.first());

    // The subagent is recorded as delegated work, not as root transcript.
    let hosted = h.ledger.of_type("tool.hosted_call");
    let subagent: Vec<_> = hosted
        .iter()
        .filter(|c| c["tool_name"] == "subagent")
        .map(|c| c["status"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(subagent, ["in_progress", "completed"]);
    let search: Vec<_> = hosted
        .iter()
        .filter(|c| c["tool_name"] == "web_search")
        .collect();
    assert_eq!(search.len(), 2);
    assert_eq!(search[1]["status"], "completed");
    assert_eq!(search[1]["summary"], "agents api sessions");
    assert!(!h.ledger.texts().iter().any(|t| t == "Subagent notes."));
    assert_eq!(
        h.ledger.texts().last().map(String::as_str),
        Some(FINAL_TEXT)
    );

    let compacted = h.ledger.of_type("context.compacted");
    assert_eq!(compacted.len(), 1);
    assert_eq!(compacted[0]["strategy_used"], "provider_managed");
}

#[tokio::test]
async fn provider_ids_are_correlation_metadata_beside_local_ids() {
    let (h, _) = managed_turn(|_| {}).await;
    let local_turn = request(1, "").turn_id;
    for event in projected(&h) {
        let metadata = event.metadata.as_ref().unwrap();
        assert_eq!(metadata["runtime_backend"], "openai_agents_api");
        assert_eq!(metadata["provider_session_id"], "sess_1");
        assert_eq!(metadata["provider_turn_id"], "turn_1");
        assert!(
            metadata["provider_trace_url"]
                .as_str()
                .unwrap()
                .ends_with("/agents/sessions/sess_1/traces"),
            "{metadata}"
        );
        // Local ids stay the event's identity.
        assert_eq!(event.context.turn_id, Some(local_turn));
    }
    let messages = h.ledger.of_type("output.message.completed");
    assert!(
        messages
            .iter()
            .all(|m| !m["message"]["id"].as_str().unwrap().starts_with("msg_")),
        "provider message ids never replace local ones"
    );
    let subagent_generation = projected(&h)
        .into_iter()
        .find(|e| {
            e.event_type == "llm.generation"
                && e.metadata.as_ref().unwrap()["provider_subagent_id"] == SUBAGENT_ID
        })
        .expect("the subagent turn is billed separately");
    assert_eq!(
        subagent_generation.metadata.unwrap()["provider_item_id"],
        "turn_sub_1"
    );
}

#[tokio::test]
async fn hidden_reasoning_and_compacted_context_never_reach_the_record() {
    let (h, _) = managed_turn(|_| {}).await;
    let record = serde_json::to_string(&events(&h)).unwrap();
    assert!(!record.contains(HIDDEN_REASONING));
    assert!(!record.contains(ENCRYPTED_CONTEXT));
    let items = h.ledger.of_type("reason.item");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["summary"],
        json!(["Checked the customer record first."])
    );
}

#[tokio::test]
async fn cost_aggregates_root_subagent_and_tool_charges() {
    let (h, outcome) = managed_turn(|_| {}).await;
    let generations = generations(&h);
    let root = generations
        .iter()
        .find(|g| g["metadata"]["response_id"] == "turn_1")
        .unwrap();
    let subagent = generations
        .iter()
        .find(|g| g["metadata"]["response_id"] == "turn_sub_1")
        .unwrap();

    // Cached tokens are a disjoint bucket on both.
    assert_eq!(root["metadata"]["usage"]["input_tokens"], 100);
    assert_eq!(root["metadata"]["usage"]["cache_read_tokens"], 20);
    assert_eq!(subagent["metadata"]["usage"]["input_tokens"], 40);
    assert_eq!(root["metadata"]["success"], true);
    assert_eq!(root["output"]["text"], FINAL_TEXT);

    // Tokens are priced from the model profile; the hosted search adds its
    // per-call price on top.
    let components = root["metadata"]["cost_components"].as_array().unwrap();
    let tokens = components
        .iter()
        .find(|c| c["kind"] == "model_tokens")
        .unwrap();
    let tokens_cost = tokens["cost_usd"].as_f64().expect("gpt-6-astra is priced");
    assert_eq!(tokens["quantity"], 150);
    let search = components
        .iter()
        .find(|c| c["name"] == "web_search_call")
        .unwrap();
    assert_eq!(search["quantity"], 1);
    assert_eq!(search["cost_usd"], 0.01);
    let estimated = root["metadata"]["usage"]["estimated_cost_usd"]
        .as_f64()
        .unwrap();
    assert!((estimated - (tokens_cost + 0.01)).abs() < 1e-9);
    assert!(
        subagent["metadata"]["usage"]["estimated_cost_usd"]
            .as_f64()
            .is_some()
    );

    // The turn reports root and subagent usage together.
    let AgentsApiTurnOutcome::Completed {
        usage: Some(usage), ..
    } = outcome
    else {
        panic!("expected known usage, got {outcome:?}");
    };
    assert_eq!(usage.input_tokens, 140);
    assert_eq!(usage.output_tokens, 38);
}

#[tokio::test]
async fn usage_the_provider_never_reports_is_an_explicit_unknown() {
    let (h, outcome) = managed_turn(|s| s.null_usage = true).await;
    let AgentsApiTurnOutcome::Completed { usage, .. } = outcome else {
        panic!("expected a completed turn, got {outcome:?}");
    };
    assert!(usage.is_none(), "unknown, never zero");
    for generation in generations(&h) {
        assert!(generation["metadata"].get("usage").is_none());
        let tokens = &generation["metadata"]["cost_components"][0];
        assert_eq!(tokens["kind"], "model_tokens");
        assert!(tokens.get("cost_usd").is_none(), "{generation}");
    }
    // Usage was read again before it was declared unknown.
    assert!(h.fake.with(|s| s.requests) > 0);
}

#[tokio::test]
async fn a_generation_names_what_late_usage_is_read_back_with() {
    // EVE-1145: the server reconciler re-reads the turn by `response_id` in
    // the provider session, with the provider that ran it.
    let (h, _) = managed_turn(|s| s.null_usage = true).await;
    let billed: Vec<EventRequest> = projected(&h)
        .into_iter()
        .filter(|e| e.event_type == "llm.generation")
        .collect();
    assert!(!billed.is_empty());
    for event in billed {
        let metadata = event.metadata.as_ref().unwrap();
        assert_eq!(metadata["everruns_provider_id"], PROVIDER_KEY);
        assert_eq!(metadata["provider_session_id"], "sess_1");
    }
    // Only accounting carries it: no other event names the provider.
    assert!(
        projected(&h)
            .iter()
            .filter(|e| e.event_type != "llm.generation")
            .all(|e| e
                .metadata
                .as_ref()
                .unwrap()
                .get("everruns_provider_id")
                .is_none())
    );
}

#[tokio::test]
async fn reconnects_duplicates_and_restarts_bill_each_provider_turn_once() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.managed_extras = true;
        s.duplicate = true;
    });
    // Die right before the root turn's accounting is saved, then again right
    // before the outcome is saved.
    h.store.crash_when(|cp| {
        cp.turn
            .as_ref()
            .is_some_and(|turn| turn.items.contains_key("usage:turn_1"))
    });
    let request = request(1, "Who is customer 123?");
    let (_, crashes) = h.run(&request).await;
    assert_eq!(crashes, 1);
    h.store
        .crash_when(|cp| cp.turn.as_ref().is_some_and(|turn| turn.outcome.is_some()));
    let before = generations(&h).len();
    let (outcome, _) = h.run(&request).await;
    assert!(matches!(outcome, AgentsApiTurnOutcome::Completed { .. }));
    // A replay of the finished turn bills nothing again.
    let (_, _) = h.run(&request).await;

    h.ledger.assert_each_record_once();
    assert_eq!(before, 2, "the crash happened before the root was billed");
    assert_eq!(generations(&h).len(), 2);
    assert_eq!(h.ledger.of_type("reason.item").len(), 1);
    assert_eq!(h.ledger.of_type("context.compacted").len(), 1);
    assert_eq!(h.ledger.of_type("tool.hosted_call").len(), 4);
}

#[tokio::test]
async fn a_cancelled_turn_bills_what_it_spent() {
    let h = Harness::new().await;
    h.fake.with(|s| s.usage_on_failure = true);
    h.executor.then(Script::Park(ParkReason::ClientResult));
    let request = request(1, "Who is customer 123?");
    let (paused, _) = h.run(&request).await;
    assert!(matches!(paused, AgentsApiTurnOutcome::Paused));
    assert!(
        generations(&h).is_empty(),
        "a paused turn is not billed yet"
    );

    let (cancel, receiver) = watch::channel(true);
    let error = h
        .driver()
        .with_cancellation(receiver)
        .run(&request)
        .await
        .unwrap_err();
    drop(cancel);
    assert!(matches!(error, AgentsApiError::Cancelled), "{error}");
    assert_eq!(h.fake.with(|s| s.cancel_posts), 1);
    let generations = generations(&h);
    assert_eq!(generations.len(), 1);
    let generation = &generations[0];
    assert_eq!(generation["metadata"]["success"], false);
    assert_eq!(
        generation["metadata"]["finish_reasons"],
        json!(["cancelled"])
    );
    assert_eq!(generation["metadata"]["usage"]["input_tokens"], 100);
}

#[tokio::test]
async fn a_cancelled_turn_without_usage_reports_an_unknown_amount() {
    let h = Harness::new().await;
    h.executor.then(Script::Park(ParkReason::ClientResult));
    let request = request(1, "Who is customer 123?");
    h.run(&request).await;
    let (_cancel, receiver) = watch::channel(true);
    let _ = h.driver().with_cancellation(receiver).run(&request).await;
    let generations = generations(&h);
    assert_eq!(generations.len(), 1);
    assert!(generations[0]["metadata"].get("usage").is_none());
    assert!(
        generations[0]["metadata"]["cost_components"][0]
            .get("cost_usd")
            .is_none()
    );
}

#[tokio::test]
async fn an_environment_failure_fails_the_turn_it_belongs_to() {
    let h = Harness::new().await;
    h.fake.with(|s| s.environment_failure = true);
    let (outcome, crashes) = h.run(&request(1, "hi")).await;
    assert_eq!(crashes, 0);
    match outcome {
        AgentsApiTurnOutcome::Failed { code, policy, .. } => {
            assert_eq!(code.as_deref(), Some("environment_failed"));
            assert!(!policy);
        }
        other => panic!("expected a failed turn, got {other:?}"),
    }
    assert_eq!(h.executor.calls.load(Ordering::SeqCst), 0);
    let generations = generations(&h);
    assert_eq!(generations.len(), 1, "a failed turn is still accounted");
    assert_eq!(generations[0]["metadata"]["success"], false);
}

#[tokio::test]
async fn a_failed_turn_bills_its_spend() {
    let h = Harness::new().await;
    h.fake.with(|s| {
        s.fail_turns = true;
        s.usage_on_failure = true;
    });
    let (outcome, _) = h.run(&request(1, "hi")).await;
    assert!(matches!(outcome, AgentsApiTurnOutcome::Failed { .. }));
    let generations = generations(&h);
    assert_eq!(generations.len(), 1);
    assert_eq!(generations[0]["metadata"]["success"], false);
    assert_eq!(generations[0]["metadata"]["usage"]["output_tokens"], 30);
    assert!(
        generations[0]["metadata"]["error"]
            .as_str()
            .unwrap()
            .contains("billing limit")
    );
}

#[test]
fn llm_generation_is_typed_event_data() {
    // Guards the CANONICAL list above against a renamed event.
    let data = everruns_core::events::LlmGenerationData::failure(
        vec![],
        vec![],
        "m".into(),
        None,
        "e".into(),
        None,
        None,
    );
    assert_eq!(EventData::from(data).event_type(), "llm.generation");
}
