use super::*;

/// Deleting a session while an event insert for it is in flight must not
/// deadlock (nightly workflow test, 2026-10-08).
///
/// An event insert reserves its sequence on the `event_sequences` row first and
/// only then, at the end of the statement, checks the events FK, which takes a
/// key-share lock on the sessions row. The delete used to lock the sessions row
/// first and then cascade into `event_sequences`: the opposite order. The insert
/// is replayed here as an explicit transaction in that same lock order, so the
/// delete deterministically lands between its two locks.
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
    let insert_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *insert_tx)
        .await
        .unwrap();
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

    let delete = tokio::spawn({
        let backend = backend.clone();
        async move { backend.delete_session(TEST_ORG_ID, session.id).await }
    });
    // Wait until the delete is blocked behind the insert's lock.
    let mut blocked = false;
    for _ in 0..200 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
        )
        .bind(insert_pid)
        .fetch_one(&pool)
        .await
        .unwrap();
        if waiting > 0 {
            blocked = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(blocked, "delete should wait for the in-flight insert");

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
    .expect("in-flight insert must complete, not deadlock");
    insert_tx.commit().await.unwrap();

    let deleted = delete
        .await
        .unwrap()
        .expect("delete must succeed, not deadlock");
    assert!(deleted);
    assert!(
        backend
            .get_session(TEST_ORG_ID, session.id)
            .await
            .unwrap()
            .is_none()
    );

    // A late insert for the deleted session fails cleanly and leaves no
    // orphaned sequence row behind.
    let late = backend
        .create_events(vec![
            event_row("turn.completed"),
            event_row("turn.completed"),
        ])
        .await;
    assert!(late.is_err(), "insert into a deleted session must fail");
    let sequences: i64 =
        sqlx::query_scalar("SELECT count(*) FROM event_sequences WHERE session_id = $1")
            .bind(session.id.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sequences, 0);
}
