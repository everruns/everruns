//! Schedules in cell state: create, limits, fire once, end.

use super::*;
use crate::platform_store::build;

const T0: i64 = 1_800_000_000_000; // 2027-01-15T08:00:00Z

fn session() -> (CellState, SessionId) {
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
    let mut state = CellState::default();
    state.sessions.insert(id, s);
    (state, id)
}

fn apply(state: &mut CellState, session: SessionId, action: Op, now_ms: i64) -> Response {
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

fn create(
    state: &mut CellState,
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
    state: &mut CellState,
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

fn stored(state: &CellState, session: SessionId, id: ScheduleId) -> SessionSchedule {
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
    let create_extra = |state: &mut CellState| {
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
