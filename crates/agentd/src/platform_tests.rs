//! Schedules in shard state: create, limits, fire once, end.

use super::*;
use crate::platform_store::build;

const T0: i64 = 1_800_000_000_000; // 2027-01-15T08:00:00Z

fn session() -> (ShardState, SessionId) {
    use everruns::hosting::spi::session_commands::SessionCommand;
    let id = SessionId::new();
    let mut s = SessionState {
        tenant: Some("org".into()),
        ..Default::default()
    };
    let binding = everruns::hosting::SessionBinding {
        framework: 1,
        behavior: everruns::hosting::AgentBuild::new("agent", "build-1"),
        harness: None,
        environment: None,
        metadata: serde_json::json!(null),
    };
    let create = SessionCommand::Create {
        binding: serde_json::to_value(binding).unwrap(),
    };
    s.command(id, create, T0, EventId::new()).unwrap();
    let mut state = ShardState::default();
    state.sessions.insert(id, s);
    (state, id)
}

fn apply(state: &mut ShardState, session: SessionId, action: Op, now_ms: i64) -> Response {
    let mut changes = Vec::new();
    state.apply(
        Command::Platform {
            session,
            action,
            now_ms,
            seed: EventId::new(),
        },
        &mut changes,
    )
}

fn claim_position(state: &mut ShardState, id: SessionId) -> i32 {
    use everruns_core::events::{EventContext, EventData, EventRequest};
    let reaction = serde_json::from_value(json!({
        "message_id": "message_1", "emoji": "👍",
        "actor": {"kind": "user", "id": "user_1"},
    }))
    .unwrap();
    state
        .sessions
        .get_mut(&id)
        .unwrap()
        .journal(
            EventRequest::new(
                id,
                EventContext::empty(),
                EventData::message_reaction(reaction, true),
            ),
            EventId::new(),
        )
        .sequence
        .unwrap()
}

fn claim_at(expected: i32, approval: i32, key: &str) -> Op {
    Op::ClaimApproval {
        expected_last_sequence: expected,
        approval_sequence: approval,
        turn_id: "turn-current".into(),
        claim_id: key.into(),
    }
}

fn claimed(response: Response, yes: bool, retry: bool) {
    let Response::Value { value } = response else {
        panic!("claim did not return a decision")
    };
    assert_eq!(value, json!({"claimed":yes,"retry":retry}));
}

#[test]
fn approval_claims_compare_positions_and_block_rerendered_cards() {
    let (mut state, id) = session();
    let first = claim_position(&mut state, id);
    claimed(
        apply(&mut state, id, claim_at(first, first, "card-one"), T0),
        true,
        false,
    );
    // Both callbacks may have read the same snapshot before either writes input.
    claimed(
        apply(&mut state, id, claim_at(first, first, "card-two"), T0),
        false,
        false,
    );

    let next_approval = claim_position(&mut state, id);
    let actual = claim_position(&mut state, id);
    claimed(
        apply(
            &mut state,
            id,
            claim_at(next_approval, next_approval, "next"),
            T0,
        ),
        false,
        true,
    );
    assert_eq!(
        state.sessions[&id]
            .platform
            .approval_claim
            .as_ref()
            .unwrap()
            .claim_id,
        "card-one"
    );
    claimed(
        apply(&mut state, id, claim_at(actual, next_approval, "next"), T0),
        true,
        false,
    );
    claimed(
        apply(&mut state, id, claim_at(actual, first, "old"), T0),
        false,
        false,
    );
}

#[test]
fn approval_claim_survives_snapshot_and_event_offload_before_decision_input() {
    let (mut state, id) = session();
    let sequence = claim_position(&mut state, id);
    claimed(
        apply(&mut state, id, claim_at(sequence, sequence, "card"), T0),
        true,
        false,
    );
    let session = state.sessions.get_mut(&id).unwrap();
    assert!(!session.platform.is_empty());
    session.events.clear();
    session.offloaded = sequence;
    let restored: SessionState =
        serde_json::from_value(serde_json::to_value(&*session).unwrap()).unwrap();
    state.sessions.insert(id, restored);
    claimed(
        apply(
            &mut state,
            id,
            claim_at(sequence, sequence, "another-render"),
            T0,
        ),
        false,
        false,
    );
    assert_eq!(
        state.sessions[&id]
            .platform
            .approval_claim
            .as_ref()
            .unwrap()
            .claim_id,
        "card"
    );
}

#[test]
fn malformed_approval_claims_do_not_consume_the_window() {
    let (mut state, id) = session();
    let sequence = claim_position(&mut state, id);
    for (position, key) in [
        (0, "card"),
        (sequence + 1, "card"),
        (sequence, ""),
        (sequence, "https://private-handle.invalid"),
    ] {
        assert!(matches!(
            apply(&mut state, id, claim_at(sequence, position, key), T0),
            Response::Rejected { .. }
        ));
        assert!(state.sessions[&id].platform.approval_claim.is_none());
    }
    claimed(
        apply(&mut state, id, claim_at(sequence, sequence, "valid"), T0),
        true,
        false,
    );
}

fn create(
    state: &mut ShardState,
    session: SessionId,
    cron: Option<&str>,
    at_ms: Option<i64>,
) -> SessionSchedule {
    let schedule = build(
        session,
        "check the queue".into(),
        cron.map(str::to_owned),
        at_ms.map(at),
        "UTC".into(),
        at(T0),
    )
    .unwrap();
    let response = apply(
        state,
        session,
        Op::CreateSchedule {
            schedule: Box::new(schedule.clone()),
        },
        T0,
    );
    assert!(matches!(response, Response::Value { .. }), "{response:?}");
    schedule
}

fn fire(
    state: &mut ShardState,
    session: SessionId,
    id: ScheduleId,
    due_ms: i64,
    now_ms: i64,
) -> serde_json::Value {
    match apply(state, session, Op::FireSchedule { id, due_ms }, now_ms) {
        Response::Value { value } => value,
        other => panic!("{other:?}"),
    }
}

fn stored(state: &ShardState, session: SessionId, id: ScheduleId) -> SessionSchedule {
    state.sessions[&session].platform.schedules[&id.to_string()].clone()
}

#[test]
fn next_cron_follows_the_timezone_and_never_the_past() {
    let (_, session) = session();
    let mut schedule = build(
        session,
        "d".into(),
        Some("0 9 * * *".into()),
        None,
        "America/Chicago".into(),
        at(T0),
    )
    .unwrap();
    // 09:00 Chicago in January is 15:00 UTC.
    let next = next_cron(&schedule, at(T0)).unwrap();
    assert_eq!(next.to_rfc3339(), "2027-01-15T15:00:00+00:00");
    assert_eq!(
        next_cron(&schedule, next).unwrap().to_rfc3339(),
        "2027-01-16T15:00:00+00:00"
    );
    schedule.timezone = "Mars/Olympus".into();
    assert!(next_cron(&schedule, at(T0)).is_err());
}

#[test]
fn an_occurrence_id_is_the_same_wherever_it_is_computed() {
    let (_, session) = session();
    let schedule = build(
        session,
        "d".into(),
        Some("*/5 * * * *".into()),
        None,
        "UTC".into(),
        at(T0),
    )
    .unwrap();
    assert_eq!(occurrence_id(&schedule), occurrence_id(&schedule.clone()));
    let mut next = schedule.clone();
    next.trigger_count += 1;
    assert_ne!(occurrence_id(&schedule), occurrence_id(&next));
}

#[test]
fn a_due_schedule_fires_once_and_moves_on() {
    let (mut state, session) = session();
    let schedule = create(&mut state, session, Some("*/5 * * * *"), None);
    let due = schedule.next_trigger_at.unwrap().timestamp_millis();
    assert_eq!(
        state.sessions[&session].platform.due(due - 1).count(),
        0,
        "not before its time"
    );
    assert_eq!(
        state.sessions[&session]
            .platform
            .due(due)
            .collect::<Vec<_>>(),
        vec![(schedule.id, due)]
    );

    let fired = fire(&mut state, session, schedule.id, due, due + 40);
    assert_eq!(fired["fired"], true, "{fired}");
    assert_eq!(fired["lag_ms"], 40);
    let after = stored(&state, session, schedule.id);
    assert_eq!(after.trigger_count, 1);
    assert_eq!(
        after.next_trigger_at.unwrap().timestamp_millis(),
        due + 5 * 60_000
    );
    let head = state.sessions[&session].last_sequence().unwrap_or(0);
    assert!(head > 0, "the occurrence is in the session's log");

    // The same fire again (a retry, or the old leader's): nothing.
    let again = fire(&mut state, session, schedule.id, due, due + 900);
    assert_eq!(again["fired"], false);
    assert_eq!(stored(&state, session, schedule.id).trigger_count, 1);
    assert_eq!(state.sessions[&session].last_sequence().unwrap_or(0), head);
}

#[test]
fn a_one_shot_fires_and_stops() {
    let (mut state, session) = session();
    let when = T0 + 60_000;
    let schedule = create(&mut state, session, None, Some(when));
    assert_eq!(
        fire(&mut state, session, schedule.id, when, when + 5)["fired"],
        true
    );
    let after = stored(&state, session, schedule.id);
    assert!(!after.enabled);
    assert!(after.next_trigger_at.is_none());
    assert_eq!(state.sessions[&session].platform.due(i64::MAX).count(), 0);
}

#[test]
fn a_schedule_past_its_runs_ends_without_firing() {
    let (mut state, session) = session();
    let schedule = create(&mut state, session, Some("*/5 * * * *"), None);
    apply(
        &mut state,
        session,
        Op::UpdateSchedule {
            id: schedule.id,
            change: ScheduleChange {
                max_runs: Some(Some(1)),
                ..Default::default()
            },
        },
        T0,
    );
    let due = schedule.next_trigger_at.unwrap().timestamp_millis();
    assert_eq!(
        fire(&mut state, session, schedule.id, due, due)["fired"],
        true
    );
    let due = stored(&state, session, schedule.id)
        .next_trigger_at
        .unwrap()
        .timestamp_millis();
    let ended = fire(&mut state, session, schedule.id, due, due);
    assert_eq!(ended["fired"], false);
    assert_eq!(ended["reason"], stop_reason::ENDED);
    assert!(!stored(&state, session, schedule.id).enabled);
}

#[test]
fn a_session_holds_five_active_schedules() {
    let (mut state, session) = session();
    let first = create(&mut state, session, Some("*/5 * * * *"), None);
    for _ in 1..MAX_ACTIVE_SCHEDULES_PER_SESSION {
        create(&mut state, session, Some("*/5 * * * *"), None);
    }
    let extra = build(
        session,
        "x".into(),
        Some("*/5 * * * *".into()),
        None,
        "UTC".into(),
        at(T0),
    )
    .unwrap();
    let create_extra = |state: &mut ShardState| {
        apply(
            state,
            session,
            Op::CreateSchedule {
                schedule: Box::new(extra.clone()),
            },
            T0,
        )
    };
    assert!(matches!(
        create_extra(&mut state),
        Response::Rejected { .. }
    ));

    // Cancelling one makes room; a retried create answers what it made.
    apply(
        &mut state,
        session,
        Op::UpdateSchedule {
            id: first.id,
            change: ScheduleChange {
                enabled: Some(false),
                ..Default::default()
            },
        },
        T0,
    );
    assert_eq!(
        state.sessions[&session]
            .platform
            .due(i64::MAX)
            .filter(|(id, _)| *id == first.id)
            .count(),
        0
    );
    assert!(matches!(create_extra(&mut state), Response::Value { .. }));
    assert!(matches!(create_extra(&mut state), Response::Value { .. }));
    assert_eq!(
        state.sessions[&session].platform.active(),
        MAX_ACTIVE_SCHEDULES_PER_SESSION
    );
}

#[test]
fn a_manual_trigger_runs_now_and_keeps_the_next_time() {
    let (mut state, session) = session();
    let schedule = create(&mut state, session, Some("0 9 * * *"), None);
    let response = apply(
        &mut state,
        session,
        Op::TriggerSchedule { id: schedule.id },
        T0 + 1,
    );
    let Response::Value { value } = response else {
        panic!("{response:?}");
    };
    assert_eq!(value["fired"], true);
    let after = stored(&state, session, schedule.id);
    assert_eq!(after.trigger_count, 1);
    assert_eq!(after.next_trigger_at, schedule.next_trigger_at);
}

#[test]
fn a_moving_session_fires_nothing() {
    let (mut state, session) = session();
    let schedule = create(&mut state, session, Some("*/5 * * * *"), None);
    state.sessions.get_mut(&session).unwrap().placement = Placement::Frozen;
    let due = schedule.next_trigger_at.unwrap().timestamp_millis();
    assert!(matches!(
        apply(
            &mut state,
            session,
            Op::FireSchedule {
                id: schedule.id,
                due_ms: due
            },
            due
        ),
        Response::Frozen
    ));
}

#[test]
fn platform_commands_round_trip() {
    let command = Command::Platform {
        session: SessionId::new(),
        action: Op::UpdateSchedule {
            id: ScheduleId::new(),
            change: ScheduleChange {
                deleted: true,
                ..Default::default()
            },
        },
        now_ms: 1,
        seed: EventId::new(),
    };
    let json = serde_json::to_string(&command).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();
    assert!(
        matches!(back, Command::Platform { action: Op::UpdateSchedule { change, .. }, .. } if change.deleted)
    );
}

#[test]
fn a_schedule_due_soon_is_seen_until_it_fires() {
    let (mut state, id) = session();
    let due = T0 + 60_000;
    let schedule = create(&mut state, id, None, Some(due));
    let platform = &state.sessions[&id].platform;
    assert!(!platform.due_before(due), "not due before its time");
    assert!(platform.due_before(due + 1), "due within the window");

    apply(
        &mut state,
        id,
        Op::FireSchedule {
            id: schedule.id,
            due_ms: due,
        },
        due,
    );
    assert!(
        !state.sessions[&id].platform.due_before(i64::MAX),
        "a one-shot schedule that fired is not due again"
    );
}

#[test]
fn the_control_plane_records_only_reactions_of_the_session() {
    use everruns_core::events::{EventContext, EventData, EventRequest};
    let session = SessionId::new();
    let data: everruns_core::MessageReactionData = serde_json::from_value(json!({
        "message_id": "message_1",
        "emoji": "👍",
        "actor": { "kind": "user", "id": "user_1" },
    }))
    .unwrap();
    let reaction = EventRequest::new(
        session,
        EventContext::empty(),
        EventData::message_reaction(data, true),
    );
    assert!(recordable(session, &reaction).is_ok());
    assert!(recordable(SessionId::new(), &reaction).is_err());
    let mut forged = reaction.clone();
    forged.event_type = "input.message".into();
    assert!(recordable(session, &forged).is_err());
    let mut renamed = reaction;
    renamed.event_type = everruns_core::MESSAGE_REACTION_REMOVED.into();
    assert!(
        recordable(session, &renamed).is_err(),
        "its data names another type"
    );
}

#[test]
fn the_control_plane_records_a_voice_calls_traces() {
    use everruns_core::events::{
        EventContext, EventData, EventRequest, VOICE_INPUT_TRANSCRIPT_DELTA, VoiceSessionEndedData,
        VoiceTranscriptData,
    };
    let session = SessionId::new();
    let ended = EventRequest::new(
        session,
        EventContext::empty(),
        EventData::from(VoiceSessionEndedData {
            voice_connection_id: "vc_1".into(),
            reason: None,
            duration_ms: Some(5),
        }),
    );
    let transcript = EventRequest::new(
        session,
        EventContext::empty(),
        EventData::voice_transcript_event(
            VoiceTranscriptData {
                voice_connection_id: "vc_1".into(),
                phase: None,
                item_id: Some("item_1".into()),
                response_id: None,
                delta: "hi".into(),
                accumulated: "hi".into(),
            },
            VOICE_INPUT_TRANSCRIPT_DELTA,
        ),
    );
    for request in [ended, transcript] {
        // As the control plane sends it: over the wire.
        let request: EventRequest =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        assert!(
            recordable(session, &request).is_ok(),
            "{}",
            request.event_type
        );
    }
}

#[test]
fn a_schedule_stops_when_its_runs_spend_its_cap() {
    let (mut state, session) = session();
    let schedule = create(&mut state, session, Some("*/5 * * * *"), None);
    let cap = |state: &mut ShardState, limit: f64| {
        apply(
            state,
            session,
            Op::UpdateSchedule {
                id: schedule.id,
                change: ScheduleChange {
                    budget_usd: Some(limit),
                    ..Default::default()
                },
            },
            T0,
        )
    };
    cap(&mut state, 0.05);
    let budget = stored(&state, session, schedule.id).budget.unwrap();
    assert_eq!((budget.limit_usd, budget.spent_usd), (0.05, 0.0));
    assert_eq!(budget.status, "active");

    let due = schedule.next_trigger_at.unwrap().timestamp_millis();
    let fired = fire(&mut state, session, schedule.id, due, due);
    assert_eq!(fired["fired"], true, "{fired}");
    let run: MessageId = serde_json::from_value(fired["message_id"].clone()).unwrap();

    // A model call of its run costs it; another message's does not.
    let call = |message: MessageId| {
        everruns_core::events::Event::new(
            session,
            everruns_core::events::EventContext {
                input_message_id: Some(message),
                ..Default::default()
            },
            everruns_core::events::TurnCompletedData::default(),
        )
    };
    let platform = &mut state.sessions.get_mut(&session).unwrap().platform;
    assert!(!platform.charge(&call(MessageId::new()), 40_000));
    assert!(platform.charge(&call(run), 30_000));
    assert!(stored(&state, session, schedule.id).enabled);
    let platform = &mut state.sessions.get_mut(&session).unwrap().platform;
    assert!(platform.charge(&call(run), 30_000));
    let stopped = stored(&state, session, schedule.id);
    let budget = stopped.budget.unwrap();
    assert!((budget.spent_usd - 0.06).abs() < 1e-9);
    assert_eq!(budget.status, "exhausted");
    assert!(!stopped.enabled);
    assert_eq!(stopped.stop_reason.as_deref(), Some("budget_exhausted"));
    // Run by hand, it does not run either.
    let response = apply(
        &mut state,
        session,
        Op::TriggerSchedule { id: schedule.id },
        due + 1,
    );
    let Response::Value { value } = response else {
        panic!("{response:?}");
    };
    assert_eq!(value["fired"], false);

    // A higher cap keeps what it spent.
    cap(&mut state, 1.0);
    let budget = stored(&state, session, schedule.id).budget.unwrap();
    assert_eq!(budget.status, "active");
    assert!((budget.spent_usd - 0.06).abs() < 1e-9);
}
