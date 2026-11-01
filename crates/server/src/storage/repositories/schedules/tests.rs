use super::*;
use crate::storage::StorageBackend;
use crate::storage::test_database::create_test_database;
use anyhow::Context;
use everruns_core::DEFAULT_ORG_ID;
use sqlx::postgres::PgPoolOptions;

fn schedule_input(session_id: SessionId) -> CreateSessionScheduleRow {
    CreateSessionScheduleRow {
        org_id: DEFAULT_ORG_ID,
        session_id,
        owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        description: format!("cached-plan-{}", uuid::Uuid::now_v7()),
        cron_expression: Some("0 * * * *".to_string()),
        scheduled_at: None,
        timezone: "UTC".to_string(),
        next_trigger_at: Some(Utc::now() - chrono::Duration::days(1)),
        ends_at: None,
        max_runs: None,
        stop_when: None,
    }
}

fn leased_input(session_id: SessionId) -> UpsertLeasedResourceRow {
    UpsertLeasedResourceRow {
        org_id: DEFAULT_ORG_ID,
        session_id,
        provider: "test".to_string(),
        resource_type: "cached-plan".to_string(),
        external_id: uuid::Uuid::now_v7().to_string(),
        display_name: None,
        owner_user_id: None,
        connection_id: None,
        lease_duration_seconds: 60,
        // Already expired, so the cleanup claim picks it up.
        lease_expires_at: Utc::now() - chrono::Duration::minutes(1),
        metadata: serde_json::json!({}),
    }
}

/// Runs every row-returning query in this repository once, so each is
/// prepared and cached on the connection that ran it.
async fn exercise(db: &Database, session_id: SessionId) -> Result<()> {
    let created = db
        .create_session_schedule(schedule_input(session_id))
        .await
        .context("create_session_schedule")?;
    db.create_session_schedule_with_limits(schedule_input(session_id), 1_000, 1_000)
        .await
        .context("create_session_schedule_with_limits")?
        .context("schedule limits")?;
    db.get_session_schedule(DEFAULT_ORG_ID, created.id)
        .await
        .context("get_session_schedule")?
        .context("schedule exists")?;
    db.list_session_schedules(DEFAULT_ORG_ID, session_id)
        .await
        .context("list_session_schedules")?;
    db.list_session_schedule_history(DEFAULT_ORG_ID, session_id)
        .await
        .context("list_session_schedule_history")?;
    db.update_session_schedule(
        DEFAULT_ORG_ID,
        created.id,
        UpdateSessionScheduleRow {
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .context("update_session_schedule")?
    .context("schedule updated")?;
    let claimed = db
        .claim_due_session_schedules("cached-plan-test", 500)
        .await
        .context("claim_due_session_schedules")?;
    // `claimed_by` is `#[sqlx(default)]`, so a list that dropped it would
    // still decode; check the value really comes back.
    let claimed = claimed
        .into_iter()
        .find(|row| row.id == created.id)
        .context("the overdue schedule should be claimed")?;
    anyhow::ensure!(
        claimed.claimed_by.as_deref() == Some("cached-plan-test"),
        "the claim should return the claimant"
    );

    let resource = db
        .upsert_leased_resource(leased_input(session_id))
        .await
        .context("upsert_leased_resource")?;
    db.list_session_leased_resources(session_id)
        .await
        .context("list_session_leased_resources")?;
    let claimed = db
        .claim_due_leased_resources(500, 300)
        .await
        .context("claim_due_leased_resources")?;
    let claimed = claimed
        .into_iter()
        .find(|row| row.id == resource.id)
        .context("the expired resource should be claimed")?;
    let started = claimed.cleanup_started_at.context("claim stamps start")?;
    db.mark_leased_resource_cleanup_failed(resource.id, started, 0, "probe")
        .await
        .context("mark_leased_resource_cleanup_failed")?
        .context("failure recorded")?;
    // No longer `cleaning`, so these match nothing, but they still prepare.
    db.mark_leased_resource_released(resource.id, started)
        .await
        .context("mark_leased_resource_released")?;
    db.release_leased_resource(ReleaseLeasedResourceRow {
        org_id: DEFAULT_ORG_ID,
        session_id,
        provider: resource.provider.clone(),
        resource_type: resource.resource_type.clone(),
        external_id: resource.external_id.clone(),
    })
    .await
    .context("release_leased_resource")?
    .context("resource released")?;
    Ok(())
}

/// EVERRUNS-4C: an additive migration must not break a running instance.
///
/// sqlx caches each prepared statement per connection. PostgreSQL re-plans a
/// cached statement after DDL on its table, but refuses with "cached plan
/// must not change result type" when the result columns changed, which is
/// exactly what `*` does once a column is added. During a rolling deploy the
/// old instance keeps its pooled connections across the new instance's
/// migration, so its scheduler poll failed on every tick.
///
/// One connection makes the second round reuse the first round's statements.
#[tokio::test]
async fn schedule_queries_survive_added_columns_on_a_live_connection() -> Result<()> {
    let (seed_pool, database) = create_test_database();
    let session_id = StorageBackend::from_database(Database::new(seed_pool))
        .create_test_session()
        .await;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url())
        .await?;
    let db = Database::new(pool.clone());

    exercise(&db, session_id).await?;

    for table in ["session_schedules", "leased_resources"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} ADD COLUMN cached_plan_probe TEXT"
        )))
        .execute(&pool)
        .await?;
    }

    exercise(&db, session_id).await
}
