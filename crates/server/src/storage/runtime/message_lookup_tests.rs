//! Single-message retrieval must not depend on the conversation window.

use super::*;
use crate::storage::CreateEventRow;

fn assert_message(actual: Option<RuntimeMessage>, expected: &RuntimeMessage) {
    assert_eq!(
        serde_json::to_value(actual.unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

async fn store(
    db: &StorageBackend,
    session_id: SessionId,
    event_type: &str,
    data: serde_json::Value,
) {
    db.create_event(CreateEventRow {
        session_id,
        event_type: event_type.into(),
        ts: Utc::now(),
        context: serde_json::json!({}),
        data,
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();
}

async fn history(db: &StorageBackend, session_id: SessionId, count: usize) {
    // Bulk fixture: real event rows and triggers, without thousands of client
    // round trips. Distinct message ids and a 1 KiB payload model long chats.
    let data = serde_json::to_value(InputMessageData::new(RuntimeMessage::user(
        "x".repeat(1024),
    )))
    .unwrap();
    sqlx::query(
        "INSERT INTO events (session_id, sequence, event_type, ts, context, data) \
         SELECT $1, allocate_event_sequence($1), 'input.message', NOW(), '{}', \
                jsonb_set($3, '{message,id}', to_jsonb('message_' || replace(gen_random_uuid()::text, '-', ''))) \
         FROM generate_series(1, $2)",
    )
    .bind(session_id.uuid())
    .bind(count as i32)
    .bind(data)
    .execute(db.database().pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn get_finds_messages_older_than_the_history_window() {
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let retriever = DbMessageRetriever::new(db.clone());
    let first = retriever
        .add(session.uuid(), InputMessage::user("original task"))
        .await
        .unwrap();
    history(
        &db,
        session,
        crate::storage::repository::MESSAGE_SAFETY_LIMIT,
    )
    .await;

    assert!(
        retriever
            .load(session)
            .await
            .unwrap()
            .iter()
            .all(|m| m.id != first.id)
    );
    assert_message(retriever.get(session, first.id).await.unwrap(), &first);
}

#[tokio::test]
async fn get_preserves_message_formats_and_session_isolation() {
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let other_session = db.create_test_session().await;
    let retriever = DbMessageRetriever::new(db.clone());

    for legacy in [false, true] {
        for agent in [false, true] {
            let message = if agent {
                RuntimeMessage::assistant("answer")
            } else {
                RuntimeMessage::user("question")
            };
            let event = if agent {
                Event::new(
                    session,
                    EventContext::empty(),
                    OutputMessageCompletedData::new(message.clone()),
                )
            } else {
                Event::new(
                    session,
                    EventContext::empty(),
                    InputMessageData::new(message.clone()),
                )
            };
            let data = if legacy {
                serde_json::to_value(&event).unwrap()
            } else {
                serde_json::to_value(&event.data).unwrap()
            };
            store(
                &db,
                session,
                if agent {
                    "output.message.completed"
                } else {
                    "input.message"
                },
                data,
            )
            .await;
            assert_message(retriever.get(session, message.id).await.unwrap(), &message);
            assert!(
                retriever
                    .get(other_session, message.id)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }
    assert!(
        retriever
            .get(session, MessageId::new())
            .await
            .unwrap()
            .is_none()
    );

    let malformed_id = MessageId::new();
    store(
        &db,
        session,
        "input.message",
        serde_json::json!({"message": {"id": malformed_id}}),
    )
    .await;
    assert!(
        retriever
            .get(session, malformed_id)
            .await
            .unwrap()
            .is_none()
    );

    // The old loader skipped malformed rows. A duplicate id must not hide a
    // later valid message, and the first valid occurrence still wins.
    let mut valid = RuntimeMessage::user("recovered input");
    valid.id = malformed_id;
    store(
        &db,
        session,
        "input.message",
        serde_json::to_value(InputMessageData::new(valid.clone())).unwrap(),
    )
    .await;
    assert_message(retriever.get(session, malformed_id).await.unwrap(), &valid);
    let mut duplicate = valid.clone();
    duplicate.content = vec![ContentPart::text("later duplicate")];
    store(
        &db,
        session,
        "input.message",
        serde_json::to_value(InputMessageData::new(duplicate)).unwrap(),
    )
    .await;
    assert_message(retriever.get(session, malformed_id).await.unwrap(), &valid);
}

#[tokio::test]
async fn input_lookup_excludes_assistant_messages_and_other_sessions() {
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let other_session = db.create_test_session().await;
    let retriever = DbMessageRetriever::new(db.clone());
    let input = retriever
        .add(session.uuid(), InputMessage::user("input"))
        .await
        .unwrap();
    let output = RuntimeMessage::assistant("output");
    store(
        &db,
        session,
        "output.message.completed",
        serde_json::to_value(OutputMessageCompletedData::new(output.clone())).unwrap(),
    )
    .await;

    let found = db
        .find_input_message_event(session, input.id)
        .await
        .unwrap()
        .unwrap();
    assert_message(
        Some(event_to_message(&found.data, &found.event_type).unwrap()),
        &input,
    );
    assert!(
        db.find_input_message_event(session, output.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.find_input_message_event(other_session, input.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
#[ignore = "message lookup benchmark; run explicitly with --ignored --nocapture"]
async fn benchmark_message_lookup_history_scaling() {
    let db = Arc::new(StorageBackend::test_database());
    let retriever = DbMessageRetriever::new(db.clone());
    for depth in [10, 1_000, 10_000] {
        let session = db.create_test_session().await;
        history(&db, session, depth).await;
        let target = retriever
            .add(session.uuid(), InputMessage::user("latest input"))
            .await
            .unwrap();
        sqlx::query("ANALYZE events")
            .execute(db.database().pool())
            .await
            .unwrap();
        // Compare the previous implementation with the exact lookup in one
        // process, against the same data/pool. This is retrieval latency, not
        // full-turn latency, and deliberately has no timing pass/fail threshold.
        for scan_history in [true, false] {
            let mut samples = Vec::new();
            for iteration in 0..31 {
                let start = std::time::Instant::now();
                let result = if scan_history {
                    retriever
                        .load(session)
                        .await
                        .unwrap()
                        .into_iter()
                        .find(|m| m.id == target.id)
                } else {
                    retriever.get(session, target.id).await.unwrap()
                };
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert_message(result, &target);
                if iteration > 0 {
                    samples.push(elapsed);
                }
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "message_lookup depth={depth} history_scan={scan_history} samples=30 p50_ms={:.3} p95_ms={:.3}",
                samples[15], samples[28]
            );
        }
    }
}
