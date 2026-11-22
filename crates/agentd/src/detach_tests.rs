//! Detached call records in a session's authority.

use super::*;
use crate::authority::Holder;
use crate::state::ShardState;

const NOW: i64 = 1_000_000;

fn held() -> (ShardState, SessionId) {
    let session = SessionId::new();
    let mut state = ShardState::default();
    let mut s = SessionState::default();
    s.authority.binding = Some(everruns::hosting::SessionBinding {
        framework: 1,
        behavior: everruns::hosting::AgentBuild::new("agent", "build-1"),
        harness: None,
        environment: None,
        metadata: json!(null),
    });
    s.authority.fence = 1;
    s.authority.holder = Some(Holder {
        name: "exec".into(),
        node: 1,
        fence: 1,
        expires_ms: NOW + 10_000,
    });
    state.sessions.insert(session, s);
    (state, session)
}

fn call(state: &mut ShardState, session: SessionId, op: CallOp, now_ms: i64) -> Response {
    let mut changes = Vec::new();
    state.apply(
        Command::Platform {
            session,
            action: crate::platform::Op::Call(op),
            now_ms,
            seed: EventId::new(),
        },
        &mut changes,
    )
}

fn detach(state: &mut ShardState, session: SessionId, id: &str) {
    let op = CallOp::Detach {
        call: id.into(),
        turn: TurnId::new(),
        holder: "runner".into(),
        ttl_ms: 30_000,
    };
    assert!(matches!(
        call(state, session, op, NOW),
        Response::Value { .. }
    ));
}

/// The turn parks: its epoch is released and nothing else is owed.
fn park(state: &mut ShardState, session: SessionId) {
    let a = &mut state.sessions.get_mut(&session).unwrap().authority;
    a.holder = None;
    a.runnable = false;
}

fn authority(state: &ShardState, session: SessionId) -> &Authority {
    &state.sessions[&session].authority
}

#[test]
fn a_live_call_holds_the_session_until_it_settles() {
    let (mut state, session) = held();
    detach(&mut state, session, "call-1");
    detach(&mut state, session, "call-2");
    park(&mut state, session);
    // Input arrives meanwhile: it waits for the calls.
    state.sessions.get_mut(&session).unwrap().authority.runnable = true;
    assert!(!authority(&state, session).owed(NOW + 1));
    assert!(
        state
            .sessions
            .get_mut(&session)
            .unwrap()
            .claim("e", 2, 5_000, NOW + 1)
            .is_none()
    );

    let settle = |id: &str| CallOp::Settle {
        call: id.into(),
        holder: "runner".into(),
    };
    // Another runner cannot settle it.
    let other = CallOp::Settle {
        call: "call-1".into(),
        holder: "intruder".into(),
    };
    let Response::Value { value } = call(&mut state, session, other, NOW + 2) else {
        panic!()
    };
    assert_eq!(value["settled"], false);
    let Response::Value { value } = call(&mut state, session, settle("call-1"), NOW + 2) else {
        panic!()
    };
    assert_eq!(value["runnable"], false, "call-2 still runs");
    let Response::Value { value } = call(&mut state, session, settle("call-2"), NOW + 3) else {
        panic!()
    };
    assert_eq!(value["runnable"], true);
    assert!(authority(&state, session).owed(NOW + 3));
    assert!(
        state
            .sessions
            .get_mut(&session)
            .unwrap()
            .claim("e", 2, 5_000, NOW + 3)
            .is_some()
    );
}

#[test]
fn an_expired_call_resumes_its_turn() {
    let (mut state, session) = held();
    detach(&mut state, session, "call-1");
    park(&mut state, session);
    assert!(!authority(&state, session).owed(NOW + 29_999));
    // Renewed by its runner: still live past the first expiry.
    let renew = CallOp::Renew {
        holder: "runner".into(),
        calls: vec!["call-1".into(), "gone".into()],
        ttl_ms: 30_000,
    };
    let Response::Value { value } = call(&mut state, session, renew, NOW + 20_000) else {
        panic!()
    };
    assert_eq!(value["held"], json!(["call-1"]));
    assert!(!authority(&state, session).owed(NOW + 40_000));
    // Its runner died: once the record expires the session owes work, and
    // the claim drops the record and resumes the turn.
    assert!(authority(&state, session).owed(NOW + 50_001));
    let s = state.sessions.get_mut(&session).unwrap();
    let generation = s.authority.resume_generation;
    assert!(s.claim("e", 2, 5_000, NOW + 50_001).is_some());
    assert!(s.authority.detached.is_empty());
    assert_eq!(s.authority.resume_generation, generation + 1);
}

#[test]
fn a_call_appends_only_after_its_turn_parked() {
    let (mut state, session) = held();
    detach(&mut state, session, "call-1");
    let event = |id| CallOp::Event {
        call: "call-1".into(),
        holder: "runner".into(),
        event_id: id,
        request: Box::new(EventRequest::new(
            session,
            EventContext::default(),
            everruns_core::events::TurnCompletedData::default(),
        )),
    };
    let id = EventId::new();
    assert!(matches!(
        call(&mut state, session, event(id), NOW + 1),
        Response::Rejected { .. }
    ));
    park(&mut state, session);
    let before = state.sessions[&session].last_sequence();
    assert!(matches!(
        call(&mut state, session, event(id), NOW + 2),
        Response::Value { .. }
    ));
    // A retry finds its event; nothing appends twice.
    assert!(matches!(
        call(&mut state, session, event(id), NOW + 3),
        Response::Value { .. }
    ));
    assert_eq!(
        state.sessions[&session].last_sequence(),
        Some(before.unwrap_or(0) + 1)
    );
    // Once its record is gone, nothing appends.
    let settle = CallOp::Settle {
        call: "call-1".into(),
        holder: "runner".into(),
    };
    call(&mut state, session, settle, NOW + 4);
    assert!(matches!(
        call(&mut state, session, event(EventId::new()), NOW + 5),
        Response::Rejected { .. }
    ));
}

#[test]
fn a_call_detaches_only_while_its_turn_runs() {
    let (mut state, session) = held();
    park(&mut state, session);
    let op = CallOp::Detach {
        call: "late".into(),
        turn: TurnId::new(),
        holder: "runner".into(),
        ttl_ms: 30_000,
    };
    assert!(matches!(
        call(&mut state, session, op, NOW),
        Response::Rejected { .. }
    ));
}

#[test]
fn call_commands_round_trip() {
    let command = Command::Platform {
        session: SessionId::new(),
        action: crate::platform::Op::Call(CallOp::Drop {
            holder: "runner".into(),
            calls: vec!["a".into()],
        }),
        now_ms: 1,
        seed: EventId::new(),
    };
    let json = serde_json::to_string(&command).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();
    assert!(matches!(
        back,
        Command::Platform {
            action: crate::platform::Op::Call(CallOp::Drop { .. }),
            ..
        }
    ));
}
