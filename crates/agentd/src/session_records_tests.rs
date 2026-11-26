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

fn claim(state: &mut ShardState, session: SessionId, now_ms: i64) -> Value {
    apply(
        state,
        session,
        RecordOp::ClaimLease {
            provider: "daytona".into(),
            resource_type: "sandbox".into(),
            external_id: "sbx-1".into(),
            stale_after_seconds: 300,
        },
        now_ms,
    )
}

fn cleaned(
    state: &mut ShardState,
    session: SessionId,
    claimed: &Value,
    error: Option<&str>,
    now_ms: i64,
) -> Value {
    let claimed: LeasedResource = serde_json::from_value(claimed.clone()).unwrap();
    apply(
        state,
        session,
        RecordOp::LeaseCleaned {
            provider: claimed.provider,
            resource_type: claimed.resource_type,
            external_id: claimed.external_id,
            claimed_at: claimed.cleanup_started_at.unwrap(),
            error: error.map(str::to_owned),
            retry_after_seconds: 60,
        },
        now_ms,
    )
}

fn lease_of(state: &ShardState, session: SessionId) -> LeasedResource {
    state.sessions[&session]
        .platform
        .records
        .leases
        .values()
        .next()
        .cloned()
        .unwrap()
}

/// The lease sweep claims a lease only once it expired, one claim at a
/// time; a release ends it and its resource, and the feed shows each step.
#[test]
fn an_expired_lease_is_claimed_once_and_released() {
    let (mut state, session) = session();
    let upsert = RecordOp::UpsertLease {
        input: lease(session, 60),
        id: LeasedResourceId::new(),
    };
    apply(&mut state, session, upsert, T0);
    assert_eq!(
        claim(&mut state, session, T0 + 59_000),
        Value::Null,
        "not expired yet"
    );
    let records = &state.sessions[&session].platform.records;
    assert_eq!(records.due_leases(at(T0 + 61_000), 300).count(), 1);

    let claimed = claim(&mut state, session, T0 + 61_000);
    assert_eq!(claimed["status"], "cleaning");
    assert_eq!(claimed["cleanup_attempts"], 1);
    assert_eq!(
        claim(&mut state, session, T0 + 62_000),
        Value::Null,
        "claimed already"
    );

    let released = cleaned(&mut state, session, &claimed, None, T0 + 63_000);
    assert_eq!(released["status"], "released");
    assert_eq!(
        lease_of(&state, session).status,
        LeasedResourceStatus::Released
    );
    let resource = &state.sessions[&session].platform.records.resources
        [&lease_of(&state, session).id.to_string()];
    assert_eq!(resource.status, SessionResourceStatus::Released);
    assert_eq!(
        claim(&mut state, session, T0 + 999_000),
        Value::Null,
        "released is never due"
    );
    let leases_fed = fed(&state)
        .iter()
        .filter(|r| matches!(r, Record::Lease { .. }))
        .count();
    assert_eq!(
        leases_fed, 3,
        "upsert, claim and release are each on the feed"
    );
}

/// A failed release is due again after its retry; a claim nobody finished
/// is due again once stale; an outcome for an older claim changes nothing.
#[test]
fn a_failed_or_abandoned_claim_comes_due_again() {
    let (mut state, session) = session();
    let upsert = RecordOp::UpsertLease {
        input: lease(session, 60),
        id: LeasedResourceId::new(),
    };
    apply(&mut state, session, upsert, T0);
    let first = claim(&mut state, session, T0 + 61_000);
    let failed = cleaned(
        &mut state,
        session,
        &first,
        Some("provider down"),
        T0 + 62_000,
    );
    assert_eq!(failed["status"], "cleanup_failed");
    assert_eq!(failed["last_cleanup_error"], "provider down");
    assert_eq!(
        claim(&mut state, session, T0 + 100_000),
        Value::Null,
        "retry not due yet"
    );

    // Claimed again after the retry, then abandoned: due again past stale.
    let second = claim(&mut state, session, T0 + 123_000);
    assert_eq!(second["cleanup_attempts"], 2);
    assert_eq!(
        claim(&mut state, session, T0 + 123_000 + 299_000),
        Value::Null
    );
    let third = claim(&mut state, session, T0 + 123_000 + 301_000);
    assert_eq!(third["cleanup_attempts"], 3);

    // The abandoned sweep reports late: its claim is not the lease's now.
    assert_eq!(
        cleaned(&mut state, session, &second, None, T0 + 500_000),
        Value::Null
    );
    assert_eq!(
        lease_of(&state, session).status,
        LeasedResourceStatus::Cleaning
    );
    assert_eq!(
        cleaned(&mut state, session, &third, None, T0 + 501_000)["status"],
        "released"
    );
}

#[test]
fn code_journal_claims_are_fenced_private_and_survive_snapshot() {
    use everruns_contracts::runtime::durable_tools::ToolCallClaimResult;
    let (mut state, session) = session();
    let token = uuid::Uuid::new_v4();
    let claim = || RecordOp::CodeClaim {
        key: "turn:effect:0".into(),
        fingerprint: "abc".into(),
        token,
    };
    let first = apply(&mut state, session, claim(), T0);
    assert!(matches!(
        serde_json::from_value::<ToolCallClaimResult>(first).unwrap(),
        ToolCallClaimResult::Claimed { .. }
    ));
    let retry = apply(&mut state, session, claim(), T0);
    assert!(matches!(
        serde_json::from_value::<ToolCallClaimResult>(retry).unwrap(),
        ToolCallClaimResult::AlreadyRunning { .. }
    ));
    assert_eq!(
        apply(
            &mut state,
            session,
            RecordOp::CodeSettle {
                key: "turn:effect:0".into(),
                token: uuid::Uuid::new_v4(),
                value: json!("wrong")
            },
            T0
        ),
        json!(false)
    );
    assert_eq!(
        apply(
            &mut state,
            session,
            RecordOp::CodeSettle {
                key: "turn:effect:0".into(),
                token,
                value: json!("private snapshot")
            },
            T0
        ),
        json!(true)
    );
    let bytes = serde_json::to_vec(&state.sessions[&session].platform.records).unwrap();
    state.sessions.get_mut(&session).unwrap().platform.records =
        serde_json::from_slice(&bytes).unwrap();
    let settled = apply(&mut state, session, claim(), T0);
    assert!(
        matches!(serde_json::from_value::<ToolCallClaimResult>(settled).unwrap(), ToolCallClaimResult::AlreadySettled { result_json, .. } if result_json == json!("private snapshot"))
    );
    let changed = apply(
        &mut state,
        session,
        RecordOp::CodeClaim {
            key: "turn:effect:0".into(),
            fingerprint: "different".into(),
            token,
        },
        T0,
    );
    assert!(matches!(
        serde_json::from_value::<ToolCallClaimResult>(changed).unwrap(),
        ToolCallClaimResult::DeterminismViolation { .. }
    ));
    assert!(fed(&state).iter().all(|record| {
        !serde_json::to_string(record)
            .unwrap()
            .contains("private snapshot")
    }));
}
