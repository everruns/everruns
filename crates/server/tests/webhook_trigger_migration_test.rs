mod test_harness;

use sqlx::{Connection, Executor, PgConnection, Row};

#[tokio::test]
async fn webhook_channel_migration_preserves_ingress_config_and_execution_context() {
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
        CREATE TEMPORARY TABLE agents (
            id UUID PRIMARY KEY,
            org_id BIGINT NOT NULL,
            public_id TEXT NOT NULL,
            name VARCHAR(255) NOT NULL,
            display_name TEXT,
            system_prompt TEXT NOT NULL,
            harness_id UUID NOT NULL,
            tags TEXT[] NOT NULL
        );
        CREATE TEMPORARY TABLE apps (
            id UUID PRIMARY KEY,
            public_id TEXT NOT NULL,
            org_id BIGINT NOT NULL,
            name VARCHAR(255) NOT NULL,
            agent_id UUID,
            harness_id UUID NOT NULL,
            owner_principal_id UUID NOT NULL,
            resolved_owner_user_id UUID,
            agent_identity_id UUID,
            channel_type VARCHAR(50),
            channel_config JSONB NOT NULL,
            channel_config_encrypted BYTEA
        );
        CREATE TEMPORARY TABLE agent_endpoints (
            id UUID PRIMARY KEY,
            app_id UUID NOT NULL,
            agent_id UUID,
            public_id VARCHAR(100) NOT NULL,
            channel_type VARCHAR(50) NOT NULL,
            channel_config JSONB NOT NULL,
            channel_config_encrypted BYTEA,
            enabled BOOLEAN NOT NULL,
            status TEXT NOT NULL,
            owner_principal_id UUID NOT NULL,
            resolved_owner_user_id UUID,
            agent_identity_id UUID
        );
        CREATE TEMPORARY TABLE agent_triggers (
            id UUID PRIMARY KEY,
            org_id BIGINT NOT NULL,
            agent_id UUID NOT NULL,
            trigger_type VARCHAR(50) NOT NULL,
            config JSONB NOT NULL,
            enabled BOOLEAN NOT NULL,
            durable_schedule_id UUID,
            execution_harness_id UUID,
            execution_owner_principal_id UUID,
            execution_resolved_owner_user_id UUID,
            execution_agent_identity_id UUID,
            execution_app_id UUID,
            status VARCHAR(50) NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            archived_at TIMESTAMPTZ,
            deleted_at TIMESTAMPTZ
        );
        CREATE TEMPORARY TABLE budgets (
            id UUID PRIMARY KEY,
            org_id BIGINT NOT NULL,
            subject_type TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            currency TEXT NOT NULL,
            "limit" DOUBLE PRECISION NOT NULL,
            soft_limit DOUBLE PRECISION,
            balance DOUBLE PRECISION NOT NULL,
            period JSONB,
            metadata JSONB,
            status TEXT NOT NULL,
            period_started_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL
        );
        CREATE TEMPORARY TABLE sessions (
            id UUID PRIMARY KEY,
            app_id UUID,
            endpoint_id UUID REFERENCES agent_endpoints(id) ON DELETE SET NULL,
            tags TEXT[] NOT NULL
        );

        INSERT INTO agents (
            id, org_id, public_id, name, display_name, system_prompt,
            harness_id, tags
        )
        VALUES (
            '00000000-0000-0000-0000-000000000001',
            42,
            'agent_00000000000000000000000000000001',
            'existing-agent',
            'Existing Agent',
            '',
            '00000000-0000-0000-0000-000000000003',
            ARRAY[]::TEXT[]
        );
        INSERT INTO apps (
            id, public_id, org_id, name, agent_id, harness_id,
            owner_principal_id, resolved_owner_user_id, agent_identity_id,
            channel_type, channel_config, channel_config_encrypted
        )
        VALUES
        (
            '00000000-0000-0000-0000-000000000002',
            'app_00000000000000000000000000000002',
            42,
            'Existing Webhook App',
            '00000000-0000-0000-0000-000000000001',
            '00000000-0000-0000-0000-000000000003',
            '00000000-0000-0000-0000-000000000004',
            '00000000-0000-0000-0000-000000000005',
            '00000000-0000-0000-0000-000000000006',
            'webhook',
            '{"legacy":"app-config"}',
            decode('aabb', 'hex')
        ),
        (
            '00000000-0000-0000-0000-00000000000a',
            'app_0000000000000000000000000000000a',
            42,
            'Grandfathered Webhook App',
            NULL,
            '00000000-0000-0000-0000-000000000003',
            '00000000-0000-0000-0000-000000000004',
            NULL,
            NULL,
            'webhook',
            '{"legacy":"agentless-app-config"}',
            NULL
        );
        INSERT INTO agent_endpoints (
            id, app_id, agent_id, public_id, channel_type, channel_config,
            channel_config_encrypted, enabled, status, owner_principal_id,
            resolved_owner_user_id, agent_identity_id
        )
        VALUES
        (
            '00000000-0000-0000-0000-000000000007',
            '00000000-0000-0000-0000-000000000002',
            '00000000-0000-0000-0000-000000000001',
            'appchan_00000000000000000000000000000007',
            'webhook',
            '{"token":"plain-secret","session_mode":"shared_session","message":"plain"}',
            NULL,
            true,
            'live',
            '00000000-0000-0000-0000-000000000004',
            '00000000-0000-0000-0000-000000000005',
            '00000000-0000-0000-0000-000000000006'
        ),
        (
            '00000000-0000-0000-0000-000000000008',
            '00000000-0000-0000-0000-000000000002',
            '00000000-0000-0000-0000-000000000001',
            'appchan_00000000000000000000000000000008',
            'webhook',
            '{}',
            decode('01020304', 'hex'),
            true,
            'draft',
            '00000000-0000-0000-0000-000000000004',
            '00000000-0000-0000-0000-000000000005',
            '00000000-0000-0000-0000-000000000006'
        ),
        (
            '00000000-0000-0000-0000-000000000009',
            '00000000-0000-0000-0000-000000000002',
            '00000000-0000-0000-0000-000000000001',
            'appchan_00000000000000000000000000000009',
            'slack',
            '{}',
            NULL,
            true,
            'live',
            '00000000-0000-0000-0000-000000000004',
            '00000000-0000-0000-0000-000000000005',
            '00000000-0000-0000-0000-000000000006'
        ),
        (
            '00000000-0000-0000-0000-000000000010',
            '00000000-0000-0000-0000-00000000000a',
            NULL,
            'appchan_00000000000000000000000000000010',
            'webhook',
            '{"token":"synthesized-secret","session_mode":"shared_session","message":"synthesized"}',
            NULL,
            true,
            'live',
            '00000000-0000-0000-0000-000000000004',
            NULL,
            NULL
        );
        INSERT INTO budgets (
            id, org_id, subject_type, subject_id, currency, "limit", soft_limit,
            balance, period, metadata, status, period_started_at, created_at, updated_at
        )
        VALUES
        (
            '00000000-0000-0000-0000-000000000011',
            42,
            'agent_endpoint',
            'appchan_00000000000000000000000000000007',
            'tokens',
            100,
            25,
            0,
            '{"type":"rolling","window":"1d"}',
            '{"policy":"preserve-me"}',
            'exhausted',
            '2026-09-01T00:00:00Z',
            '2026-08-01T00:00:00Z',
            '2026-09-01T01:00:00Z'
        ),
        (
            '00000000-0000-0000-0000-000000000012',
            42,
            'agent_endpoint',
            'appchan_00000000000000000000000000000009',
            'usd',
            50,
            NULL,
            40,
            NULL,
            '{"policy":"unrelated"}',
            'active',
            NULL,
            '2026-08-02T00:00:00Z',
            '2026-09-02T01:00:00Z'
        );
        INSERT INTO sessions (id, app_id, endpoint_id, tags)
        VALUES
        (
            '00000000-0000-0000-0000-000000000013',
            '00000000-0000-0000-0000-000000000002',
            '00000000-0000-0000-0000-000000000007',
            ARRAY[
                'legacy-tag',
                'app_channel_type:webhook',
                '__internal:app_invocation'
            ]
        ),
        (
            '00000000-0000-0000-0000-000000000014',
            '00000000-0000-0000-0000-000000000002',
            '00000000-0000-0000-0000-000000000008',
            ARRAY[
                'app:app_00000000000000000000000000000002',
                'app:app_00000000000000000000000000000002',
                'app_channel:appchan_00000000000000000000000000000008',
                'app_channel:appchan_00000000000000000000000000000008'
            ]
        ),
        (
            '00000000-0000-0000-0000-000000000015',
            '00000000-0000-0000-0000-000000000002',
            '00000000-0000-0000-0000-000000000009',
            ARRAY['unrelated-tag']
        );
        "#,
    )
    .execute(&mut *transaction)
    .await
    .expect("seed pre-migration schema");
    sqlx::raw_sql(include_str!(
        "../migrations/134_synthesize_agents_for_agentless_apps.sql"
    ))
    .execute(&mut *transaction)
    .await
    .expect("synthesize agents for grandfathered Apps");
    let synthesized_agent_id = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT agent_id FROM apps WHERE id = '00000000-0000-0000-0000-00000000000a'",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read synthesized agent id");
    sqlx::query(
        "UPDATE agent_endpoints AS endpoint
         SET agent_id = app.agent_id
         FROM apps AS app
         WHERE endpoint.app_id = app.id AND endpoint.agent_id IS NULL",
    )
    .execute(&mut *transaction)
    .await
    .expect("model endpoint backfill after agent synthesis");
    let apps_without_agents =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM apps WHERE agent_id IS NULL")
            .fetch_one(&mut *transaction)
            .await
            .expect("count Apps without agents");
    assert_eq!(apps_without_agents, 0);

    sqlx::raw_sql(include_str!(
        "../migrations/138_migrate_app_webhooks_to_agent_triggers.sql"
    ))
    .execute(&mut *transaction)
    .await
    .expect("run webhook trigger migration");

    let triggers = sqlx::query(
        r#"
        SELECT ingress_id, config, config_encrypted, enabled, org_id, agent_id,
               execution_harness_id, execution_owner_principal_id,
               execution_resolved_owner_user_id, execution_agent_identity_id,
               execution_app_id
        FROM agent_triggers
        ORDER BY ingress_id
        "#,
    )
    .fetch_all(&mut *transaction)
    .await
    .expect("read migrated triggers");
    assert_eq!(triggers.len(), 3);
    assert_eq!(
        triggers[0].get::<String, _>("ingress_id"),
        "appchan_00000000000000000000000000000007"
    );
    assert_eq!(
        triggers[0].get::<serde_json::Value, _>("config"),
        serde_json::json!({
            "token": "plain-secret",
            "session_mode": "shared_session",
            "message": "plain",
        })
    );
    assert_eq!(
        triggers[0].get::<Option<Vec<u8>>, _>("config_encrypted"),
        None
    );
    assert!(triggers[0].get::<bool, _>("enabled"));
    assert_eq!(
        triggers[1].get::<Option<Vec<u8>>, _>("config_encrypted"),
        Some(vec![1, 2, 3, 4])
    );
    assert!(!triggers[1].get::<bool, _>("enabled"));
    assert_eq!(
        triggers[2].get::<uuid::Uuid, _>("agent_id"),
        synthesized_agent_id
    );
    assert!(triggers[2].get::<bool, _>("enabled"));

    let migrated_budget = sqlx::query(
        r#"
        SELECT subject_type, subject_id, currency, "limit", soft_limit, balance,
               period, metadata, status, period_started_at, created_at
        FROM budgets
        WHERE id = '00000000-0000-0000-0000-000000000011'
        "#,
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read migrated webhook endpoint budget");
    assert_eq!(
        migrated_budget.get::<String, _>("subject_type"),
        "app_channel"
    );
    assert_eq!(
        migrated_budget.get::<String, _>("subject_id"),
        "appchan_00000000000000000000000000000007"
    );
    assert_eq!(migrated_budget.get::<String, _>("currency"), "tokens");
    assert_eq!(migrated_budget.get::<f64, _>("limit"), 100.0);
    assert_eq!(
        migrated_budget.get::<Option<f64>, _>("soft_limit"),
        Some(25.0)
    );
    assert_eq!(migrated_budget.get::<f64, _>("balance"), 0.0);
    assert_eq!(
        migrated_budget.get::<Option<serde_json::Value>, _>("period"),
        Some(serde_json::json!({"type": "rolling", "window": "1d"}))
    );
    assert_eq!(
        migrated_budget.get::<Option<serde_json::Value>, _>("metadata"),
        Some(serde_json::json!({"policy": "preserve-me"}))
    );
    assert_eq!(migrated_budget.get::<String, _>("status"), "exhausted");
    assert_eq!(
        migrated_budget
            .get::<Option<chrono::DateTime<chrono::Utc>>, _>("period_started_at")
            .expect("period start"),
        "2026-09-01T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );
    assert_eq!(
        migrated_budget.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        "2026-08-01T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap()
    );

    let unrelated_budget = sqlx::query(
        "SELECT subject_type, metadata
         FROM budgets
         WHERE id = '00000000-0000-0000-0000-000000000012'",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read unrelated endpoint budget");
    assert_eq!(
        unrelated_budget.get::<String, _>("subject_type"),
        "agent_endpoint"
    );
    assert_eq!(
        unrelated_budget.get::<Option<serde_json::Value>, _>("metadata"),
        Some(serde_json::json!({"policy": "unrelated"}))
    );
    let legacy_session = sqlx::query(
        "SELECT endpoint_id, tags
         FROM sessions
         WHERE id = '00000000-0000-0000-0000-000000000013'",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read legacy webhook session");
    assert_eq!(
        legacy_session.get::<Option<uuid::Uuid>, _>("endpoint_id"),
        None
    );
    assert_eq!(
        legacy_session.get::<Vec<String>, _>("tags"),
        vec![
            "legacy-tag",
            "app_channel_type:webhook",
            "__internal:app_invocation",
            "app:app_00000000000000000000000000000002",
            "app_channel:appchan_00000000000000000000000000000007",
        ]
    );
    let already_tagged = sqlx::query(
        "SELECT tags
         FROM sessions
         WHERE id = '00000000-0000-0000-0000-000000000014'",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read already-tagged webhook session")
    .get::<Vec<String>, _>("tags");
    assert_eq!(
        already_tagged
            .iter()
            .filter(|tag| *tag == "app:app_00000000000000000000000000000002")
            .count(),
        1
    );
    assert_eq!(
        already_tagged
            .iter()
            .filter(|tag| { *tag == "app_channel:appchan_00000000000000000000000000000008" })
            .count(),
        1
    );
    let unrelated_session = sqlx::query(
        "SELECT endpoint_id, tags
         FROM sessions
         WHERE id = '00000000-0000-0000-0000-000000000015'",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read unrelated endpoint session");
    assert_eq!(
        unrelated_session
            .get::<Option<uuid::Uuid>, _>("endpoint_id")
            .expect("unrelated endpoint remains")
            .to_string(),
        "00000000-0000-0000-0000-000000000009"
    );
    assert_eq!(
        unrelated_session.get::<Vec<String>, _>("tags"),
        vec!["unrelated-tag"]
    );

    let migrated = &triggers[0];
    assert_eq!(migrated.get::<i64, _>("org_id"), 42);
    for (column, expected) in [
        ("agent_id", "00000000-0000-0000-0000-000000000001"),
        (
            "execution_harness_id",
            "00000000-0000-0000-0000-000000000003",
        ),
        (
            "execution_owner_principal_id",
            "00000000-0000-0000-0000-000000000004",
        ),
        (
            "execution_resolved_owner_user_id",
            "00000000-0000-0000-0000-000000000005",
        ),
        (
            "execution_agent_identity_id",
            "00000000-0000-0000-0000-000000000006",
        ),
        ("execution_app_id", "00000000-0000-0000-0000-000000000002"),
    ] {
        assert_eq!(
            migrated.get::<uuid::Uuid, _>(column).to_string(),
            expected,
            "{column} must retain the endpoint execution context"
        );
    }

    let remaining_channel_types =
        sqlx::query_scalar::<_, String>("SELECT channel_type FROM agent_endpoints")
            .fetch_all(&mut *transaction)
            .await
            .expect("read remaining agent endpoints");
    assert_eq!(remaining_channel_types, vec!["slack"]);
    let app = sqlx::query(
        "SELECT channel_type, channel_config, channel_config_encrypted
         FROM apps
         WHERE id = '00000000-0000-0000-0000-000000000002'",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read migrated App");
    assert_eq!(app.get::<Option<String>, _>("channel_type"), None);
    assert_eq!(
        app.get::<serde_json::Value, _>("channel_config"),
        serde_json::json!({})
    );
    assert_eq!(
        app.get::<Option<Vec<u8>>, _>("channel_config_encrypted"),
        None
    );

    transaction
        .rollback()
        .await
        .expect("roll back migration test");
}
