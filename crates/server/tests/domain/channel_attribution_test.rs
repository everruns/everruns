//! Evidence for attributing sessions and budgets to endpoints (EVE-1004,
//! migration 137).
//!
//! `sessions.app_id` and the `app`/`app_channel` budget subjects recorded "which
//! bundle" when the useful grain is "which exposure". Migration 137 adds
//! `sessions.channel_id` and the `agent_channel` budget subject, and converts
//! the existing rows.
//!
//! The risk the migration carries is not that the new columns are missing — it
//! is that a conversion quietly guesses an endpoint for an ambiguous session,
//! resets an in-flight budget window, or loses a cap an operator configured.
//! These tests pin each of those.
//!
//! The `app` budget fan-out this migration performed is no longer exercised
//! here. Migration 151 (EVE-1129) removed `app` from the subject-type CHECK
//! once the ceiling had been converted onto the agent, so the pre-migration row
//! those tests seeded can no longer be inserted — there is no input shape left
//! to run 137's fan-out against. The properties they pinned (limit and recorded
//! spend carried over, in-flight window not reset, an existing tighter cap
//! never loosened) are pinned on the successor conversion in
//! `app_budget_retirement_test.rs`.
//!
//! The backfill statements are re-executed here against freshly seeded rows
//! rather than asserted over whatever the migration already processed: the test
//! database is migrated before the test runs, so the only way to observe the
//! rules is to apply them to new pre-migration-shaped data. They are mirrored
//! from `crates/server/migrations/137_endpoint_attribution.sql`; change them
//! together.
//!
//! Run with: cargo test -p everruns-server --test domain channel_attribution_test:: -- --test-threads=1

use crate::test_harness;

use sqlx::PgPool;
use test_harness::get_database_url;
use uuid::Uuid;

/// Mirrors the tag-driven backfill in 137. The three tag spellings exist
/// because the convention grew per transport.
const BACKFILL_FROM_TAG: &str = r#"
    UPDATE sessions AS s
    SET channel_id = ae.id
    FROM agent_channels AS ae
    WHERE s.channel_id IS NULL
      AND s.app_id = ae.app_id
      AND (
            ('app_channel:' || ae.public_id) = ANY (s.tags)
         OR ('slack:endpoint:' || ae.public_id) = ANY (s.tags)
         OR ('fcp:endpoint:' || ae.public_id) = ANY (s.tags)
      )
"#;

/// Mirrors the single-endpoint backfill in 137.
const BACKFILL_FROM_SOLE_ENDPOINT: &str = r#"
    UPDATE sessions AS s
    SET channel_id = single.id
    FROM (
        SELECT app_id, (ARRAY_AGG(id))[1] AS id
        FROM agent_channels
        GROUP BY app_id
        HAVING COUNT(*) = 1
    ) AS single
    WHERE s.channel_id IS NULL
      AND s.app_id = single.app_id
"#;

async fn pool() -> PgPool {
    PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL")
}

fn hex32() -> String {
    Uuid::new_v4().simple().to_string()
}

/// One isolated org with an agent, a workspace and an owner principal. Apps and
/// endpoints are added per test, because how many endpoints an App has is the
/// variable under test.
struct Org {
    org_id: i64,
    agent_id: Uuid,
    harness_id: Uuid,
    workspace_id: Uuid,
    owner_principal_id: Uuid,
}

async fn seed_org(pool: &PgPool, label: &str) -> Org {
    let org_id: i64 = sqlx::query_scalar(
        "INSERT INTO organizations (public_id, name) VALUES ($1, $2) RETURNING org_id",
    )
    .bind(format!("org_{}", hex32()))
    .bind(format!("{label}-{}", hex32()))
    .fetch_one(pool)
    .await
    .expect("seed organization");

    let owner_principal_id = Uuid::now_v7();
    sqlx::query("INSERT INTO principals (id, public_id, org_id, kind) VALUES ($1, $2, $3, 'user')")
        .bind(owner_principal_id)
        .bind(format!("principal_{}", hex32()))
        .bind(org_id)
        .execute(pool)
        .await
        .expect("seed principal");

    let workspace_id = Uuid::now_v7();
    sqlx::query("INSERT INTO workspaces (id, org_id, public_id, name) VALUES ($1, $2, $3, $4)")
        .bind(workspace_id)
        .bind(org_id)
        .bind(format!("wsp_{}", hex32()))
        .bind(format!("workspace-{}", hex32()))
        .execute(pool)
        .await
        .expect("seed workspace");

    let harness_id = Uuid::now_v7();
    sqlx::query("INSERT INTO harnesses (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(harness_id)
        .bind(org_id)
        .bind(format!("harness-{}", hex32()))
        .execute(pool)
        .await
        .expect("seed harness");

    let agent_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id)
         VALUES ($1, $2, $3, $4, '', $5)",
    )
    .bind(agent_id)
    .bind(org_id)
    .bind(format!("agent_{}", hex32()))
    .bind(format!("agent-{}", hex32()))
    .bind(harness_id)
    .execute(pool)
    .await
    .expect("seed agent");

    Org {
        org_id,
        agent_id,
        harness_id,
        workspace_id,
        owner_principal_id,
    }
}

async fn seed_app(pool: &PgPool, org: &Org) -> (Uuid, String) {
    let app_id = Uuid::now_v7();
    let public_id = format!("app_{}", hex32());
    sqlx::query(
        "INSERT INTO apps (id, org_id, public_id, name, harness_id, agent_id, status,
                           agent_version_policy, owner_principal_id, channel_type, channel_config)
         VALUES ($1, $2, $3, $4, $5, $6, 'published', 'default', $7, 'slack', '{}'::jsonb)",
    )
    .bind(app_id)
    .bind(org.org_id)
    .bind(&public_id)
    .bind(format!("app-{}", hex32()))
    .bind(org.harness_id)
    .bind(org.agent_id)
    .bind(org.owner_principal_id)
    .execute(pool)
    .await
    .expect("seed app");
    (app_id, public_id)
}

async fn seed_channel(
    pool: &PgPool,
    org: &Org,
    app_id: Uuid,
    channel_type: &str,
) -> (Uuid, String) {
    let channel_id = Uuid::now_v7();
    let public_id = format!("appchan_{}", hex32());
    sqlx::query(
        "INSERT INTO agent_channels (id, agent_id, app_id, legacy_alias_id,
                                      public_id, channel_type,
                                      channel_config, enabled, status, agent_version_policy,
                                      owner_principal_id)
         VALUES ($1, $2, $3, (SELECT public_id FROM apps WHERE id = $3),
                 $4, $5, '{}'::jsonb, true, 'live', 'default', $6)",
    )
    .bind(channel_id)
    .bind(org.agent_id)
    .bind(app_id)
    .bind(&public_id)
    .bind(channel_type)
    .bind(org.owner_principal_id)
    .execute(pool)
    .await
    .expect("seed endpoint");
    (channel_id, public_id)
}

async fn seed_session(pool: &PgPool, org: &Org, app_id: Option<Uuid>, tags: &[String]) -> Uuid {
    let session_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sessions (id, org_id, workspace_id, app_id, owner_principal_id, tags, status)
         VALUES ($1, $2, $3, $4, $5, $6, 'started')",
    )
    .bind(session_id)
    .bind(org.org_id)
    .bind(org.workspace_id)
    .bind(app_id)
    .bind(org.owner_principal_id)
    .bind(tags)
    .execute(pool)
    .await
    .expect("seed session");
    session_id
}

async fn endpoint_of(pool: &PgPool, session_id: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT channel_id FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(pool)
        .await
        .expect("read channel_id")
}

async fn run_session_backfill(pool: &PgPool) {
    sqlx::query(BACKFILL_FROM_TAG)
        .execute(pool)
        .await
        .expect("tag backfill");
    sqlx::query(BACKFILL_FROM_SOLE_ENDPOINT)
        .execute(pool)
        .await
        .expect("sole-endpoint backfill");
}

/// The routing tag names the door. An App with several endpoints is ambiguous
/// without it, and the backfill must leave that session unattributed rather
/// than pick one — `knowledge/runtime-resources/session-source-and-facets.md`
/// says derive structurally or leave unknown, never infer.
#[tokio::test]
async fn tagged_session_resolves_and_ambiguous_session_stays_null() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-tagged").await;
    let (app_id, _) = seed_app(&pool, &org).await;
    let (_slack_channel, _) = seed_channel(&pool, &org, app_id, "slack").await;
    let (a2a_channel, a2a_public_id) = seed_channel(&pool, &org, app_id, "a2a").await;

    let tagged = seed_session(
        &pool,
        &org,
        Some(app_id),
        &[format!("app_channel:{a2a_public_id}")],
    )
    .await;
    let untagged = seed_session(&pool, &org, Some(app_id), &[]).await;

    run_session_backfill(&pool).await;

    assert_eq!(
        endpoint_of(&pool, tagged).await,
        Some(a2a_channel),
        "a routing tag names exactly one endpoint and must be followed"
    );
    assert_eq!(
        endpoint_of(&pool, untagged).await,
        None,
        "an App with two endpoints cannot say which door an untagged session used"
    );
}

/// Slack and FCP never wrote `app_channel:<id>`; they write their own prefix.
/// All three are server-written and carry the endpoint's public id, so all
/// three are structural evidence rather than a guess.
#[tokio::test]
async fn per_transport_channel_tags_are_recognised() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-transport").await;
    let (app_id, _) = seed_app(&pool, &org).await;
    let (slack_channel, slack_public_id) = seed_channel(&pool, &org, app_id, "slack").await;
    let (fcp_channel, fcp_public_id) = seed_channel(&pool, &org, app_id, "fcp").await;

    let slack_session = seed_session(
        &pool,
        &org,
        Some(app_id),
        &[format!("slack:endpoint:{slack_public_id}")],
    )
    .await;
    let fcp_session = seed_session(
        &pool,
        &org,
        Some(app_id),
        &[format!("fcp:endpoint:{fcp_public_id}")],
    )
    .await;

    run_session_backfill(&pool).await;

    assert_eq!(endpoint_of(&pool, slack_session).await, Some(slack_channel));
    assert_eq!(endpoint_of(&pool, fcp_session).await, Some(fcp_channel));
}

/// An App with exactly one endpoint is unambiguous even without a tag. This is
/// the rule that recovers sessions created before the routing tags existed.
#[tokio::test]
async fn sole_channel_resolves_untagged_session_and_no_app_stays_null() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-solo").await;
    let (app_id, _) = seed_app(&pool, &org).await;
    let (channel_id, _) = seed_channel(&pool, &org, app_id, "fcp").await;

    let on_app = seed_session(&pool, &org, Some(app_id), &[]).await;
    let ad_hoc = seed_session(&pool, &org, None, &[]).await;

    run_session_backfill(&pool).await;

    assert_eq!(endpoint_of(&pool, on_app).await, Some(channel_id));
    assert_eq!(
        endpoint_of(&pool, ad_hoc).await,
        None,
        "user, API and platform sessions have no endpoint and must not acquire one"
    );
}

/// The FK is `ON DELETE SET NULL`: retiring an endpoint must not delete the
/// sessions that ran through it, and must not leave a dangling pointer either.
#[tokio::test]
async fn deleting_an_channel_clears_the_session_pointer() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-fk").await;
    let (app_id, _) = seed_app(&pool, &org).await;
    let (channel_id, _) = seed_channel(&pool, &org, app_id, "slack").await;
    let session_id = seed_session(&pool, &org, Some(app_id), &[]).await;

    run_session_backfill(&pool).await;
    assert_eq!(endpoint_of(&pool, session_id).await, Some(channel_id));

    sqlx::query("DELETE FROM agent_channels WHERE id = $1")
        .bind(channel_id)
        .execute(&pool)
        .await
        .expect("delete endpoint");

    let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&pool)
        .await
        .expect("count session");
    assert_eq!(still_there, 1, "the session must survive its endpoint");
    assert_eq!(endpoint_of(&pool, session_id).await, None);
}

/// The subject types are a closed set, and neither App-shaped level is in it
/// any more. Migration 151 removed `app` once its ceiling had been converted
/// onto the agent (EVE-1129); migration 153 removed `app_channel` once trigger
/// ingress had a structural subject to carry it (EVE-1138).
#[tokio::test]
async fn subject_type_check_rejects_the_retired_app_levels() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-subjects").await;

    async fn insert(pool: &PgPool, org_id: i64, subject_type: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                    period, status)
               VALUES ($1, $2, $3, 'USD', 10.0, 10.0, '{"kind":"calendar","unit":"month"}'::jsonb,
                       'active')"#,
        )
        .bind(org_id)
        .bind(subject_type)
        .bind(format!("subject_{}", hex32()))
        .execute(pool)
        .await
        .map(|_| ())
    }

    for subject_type in [
        "session",
        "agent",
        "user",
        "org",
        "agent_trigger",
        "agent_channel",
    ] {
        insert(&pool, org.org_id, subject_type)
            .await
            .unwrap_or_else(|err| panic!("subject_type {subject_type} must be accepted: {err}"));
    }

    for subject_type in ["app", "app_channel", "endpoint"] {
        assert!(
            insert(&pool, org.org_id, subject_type).await.is_err(),
            "subject_type {subject_type} must be rejected"
        );
    }
}

/// Nothing is left on either retired level. Migration 153 re-keys every
/// `app_channel` row onto `agent_trigger` and raises if one cannot be matched,
/// so a surviving row would mean the migration ran and silently lost a cap.
#[tokio::test]
async fn no_budget_remains_on_a_retired_app_level() {
    let pool = pool().await;
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM budgets WHERE subject_type IN ('app', 'app_channel')",
    )
    .fetch_one(&pool)
    .await
    .expect("count budgets on retired levels");
    assert_eq!(remaining, 0);
}
