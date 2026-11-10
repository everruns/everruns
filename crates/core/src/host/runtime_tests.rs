use super::*;
use crate::{DEFAULT_ORG_ID, org_public_id_from_internal};

#[test]
fn invalid_public_id_does_not_fall_back_to_default() {
    for invalid in [
        "",
        "not-an-org",
        "org_short",
        "org_ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
        "ORG_00000000000000000000000000000001",
    ] {
        let mapped = in_process_internal_org_id(invalid);
        assert_ne!(mapped, crate::DEFAULT_ORG_ID);
        assert!(
            mapped >= 2,
            "invalid input {invalid:?} should not map to default"
        );
    }
}

#[test]
fn zero_public_id_does_not_fall_back_to_default() {
    // org_public_id_from_internal never produces this; a hand-crafted
    // all-zeros id is treated as invalid (raw == 0).
    let mapped = in_process_internal_org_id("org_00000000000000000000000000000000");
    assert_ne!(mapped, crate::DEFAULT_ORG_ID);
    assert!(mapped >= 2, "all-zero id should not map to default");
}

#[test]
fn synthetic_public_id_round_trips_with_internal_helper() {
    for internal in [1_i64, 2, 42, 1_000_000, i64::MAX - 1, i64::MAX] {
        let public = org_public_id_from_internal(internal);
        assert_eq!(
            in_process_internal_org_id(&public),
            internal,
            "round-trip failed for internal={internal}"
        );
    }
}

#[test]
fn distinct_synthetic_ids_map_to_distinct_internal_ids() {
    let a = org_public_id_from_internal(7);
    let b = org_public_id_from_internal(8);
    assert_ne!(a, b);
    assert_ne!(
        in_process_internal_org_id(&a),
        in_process_internal_org_id(&b)
    );
}

#[test]
fn high_entropy_uuid_style_id_hashes_into_reserved_range() {
    // First valid UUID-style id whose raw u128 exceeds i64::MAX
    // (top bit of the u128 set). It must hash to a positive i64 that
    // is neither 0 nor DEFAULT_ORG_ID.
    let high = "org_80000000000000000000000000000000";
    let mapped = in_process_internal_org_id(high);
    assert!(mapped >= 2, "mapped id {mapped} must be >= 2");
    assert_ne!(mapped, DEFAULT_ORG_ID);

    // Mapping is deterministic.
    assert_eq!(mapped, in_process_internal_org_id(high));
}

#[test]
fn high_entropy_ids_are_isolated_from_each_other() {
    let a = in_process_internal_org_id("org_80000000000000000000000000000001");
    let b = in_process_internal_org_id("org_80000000000000000000000000000002");
    assert_ne!(a, b);
    assert_ne!(a, DEFAULT_ORG_ID);
    assert_ne!(b, DEFAULT_ORG_ID);
}

#[test]
fn oversize_input_is_bounded_and_does_not_collide_silently() {
    // Inputs past HASH_INPUT_CAP_BYTES are truncated before hashing, so
    // two oversize strings that agree on the first cap bytes map to the
    // same internal id. We only assert the result stays in the safe
    // [2, i64::MAX] range and is not DEFAULT_ORG_ID — the cap exists to
    // bound work, not to widen the input space.
    let oversize = "x".repeat(super::HASH_INPUT_CAP_BYTES * 4);
    let mapped = in_process_internal_org_id(&oversize);
    assert!(mapped >= 2);
    assert_ne!(mapped, DEFAULT_ORG_ID);
}

/// A session store that keeps execution state.
struct StateKeeping {
    inner: crate::host::InMemorySessionStore,
    states: std::sync::Mutex<Vec<SessionExecutionState>>,
}

#[async_trait]
impl SessionStore for StateKeeping {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        self.inner.get_session(session_id).await
    }
}

#[async_trait]
impl SessionMutator for StateKeeping {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.inner.update_session_title(session_id, title).await
    }
}

#[async_trait]
impl RuntimeSessionStore for StateKeeping {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        RuntimeSessionStore::add_session(&self.inner, session).await
    }

    async fn set_execution_state(
        &self,
        _session_id: SessionId,
        state: SessionExecutionState,
    ) -> Result<()> {
        self.states.lock().unwrap().push(state);
        Ok(())
    }
}

/// The turn's state changes reach a store that keeps them (a control plane
/// whose clients read session state), and a missing session still fails.
#[tokio::test]
async fn execution_state_reaches_a_store_that_keeps_it() {
    let store = Arc::new(StateKeeping {
        inner: crate::host::InMemorySessionStore::new(),
        states: Default::default(),
    });
    let session = SessionId::new();
    RuntimeSessionStore::add_session(
        store.as_ref(),
        ExecutionSession::new(
            session,
            everruns_contracts::typed_id::WorkspaceId::new(),
            everruns_contracts::typed_id::HarnessId::new(),
        ),
    )
    .await
    .unwrap();
    let runtime = InProcessRuntimeBuilder::new()
        .model_spec(everruns_contracts::model_spec::ModelSpec::on(
            "test", "model",
        ))
        .backends(HostBackends::in_memory().with_session_store(store.clone()))
        .build()
        .await
        .unwrap();
    RuntimeHostAdapter::set_session_status(&runtime, 1, session, SessionExecutionState::Idle)
        .await
        .unwrap();
    assert_eq!(
        *store.states.lock().unwrap(),
        vec![SessionExecutionState::Idle]
    );
    assert!(
        RuntimeHostAdapter::set_session_status(
            &runtime,
            1,
            SessionId::new(),
            SessionExecutionState::Idle
        )
        .await
        .is_err()
    );
    assert_eq!(store.states.lock().unwrap().len(), 1);
}
