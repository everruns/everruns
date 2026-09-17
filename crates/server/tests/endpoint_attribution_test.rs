//! Evidence for attributing sessions and budgets to endpoints (EVE-1004,
//! migration 137).
//!
//! `sessions.app_id` and the `app`/`app_channel` budget subjects record "which
//! bundle" when the useful grain is "which exposure". Migration 137 adds
//! `sessions.endpoint_id` and the `agent_endpoint` budget subject, and converts
//! the existing rows.
//!
//! The risk the migration carries is not that the new columns are missing — it
//! is that a conversion quietly guesses an endpoint for an ambiguous session,
//! resets an in-flight budget window, or loses a cap an operator configured.
//! These tests pin each of those.
//!
//! The backfill statements are re-executed here against freshly seeded rows
//! rather than asserted over whatever the migration already processed: the test
//! database is migrated before the test runs, so the only way to observe the
//! rules is to apply them to new pre-migration-shaped data. They are mirrored
//! from `crates/server/migrations/137_endpoint_attribution.sql`; change them
//! together.
//!
//! Run with: cargo test -p everruns-server --test endpoint_attribution_test -- --test-threads=1

mod test_harness;

use sqlx::PgPool;
use test_harness::get_database_url;
use uuid::Uuid;

/// Mirrors the tag-driven backfill in 137. The three tag spellings exist
/// because the convention grew per transport.
const BACKFILL_FROM_TAG: &str = r#"
    UPDATE sessions AS s
    SET endpoint_id = ae.id
    FROM agent_endpoints AS ae
    WHERE s.endpoint_id IS NULL
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
    SET endpoint_id = single.id
    FROM (
        SELECT app_id, (ARRAY_AGG(id))[1] AS id
        FROM agent_endpoints
        GROUP BY app_id
        HAVING COUNT(*) = 1
    ) AS single
    WHERE s.endpoint_id IS NULL
      AND s.app_id = single.app_id
"#;

/// Mirrors the `app` budget fan-out in 137.
const FANOUT_APP_BUDGETS: &str = r#"
    INSERT INTO budgets (
        org_id, subject_type, subject_id, currency, "limit", soft_limit,
        balance, period, metadata, status, period_started_at
    )
    SELECT
        b.org_id, 'agent_endpoint', ae.public_id, b.currency, b."limit",
        b.soft_limit, b.balance, b.period,
        COALESCE(b.metadata, '{}'::jsonb) || jsonb_build_object(
            'converted_from', 'app',
            'converted_from_subject_id', b.subject_id
        ),
        b.status, b.period_started_at
    FROM budgets AS b
    JOIN apps AS app ON app.public_id = b.subject_id AND app.org_id = b.org_id
    JOIN agent_endpoints AS ae ON ae.app_id = app.id
    WHERE b.subject_type = 'app'
      AND b.org_id = $1
      AND NOT EXISTS (
          SELECT 1 FROM budgets AS existing
          WHERE existing.org_id = b.org_id
            AND existing.subject_type = 'agent_endpoint'
            AND existing.subject_id = ae.public_id
      )
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

async fn seed_endpoint(
    pool: &PgPool,
    org: &Org,
    app_id: Uuid,
    channel_type: &str,
) -> (Uuid, String) {
    let endpoint_id = Uuid::now_v7();
    let public_id = format!("appchan_{}", hex32());
    sqlx::query(
        "INSERT INTO agent_endpoints (id, agent_id, app_id, public_id, channel_type,
                                      channel_config, enabled, status, agent_version_policy,
                                      owner_principal_id)
         VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, true, 'live', 'default', $6)",
    )
    .bind(endpoint_id)
    .bind(org.agent_id)
    .bind(app_id)
    .bind(&public_id)
    .bind(channel_type)
    .bind(org.owner_principal_id)
    .execute(pool)
    .await
    .expect("seed endpoint");
    (endpoint_id, public_id)
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
    sqlx::query_scalar("SELECT endpoint_id FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(pool)
        .await
        .expect("read endpoint_id")
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
    let (_slack_endpoint, _) = seed_endpoint(&pool, &org, app_id, "slack").await;
    let (a2a_endpoint, a2a_public_id) = seed_endpoint(&pool, &org, app_id, "a2a").await;

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
        Some(a2a_endpoint),
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
async fn per_transport_endpoint_tags_are_recognised() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-transport").await;
    let (app_id, _) = seed_app(&pool, &org).await;
    let (slack_endpoint, slack_public_id) = seed_endpoint(&pool, &org, app_id, "slack").await;
    let (fcp_endpoint, fcp_public_id) = seed_endpoint(&pool, &org, app_id, "fcp").await;

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

    assert_eq!(
        endpoint_of(&pool, slack_session).await,
        Some(slack_endpoint)
    );
    assert_eq!(endpoint_of(&pool, fcp_session).await, Some(fcp_endpoint));
}

/// An App with exactly one endpoint is unambiguous even without a tag. This is
/// the rule that recovers sessions created before the routing tags existed.
#[tokio::test]
async fn sole_endpoint_resolves_untagged_session_and_no_app_stays_null() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-solo").await;
    let (app_id, _) = seed_app(&pool, &org).await;
    let (endpoint_id, _) = seed_endpoint(&pool, &org, app_id, "fcp").await;

    let on_app = seed_session(&pool, &org, Some(app_id), &[]).await;
    let ad_hoc = seed_session(&pool, &org, None, &[]).await;

    run_session_backfill(&pool).await;

    assert_eq!(endpoint_of(&pool, on_app).await, Some(endpoint_id));
    assert_eq!(
        endpoint_of(&pool, ad_hoc).await,
        None,
        "user, API and platform sessions have no endpoint and must not acquire one"
    );
}

/// The FK is `ON DELETE SET NULL`: retiring an endpoint must not delete the
/// sessions that ran through it, and must not leave a dangling pointer either.
#[tokio::test]
async fn deleting_an_endpoint_clears_the_session_pointer() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-fk").await;
    let (app_id, _) = seed_app(&pool, &org).await;
    let (endpoint_id, _) = seed_endpoint(&pool, &org, app_id, "slack").await;
    let session_id = seed_session(&pool, &org, Some(app_id), &[]).await;

    run_session_backfill(&pool).await;
    assert_eq!(endpoint_of(&pool, session_id).await, Some(endpoint_id));

    sqlx::query("DELETE FROM agent_endpoints WHERE id = $1")
        .bind(endpoint_id)
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

/// `agent_endpoint` joins the subject types; the App-shaped ones stay until the
/// deletion phase (EVE-1011), because budgets still reference them.
#[tokio::test]
async fn subject_type_check_accepts_agent_endpoint_alongside_the_legacy_types() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-subjects").await;

    for subject_type in ["session", "agent", "user", "org", "app", "agent_endpoint"] {
        sqlx::query(
            r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                    period, status)
               VALUES ($1, $2, $3, 'USD', 10.0, 10.0, '{"kind":"calendar","unit":"month"}'::jsonb,
                       'active')"#,
        )
        .bind(org.org_id)
        .bind(subject_type)
        .bind(format!("subject_{}", hex32()))
        .execute(&pool)
        .await
        .unwrap_or_else(|err| panic!("subject_type {subject_type} must be accepted: {err}"));
    }

    let rejected = sqlx::query(
        r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                period, status)
           VALUES ($1, 'endpoint', 'whatever', 'USD', 10.0, 10.0,
                   '{"kind":"calendar","unit":"month"}'::jsonb, 'active')"#,
    )
    .bind(org.org_id)
    .execute(&pool)
    .await;
    assert!(
        rejected.is_err(),
        "the check constraint must still be closed over the known subject types"
    );
}

/// The conversion is the whole point of the migration: an operator who capped
/// an App's spend has not consented to an uncapped agent. Each endpoint keeps
/// the App's limit rather than a share of it — dividing would tighten every
/// existing cap without asking — and the in-flight window is carried over so
/// the conversion does not hand back a fresh allowance.
#[tokio::test]
async fn app_budget_fans_out_per_endpoint_preserving_limit_and_window() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-fanout").await;
    let (app_id, app_public_id) = seed_app(&pool, &org).await;
    let (_, first_public_id) = seed_endpoint(&pool, &org, app_id, "slack").await;
    let (_, second_public_id) = seed_endpoint(&pool, &org, app_id, "a2a").await;

    let window_start = chrono::Utc::now() - chrono::Duration::days(9);
    sqlx::query(
        r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                period, status, period_started_at)
           VALUES ($1, 'app', $2, 'USD', 100.0, 40.0,
                   '{"kind":"calendar","unit":"month"}'::jsonb, 'active', $3)"#,
    )
    .bind(org.org_id)
    .bind(&app_public_id)
    .bind(window_start)
    .execute(&pool)
    .await
    .expect("seed app budget");

    sqlx::query(FANOUT_APP_BUDGETS)
        .bind(org.org_id)
        .execute(&pool)
        .await
        .expect("fan out app budgets");

    for endpoint_public_id in [&first_public_id, &second_public_id] {
        let (limit, balance, started, converted_from): (
            f64,
            f64,
            chrono::DateTime<chrono::Utc>,
            Option<String>,
        ) = sqlx::query_as(
            r#"SELECT "limit", balance, period_started_at, metadata->>'converted_from'
               FROM budgets
               WHERE org_id = $1 AND subject_type = 'agent_endpoint' AND subject_id = $2"#,
        )
        .bind(org.org_id)
        .bind(endpoint_public_id)
        .fetch_one(&pool)
        .await
        .expect("every endpoint of the App gets a budget");

        assert_eq!(limit, 100.0, "the cap is preserved, not divided");
        assert_eq!(balance, 40.0, "spend already recorded is carried over");
        assert_eq!(
            started.timestamp(),
            window_start.timestamp(),
            "an in-flight window must not reset to now"
        );
        assert_eq!(converted_from.as_deref(), Some("app"));
    }

    // The App budget stays enforced until the subject type is dropped, so the
    // original ceiling keeps binding across the endpoints in the meantime.
    let app_budgets: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM budgets WHERE org_id = $1 AND subject_type = 'app'",
    )
    .bind(org.org_id)
    .fetch_one(&pool)
    .await
    .expect("count app budgets");
    assert_eq!(app_budgets, 1);
}

/// Running the fan-out twice must not double the endpoint's allowance. The
/// migration's `NOT EXISTS` guard is what makes it safe to re-apply.
#[tokio::test]
async fn app_budget_fanout_does_not_overwrite_an_existing_endpoint_budget() {
    let pool = pool().await;
    let org = seed_org(&pool, "attribution-idempotent").await;
    let (app_id, app_public_id) = seed_app(&pool, &org).await;
    let (_, endpoint_public_id) = seed_endpoint(&pool, &org, app_id, "slack").await;

    // An endpoint-scoped cap the operator set directly. It is tighter than the
    // App's, and the fan-out must not loosen it.
    sqlx::query(
        r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                period, status)
           VALUES ($1, 'agent_endpoint', $2, 'USD', 5.0, 5.0,
                   '{"kind":"calendar","unit":"month"}'::jsonb, 'active')"#,
    )
    .bind(org.org_id)
    .bind(&endpoint_public_id)
    .execute(&pool)
    .await
    .expect("seed endpoint budget");

    sqlx::query(
        r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                period, status)
           VALUES ($1, 'app', $2, 'USD', 100.0, 100.0,
                   '{"kind":"calendar","unit":"month"}'::jsonb, 'active')"#,
    )
    .bind(org.org_id)
    .bind(&app_public_id)
    .execute(&pool)
    .await
    .expect("seed app budget");

    sqlx::query(FANOUT_APP_BUDGETS)
        .bind(org.org_id)
        .execute(&pool)
        .await
        .expect("fan out app budgets");

    let limits: Vec<f64> = sqlx::query_scalar(
        r#"SELECT "limit" FROM budgets
           WHERE org_id = $1 AND subject_type = 'agent_endpoint' AND subject_id = $2"#,
    )
    .bind(org.org_id)
    .bind(&endpoint_public_id)
    .fetch_all(&pool)
    .await
    .expect("read endpoint budgets");

    assert_eq!(limits.len(), 1, "the fan-out must not add a second cap");
    assert_eq!(
        limits[0], 5.0,
        "the operator's tighter endpoint cap must survive the conversion"
    );
}

#[tokio::test]
async fn app_channel_budgets_only_survive_for_webhook_triggers() {
    let pool = pool().await;
    let unsupported: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM budgets AS budget
         WHERE budget.subject_type = 'app_channel'
           AND NOT EXISTS (
               SELECT 1
               FROM agent_triggers AS trigger
               WHERE trigger.org_id = budget.org_id
                 AND trigger.trigger_type = 'webhook'
                 AND trigger.ingress_id = budget.subject_id
           )",
    )
    .fetch_one(&pool)
    .await
    .expect("count unsupported app_channel budgets");
    assert_eq!(unsupported, 0);
}
