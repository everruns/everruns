use super::*;
use chrono::TimeZone;
use everruns_contracts::typed_id::SessionId;
use everruns_core::{EventContext, OUTPUT_MESSAGE_DELTA};

fn at(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(1_800_000_000_000 + ms).unwrap()
}

fn obs(event_type: &str, ms: i64) -> Observation<'_> {
    Observation {
        event_type,
        ts: at(ms),
        llm_duration_ms: None,
        ttft_ms: None,
        tool_duration_ms: None,
        input_created_at: None,
    }
}

fn llm(ms: i64, duration: u64, ttft: u64) -> Observation<'static> {
    Observation {
        llm_duration_ms: Some(duration),
        ttft_ms: Some(ttft),
        ..obs(LLM_GENERATION, ms)
    }
}

fn tool(ms: i64, duration: u64) -> Observation<'static> {
    Observation {
        tool_duration_ms: Some(duration),
        ..obs(TOOL_COMPLETED, ms)
    }
}

/// Replay `events` up to the last one, which ends the turn.
fn run(events: &[Observation<'_>]) -> (Vec<PhaseGap>, Option<TurnLatency>) {
    let (end, turn) = events.split_last().expect("a terminal event");
    let outcome = terminal_outcome(end.event_type).expect("terminal");
    summarize(turn, outcome, end.ts)
}

/// A turn with one tool call: input -> reason (LLM asks for a tool) -> act ->
/// reason (final answer). Every hand-off and the model/tool split is measured.
#[test]
fn tool_turn_breaks_down_queue_prep_llm_and_tool_time() {
    let (gaps, latency) = run(&[
        obs(INPUT_MESSAGE, 0),
        obs(TURN_STARTED, 400),   // 400ms pickup
        obs(REASON_STARTED, 900), // 500ms gap: process_input -> reason
        llm(2_000, 1_000, 300),   // request at 1000 (100ms prep), first token 1300
        obs(REASON_COMPLETED, 2_050),
        obs(ACT_STARTED, 3_050), // 1000ms gap: reason -> act
        tool(3_250, 200),
        obs(ACT_COMPLETED, 3_300),
        obs(REASON_STARTED, 5_300), // 2000ms gap: act -> reason
        llm(6_500, 1_000, 200),     // request at 5500 (200ms prep)
        obs(REASON_COMPLETED, 6_550),
        obs(TURN_COMPLETED, 6_600),
    ]);

    assert_eq!(
        gaps,
        vec![
            PhaseGap {
                phase: "reason",
                ms: 500
            },
            PhaseGap {
                phase: "act",
                ms: 1_000
            },
            PhaseGap {
                phase: "reason",
                ms: 2_000
            },
        ]
    );
    assert_eq!(
        latency,
        Some(TurnLatency {
            outcome: "completed",
            pickup_ms: Some(400),
            first_token_ms: Some(1_300),
            turn_ms: 6_200,
            total_ms: Some(6_600),
            llm_ms: 2_000,
            tool_ms: 200,
            phase_gap_ms: 3_500,
            max_phase_gap_ms: 2_000,
            prep_ms: 300,
            overhead_ms: 4_000,
            phases: 3,
            llm_calls: 2,
            tool_calls: 1,
        })
    );
}

#[test]
fn steering_message_mid_turn_keeps_the_original_input_time() {
    let (_, latency) = run(&[
        obs(INPUT_MESSAGE, 0),
        obs(TURN_STARTED, 100),
        obs(INPUT_MESSAGE, 500),
        obs(TURN_FAILED, 1_000),
    ]);
    let latency = latency.expect("summary");
    assert_eq!(latency.outcome, "failed");
    assert_eq!(latency.pickup_ms, Some(100));
    assert_eq!(latency.total_ms, Some(1_000));
}

/// A summary from a partial event stream omits what it cannot know instead of
/// reporting zero or a wrong number.
#[test]
fn missing_input_message_omits_input_relative_fields() {
    let (_, latency) = run(&[
        obs(TURN_STARTED, 0),
        llm(1_000, 500, 100),
        obs(TURN_CANCELLED, 1_200),
    ]);
    let latency = latency.expect("summary");
    assert_eq!(latency.outcome, "cancelled");
    assert_eq!(latency.pickup_ms, None);
    assert_eq!(latency.first_token_ms, None);
    assert_eq!(latency.total_ms, None);
    assert_eq!(latency.turn_ms, 1_200);
}

#[test]
fn terminal_event_without_a_started_turn_reports_nothing() {
    let (_, latency) = run(&[obs(INPUT_MESSAGE, 0), obs(TURN_COMPLETED, 100)]);
    assert_eq!(latency, None);
}

#[test]
fn parallel_tools_never_make_overhead_negative() {
    let (_, latency) = run(&[
        obs(TURN_STARTED, 0),
        tool(100, 900),
        tool(100, 900),
        obs(TURN_COMPLETED, 1_000),
    ]);
    assert_eq!(latency.expect("summary").overhead_ms, 0);
}

/// The listener reads durations straight from the typed event payloads.
#[test]
fn observation_reads_llm_and_tool_durations_from_events() {
    let session = SessionId::new();
    let generation = Event::new(
        session,
        EventContext::empty(),
        everruns_core::events::deserialize_event_data(
            LLM_GENERATION,
            serde_json::json!({
                "messages": [],
                "output": {},
                "metadata": {
                    "model": "llmsim",
                    "success": true,
                    "duration_ms": 1200,
                    "time_to_first_token_ms": 250
                }
            }),
        ),
    );
    let o = Observation::from_event(&generation);
    assert_eq!(o.event_type, LLM_GENERATION);
    assert_eq!(o.llm_duration_ms, Some(1_200));
    assert_eq!(o.ttft_ms, Some(250));

    assert!(
        !TURN_EVENT_TYPES.contains(&OUTPUT_MESSAGE_DELTA),
        "token deltas are not read back"
    );
}

/// The chat API never routes `input.message` through the listener, so the
/// input time is read from the turn's UUIDv7 input message id instead.
#[test]
fn input_time_comes_from_the_uuid_v7_message_id() {
    let (_, latency) = run(&[
        Observation {
            input_created_at: Some(at(0)),
            ..obs(TURN_STARTED, 250)
        },
        obs(TURN_COMPLETED, 1_000),
    ]);
    let latency = latency.expect("summary");
    assert_eq!(latency.pickup_ms, Some(250));
    assert_eq!(latency.total_ms, Some(1_000));
}

#[test]
fn uuid_v7_time_reads_v7_ids_only() {
    let ts = uuid::Timestamp::from_unix(uuid::NoContext, 1_800_000_000, 123_000_000);
    let v7 = uuid::Uuid::new_v7(ts);
    assert_eq!(
        uuid_v7_time(v7),
        DateTime::from_timestamp(1_800_000_000, 123_000_000)
    );
    assert_eq!(uuid_v7_time(uuid::Uuid::new_v4()), None);
}

/// The load-test shape: a worker spread the turn's events over several server
/// replicas, so the replica that sees `turn.completed` saw nothing else. The
/// breakdown is replayed from storage and still counts the model time.
#[tokio::test]
async fn a_turn_whose_events_this_replica_never_saw_is_replayed_from_storage() {
    use crate::storage::{CreateEventRow, CreateSessionRow, StorageBackend};
    use everruns_contracts::typed_id::{PrincipalId, TurnId};

    let db = Arc::new(StorageBackend::test_database());
    let session = db
        .create_session(CreateSessionRow {
            org_id: everruns_core::DEFAULT_ORG_ID,
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    let turn = TurnId::new();
    let other_turn = TurnId::new();
    let store = |turn: &TurnId, event_type: &str, ms: i64, data: serde_json::Value| {
        let db = db.clone();
        let row = CreateEventRow {
            session_id: session,
            event_type: event_type.to_string(),
            ts: at(ms),
            context: serde_json::json!({ "turn_id": turn.to_string() }),
            data,
            metadata: None,
            tags: None,
        };
        async move { db.create_event(row).await.unwrap() }
    };
    let generation = |duration: u64| {
        serde_json::json!({
            "messages": [],
            "output": {},
            "metadata": { "model": "llmsim", "success": true, "duration_ms": duration }
        })
    };
    // An earlier turn in the same session is not mixed in.
    store(&other_turn, TURN_STARTED, -5_000, serde_json::json!({})).await;
    store(&other_turn, LLM_GENERATION, -4_000, generation(900)).await;
    store(&turn, TURN_STARTED, 0, serde_json::json!({})).await;
    store(&turn, REASON_STARTED, 100, serde_json::json!({})).await;
    store(&turn, LLM_GENERATION, 2_100, generation(2_000)).await;
    store(&turn, REASON_COMPLETED, 2_150, serde_json::json!({})).await;

    let mut end = Event::new(
        session,
        EventContext::empty(),
        everruns_core::events::deserialize_event_data(TURN_COMPLETED, serde_json::json!({})),
    );
    end.event_type = TURN_COMPLETED.to_string();
    end.ts = at(2_200);
    end.context.turn_id = Some(turn);

    let (gaps, latency) = TurnLatencyListener::new(db)
        .replay(&end)
        .await
        .expect("a breakdown");
    assert_eq!(latency.llm_ms, 2_000);
    assert_eq!(latency.llm_calls, 1);
    assert_eq!(latency.turn_ms, 2_200);
    assert_eq!(latency.overhead_ms, 200);
    assert_eq!(
        gaps,
        vec![PhaseGap {
            phase: "reason",
            ms: 100
        }]
    );
}
