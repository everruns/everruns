//! Migration 184 retires Agent Versions (change-history phase 5).
//!
//! Seeds a database migrated to 183 with what versions left behind:
//! published, automatic, rolled-back and forked versions, history recorded
//! since phase 1 (including a restore), and pins on an app, a channel, a
//! trigger, a participant, a session and a trace score. Then runs 184 and
//! checks that every version became a revision in one created_at-ordered
//! timeline with its summary as the reason, a restore still points at the
//! revision it brought back, sessions carry `agent_revision`, the pins and the
//! table are gone, and fork lineage is intact.
//!
//! Run with: cargo test -p everruns-server --test domain agent_versions_retirement_migration_test

use crate::test_harness::database_migrated_before;

use sqlx::{PgPool, Row};

const SEED: &str = r#"
INSERT INTO organizations (org_id, public_id, name)
VALUES (9180, 'org_00000000000000000000000000009180', 'Versions org');

INSERT INTO principals (id, public_id, org_id, kind)
VALUES ('00000000-0000-7000-8000-000000000001', 'principal_00000000000000000000000000000001', 9180, 'user');

INSERT INTO harnesses (id, org_id, name)
VALUES ('00000000-0000-7000-8000-000000000002', 9180, 'versions-harness');

INSERT INTO workspaces (id, org_id, public_id, name)
VALUES ('00000000-0000-7000-8000-000000000003', 9180, 'wsp_00000000000000000000000000000003', 'ws');

-- Agent A: published, automatic and rolled-back versions, a default, and
-- history recorded since phase 1 (an update and a restore of it).
INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id, intro_markdown, mcp_servers)
VALUES ('00000000-0000-7000-8000-00000000000a', 9180, 'agent_0000000000000000000000000000000a',
        'versioned', 'current prompt', '00000000-0000-7000-8000-000000000002', 'Hello there',
        '{"docs": {"url": "https://example.com/mcp"}}');
-- Agent C: history but no versions; must not be touched.
INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id)
VALUES ('00000000-0000-7000-8000-00000000000c', 9180, 'agent_0000000000000000000000000000000c',
        'unversioned', 'c prompt', '00000000-0000-7000-8000-000000000002');

INSERT INTO agent_versions (id, org_id, public_id, agent_id, version_number, version, is_published,
                            change_kind, summary, config_hash, authored_config, resolved_config, created_at)
VALUES
  ('00000000-0000-7000-8000-0000000000a1', 9180, 'agentver_000000000000000000000000000000a1',
   '00000000-0000-7000-8000-00000000000a', 1, '0.1.0', true, 'manual', 'Initial release', 'h1',
   '{"system_prompt": "prompt one", "mcp_servers": {"docs": {"url": "https://example.com/mcp"}}}', '{}',
   '2026-01-01T00:00:00Z'),
  ('00000000-0000-7000-8000-0000000000a2', 9180, 'agentver_000000000000000000000000000000a2',
   '00000000-0000-7000-8000-00000000000a', 2, 'draft.2', false, 'auto', NULL, 'h2',
   '{"system_prompt": "prompt two"}', '{}', '2026-01-02T00:00:00Z'),
  ('00000000-0000-7000-8000-0000000000a3', 9180, 'agentver_000000000000000000000000000000a3',
   '00000000-0000-7000-8000-00000000000a', 3, '0.1.1', true, 'rollback', 'Rolled back to 0.1.0', 'h3',
   '{"system_prompt": "prompt one"}', '{}', '2026-01-04T00:00:00Z');

UPDATE agents SET default_version_id = '00000000-0000-7000-8000-0000000000a3'
WHERE id = '00000000-0000-7000-8000-00000000000a';

-- Agent B: forked from A's version 1.
INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id,
                    forked_from_agent_id, forked_from_version_id, root_agent_id)
VALUES ('00000000-0000-7000-8000-00000000000b', 9180, 'agent_0000000000000000000000000000000b',
        'forked', 'prompt one', '00000000-0000-7000-8000-000000000002',
        '00000000-0000-7000-8000-00000000000a', '00000000-0000-7000-8000-0000000000a1',
        '00000000-0000-7000-8000-00000000000a');
INSERT INTO agent_versions (id, org_id, public_id, agent_id, version_number, version, is_published,
                            change_kind, summary, config_hash, authored_config, resolved_config, created_at)
VALUES ('00000000-0000-7000-8000-0000000000b1', 9180, 'agentver_000000000000000000000000000000b1',
        '00000000-0000-7000-8000-00000000000b', 1, '0.1.0', true, 'fork', 'Forked from versioned', 'hb',
        '{"system_prompt": "prompt one"}', '{}', '2026-01-05T00:00:00Z');

INSERT INTO entity_changes (id, org_id, entity_kind, entity_ref, command, action, actor_kind, surface,
                            revision, snapshot, snapshot_hash, created_at, restored_from_revision)
VALUES
  ('00000000-0000-7000-8000-0000000000e1', 9180, 'agent', 'agent_0000000000000000000000000000000a',
   'update_agent', 'updated', 'user', 'api', 1, '{"system_prompt": "prompt three"}', 'x1',
   '2026-01-03T00:00:00Z', NULL),
  ('00000000-0000-7000-8000-0000000000e2', 9180, 'agent', 'agent_0000000000000000000000000000000a',
   'restore_entity', 'restored', 'user', 'api', 2, '{"system_prompt": "prompt three"}', 'x1',
   '2026-01-06T00:00:00Z', 1),
  ('00000000-0000-7000-8000-0000000000e3', 9180, 'agent', 'agent_0000000000000000000000000000000c',
   'update_agent', 'updated', 'user', 'api', 1, '{"system_prompt": "c prompt"}', 'xc',
   '2026-01-03T00:00:00Z', NULL);

-- Pins: an app, a channel and a trigger, a participant, a captured session.
INSERT INTO apps (id, org_id, public_id, name, harness_id, agent_id, owner_principal_id,
                  agent_version_policy, agent_version_id)
VALUES ('00000000-0000-7000-8000-000000000010', 9180, 'app_00000000000000000000000000000010', 'pinned app',
        '00000000-0000-7000-8000-000000000002', '00000000-0000-7000-8000-00000000000a',
        '00000000-0000-7000-8000-000000000001', 'pinned', '00000000-0000-7000-8000-0000000000a1');
INSERT INTO agent_channels (id, agent_id, public_id, channel_type, owner_principal_id,
                            agent_version_policy, agent_version_id)
VALUES ('00000000-0000-7000-8000-000000000011', '00000000-0000-7000-8000-00000000000a',
        'appchan_00000000000000000000000000000011', 'webhook', '00000000-0000-7000-8000-000000000001',
        'pinned', '00000000-0000-7000-8000-0000000000a1'),
       ('00000000-0000-7000-8000-000000000012', '00000000-0000-7000-8000-00000000000a',
        'appchan_00000000000000000000000000000012', 'slack', '00000000-0000-7000-8000-000000000001',
        'default', NULL);
INSERT INTO agent_triggers (id, org_id, agent_id, agent_version_policy)
VALUES ('00000000-0000-7000-8000-000000000013', 9180, '00000000-0000-7000-8000-00000000000a', 'latest');

INSERT INTO sessions (id, org_id, harness_id, agent_id, agent_version_id, agent_config_hash,
                      owner_principal_id, workspace_id)
VALUES ('00000000-0000-7000-8000-000000000020', 9180, '00000000-0000-7000-8000-000000000002',
        '00000000-0000-7000-8000-00000000000a', '00000000-0000-7000-8000-0000000000a3', 'h3',
        '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000003'),
       ('00000000-0000-7000-8000-000000000021', 9180, '00000000-0000-7000-8000-000000000002',
        '00000000-0000-7000-8000-00000000000a', NULL, NULL,
        '00000000-0000-7000-8000-000000000001', '00000000-0000-7000-8000-000000000003');
INSERT INTO session_participants (org_id, session_id, kind, agent_id, agent_version_id, principal_id, role)
VALUES (9180, '00000000-0000-7000-8000-000000000020', 'agent', '00000000-0000-7000-8000-00000000000a',
        '00000000-0000-7000-8000-0000000000a1', '00000000-0000-7000-8000-000000000001', 'host');

INSERT INTO observers (id, org_id, public_id, name)
VALUES ('00000000-0000-7000-8000-000000000030', 9180, 'observer_00000000000000000000000000000030', 'obs');
INSERT INTO trace_scores (org_id, public_id, observer_id, scorer_key, session_id, turn_id, agent_id, agent_version_id)
VALUES (9180, 'score_00000000000000000000000000000031', '00000000-0000-7000-8000-000000000030', 'k',
        '00000000-0000-7000-8000-000000000020', 'turn_1', '00000000-0000-7000-8000-00000000000a',
        '00000000-0000-7000-8000-0000000000a1');
"#;

const AGENT_A: &str = "agent_0000000000000000000000000000000a";
const AGENT_B: &str = "agent_0000000000000000000000000000000b";
const AGENT_C: &str = "agent_0000000000000000000000000000000c";

async fn column_exists(pool: &PgPool, table: &str, column: &str) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM information_schema.columns
             WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2
         )",
    )
    .bind(table)
    .bind(column)
    .fetch_one(pool)
    .await
    .expect("read information_schema")
}

#[tokio::test]
async fn versions_become_one_history_timeline_and_pins_are_dropped() {
    let database = database_migrated_before(184).await;
    let pool = &database.pool;
    sqlx::raw_sql(SEED)
        .execute(pool)
        .await
        .expect("seed pre-migration versions and pins");

    let mut transaction = pool.begin().await.expect("begin migration");
    sqlx::raw_sql(include_str!(
        "../../migrations/184_retire_agent_versions.sql"
    ))
    .execute(&mut *transaction)
    .await
    .expect("run migration 184");
    transaction.commit().await.expect("commit migration");

    let rows = sqlx::query(
        "SELECT entity_ref, revision, restored_from_revision, action, command,
                actor_kind, surface, reason, snapshot
         FROM entity_changes
         WHERE org_id = 9180
         ORDER BY entity_ref, revision",
    )
    .fetch_all(pool)
    .await
    .expect("read history");
    let timeline = |entity: &str| {
        rows.iter()
            .filter(|row| row.get::<String, _>("entity_ref") == entity)
            .map(|row| {
                (
                    row.get::<i64, _>("revision"),
                    row.get::<String, _>("command"),
                    row.get::<Option<String>, _>("reason"),
                )
            })
            .collect::<Vec<_>>()
    };

    // Agent A: three versions and two later entries, one timeline by time.
    assert_eq!(
        timeline(AGENT_A),
        vec![
            (
                1,
                "migrate_agent_versions".to_string(),
                Some("Agent version 0.1.0: Initial release".to_string())
            ),
            (
                2,
                "migrate_agent_versions".to_string(),
                Some("Automatic snapshot draft.2".to_string())
            ),
            (3, "update_agent".to_string(), None),
            (
                4,
                "migrate_agent_versions".to_string(),
                Some("Agent version 0.1.1: Rolled back to 0.1.0".to_string())
            ),
            (5, "restore_entity".to_string(), None),
        ]
    );
    let restore = rows
        .iter()
        .find(|row| row.get::<String, _>("command") == "restore_entity")
        .expect("restore entry");
    assert_eq!(
        restore.get::<Option<i64>, _>("restored_from_revision"),
        Some(3),
        "a restore follows the revision it brought back"
    );

    let migrated = rows
        .iter()
        .find(|row| {
            row.get::<String, _>("entity_ref") == AGENT_A && row.get::<i64, _>("revision") == 1
        })
        .expect("first migrated version");
    assert_eq!(migrated.get::<String, _>("actor_kind"), "system");
    assert_eq!(migrated.get::<String, _>("surface"), "internal");
    assert_eq!(migrated.get::<String, _>("action"), "updated");
    let snapshot: serde_json::Value = migrated.get("snapshot");
    assert_eq!(snapshot["id"], AGENT_A);
    assert_eq!(snapshot["system_prompt"], "prompt one");
    assert_eq!(
        snapshot["mcpServers"]["docs"]["url"], "https://example.com/mcp",
        "mcp_servers takes the agent's wire name"
    );
    assert!(snapshot.get("mcp_servers").is_none());
    assert_eq!(
        snapshot["intro_markdown"], "Hello there",
        "fields versions never captured come from the current agent"
    );

    // Agent B: its fork version is a `forked` revision.
    assert_eq!(timeline(AGENT_B).len(), 1);
    let forked = rows
        .iter()
        .find(|row| row.get::<String, _>("entity_ref") == AGENT_B)
        .expect("fork revision");
    assert_eq!(forked.get::<String, _>("action"), "forked");

    // Agent C had no versions; its history is untouched.
    assert_eq!(
        timeline(AGENT_C),
        vec![(1, "update_agent".to_string(), None)]
    );

    // Sessions record the revision of the version they captured.
    let revisions =
        sqlx::query("SELECT id::text, agent_revision FROM sessions WHERE org_id = 9180")
            .fetch_all(pool)
            .await
            .expect("read sessions")
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("id"),
                    row.get::<Option<i64>, _>("agent_revision"),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        revisions["00000000-0000-7000-8000-000000000020"],
        Some(4),
        "the captured version 0.1.1 is revision 4"
    );
    assert_eq!(revisions["00000000-0000-7000-8000-000000000021"], None);

    // Fork lineage stays; every pin and the versions table are gone.
    let lineage = sqlx::query(
        "SELECT forked_from_agent_id::text, root_agent_id::text FROM agents WHERE public_id = $1",
    )
    .bind(AGENT_B)
    .fetch_one(pool)
    .await
    .expect("read fork lineage");
    assert_eq!(
        lineage.get::<Option<String>, _>(0).as_deref(),
        Some("00000000-0000-7000-8000-00000000000a")
    );
    assert_eq!(
        lineage.get::<Option<String>, _>(1).as_deref(),
        Some("00000000-0000-7000-8000-00000000000a")
    );
    for (table, column) in [
        ("agents", "default_version_id"),
        ("agents", "forked_from_version_id"),
        ("agent_channels", "agent_version_policy"),
        ("agent_channels", "agent_version_id"),
        ("agent_triggers", "agent_version_policy"),
        ("agent_triggers", "agent_version_id"),
        ("apps", "agent_version_policy"),
        ("apps", "agent_version_id"),
        ("session_participants", "agent_version_id"),
        ("sessions", "agent_version_id"),
        ("sessions", "agent_config_hash"),
        ("trace_scores", "agent_version_id"),
    ] {
        assert!(
            !column_exists(pool, table, column).await,
            "{table}.{column} should be dropped"
        );
    }
    let versions_table = sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_regclass('public.agent_versions')::text",
    )
    .fetch_one(pool)
    .await
    .expect("look up agent_versions");
    assert_eq!(versions_table, None);

    // The pinned channel and participant are still there, now unpinned.
    let channels: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_channels
         WHERE agent_id = '00000000-0000-7000-8000-00000000000a'",
    )
    .fetch_one(pool)
    .await
    .expect("count channels");
    assert_eq!(channels, 2);
    let participants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM session_participants WHERE org_id = 9180 AND kind = 'agent'",
    )
    .fetch_one(pool)
    .await
    .expect("count participants");
    assert_eq!(participants, 1);
}
