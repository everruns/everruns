use crate::test_harness;

use sqlx::{Connection, Executor, PgConnection, Row};

/// A row whose registry identity is the id the provider listed (a GPT-6.1 Sol
/// row moved to OpenRouter as `gpt-6.1-sol`, listed as `openai/gpt-6.1-sol`)
/// is healed; a row no listed sibling vouches for stays stale.
#[tokio::test]
async fn rows_listed_under_the_providers_spelling_are_healed() {
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
            id UUID NOT NULL,
            provider_id UUID NOT NULL,
            model_id TEXT NOT NULL,
            source TEXT NOT NULL,
            provider_metadata JSONB,
            last_seen_at TIMESTAMPTZ
        );
        INSERT INTO providers VALUES
            (1, '00000000-0000-0000-0000-000000000001', '2026-10-31 00:13:24+00');
        INSERT INTO models VALUES
            -- Listed by the last sync under OpenRouter's spelling.
            (1, '00000000-0000-0000-0000-0000000000a1', '00000000-0000-0000-0000-000000000001',
             'openai/gpt-6.1-sol', 'discovered',
             '{"everruns_catalog": {"profile_key": "custom/openrouter/openai/gpt-6.1-sol"}}',
             '2026-10-31 00:13:24+00'),
            -- Moved here from the OpenAI provider; never matched by id.
            (1, '00000000-0000-0000-0000-0000000000a2', '00000000-0000-0000-0000-000000000001',
             'gpt-6.1-sol', 'discovered',
             '{"everruns_catalog": {"profile_key": "openai/gpt-6.1-sol"}}',
             '2026-10-07 03:23:26+00'),
            -- Its spelling is not listed: genuinely missing.
            (1, '00000000-0000-0000-0000-0000000000a3', '00000000-0000-0000-0000-000000000001',
             'gpt-6-sol', 'discovered',
             '{"everruns_catalog": {"profile_key": "openai/gpt-6-sol"}}',
             '2026-10-07 03:23:26+00'),
            -- No identity recorded.
            (1, '00000000-0000-0000-0000-0000000000a4', '00000000-0000-0000-0000-000000000001',
             'gone', 'discovered', NULL, '2026-10-07 03:23:26+00');
        "#,
    )
    .execute(&mut *transaction)
    .await
    .expect("seed models");

    sqlx::raw_sql(include_str!(
        "../../migrations/20261106051921_heal_models_listed_under_another_spelling.sql"
    ))
    .execute(&mut *transaction)
    .await
    .expect("run migration");

    let rows = sqlx::query(
        r#"
        SELECT m.model_id,
               m.last_seen_at IS NULL OR m.last_seen_at < p.last_synced_at AS stale
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
            ("gone".to_string(), true),
            ("gpt-6-sol".to_string(), true),
            ("gpt-6.1-sol".to_string(), false),
            ("openai/gpt-6.1-sol".to_string(), false),
        ]
    );

    transaction.rollback().await.expect("roll back");
}
