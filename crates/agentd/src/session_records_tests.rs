//! A session's resources, leases and dreaming state in shard state.

use super::*;
use crate::state::ShardState;

const T0: i64 = 1_800_000_000_000;

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
        metadata: json!(null),
    };
    let create = SessionCommand::Create {
        binding: serde_json::to_value(binding).unwrap(),
    };
    s.command(id, create, T0, EventId::new()).unwrap();
    let mut state = ShardState::default();
    state.sessions.insert(id, s);
    (state, id)
}

fn apply(state: &mut ShardState, session: SessionId, op: RecordOp, now_ms: i64) -> Value {
    let mut changes = Vec::new();
    match state.apply(
        Command::Platform {
            session,
            action: Op::Records(op),
            now_ms,
            seed: EventId::new(),
        },
        &mut changes,
    ) {
        Response::Value { value } => value,
        other => panic!("{other:?}"),
    }
}

fn fed(state: &ShardState) -> Vec<Record> {
    state
        .feed
        .records
        .iter()
        .map(|r| r.record.clone())
        .collect()
}

fn lease(session: SessionId, seconds: u32) -> UpsertLeasedResource {
    UpsertLeasedResource {
        session_id: session,
        provider: "daytona".into(),
        resource_type: "sandbox".into(),
        external_id: "sbx-1".into(),
        display_name: Some("box".into()),
        owner_user_id: None,
        connection_id: None,
        lease_duration_seconds: seconds,
        metadata: json!({ "a": 1 }),
    }
}

#[test]
fn a_resource_is_kept_updated_removed_and_reported_on_the_feed() {
    let (mut state, session) = session();
    let entry = RegisterSessionResource {
        session_id: session,
        resource_id: "sandbox-1".into(),
        kind: "sandbox".into(),
        display_name: "Sandbox".into(),
        status: SessionResourceStatus::Active,
        metadata: json!({}),
    };
    let made = apply(
        &mut state,
        session,
        RecordOp::RegisterResource { entry },
        T0,
    );
    assert_eq!(made["resource_id"], "sandbox-1");
    let updated = apply(
        &mut state,
        session,
        RecordOp::ResourceStatus {
            resource_id: "sandbox-1".into(),
            status: SessionResourceStatus::Completed,
        },
        T0 + 1000,
    );
    assert_eq!(updated["status"], "completed");
    assert_eq!(updated["created_at"], made["created_at"]);
    let missing = apply(
        &mut state,
        session,
        RecordOp::ResourceStatus {
            resource_id: "nope".into(),
            status: SessionResourceStatus::Failed,
        },
        T0,
    );
    assert!(missing.is_null());
    let removed = apply(
        &mut state,
        session,
        RecordOp::DeregisterResource {
            resource_id: "sandbox-1".into(),
        },
        T0 + 2000,
    );
    assert_eq!(removed, json!(true));
    assert!(
        state.sessions[&session]
            .platform
            .records
            .resources
            .is_empty()
    );

    let reported: Vec<Value> = fed(&state)
        .into_iter()
        .map(|record| match record {
            Record::SessionResource {
                resource_id,
                resource,
                ..
            } => {
                assert_eq!(resource_id, "sandbox-1");
                resource
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(reported.len(), 3);
    assert_eq!(reported[1]["status"], "completed");
    assert!(reported[2].is_null(), "a removal reports null");
}

#[test]
fn a_renewed_lease_extends_the_same_one_and_a_release_keeps_it_visible() {
    let (mut state, session) = session();
    let first_id = LeasedResourceId::new();
    let first = apply(
        &mut state,
        session,
        RecordOp::UpsertLease {
            input: lease(session, 60),
            id: first_id,
        },
        T0,
    );
    let renewed = apply(
        &mut state,
        session,
        RecordOp::UpsertLease {
            input: lease(session, 120),
            id: LeasedResourceId::new(),
        },
        T0 + 30_000,
    );
    assert_eq!(renewed["id"], first["id"], "a renewal keeps its id");
    assert_eq!(renewed["id"], json!(first_id));
    let expires: DateTime<Utc> =
        serde_json::from_value(renewed["lease_expires_at"].clone()).unwrap();
    assert_eq!(expires, at(T0 + 30_000 + 120_000));
    let released = apply(
        &mut state,
        session,
        RecordOp::ReleaseLease {
            provider: "daytona".into(),
            resource_type: "sandbox".into(),
            external_id: "sbx-1".into(),
        },
        T0 + 40_000,
    );
    assert_eq!(released["status"], "released");
    let records = &state.sessions[&session].platform.records;
    assert_eq!(records.leases.len(), 1);
    let registered = &records.resources[&first_id.to_string()];
    assert_eq!(registered.status, SessionResourceStatus::Released);
    assert_eq!(
        registered.metadata[everruns_contracts::runtime::LEASED_RESOURCE_EXTERNAL_ID_KEY],
        "sbx-1",
        "a lease registers a resource ownership checks find"
    );
    let leases = fed(&state)
        .into_iter()
        .filter(|record| matches!(record, Record::Lease { .. }))
        .count();
    assert_eq!(leases, 3, "each change goes on the feed for the sweeper");
}

#[test]
fn a_lease_or_resource_of_another_session_is_refused() {
    let (mut state, session) = session();
    let mut changes = Vec::new();
    let response = state.apply(
        Command::Platform {
            session,
            action: Op::Records(RecordOp::UpsertLease {
                input: lease(SessionId::new(), 60),
                id: LeasedResourceId::new(),
            }),
            now_ms: T0,
            seed: EventId::new(),
        },
        &mut changes,
    );
    assert!(
        matches!(response, Response::Rejected { .. }),
        "{response:?}"
    );
    assert!(state.feed.records.is_empty());
}

#[test]
fn dreaming_state_is_a_compare_exchange_and_its_due_job_is_found() {
    let (mut state, session) = session();
    let job = everruns_core::engine::dreaming::runner::job_key(session);
    let dream = |expected: Option<i64>, due_at: Option<i64>| RecordOp::Dream {
        key: job.clone(),
        expected,
        value: "state".into(),
        owned: true,
        due_at,
    };
    assert_eq!(
        apply(&mut state, session, dream(None, Some(100)), T0),
        json!(true)
    );
    assert_eq!(
        apply(&mut state, session, dream(None, Some(100)), T0),
        json!(false),
        "a create finds the row there"
    );
    assert_eq!(
        apply(&mut state, session, dream(Some(7), Some(100)), T0),
        json!(false),
        "a stale revision changes nothing"
    );
    let platform = &state.sessions[&session].platform;
    assert!(platform.dream_due(session, 100));
    assert!(!platform.dream_due(session, 99));
    assert_eq!(
        apply(&mut state, session, dream(Some(1), None), T0),
        json!(true)
    );
    let platform = &state.sessions[&session].platform;
    assert_eq!(platform.records.dreaming[&job].revision, 2);
    assert!(!platform.dream_due(session, i64::MAX));
}

#[test]
fn a_subagent_is_reported_once_with_its_create() {
    use everruns_contracts::dataplane::SubagentOrigin;
    let (mut state, session) = session();
    let origin = SubagentOrigin {
        parent_session_id: SessionId::new().to_string(),
        harness_id: "harness".into(),
        agent_id: None,
        title: Some("Researcher".into()),
        locale: None,
    };
    for _ in 0..2 {
        let mut changes = Vec::new();
        let response = state.apply(
            Command::Platform {
                session,
                action: Op::Subagent {
                    origin: origin.clone(),
                },
                now_ms: T0,
                seed: EventId::new(),
            },
            &mut changes,
        );
        assert!(matches!(response, Response::Ok), "{response:?}");
    }
    assert_eq!(
        state.sessions[&session].platform.origin.as_ref(),
        Some(&origin)
    );
    assert_eq!(
        fed(&state),
        vec![Record::Subagent {
            session_id: session.to_string(),
            origin,
        }],
        "a retried create reports nothing new"
    );
}
