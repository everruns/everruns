//! Late Agents API usage against PostgreSQL (EVE-1145): the `usage_pending`
//! claim applies a turn's late usage once, and the journal admits one late
//! debit per generation.
//!
//! Run with: cargo test -p everruns-server --test server_integration late_generation_usage_test:: -- --test-threads=1

use crate::session_row_fixture::base_session_row;
use crate::test_harness::get_database_url;

use everruns_server::storage::models::CreateUsageJournalRow;
use everruns_server::storage::{
    CreatePendingUsageGeneration, Database, LateGenerationUsage, StorageBackend,
};
use sqlx::PgPool;
use uuid::Uuid;

const TEST_ORG_ID: i64 = 1;

async fn backend() -> StorageBackend {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL");
    StorageBackend::Postgres(Database::new(pool))
}

async fn pending(backend: &StorageBackend) -> (Uuid, Uuid) {
    let session = backend
        .create_session(base_session_row(TEST_ORG_ID))
        .await
        .unwrap();
    let id = backend
        .create_pending_usage_generation(CreatePendingUsageGeneration {
            org_id: TEST_ORG_ID,
            session_id: session.id.uuid(),
            turn_id: None,
            event_id: None,
            model: "gpt-6-astra".into(),
            provider: Some("openai".into()),
            estimated_cost_usd: Some(0.01),
            duration_ms: None,
            finish_reason: None,
            provider_response_id: format!("turn_{}", Uuid::now_v7().simple()),
            provider_session_id: "sess_1".into(),
            provider_config_id: "provider_x".into(),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    (session.id.uuid(), id)
}

fn usage() -> LateGenerationUsage {
    LateGenerationUsage {
        input_tokens: 800,
        output_tokens: 100,
        cache_read_tokens: 200,
        cache_creation_tokens: 0,
        estimated_cost_usd: Some(0.5),
    }
}

#[tokio::test]
async fn concurrent_appliers_apply_late_usage_exactly_once() {
    let backend = backend().await;
    let (_, id) = pending(&backend).await;
    let listed = backend
        .list_pending_usage_generations(30, 1000)
        .await
        .unwrap();
    assert!(listed.iter().any(|row| row.id == id));

    let late = usage();
    let (a, b) = tokio::join!(
        backend.apply_late_generation_usage(id, &late),
        backend.apply_late_generation_usage(id, &late),
    );
    assert_eq!(
        [a.unwrap(), b.unwrap()].iter().filter(|won| **won).count(),
        1,
        "exactly one caller applies the usage"
    );
    assert!(
        !backend
            .apply_late_generation_usage(id, &late)
            .await
            .unwrap()
    );

    let record = backend.get_generation_usage(id).await.unwrap().unwrap();
    assert_eq!(
        (
            record.input_tokens,
            record.output_tokens,
            record.cache_read_tokens
        ),
        (800, 100, 200)
    );
    assert!(!record.usage_pending);
    assert!((record.estimated_cost_usd.unwrap() - 0.51).abs() < 1e-9);
    let listed = backend
        .list_pending_usage_generations(30, 1000)
        .await
        .unwrap();
    assert!(listed.iter().all(|row| row.id != id));
}

#[tokio::test]
async fn a_failed_read_delays_and_counts_until_the_last_attempt() {
    let backend = backend().await;
    let (_, id) = pending(&backend).await;
    backend
        .mark_llm_generation_reconciliation_failed(id, 3600)
        .await
        .unwrap();
    let record = backend.get_generation_usage(id).await.unwrap().unwrap();
    assert_eq!(record.reconciliation_attempts, 1);
    assert!(record.usage_pending);
    let listed = backend
        .list_pending_usage_generations(30, 1000)
        .await
        .unwrap();
    assert!(listed.iter().all(|row| row.id != id), "delayed");

    // Ready again, but out of attempts: it stays an explicit unknown.
    backend
        .mark_llm_generation_reconciliation_failed(id, 0)
        .await
        .unwrap();
    let listed = backend
        .list_pending_usage_generations(2, 1000)
        .await
        .unwrap();
    assert!(listed.iter().all(|row| row.id != id));
    let listed = backend
        .list_pending_usage_generations(3, 1000)
        .await
        .unwrap();
    assert!(listed.iter().any(|row| row.id == id));
}

#[tokio::test]
async fn the_journal_admits_one_late_debit_per_generation() {
    let backend = backend().await;
    let (session_id, id) = pending(&backend).await;
    let journal = || CreateUsageJournalRow {
        org_id: TEST_ORG_ID,
        kind: "llm_generation".into(),
        source_type: Some("llm_generation_late_usage".into()),
        source_id: Some(id.to_string()),
        event_id: None,
        session_id: Some(session_id),
        turn_id: None,
        user_id: None,
        principal_id: None,
        agent_id: None,
        harness_id: None,
        measures: serde_json::json!({}),
        metadata: serde_json::json!({}),
    };
    backend.create_usage_journal_entry(journal()).await.unwrap();
    assert!(
        backend.create_usage_journal_entry(journal()).await.is_err(),
        "a second late debit of the same generation is refused"
    );
}
