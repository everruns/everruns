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
