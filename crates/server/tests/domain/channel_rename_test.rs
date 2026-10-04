use crate::test_harness;
use serde_json::Value;
use sqlx::PgPool;

#[tokio::test]
async fn channel_migration_preserves_identity_secrets_attribution_and_spend_controls() {
    let pool = PgPool::connect(&test_harness::get_database_url())
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    let schema = format!("channel_rename_{}", uuid::Uuid::new_v4().simple());
    let setup = format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path TO {schema};
         CREATE TABLE agents (id UUID PRIMARY KEY, org_id BIGINT);
         CREATE TABLE virtual_users (id UUID PRIMARY KEY, org_id BIGINT);
         CREATE TABLE agent_endpoints (
             id UUID PRIMARY KEY, public_id TEXT UNIQUE, agent_id UUID,
             virtual_user_id UUID, channel_config JSONB,
             channel_config_encrypted BYTEA, auth_encrypted BYTEA,
             legacy_alias_id TEXT, owner_principal_id UUID, status TEXT
         );
         CREATE VIEW app_channels AS SELECT * FROM agent_endpoints;
         CREATE TABLE sessions (id UUID PRIMARY KEY, endpoint_id UUID REFERENCES agent_endpoints(id), app_id UUID);
         CREATE INDEX idx_sessions_endpoint_id ON sessions(endpoint_id);
         CREATE FUNCTION update_updated_at_column() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END; $$;
         CREATE TRIGGER update_agent_endpoints_updated_at BEFORE UPDATE ON agent_endpoints FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
         CREATE FUNCTION enforce_endpoint_virtual_user_org() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END; $$;
         CREATE TRIGGER endpoint_virtual_user_org_guard BEFORE INSERT OR UPDATE OF virtual_user_id,agent_id ON agent_endpoints FOR EACH ROW EXECUTE FUNCTION enforce_endpoint_virtual_user_org();
         CREATE TABLE budgets (id UUID, subject_type TEXT CONSTRAINT budgets_subject_type_check CHECK(subject_type IN ('agent_endpoint','agent_trigger')), subject_id TEXT, balance DOUBLE PRECISION, period_started_at TIMESTAMPTZ);
         CREATE TABLE payment_policies (id UUID, subject_type TEXT CONSTRAINT payment_policies_subject_type_check CHECK(subject_type IN ('agent_endpoint','agent')), subject_id TEXT, max_amount_usd_per_day DOUBLE PRECISION);
         INSERT INTO agent_endpoints VALUES ('00000000-0000-0000-0000-000000000001', 'appchan_00000000000000000000000000000001', NULL, NULL, '{{}}', '\\x010203', '\\x040506', 'app_existing', '00000000-0000-0000-0000-000000000002', 'live');
         INSERT INTO sessions VALUES ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000004');
         INSERT INTO budgets VALUES ('00000000-0000-0000-0000-000000000005', 'agent_endpoint', 'appchan_00000000000000000000000000000001', 17.5, '2026-01-01T00:00:00Z');
         INSERT INTO payment_policies VALUES ('00000000-0000-0000-0000-000000000006', 'agent_endpoint', 'appchan_00000000000000000000000000000001', 23.5);"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(setup.as_str()))
        .execute(&mut *tx)
        .await
        .unwrap();
    let channel_before: Value = sqlx::query_scalar("SELECT to_jsonb(c) FROM agent_endpoints c")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let budget_before: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) - 'subject_type' FROM budgets b")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let policy_before: Value =
        sqlx::query_scalar("SELECT to_jsonb(p) - 'subject_type' FROM payment_policies p")
            .fetch_one(&mut *tx)
            .await
            .unwrap();

    sqlx::raw_sql(include_str!("../../migrations/166_agent_channels.sql"))
        .execute(&mut *tx)
        .await
        .unwrap();

    for table in ["agent_channels", "app_channels"] {
        let query = format!("SELECT to_jsonb(c) FROM {table} c");
        let actual: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(query.as_str()))
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            actual, channel_before,
            "{table} preserves the entire record"
        );
    }
    let pointer: uuid::Uuid = sqlx::query_scalar("SELECT channel_id FROM sessions")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(pointer.to_string(), "00000000-0000-0000-0000-000000000001");
    for (table, before) in [
        ("budgets", budget_before),
        ("payment_policies", policy_before),
    ] {
        let query = format!("SELECT subject_type, to_jsonb(c) - 'subject_type' FROM {table} c");
        let (subject, actual): (String, Value) =
            sqlx::query_as(sqlx::AssertSqlSafe(query.as_str()))
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(subject, "agent_channel");
        assert_eq!(actual, before, "{table} does not reset or widen spending");
    }
    let rejected = sqlx::query(
        "UPDATE agent_channels SET virtual_user_id = '00000000-0000-0000-0000-000000000099'",
    )
    .execute(&mut *tx)
    .await
    .unwrap_err();
    assert!(
        rejected
            .to_string()
            .contains("Channel virtual user must belong")
    );
    tx.rollback().await.unwrap();
}
