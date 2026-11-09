use super::*;

/// Deleting a session while an event insert for it is in flight neither waits
/// for the insert nor deadlocks with it, and events written around the delete
/// are kept (the purge this replaced had to order its locks against the insert;
/// nightly workflow test, 2026-10-08).
#[tokio::test]
async fn test_delete_session_during_event_insert_does_not_deadlock() {
    let pool = create_test_pool().await;
    let backend = StorageBackend::from_database(Database::new(pool.clone()));
    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            owner_principal_id,
            ..base_session_row(TEST_ORG_ID)
        })
        .await
        .expect("Failed to create session");
    let event_row = |event_type: &str| CreateEventRow {
        session_id: session.id,
        event_type: event_type.to_string(),
        ts: Utc::now(),
        context: json!({}),
        data: json!({}),
        metadata: None,
        tags: None,
    };
    backend
        .create_event(event_row("session.started"))
        .await
        .expect("Failed to create first event");

    // First half of an insert: reserve a sequence (row lock on event_sequences).
    let mut insert_tx = pool.begin().await.unwrap();
    let sequence: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO event_sequences (session_id, next_sequence, updated_at)
        VALUES ($1, 2, NOW())
        ON CONFLICT (session_id) DO UPDATE
        SET next_sequence = event_sequences.next_sequence + EXCLUDED.next_sequence - 1
        RETURNING next_sequence - 1
        "#,
    )
    .bind(session.id.uuid())
    .fetch_one(&mut *insert_tx)
    .await
    .unwrap();

    // The delete only marks the session row, so it does not wait for the
    // insert's lock on the sequence row.
    let deleted = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        backend.delete_session(TEST_ORG_ID, session.id),
    )
    .await
    .expect("delete must not wait for the in-flight insert")
    .expect("delete must succeed");
    assert!(deleted);

    // Second half: the event row itself, whose FK check locks the sessions row.
    sqlx::query(
        r#"
        INSERT INTO events (session_id, sequence, event_type, ts, context, data)
        VALUES ($1, $2, 'turn.started', NOW(), '{}', '{}')
        "#,
    )
    .bind(session.id.uuid())
    .bind(sequence)
    .execute(&mut *insert_tx)
    .await
    .expect("in-flight insert must complete");
    insert_tx.commit().await.unwrap();

    assert!(
        backend
            .get_session(TEST_ORG_ID, session.id)
            .await
            .unwrap()
            .is_none()
    );

    // A worker still finishing the turn writes into the deleted session; the
    // events are stored with the rest of its history.
    backend
        .create_events(vec![
            event_row("turn.completed"),
            event_row("turn.completed"),
        ])
        .await
        .expect("late events for a deleted session are kept");
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE session_id = $1")
        .bind(session.id.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(events, 4);
}

/// Deleting a session that has taken a turn keeps everything it wrote: the
/// row is marked deleted, and its events and financial records stay attached.
/// (EVE-919 was the purge this replaced answering 500 for any real session.)
#[tokio::test]
async fn delete_session_keeps_events_and_usage_records() {
    let backend = create_test_backend().await;
    let pool = create_test_pool().await;

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            owner_principal_id,
            ..base_session_row(TEST_ORG_ID)
        })
        .await
        .expect("Failed to create session");

    let event = backend
        .create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: json!({"turn_id": Uuid::now_v7().to_string(), "role": "user"}),
            data: json!({"message": {"role": "user", "content": []}}),
            metadata: None,
            tags: None,
        })
        .await
        .expect("Failed to create event");

    let journal_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO usage_journal (id, org_id, kind, session_id, event_id)
        VALUES ($1, $2, 'llm.generation', $3, $4)
        "#,
    )
    .bind(journal_id)
    .bind(TEST_ORG_ID)
    .bind(session.id.uuid())
    .bind(event.id)
    .execute(&pool)
    .await
    .expect("Failed to record usage journal entry");

    let ledger_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO usage_ledger (id, journal_id, org_id, session_id, currency, amount, meter_source)
        VALUES ($1, $2, $3, $4, 'usd', 1.25, 'llm')
        "#,
    )
    .bind(ledger_id)
    .bind(journal_id)
    .bind(TEST_ORG_ID)
    .bind(session.id.uuid())
    .execute(&pool)
    .await
    .expect("Failed to record usage ledger entry");

    let deleted = backend
        .delete_session(TEST_ORG_ID, session.id)
        .await
        .expect("Deleting a session that has taken a turn must not error");
    assert!(deleted, "the session was marked deleted");

    let deleted_at: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT deleted_at FROM sessions WHERE id = $1")
            .bind(session.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("the session row stays");
    assert!(deleted_at.is_some(), "the row is marked, not removed");
    assert!(
        backend
            .get_session(TEST_ORG_ID, session.id)
            .await
            .unwrap()
            .is_none(),
        "a deleted session is not found"
    );

    let events_left: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE session_id = $1")
        .bind(session.id.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(events_left, 1, "the session's events stay");

    // The financial record stays attached to the session it came from.
    let (journal_session, journal_event): (Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("SELECT session_id, event_id FROM usage_journal WHERE id = $1")
            .bind(journal_id)
            .fetch_one(&pool)
            .await
            .expect("usage journal entry survives session deletion");
    assert_eq!(journal_session, Some(session.id.uuid()));
    assert_eq!(journal_event, Some(event.id.uuid()));

    let (ledger_session, amount): (Option<Uuid>, f64) =
        sqlx::query_as("SELECT session_id, amount FROM usage_ledger WHERE id = $1")
            .bind(ledger_id)
            .fetch_one(&pool)
            .await
            .expect("usage ledger entry survives session deletion");
    assert_eq!(ledger_session, Some(session.id.uuid()));
    assert_eq!(amount, 1.25, "the amount charged is untouched");
}
