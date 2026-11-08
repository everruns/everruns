use super::*;
use everruns_contracts::typed_id::{HarnessId, SessionId};
use std::sync::atomic::{AtomicU32, Ordering};

fn session() -> ExecutionSession {
    ExecutionSession::with_own_workspace(SessionId::new(), HarnessId::new())
}

/// Read `key` through the memo, counting how often it reaches the store.
async fn read(reads: &PhaseReads, key: Key, calls: &AtomicU32) -> Result<Option<ExecutionSession>> {
    reads
        .read_through(sessions, key, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(session()))
        })
        .await
}

/// Read `key` as `session` does.
async fn read_in_turn(
    reads: &PhaseReads,
    key: Key,
    calls: &AtomicU32,
) -> Result<Option<ExecutionSession>> {
    reads
        .session_with(key, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some(session()))
        })
        .await
}

#[tokio::test]
async fn a_later_phase_starts_from_the_turns_reads() {
    let turn = TurnSlot::default();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());

    let reason = PhaseReads::new().with_turn(turn.clone());
    read_in_turn(&reason, key, &calls).await.unwrap();
    reason.note_event(crate::core::REASON_STARTED);

    // The act phase's setup reuses what the reason phase read.
    let act = PhaseReads::new().with_turn(turn.clone());
    read_in_turn(&act, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(act.with_state(|s| s.from_turn), 1);

    // Once the phase started, the turn's session still answers.
    act.note_event(crate::core::ACT_STARTED);
    read_in_turn(&act, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // A session write drops the turn's copy: the next read, during setup
    // or after it, fetches once and keeps it again.
    let next = PhaseReads::new().with_turn(turn);
    next.invalidate_session(key.0, key.1);
    read_in_turn(&next, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    next.note_event(crate::core::REASON_STARTED);
    next.invalidate_session(key.0, key.1);
    read_in_turn(&next, key, &calls).await.unwrap();
    read_in_turn(&next, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn after_setup_a_status_write_is_seen_without_a_fetch() {
    use everruns_contracts::runtime::session::SessionExecutionState;
    let turn = TurnSlot::default();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    let reason = PhaseReads::new().with_turn(turn);
    read_in_turn(&reason, key, &calls).await.unwrap();
    reason.note_event(crate::core::REASON_STARTED);

    reason.session_status_written(key.0, key.1, SessionExecutionState::WaitingForToolResults);
    let got = read_in_turn(&reason, key, &calls).await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(got.status, SessionExecutionState::WaitingForToolResults);
}

#[tokio::test]
async fn without_a_turn_reads_after_setup_stay_fresh() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    read_in_turn(&reads, key, &calls).await.unwrap();
    reads.note_event(crate::core::REASON_STARTED);
    read_in_turn(&reads, key, &calls).await.unwrap();
    read_in_turn(&reads, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_status_write_keeps_the_turns_reads_with_the_new_status() {
    use everruns_contracts::runtime::session::SessionExecutionState;
    let turn = TurnSlot::default();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());

    let reason = PhaseReads::new().with_turn(turn.clone());
    read_in_turn(&reason, key, &calls).await.unwrap();
    reason.session_status_written(key.0, key.1, SessionExecutionState::Active);

    // The next phase's setup is answered from the turn, status patched.
    let act = PhaseReads::new().with_turn(turn);
    let got = read_in_turn(&act, key, &calls).await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(got.status, SessionExecutionState::Active);
}

#[tokio::test]
async fn setup_reads_reach_the_store_once() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    let first = read(&reads, key, &calls).await.unwrap().unwrap();
    let again = read(&reads, key, &calls).await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.id, again.id);
    // Another org's record with the same id is a different read.
    read(&reads, (2, key.1), &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_read_in_flight_is_awaited_not_repeated() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    let (open, opened) = tokio::sync::oneshot::channel::<()>();
    // The first read stays in flight until the second has started waiting.
    let first = reads.read_through(sessions, key, || async {
        calls.fetch_add(1, Ordering::SeqCst);
        opened.await.ok();
        Ok(Some(session()))
    });
    let second = async {
        tokio::task::yield_now().await;
        read(&reads, key, &calls).await
    };
    let release = async {
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        open.send(()).ok();
    };
    let (first, second, ()) = tokio::join!(first, second, release);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.unwrap().unwrap().id, second.unwrap().unwrap().id);
}

#[tokio::test]
async fn a_seed_does_not_replace_a_fetched_record() {
    let reads = PhaseReads::new();
    let fetched = session();
    let key = (1, fetched.id.uuid());
    let stored = fetched.clone();
    reads
        .read_through(sessions, key, || async { Ok(Some(stored)) })
        .await
        .unwrap();
    let mut other = fetched.clone();
    other.locale = Some("uk".to_string());
    reads.seed(1, &other, None);
    let calls = AtomicU32::new(0);
    let got = read(&reads, key, &calls).await.unwrap().unwrap();
    assert_eq!(got.locale, fetched.locale);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_phase_start_event_ends_memoizing() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    read(&reads, key, &calls).await.unwrap();
    // Events before the start event leave setup running.
    reads.note_event(crate::core::INPUT_MESSAGE);
    read(&reads, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    reads.note_event(crate::core::REASON_STARTED);
    read(&reads, key, &calls).await.unwrap();
    read(&reads, key, &calls).await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "reads after setup are fresh"
    );
}

#[tokio::test]
async fn a_session_write_drops_the_memoized_session() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    read(&reads, key, &calls).await.unwrap();
    reads.invalidate_session(key.0, key.1);
    read(&reads, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn failed_reads_are_not_memoized() {
    let reads = PhaseReads::new();
    let key = (1, Uuid::now_v7());
    let failed = reads
        .read_through(sessions, key, || async {
            Err(everruns_contracts::error::AgentLoopError::store("down"))
        })
        .await;
    assert!(failed.is_err());
    let calls = AtomicU32::new(0);
    read(&reads, key, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_seeded_session_needs_no_fetch() {
    let reads = PhaseReads::new();
    let seeded = session();
    reads.seed(1, &seeded, None);
    let calls = AtomicU32::new(0);
    let got = read(&reads, (1, seeded.id.uuid()), &calls)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.id, seeded.id);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn lifecycle_and_definition_setup_share_one_source_read() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let id = everruns_contracts::typed_id::AgentId::new();
    let key = (1, id.uuid());
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok((
            Ok(Some(AgentDefinition::new(id, "agent", "pinned prompt"))),
            None,
        ))
    };
    assert!(reads.blocker(agents, key, fetch).await.unwrap().is_none());
    assert_eq!(
        reads
            .definition(agents, key, fetch)
            .await
            .unwrap()
            .unwrap()
            .system_prompt,
        "pinned prompt"
    );
    assert!(reads.blocker(agents, key, fetch).await.unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    reads.definition(agents, (2, key.1), fetch).await.unwrap();
    PhaseReads::new()
        .definition(agents, key, fetch)
        .await
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "org and phase boundaries remain isolated"
    );
}

#[tokio::test]
async fn missing_harness_is_a_cached_negative_definition_and_deleted_blocker() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok((Ok(None), Some(DependencyBlocker::HarnessDeleted)))
    };
    assert!(
        reads
            .definition(harnesses, key, fetch)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        reads.blocker(harnesses, key, fetch).await.unwrap(),
        Some(DependencyBlocker::HarnessDeleted)
    ));
    assert!(
        reads
            .definition(harnesses, key, fetch)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn projection_failures_retry_without_losing_the_successful_lifecycle_probe() {
    use everruns_contracts::error::AgentLoopError;
    for blocker in [
        None,
        Some(DependencyBlocker::AgentArchived),
        Some(DependencyBlocker::AgentDeleted),
    ] {
        let reads = PhaseReads::new();
        let calls = AtomicU32::new(0);
        let key = (1, Uuid::now_v7());
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((
                Err(AgentLoopError::config("original projection refusal")),
                blocker,
            ))
        };
        let got = reads.blocker(agents, key, fetch).await.unwrap();
        assert_eq!(
            got.map(DependencyBlocker::message),
            blocker.map(DependencyBlocker::message)
        );
        for _ in 0..2 {
            let error = reads.definition(agents, key, fetch).await.unwrap_err();
            assert!(
                matches!(error, AgentLoopError::Configuration(message) if message == "original projection refusal")
            );
            reads.blocker(agents, key, fetch).await.unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            reads.with_state(|state| state.saved),
            2,
            "failed projections are not saved reads"
        );
        let recovered = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((
                Ok(Some(AgentDefinition::new(
                    everruns_contracts::typed_id::AgentId::from_uuid(key.1),
                    "recovered",
                    "valid projection",
                ))),
                None,
            ))
        };
        let got = reads
            .definition(agents, key, recovered)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.system_prompt, "valid projection");
        assert!(
            reads
                .blocker(agents, key, recovered)
                .await
                .unwrap()
                .is_none()
        );
        reads.definition(agents, key, recovered).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            4,
            "recovery becomes a successful memoized projection"
        );
    }
}

#[tokio::test]
async fn failed_combined_source_reads_are_never_cached() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Err(everruns_contracts::error::AgentLoopError::store(
            "source unavailable",
        ))
    };
    assert!(reads.blocker(agents, key, fetch).await.is_err());
    assert!(reads.definition(agents, key, fetch).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(reads.with_state(|state| state.agents.is_empty()));
}

#[tokio::test]
async fn expired_definitions_and_blockers_are_refetched() {
    let reads = PhaseReads::new();
    let key = (1, Uuid::now_v7());
    let calls = AtomicU32::new(0);
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok((Ok(Some(HarnessDefinition::new("harness", "prompt"))), None))
    };
    reads.definition(harnesses, key, fetch).await.unwrap();
    reads
        .with_state(|state| state.harnesses.get_mut(&key).unwrap().at = Instant::now() - FRESH_FOR);
    reads.blocker(harnesses, key, fetch).await.unwrap();
    reads.definition(harnesses, key, fetch).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn phase_start_discards_an_in_flight_source_read_and_later_seeds() {
    for event in [crate::core::REASON_STARTED, crate::core::ACT_STARTED] {
        let reads = PhaseReads::new();
        let id = everruns_contracts::typed_id::AgentId::new();
        let definition = AgentDefinition::new(id, "agent", "pinned");
        let key = (1, id.uuid());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let pending_reads = reads.clone();
        let pending_definition = definition.clone();
        let pending = tokio::spawn(async move {
            pending_reads
                .definition(agents, key, || async {
                    started_tx.send(()).unwrap();
                    finish_rx.await.unwrap();
                    Ok((Ok(Some(pending_definition)), None))
                })
                .await
        });
        started_rx.await.unwrap();
        reads.note_event(event);
        finish_tx.send(()).unwrap();
        assert_eq!(pending.await.unwrap().unwrap().unwrap().id, id);
        reads.seed(1, &session(), Some(&definition));
        assert!(reads.with_state(|state| state.agents.is_empty() && state.sessions.is_empty()));
        let calls = AtomicU32::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((Ok(Some(definition.clone())), None))
        };
        reads.definition(agents, key, fetch).await.unwrap();
        reads.blocker(agents, key, fetch).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn seeded_batch_agent_keeps_its_pinned_definition_without_a_fetch() {
    let reads = PhaseReads::new();
    let id = everruns_contracts::typed_id::AgentId::new();
    let pinned = AgentDefinition::new(id, "agent", "pinned batch version");
    reads.seed(1, &session(), Some(&pinned));
    let fetch = || async {
        Err(everruns_contracts::error::AgentLoopError::store(
            "must not fetch current version",
        ))
    };
    assert!(
        reads
            .blocker(agents, (1, id.uuid()), fetch)
            .await
            .unwrap()
            .is_none()
    );
    let got = reads
        .definition(agents, (1, id.uuid()), fetch)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.system_prompt, pinned.system_prompt);
}

#[tokio::test]
async fn simultaneous_definition_and_lifecycle_reads_share_the_in_flight_cell() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let id = everruns_contracts::typed_id::AgentId::new();
    let key = (1, id.uuid());
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok((
            Ok(Some(AgentDefinition::new(id, "agent", "current"))),
            Some(DependencyBlocker::AgentArchived),
        ))
    };
    let (definition, blocker) = tokio::join!(
        reads.definition(agents, key, fetch),
        reads.blocker(agents, key, fetch)
    );
    assert_eq!(definition.unwrap().unwrap().id, id);
    assert!(matches!(
        blocker.unwrap(),
        Some(DependencyBlocker::AgentArchived)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pinned_batch_agent_wins_an_in_flight_current_prefetch_without_losing_lifecycle() {
    let reads = PhaseReads::new();
    let id = everruns_contracts::typed_id::AgentId::new();
    let key = (1, id.uuid());
    let current = AgentDefinition::new(id, "agent", "current version");
    let pinned = AgentDefinition::new(id, "agent", "pinned batch version");
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let pending_reads = reads.clone();
    let pending = tokio::spawn(async move {
        pending_reads
            .definition(agents, key, || async {
                started.send(()).unwrap();
                released.await.unwrap();
                Ok((Ok(Some(current)), Some(DependencyBlocker::AgentArchived)))
            })
            .await
    });
    ready.await.unwrap();
    reads.seed(1, &session(), Some(&pinned));
    release.send(()).unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap().unwrap().system_prompt,
        pinned.system_prompt
    );
    let must_not_fetch = || async {
        Err(everruns_contracts::error::AgentLoopError::store(
            "prefetch must be reused",
        ))
    };
    assert!(matches!(
        reads.blocker(agents, key, must_not_fetch).await.unwrap(),
        Some(DependencyBlocker::AgentArchived)
    ));
    assert_eq!(
        reads
            .definition(agents, key, must_not_fetch)
            .await
            .unwrap()
            .unwrap()
            .system_prompt,
        pinned.system_prompt
    );
    // Closing the phase also detaches the pinned overlay, so later reads
    // cannot reuse either its definition or its lifecycle probe.
    reads.note_event(crate::core::REASON_STARTED);
    assert!(reads.definition(agents, key, must_not_fetch).await.is_err());
}

#[tokio::test]
async fn pinned_batch_agent_replaces_an_initialized_current_definition_only() {
    let reads = PhaseReads::new();
    let id = everruns_contracts::typed_id::AgentId::new();
    let key = (1, id.uuid());
    reads
        .definition(agents, key, || async {
            Ok((
                Ok(Some(AgentDefinition::new(id, "agent", "current"))),
                Some(DependencyBlocker::AgentDeleted),
            ))
        })
        .await
        .unwrap();
    let pinned = AgentDefinition::new(id, "agent", "pinned");
    reads.seed(1, &session(), Some(&pinned));
    let must_not_fetch =
        || async { Err(everruns_contracts::error::AgentLoopError::store("cached")) };
    assert_eq!(
        reads
            .definition(agents, key, must_not_fetch)
            .await
            .unwrap()
            .unwrap()
            .system_prompt,
        "pinned"
    );
    assert!(matches!(
        reads.blocker(agents, key, must_not_fetch).await.unwrap(),
        Some(DependencyBlocker::AgentDeleted)
    ));
}

#[tokio::test]
async fn a_waiting_definition_retries_an_uncloneable_projection_error() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let key = (1, Uuid::now_v7());
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok((
            Err(everruns_contracts::error::AgentLoopError::config(
                "original projection error",
            )),
            Some(DependencyBlocker::AgentArchived),
        ))
    };
    let (first, second) = tokio::join!(
        reads.definition(agents, key, fetch),
        reads.definition(agents, key, fetch)
    );
    for error in [first.unwrap_err(), second.unwrap_err()] {
        assert!(
            matches!(error, everruns_contracts::error::AgentLoopError::Configuration(message) if message == "original projection error")
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(matches!(
        reads.blocker(agents, key, fetch).await.unwrap(),
        Some(DependencyBlocker::AgentArchived)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn turn_context_cells_are_org_session_input_scoped_and_invalidated_on_write() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let session = session();
    let sid = session.id.uuid();
    let input = Uuid::now_v7();
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(TurnContext {
            agent: None,
            session: session.clone(),
            messages: vec![],
            model: None,
            mcp_tool_definitions: vec![],
        })
    };
    for key in [
        (1, sid, input),
        (1, sid, input),
        (2, sid, input),
        (1, sid, Uuid::now_v7()),
        (1, Uuid::now_v7(), input),
    ] {
        reads.read_through(turn_contexts, key, fetch).await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    reads.invalidate_session(1, sid);
    reads
        .read_through(turn_contexts, (1, sid, input), fetch)
        .await
        .unwrap();
    reads
        .read_through(turn_contexts, (2, sid, input), fetch)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    reads.note_event(crate::core::ACT_STARTED);
    reads
        .read_through(turn_contexts, (2, sid, input), fetch)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn provider_credentials_never_cross_org_provider_or_session_keys() {
    let reads = PhaseReads::new();
    let calls = AtomicU32::new(0);
    let first = Uuid::now_v7();
    let second = Uuid::now_v7();
    let keys = [
        (1, "provider-a".to_owned(), Some(first)),
        (1, "provider-a".to_owned(), Some(first)),
        (1, "provider-a".to_owned(), Some(second)),
        (1, "provider-a".to_owned(), None),
        (2, "provider-a".to_owned(), Some(first)),
        (1, "provider-b".to_owned(), Some(first)),
    ];
    for key in keys {
        let expected = format!("test-credential-{key:?}");
        let mut config = ProviderConfig::for_provider(
            key.1.clone(),
            everruns_contracts::provider::DriverId::OpenAI,
        );
        config.api_key = Some(expected.clone());
        let got = reads
            .read_through(providers, key, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Some(config))
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.api_key.as_deref(), Some(expected.as_str()));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn provider_credentials_retry_errors_expire_and_are_detached_at_phase_start() {
    let reads = PhaseReads::new();
    let key = (1, "provider".to_owned(), Some(Uuid::now_v7()));
    let failed: Result<Option<ProviderConfig>> = reads
        .read_through(providers, key.clone(), || async {
            Err(everruns_contracts::error::AgentLoopError::store(
                "credential read refused",
            ))
        })
        .await;
    assert!(failed.is_err());
    let calls = AtomicU32::new(0);
    let fetch = || async {
        let version = calls.fetch_add(1, Ordering::SeqCst);
        let mut config = ProviderConfig::new(everruns_contracts::provider::DriverId::OpenAI);
        config.api_key = Some(format!("test-credential-{version}"));
        Ok(Some(config))
    };
    let first = reads
        .read_through(providers, key.clone(), fetch)
        .await
        .unwrap()
        .unwrap();
    let cached = reads
        .read_through(providers, key.clone(), fetch)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cached.api_key, first.api_key);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    reads
        .with_state(|state| state.providers.get_mut(&key).unwrap().at = Instant::now() - FRESH_FOR);
    let fresh = reads
        .read_through(providers, key.clone(), fetch)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(fresh.api_key, first.api_key);
    reads.note_event(crate::core::REASON_STARTED);
    assert!(reads.with_state(|state| state.providers.is_empty()));
    let after = reads
        .read_through(providers, key.clone(), fetch)
        .await
        .unwrap()
        .unwrap();
    let again = reads
        .read_through(providers, key, fetch)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(after.api_key, again.api_key);
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert!(reads.with_state(|state| state.providers.is_empty()));
}
