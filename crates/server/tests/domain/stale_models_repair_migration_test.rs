use crate::test_harness;

use sqlx::{Connection, Executor, PgConnection, Row};

/// Rows a pre-#5095 sync falsely marked stale are repaired; a model the
/// provider genuinely stopped listing in an earlier sync stays stale.
#[tokio::test]
async fn falsely_stale_models_are_repaired_and_unlisted_ones_kept() {
    let mut connection = PgConnection::connect(&test_harness::get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let mut transaction = connection.begin().await.expect("begin transaction");
    transaction
        .execute("SET LOCAL search_path TO pg_temp, public")
        .await
        .expect("set isolated migration search path");
    sqlx::raw_sql(
        r#"
        CREATE TEMPORARY TABLE providers (
            org_id BIGINT NOT NULL,
            id UUID NOT NULL,
            last_synced_at TIMESTAMPTZ
        );
        CREATE TEMPORARY TABLE models (
            org_id BIGINT NOT NULL,
            provider_id UUID NOT NULL,
            model_id TEXT NOT NULL,
            source TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL,
            last_seen_at TIMESTAMPTZ
        );
        INSERT INTO providers VALUES
            (1, '00000000-0000-0000-0000-000000000001', '2026-11-01 04:47:48+00'),
            (1, '00000000-0000-0000-0000-000000000002', NULL);
        INSERT INTO models VALUES
            -- Seen by the last sync, stamped with its start time.
            (1, '00000000-0000-0000-0000-000000000001', 'seen', 'discovered',
             '2026-10-01 00:00:00+00', '2026-11-01 04:47:46+00'),
            -- Created by the last sync, last_seen_at left NULL.
            (1, '00000000-0000-0000-0000-000000000001', 'created', 'discovered',
             '2026-11-01 04:47:47+00', NULL),
            -- Last seen by an earlier sync: genuinely no longer listed.
            (1, '00000000-0000-0000-0000-000000000001', 'gone', 'discovered',
             '2026-10-01 00:00:00+00', '2026-10-20 00:00:00+00'),
            -- Created by an earlier sync and never seen again.
            (1, '00000000-0000-0000-0000-000000000001', 'gone_null', 'discovered',
             '2026-10-01 00:00:00+00', NULL),
            -- Predefined rows are never stale.
            (1, '00000000-0000-0000-0000-000000000001', 'predefined', 'predefined',
             '2026-11-01 04:47:47+00', NULL),
            -- Provider never synced.
            (1, '00000000-0000-0000-0000-000000000002', 'unsynced', 'discovered',
             '2026-11-01 04:47:47+00', NULL);
        "#,
    )
    .execute(&mut *transaction)
    .await
    .expect("seed models");

    sqlx::raw_sql(include_str!(
        "../../migrations/20261104025416_repair_falsely_stale_models.sql"
    ))
    .execute(&mut *transaction)
    .await
    .expect("run migration");

    let rows = sqlx::query(
        r#"
        SELECT m.model_id,
               m.source = 'discovered'
               AND p.last_synced_at IS NOT NULL
               AND (m.last_seen_at IS NULL OR m.last_seen_at < p.last_synced_at) AS stale
        FROM models m JOIN providers p ON p.id = m.provider_id
        ORDER BY m.model_id
        "#,
    )
    .fetch_all(&mut *transaction)
    .await
    .expect("read models");
    let stale: Vec<(String, bool)> = rows
        .iter()
        .map(|row| (row.get("model_id"), row.get("stale")))
        .collect();
    assert_eq!(
        stale,
        vec![
            ("created".to_string(), false),
            ("gone".to_string(), true),
            ("gone_null".to_string(), true),
            ("predefined".to_string(), false),
            ("seen".to_string(), false),
            ("unsynced".to_string(), false),
        ]
    );

    transaction.rollback().await.expect("roll back");
}
