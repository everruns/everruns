//! `EventService::emit_conditional`: the single-writer append primitive.

use super::*;

fn sequence_conflict(error: &anyhow::Error) -> Option<(Option<i32>, Option<i32>)> {
    match error.downcast_ref::<everruns_core::host::EventLogError>() {
        Some(everruns_core::host::EventLogError::SequenceConflict {
            expected, actual, ..
        }) => Some((*expected, *actual)),
        _ => None,
    }
}

/// A conditional emit commits at the expected last sequence and returns
/// the next one; a stale expectation fails with the conflict carrying the
/// actual last sequence and writes nothing.
#[tokio::test]
async fn emit_conditional_commits_only_at_the_expected_last_sequence() {
    let db = Arc::new(StorageBackend::test_database());
    let event_service = EventService::new(db.clone(), EventDelivery::in_memory());
    let agent_id = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    let session = db
        .create_session(test_session_input(agent_id))
        .await
        .unwrap()
        .id;
    let said = |text: &str| {
        EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant(text)),
        )
    };
    let stored = || async {
        event_service
            .list(session.uuid(), None, None, &[], &[], None, None)
            .await
            .unwrap()
            .into_iter()
            .map(|event| event.sequence.unwrap())
            .collect::<Vec<_>>()
    };

    // An empty log ends at `None`.
    let error = event_service
        .emit_conditional(said("stale"), Some(1))
        .await
        .unwrap_err();
    assert_eq!(sequence_conflict(&error), Some((Some(1), None)));
    let first = event_service
        .emit_conditional(said("one"), None)
        .await
        .unwrap();
    assert_eq!(first.sequence, Some(1));

    // An unconditional emit moves the end; the conditional path sees it.
    event_service.emit(said("two")).await.unwrap();
    for stale in [None, Some(1), Some(3)] {
        let error = event_service
            .emit_conditional(said("stale"), stale)
            .await
            .unwrap_err();
        assert_eq!(sequence_conflict(&error), Some((stale, Some(2))));
    }
    assert_eq!(stored().await, vec![1, 2]);

    let third = event_service
        .emit_conditional(said("three"), Some(2))
        .await
        .unwrap();
    assert_eq!(third.sequence, Some(3));
    // Rejected attempts consumed no sequence: plain emits continue at 4.
    let fourth = event_service.emit(said("four")).await.unwrap();
    assert_eq!(fourth.sequence, Some(4));
    assert_eq!(stored().await, vec![1, 2, 3, 4]);
    let summary = db
        .get_session(DEFAULT_ORG_ID, session)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.event_count, 4);
}

/// Writers racing with the same expectation: exactly one commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn emit_conditional_admits_exactly_one_of_concurrent_writers() {
    let db = Arc::new(StorageBackend::test_database());
    let event_service = EventService::new(db.clone(), EventDelivery::in_memory());
    let agent_id = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    for seeded in [false, true] {
        let session = db
            .create_session(test_session_input(agent_id))
            .await
            .unwrap()
            .id;
        let said = move |text: String| {
            EventRequest::new(
                session,
                EventContext::empty(),
                OutputMessageCompletedData::new(RuntimeMessage::assistant(text)),
            )
        };
        // Both the first event of a session (no event_sequences row yet)
        // and a later one.
        let expected = if seeded {
            event_service.emit(said("seed".into())).await.unwrap();
            Some(1)
        } else {
            None
        };
        let writers = (0..8)
            .map(|writer| {
                let event_service = event_service.clone();
                tokio::spawn(async move {
                    event_service
                        .emit_conditional(said(format!("writer {writer}")), expected)
                        .await
                })
            })
            .collect::<Vec<_>>();
        let mut won = Vec::new();
        for writer in writers {
            match writer.await.unwrap() {
                Ok(event) => won.push(event.sequence),
                Err(error) => assert_eq!(
                    sequence_conflict(&error),
                    Some((expected, Some(expected.unwrap_or(0) + 1))),
                    "{error:#}"
                ),
            }
        }
        assert_eq!(won, vec![Some(expected.unwrap_or(0) + 1)]);
        let events = event_service
            .list(session.uuid(), None, None, &[], &[], None, None)
            .await
            .unwrap();
        assert_eq!(events.len(), expected.unwrap_or(0) as usize + 1);
    }
}

#[tokio::test]
async fn emit_conditional_rejects_ephemeral_requests() {
    let db = Arc::new(StorageBackend::test_database());
    let event_service = EventService::new(db, EventDelivery::in_memory());
    let delta = EventRequest::new(
        SessionId::new(),
        EventContext::empty(),
        everruns_core::events::OutputMessageDeltaData {
            turn_id: everruns_contracts::typed_id::TurnId::new(),
            message_id: everruns_contracts::typed_id::MessageId::new(),
            delta: "x".into(),
            accumulated: "x".into(),
            phase: None,
        },
    );
    let error = event_service
        .emit_conditional(delta, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<everruns_core::host::EventLogError>(),
        Some(everruns_core::host::EventLogError::InvalidAppend { .. })
    ));
}
