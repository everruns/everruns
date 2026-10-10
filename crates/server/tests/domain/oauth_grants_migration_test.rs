//! Migration 199 adds MCP OAuth grants and backfills them from live refresh
//! tokens, so clients connected before grants existed keep working.
//!
//! Seeds a database migrated to 197 with two users, two clients, live and
//! expired refresh tokens, and a token for a client that no longer exists.
//! Then runs 199 and checks one full-access, all-organizations grant per live
//! client and user, every live token linked to its grant, and nothing invented
//! for expired-only or orphaned connections.
//!
//! Run with: cargo test -p everruns-server --test domain oauth_grants_migration_test

use crate::test_harness::database_migrated_before;

use sqlx::Row;

const SEED: &str = r#"
INSERT INTO users (id, email, name)
VALUES ('00000000-0000-7000-8000-0000000000a1', 'alice-199@example.com', 'Alice'),
       ('00000000-0000-7000-8000-0000000000b1', 'bob-199@example.com', 'Bob');

INSERT INTO oauth_clients (client_id, client_secret_hash, client_name, redirect_uris)
VALUES ('mcp_client_claude', 'h', 'Claude', '["https://claude.ai/api/mcp/auth_callback"]'),
       ('mcp_client_cursor', 'h', 'Cursor', '["http://127.0.0.1:1234/callback"]');

INSERT INTO oauth_refresh_tokens (token_hash, client_id, user_id, org_id, expires_at, created_at)
VALUES
  -- Alice: two live Claude tokens (one grant, earliest created_at wins).
  ('t1', 'mcp_client_claude', '00000000-0000-7000-8000-0000000000a1', 1, NOW() + INTERVAL '10 days', '2026-01-02T00:00:00Z'),
  ('t2', 'mcp_client_claude', '00000000-0000-7000-8000-0000000000a1', 1, NOW() + INTERVAL '20 days', '2026-01-05T00:00:00Z'),
  -- Alice: Cursor only expired, so no grant.
  ('t3', 'mcp_client_cursor', '00000000-0000-7000-8000-0000000000a1', 1, NOW() - INTERVAL '1 day', '2026-01-01T00:00:00Z'),
  -- Bob: a live Cursor token.
  ('t4', 'mcp_client_cursor', '00000000-0000-7000-8000-0000000000b1', 1, NOW() + INTERVAL '5 days', '2026-01-03T00:00:00Z'),
  -- Bob: a token for a client that was never registered.
  ('t5', 'mcp_client_gone', '00000000-0000-7000-8000-0000000000b1', 1, NOW() + INTERVAL '5 days', '2026-01-03T00:00:00Z');
"#;

#[tokio::test]
async fn live_refresh_tokens_become_full_access_grants() {
    let database = database_migrated_before(199).await;
    let pool = &database.pool;
    sqlx::raw_sql(SEED)
        .execute(pool)
        .await
        .expect("seed pre-migration refresh tokens");

    let mut transaction = pool.begin().await.expect("begin migration");
    sqlx::raw_sql(include_str!("../../migrations/199_oauth_grants.sql"))
        .execute(&mut *transaction)
        .await
        .expect("run migration 199");
    transaction.commit().await.expect("commit migration");

    let grants = sqlx::query(
        "SELECT g.id, g.client_id, u.name, g.access, g.allowed_org_ids IS NULL AS all_orgs,
                g.created_at::text AS created_at, g.revoked_at IS NULL AS active
         FROM oauth_grants g JOIN users u ON u.id = g.user_id
         ORDER BY u.name, g.client_id",
    )
    .fetch_all(pool)
    .await
    .expect("read grants");
    let summary: Vec<(String, String, String, bool, bool)> = grants
        .iter()
        .map(|row| {
            (
                row.get("name"),
                row.get("client_id"),
                row.get("access"),
                row.get("all_orgs"),
                row.get("active"),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                "Alice".to_string(),
                "mcp_client_claude".to_string(),
                "read_and_run".to_string(),
                true,
                true
            ),
            (
                "Bob".to_string(),
                "mcp_client_cursor".to_string(),
                "read_and_run".to_string(),
                true,
                true
            ),
        ]
    );
    assert!(
        grants[0]
            .get::<String, _>("created_at")
            .starts_with("2026-01-02"),
        "a backfilled grant dates from the client's first live token"
    );

    let links = sqlx::query(
        "SELECT rt.token_hash, g.client_id AS grant_client
         FROM oauth_refresh_tokens rt LEFT JOIN oauth_grants g ON g.id = rt.grant_id
         ORDER BY rt.token_hash",
    )
    .fetch_all(pool)
    .await
    .expect("read token links");
    let links: Vec<(String, Option<String>)> = links
        .iter()
        .map(|row| (row.get("token_hash"), row.get("grant_client")))
        .collect();
    assert_eq!(
        links,
        vec![
            ("t1".to_string(), Some("mcp_client_claude".to_string())),
            ("t2".to_string(), Some("mcp_client_claude".to_string())),
            ("t3".to_string(), None),
            ("t4".to_string(), Some("mcp_client_cursor".to_string())),
            ("t5".to_string(), None),
        ]
    );
}
