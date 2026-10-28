//! Compare history-scan and indexed single-message reads at increasing depths.
//!
//! Usage: cargo bench -p everruns-server --bench message_lookup
//! Uses the isolated test database (embedded PostgreSQL by default, or
//! EVERRUNS_TEST_POSTGRES_URL). Reports lookup latency, not full-turn latency.
//! Keep the same build profile and database configuration when comparing runs.

use everruns_contracts::typed_id::SessionId;
use everruns_core::{InputMessage, MessageRetriever, RuntimeMessage, events::InputMessageData};
use everruns_server::storage::{DbMessageRetriever, StorageBackend};
use std::sync::Arc;

fn assert_message(actual: Option<RuntimeMessage>, expected: &RuntimeMessage) {
    assert_eq!(
        serde_json::to_value(actual.unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
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

#[tokio::main]
async fn main() {
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
