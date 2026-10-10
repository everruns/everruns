//! Migration 206 lists catalog servers people already signed in to in their
//! My MCP servers (knowledge/integrations/user-mcp-servers.md, D8).
//!
//! Seeds a database migrated to 204 with two people and an agent's service
//! account, personal and agent grants on active, archived and custom servers,
//! and name clashes. Then runs 206 and checks one row per person and active
//! preset, the shared suffix rule, and nothing for the agent, archived presets,
//! custom servers or presets that were already listed.
//!
//! Run with: cargo test -p everruns-server --test domain user_mcp_servers_from_grants_migration_test

use crate::test_harness::database_migrated_before;

use sqlx::Row;

const SEED: &str = r#"
INSERT INTO virtual_users (id, org_id, usage, name)
VALUES ('00000000-0000-7000-8000-0000000205a1', 1, 'end_user', 'Alice'),
       ('00000000-0000-7000-8000-0000000205b1', 1, 'end_user', 'Bob'),
       ('00000000-0000-7000-8000-0000000205c1', 1, 'service', 'Agent');

INSERT INTO mcp_servers (id, org_id, name, url, settings, status)
VALUES ('00000000-0000-7000-8000-000000020501', 1, 'visti', 'https://mcp.visti.example/mcp', '{"auth_mode": "oauth"}', 'active'),
       ('00000000-0000-7000-8000-000000020502', 1, 'linear', 'https://mcp.linear.app/mcp', '{"auth_mode": "oauth"}', 'active'),
       -- A preset whose name is the suffixed name `linear` would take.
       ('00000000-0000-7000-8000-000000020503', 1, 'linear-2', 'https://mcp2.linear.app/mcp', '{"auth_mode": "oauth"}', 'active'),
       ('00000000-0000-7000-8000-000000020504', 1, 'old', 'https://old.example/mcp', '{"auth_mode": "oauth"}', 'archived');

-- Alice already has a custom `Linear` (same tool prefix as the preset).
-- Bob already listed visti under another name.
INSERT INTO mcp_servers (id, org_id, owner_virtual_user_id, catalog_mcp_server_id, name, url, settings)
VALUES ('00000000-0000-7000-8000-000000020506', 1, '00000000-0000-7000-8000-0000000205a1', NULL, 'Linear', 'https://notes.example/mcp', '{"auth_mode": "oauth"}'),
       ('00000000-0000-7000-8000-000000020505', 1, '00000000-0000-7000-8000-0000000205b1', '00000000-0000-7000-8000-000000020501', 'my-visti', 'https://mcp.visti.example/mcp', '{"auth_mode": "none"}');

INSERT INTO virtual_user_connections (virtual_user_id, provider, connection_type, created_at)
VALUES ('00000000-0000-7000-8000-0000000205a1', 'mcp_oauth_00000000-0000-7000-8000-000000020501', 'oauth', '2026-01-01T00:00:00Z'),
       ('00000000-0000-7000-8000-0000000205a1', 'mcp_oauth_00000000-0000-7000-8000-000000020502', 'oauth', '2026-01-02T00:00:00Z'),
       ('00000000-0000-7000-8000-0000000205a1', 'mcp_oauth_00000000-0000-7000-8000-000000020503', 'oauth', '2026-01-03T00:00:00Z'),
       ('00000000-0000-7000-8000-0000000205a1', 'mcp_oauth_00000000-0000-7000-8000-000000020504', 'oauth', '2026-01-04T00:00:00Z'),
       ('00000000-0000-7000-8000-0000000205a1', 'mcp_oauth_00000000-0000-7000-8000-000000020506', 'oauth', '2026-01-05T00:00:00Z'),
       ('00000000-0000-7000-8000-0000000205a1', 'github', 'oauth', '2026-01-06T00:00:00Z'),
       ('00000000-0000-7000-8000-0000000205b1', 'mcp_oauth_00000000-0000-7000-8000-000000020501', 'oauth', '2026-01-01T00:00:00Z'),
       ('00000000-0000-7000-8000-0000000205c1', 'mcp_oauth_00000000-0000-7000-8000-000000020501', 'oauth', '2026-01-01T00:00:00Z');
"#;

#[tokio::test]
async fn personal_catalog_sign_ins_are_listed_once() {
    let database = database_migrated_before(206).await;
    let pool = &database.pool;
    sqlx::raw_sql(SEED)
        .execute(pool)
        .await
        .expect("seed pre-migration grants");

    let mut transaction = pool.begin().await.expect("begin migration");
    sqlx::raw_sql(include_str!(
        "../../migrations/206_user_mcp_servers_from_grants.sql"
    ))
    .execute(&mut *transaction)
    .await
    .expect("run migration 206");
    transaction.commit().await.expect("commit migration");

    let rows = sqlx::query(
        "SELECT v.name AS owner, s.name, p.name AS preset, s.status, s.deferred,
                s.settings->>'auth_mode' AS auth_mode, s.url
         FROM mcp_servers s
         JOIN virtual_users v ON v.id = s.owner_virtual_user_id
         LEFT JOIN mcp_servers p ON p.id = s.catalog_mcp_server_id
         ORDER BY v.name, s.name",
    )
    .fetch_all(pool)
    .await
    .expect("read user servers");
    let summary: Vec<(String, String, Option<String>)> = rows
        .iter()
        .map(|row| (row.get("owner"), row.get("name"), row.get("preset")))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("Alice".into(), "Linear".into(), None),
            // `linear` clashes with Alice's custom `Linear`; the backfill takes
            // `linear-2` first (connected earlier), so the `linear-2` preset
            // moves on to `linear-2-2`.
            ("Alice".into(), "linear-2".into(), Some("linear".into())),
            ("Alice".into(), "linear-2-2".into(), Some("linear-2".into())),
            ("Alice".into(), "visti".into(), Some("visti".into())),
            ("Bob".into(), "my-visti".into(), Some("visti".into())),
        ]
    );
    let visti = rows
        .iter()
        .find(|row| row.get::<String, _>("name") == "visti")
        .unwrap();
    assert_eq!(visti.get::<String, _>("status"), "active");
    assert!(visti.get::<bool, _>("deferred"));
    assert_eq!(visti.get::<String, _>("auth_mode"), "none");
    assert_eq!(
        visti.get::<String, _>("url"),
        "https://mcp.visti.example/mcp"
    );
}
