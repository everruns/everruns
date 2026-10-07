//! Evidence for retiring the `app` payment-policy subject (EVE-1130,
//! migration 152).
//!
//! EVE-1004 moved budgets off App-shaped subjects and never reached payment
//! policies, which are a separate subsystem with their own subject-type list.
//! The API, the CHECK constraint and the UI dropdown all still accepted `app`.
//!
//! The precise shape of the bug matters for what the migration owes these rows.
//! An `app` policy was accepted and stored, but `subject_candidates` never
//! produced an `app` pair and a policy authorizes a payment only on an exact
//! `(subject_type, subject_id)` match — so no `app` policy has ever authorized
//! anything. They are converted rather than deleted anyway: the operator who
//! wrote one expressed an intent about spend, and is owed having it start
//! working rather than vanish.
//!
//! Every test runs inside one transaction that is never committed, so the
//! reconstructed pre-migration rows never land and the CHECK constraint is
//! never absent for anyone else.
//!
//! The statement is mirrored from
//! `crates/server/migrations/152_payment_policies_retire_app_subject.sql`;
//! change them together.
//!
//! Run with: cargo test -p everruns-server --test domain payment_policy_app_subject_test:: -- --test-threads=1

use crate::test_harness;

use sqlx::{PgPool, Postgres, Transaction};
use test_harness::get_database_url;
use uuid::Uuid;

/// Mirrors the conversion in 152.
const CONVERT_APP_POLICIES: &str = r#"
    INSERT INTO payment_policies (
        org_id, payment_account_id, subject_type, subject_id,
        allowed_capabilities, allowed_hosts, rail_preference,
        max_amount_usd_per_request, max_amount_usd_per_turn, max_amount_usd_per_day,
        require_approval_above_usd, status, metadata
    )
    SELECT
        p.org_id, p.payment_account_id, 'agent', a.public_id,
        p.allowed_capabilities, p.allowed_hosts, p.rail_preference,
        p.max_amount_usd_per_request, p.max_amount_usd_per_turn, p.max_amount_usd_per_day,
        p.require_approval_above_usd, p.status,
        COALESCE(p.metadata, '{}'::jsonb) || jsonb_build_object(
            'converted_from', 'app',
            'converted_from_subject_id', p.subject_id
        )
    FROM payment_policies AS p
    JOIN apps AS app ON app.public_id = p.subject_id AND app.org_id = p.org_id
    JOIN agents AS a ON a.id = app.agent_id AND a.org_id = p.org_id
    WHERE p.subject_type = 'app'
      AND p.org_id = $1
      AND NOT EXISTS (
          SELECT 1 FROM payment_policies AS existing
          WHERE existing.org_id = p.org_id
            AND existing.subject_type = 'agent'
            AND existing.subject_id = a.public_id
            AND existing.payment_account_id = p.payment_account_id
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

struct Fixture {
    org_id: i64,
    agent_public_id: String,
    account_id: Uuid,
    app_public_id: String,
}

async fn seed(tx: &mut Transaction<'_, Postgres>, label: &str, with_agent: bool) -> Fixture {
    let org_id: i64 = sqlx::query_scalar(
        "INSERT INTO organizations (public_id, name) VALUES ($1, $2) RETURNING org_id",
    )
    .bind(format!("org_{}", hex32()))
    .bind(format!("{label}-{}", hex32()))
    .fetch_one(&mut **tx)
    .await
    .expect("seed organization");

    let owner_principal_id = Uuid::now_v7();
    sqlx::query("INSERT INTO principals (id, public_id, org_id, kind) VALUES ($1, $2, $3, 'user')")
        .bind(owner_principal_id)
        .bind(format!("principal_{}", hex32()))
        .bind(org_id)
        .execute(&mut **tx)
        .await
        .expect("seed principal");

    let harness_id = Uuid::now_v7();
    sqlx::query("INSERT INTO harnesses (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(harness_id)
        .bind(org_id)
        .bind(format!("harness-{}", hex32()))
        .execute(&mut **tx)
        .await
        .expect("seed harness");

    let agent_id = Uuid::now_v7();
    let agent_public_id = format!("agent_{}", hex32());
    sqlx::query(
        "INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id)
         VALUES ($1, $2, $3, $4, '', $5)",
    )
    .bind(agent_id)
    .bind(org_id)
    .bind(&agent_public_id)
    .bind(format!("agent-{}", hex32()))
    .bind(harness_id)
    .execute(&mut **tx)
    .await
    .expect("seed agent");

    let app_public_id = format!("app_{}", hex32());
    sqlx::query(
        "INSERT INTO apps (id, org_id, public_id, name, harness_id, agent_id, status,
                           owner_principal_id, channel_type, channel_config)
         VALUES ($1, $2, $3, $4, $5, $6, 'published', $7, 'slack', '{}'::jsonb)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(&app_public_id)
    .bind(format!("app-{}", hex32()))
    .bind(harness_id)
    .bind(with_agent.then_some(agent_id))
    .bind(owner_principal_id)
    .execute(&mut **tx)
    .await
    .expect("seed app");

    let account_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO payment_accounts (id, org_id, owner_type, owner_id, rail, label, status)
         VALUES ($1, $2, 'organization', 'org', 'x402_base', 'test account', 'active')",
    )
    .bind(account_id)
    .bind(org_id)
    .execute(&mut **tx)
    .await
    .expect("seed payment account");

    Fixture {
        org_id,
        agent_public_id,
        account_id,
        app_public_id,
    }
}

/// Reconstruct a pre-152 `app` policy. The CHECK rejects the subject type now,
/// which is the migration working, so it comes off for the insert and goes
/// straight back on `NOT VALID` — re-adding it plain would revalidate the row
/// just seeded.
async fn seed_app_policy(tx: &mut Transaction<'_, Postgres>, fx: &Fixture, per_request: f64) {
    sqlx::query("ALTER TABLE payment_policies DROP CONSTRAINT payment_policies_subject_type_check")
        .execute(&mut **tx)
        .await
        .expect("drop check");
    let inserted = sqlx::query(
        "INSERT INTO payment_policies (org_id, payment_account_id, subject_type, subject_id,
                                       allowed_capabilities, allowed_hosts,
                                       max_amount_usd_per_request, status)
         VALUES ($1, $2, 'app', $3, ARRAY['weather.lookup'], ARRAY['api.example.com'], $4,
                 'active')",
    )
    .bind(fx.org_id)
    .bind(fx.account_id)
    .bind(&fx.app_public_id)
    .bind(per_request)
    .execute(&mut **tx)
    .await;
    sqlx::query(
        "ALTER TABLE payment_policies ADD CONSTRAINT payment_policies_subject_type_check
         CHECK (subject_type IN ('user', 'virtual_user', 'agent', 'agent_channel', 'session',
                                 'org'))
         NOT VALID",
    )
    .execute(&mut **tx)
    .await
    .expect("restore check");
    inserted.expect("seed app policy");
}

async fn agent_policies(
    tx: &mut Transaction<'_, Postgres>,
    fx: &Fixture,
) -> Vec<(f64, Vec<String>, Option<String>)> {
    sqlx::query_as(
        "SELECT max_amount_usd_per_request, allowed_capabilities, metadata->>'converted_from'
         FROM payment_policies
         WHERE org_id = $1 AND subject_type = 'agent' AND subject_id = $2",
    )
    .bind(fx.org_id)
    .bind(&fx.agent_public_id)
    .fetch_all(&mut **tx)
    .await
    .expect("read agent policies")
}

/// The intent survives onto the agent, limits and allow-lists intact, and is
/// traceable back to the App it came from.
#[tokio::test]
async fn app_policy_converts_onto_the_agent() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let fx = seed(&mut tx, "payment-retire-convert", true).await;
    seed_app_policy(&mut tx, &fx, 2.5).await;

    sqlx::query(CONVERT_APP_POLICIES)
        .bind(fx.org_id)
        .execute(&mut *tx)
        .await
        .expect("convert app policies");

    let policies = agent_policies(&mut tx, &fx).await;
    assert_eq!(policies.len(), 1, "exactly one agent policy");
    let (per_request, capabilities, converted_from) = &policies[0];
    assert_eq!(*per_request, 2.5, "the per-request limit is preserved");
    assert_eq!(capabilities, &vec!["weather.lookup".to_string()]);
    assert_eq!(converted_from.as_deref(), Some("app"));
}

/// A payment policy is permissive — the first match that allows the request
/// authorizes it — so duplicating one onto an agent that already has an
/// equivalent policy cannot tighten anything and would leave the operator two
/// rows to keep in sync. This is the opposite of the budget conversion, where
/// inserting alongside is what keeps the ceiling from loosening.
#[tokio::test]
async fn conversion_does_not_duplicate_an_equivalent_agent_policy() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let fx = seed(&mut tx, "payment-retire-dedupe", true).await;

    sqlx::query(
        "INSERT INTO payment_policies (org_id, payment_account_id, subject_type, subject_id,
                                       max_amount_usd_per_request, status)
         VALUES ($1, $2, 'agent', $3, 9.0, 'active')",
    )
    .bind(fx.org_id)
    .bind(fx.account_id)
    .bind(&fx.agent_public_id)
    .execute(&mut *tx)
    .await
    .expect("seed existing agent policy");

    seed_app_policy(&mut tx, &fx, 2.5).await;
    sqlx::query(CONVERT_APP_POLICIES)
        .bind(fx.org_id)
        .execute(&mut *tx)
        .await
        .expect("convert app policies");

    let policies = agent_policies(&mut tx, &fx).await;
    assert_eq!(policies.len(), 1, "no duplicate: {policies:?}");
    assert_eq!(policies[0].0, 9.0, "the operator's own policy is untouched");
}

/// A draft App has no agent — `apps.agent_id` is nullable — so there is nothing
/// to convert onto. It authorized nothing before and authorizes nothing after.
#[tokio::test]
async fn an_agentless_app_policy_has_no_conversion_target() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let fx = seed(&mut tx, "payment-retire-draft", false).await;
    seed_app_policy(&mut tx, &fx, 2.5).await;

    sqlx::query(CONVERT_APP_POLICIES)
        .bind(fx.org_id)
        .execute(&mut *tx)
        .await
        .expect("convert app policies");

    assert!(
        agent_policies(&mut tx, &fx).await.is_empty(),
        "an agent-less App must not manufacture an agent policy"
    );
}

/// The subject types are a closed set: `app` is out, `agent_channel` is in.
#[tokio::test]
async fn subject_type_check_rejects_app_and_accepts_agent_channel() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let fx = seed(&mut tx, "payment-retire-check", true).await;

    for subject_type in [
        "user",
        "virtual_user",
        "agent",
        "agent_channel",
        "session",
        "org",
    ] {
        sqlx::query(
            "INSERT INTO payment_policies (org_id, payment_account_id, subject_type, subject_id,
                                           status)
             VALUES ($1, $2, $3, $4, 'active')",
        )
        .bind(fx.org_id)
        .bind(fx.account_id)
        .bind(subject_type)
        .bind(format!("subject_{}", hex32()))
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|err| panic!("subject_type {subject_type} must be accepted: {err}"));
    }

    let rejected = sqlx::query(
        "INSERT INTO payment_policies (org_id, payment_account_id, subject_type, subject_id, status)
         VALUES ($1, $2, 'app', 'app_whatever', 'active')",
    )
    .bind(fx.org_id)
    .bind(fx.account_id)
    .execute(&mut *tx)
    .await;
    assert!(rejected.is_err(), "`app` must be rejected");
}
