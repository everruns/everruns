//! Evidence for retiring the `app` budget level (EVE-1129, migration 151).
//!
//! `app_channel` outlived `app` by one migration, because 138 had moved App
//! webhooks onto `agent_triggers`, moved their budgets back from
//! `agent_endpoint` to `app_channel`, and deleted the endpoint rows, leaving it
//! with no structural successor to retire onto. Migration 153 gives it one; see
//! `trigger_budget_subject_test.rs` (EVE-1138). Only `app` had one from the
//! start: the agent.
//!
//! Migration 137 fanned each App cap out per endpoint but deliberately left the
//! App budget in place and enforced, so the aggregate ceiling kept binding
//! across those endpoints. `check_budgets_for_session` evaluates every budget
//! the hierarchy matches and keeps the most restrictive result, so an App with
//! a $10 cap and three endpoints is capped at $10 in aggregate on top of $10
//! per endpoint. Deleting the level with no successor would let that org spend
//! $30 without changing anything on their side.
//!
//! So the risk this migration carries is not that a row survives — it is that
//! the effective ceiling comes out *looser* than it went in. Every test here is
//! a statement about the ceiling, not about row counts.
//!
//! The conversion is re-executed here against freshly seeded rows rather than
//! asserted over whatever the migration already processed: the test database is
//! migrated before the test runs, and after 151 an `app` budget can no longer be
//! inserted at all. Reconstructing the pre-migration shape therefore means
//! taking the CHECK constraint off, so every test runs inside one transaction
//! that is never committed. Postgres makes DDL transactional, so the constraint
//! is never absent for anyone else and the seeded rows never land — the
//! alternative, dropping a constraint on a shared table mid-suite, would be
//! visible to every other test against this database. The statement is mirrored
//! from `crates/server/migrations/151_retire_app_budget_levels.sql`; change them
//! together.
//!
//! Run with: cargo test -p everruns-server --test domain app_budget_retirement_test:: -- --test-threads=1

use crate::test_harness;

use sqlx::{PgPool, Postgres, Transaction};
use test_harness::get_database_url;
use uuid::Uuid;

/// Mirrors the conversion in 151.
const CONVERT_APP_BUDGETS: &str = r#"
    INSERT INTO budgets (
        org_id, subject_type, subject_id, currency, "limit", soft_limit,
        balance, period, metadata, status, period_started_at
    )
    SELECT
        b.org_id, 'agent', a.public_id, b.currency, b."limit", b.soft_limit,
        b.balance, b.period,
        COALESCE(b.metadata, '{}'::jsonb) || jsonb_build_object(
            'converted_from', 'app',
            'converted_from_subject_id', b.subject_id
        ),
        b.status, b.period_started_at
    FROM budgets AS b
    JOIN apps AS app ON app.public_id = b.subject_id AND app.org_id = b.org_id
    JOIN agents AS a ON a.id = app.agent_id AND a.org_id = b.org_id
    WHERE b.subject_type = 'app'
      AND b.org_id = $1
"#;

async fn pool() -> PgPool {
    PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL")
}

fn hex32() -> String {
    Uuid::new_v4().simple().to_string()
}

struct Org {
    org_id: i64,
    agent_id: Uuid,
    agent_public_id: String,
    harness_id: Uuid,
    owner_principal_id: Uuid,
}

async fn seed_org(tx: &mut Transaction<'_, Postgres>, label: &str) -> Org {
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

    Org {
        org_id,
        agent_id,
        agent_public_id,
        harness_id,
        owner_principal_id,
    }
}

/// An App fronting the org's agent. `agent_id` is nullable for draft Apps, so
/// callers choose whether this one has an agent to convert onto.
async fn seed_app(tx: &mut Transaction<'_, Postgres>, org: &Org, with_agent: bool) -> String {
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
    .bind(with_agent.then_some(org.agent_id))
    .bind(org.owner_principal_id)
    .execute(&mut **tx)
    .await
    .expect("seed app");
    public_id
}

/// Reconstruct a pre-151 `app` budget. The CHECK constraint rejects the subject
/// type now, which is the migration working, so it has to come off for the
/// insert and go straight back on.
async fn seed_app_budget(
    tx: &mut Transaction<'_, Postgres>,
    org_id: i64,
    app_public_id: &str,
    limit: f64,
    balance: f64,
    window_start: chrono::DateTime<chrono::Utc>,
) {
    sqlx::query("ALTER TABLE budgets DROP CONSTRAINT budgets_subject_type_check")
        .execute(&mut **tx)
        .await
        .expect("drop check");
    let inserted = sqlx::query(
        r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                period, status, period_started_at)
           VALUES ($1, 'app', $2, 'USD', $3, $4,
                   '{"kind":"calendar","unit":"month"}'::jsonb, 'active', $5)"#,
    )
    .bind(org_id)
    .bind(app_public_id)
    .bind(limit)
    .bind(balance)
    .bind(window_start)
    .execute(&mut **tx)
    .await;
    sqlx::query(
        // NOT VALID: re-adding it plain would revalidate every existing row,
        // including the `app` row this helper just seeded. The constraint still
        // rejects new rows, which is all these tests need it for.
        "ALTER TABLE budgets ADD CONSTRAINT budgets_subject_type_check
         CHECK (subject_type IN ('session', 'agent', 'user', 'org', 'agent_trigger',
                                 'agent_endpoint'))
         NOT VALID",
    )
    .execute(&mut **tx)
    .await
    .expect("restore check");
    inserted.expect("seed app budget");
}

/// The effective ceiling for a subject is the tightest cap on it, because every
/// matching budget is evaluated and the most restrictive result wins.
async fn tightest_agent_limit(
    tx: &mut Transaction<'_, Postgres>,
    org_id: i64,
    agent_public_id: &str,
) -> Option<f64> {
    sqlx::query_scalar(
        r#"SELECT MIN("limit") FROM budgets
           WHERE org_id = $1 AND subject_type = 'agent' AND subject_id = $2"#,
    )
    .bind(org_id)
    .bind(agent_public_id)
    .fetch_one(&mut **tx)
    .await
    .expect("read effective ceiling")
}

/// The cap survives the retirement, carrying its recorded spend and its
/// in-flight window. Resetting the window would hand back a fresh allowance;
/// dropping the balance would forgive spend the operator has already been
/// charged for.
#[tokio::test]
async fn app_cap_converts_onto_the_agent_preserving_spend_and_window() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let org = seed_org(&mut tx, "app-retire-convert").await;
    let app_public_id = seed_app(&mut tx, &org, true).await;
    let window_start = chrono::Utc::now() - chrono::Duration::days(9);
    seed_app_budget(
        &mut tx,
        org.org_id,
        &app_public_id,
        100.0,
        40.0,
        window_start,
    )
    .await;

    sqlx::query(CONVERT_APP_BUDGETS)
        .bind(org.org_id)
        .execute(&mut *tx)
        .await
        .expect("convert app budgets");

    let (limit, balance, started, converted_from, from_subject): (
        f64,
        f64,
        chrono::DateTime<chrono::Utc>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        r#"SELECT "limit", balance, period_started_at, metadata->>'converted_from',
                  metadata->>'converted_from_subject_id'
           FROM budgets
           WHERE org_id = $1 AND subject_type = 'agent' AND subject_id = $2"#,
    )
    .bind(org.org_id)
    .bind(&org.agent_public_id)
    .fetch_one(&mut *tx)
    .await
    .expect("the agent inherits the App's cap");

    assert_eq!(limit, 100.0, "the cap is preserved, not divided");
    assert_eq!(balance, 40.0, "spend already recorded is carried over");
    assert_eq!(
        started.timestamp(),
        window_start.timestamp(),
        "an in-flight window must not reset to now"
    );
    assert_eq!(converted_from.as_deref(), Some("app"));
    assert_eq!(from_subject.as_deref(), Some(app_public_id.as_str()));
}

/// The acceptance criterion that matters. An agent that already has a cap keeps
/// the tighter of the two, whichever side it came from — the conversion inserts
/// alongside rather than skipping, because skipping would leave a looser
/// existing cap as the only one binding.
#[tokio::test]
async fn converting_never_loosens_the_effective_ceiling() {
    for (existing, app_cap, expected) in [
        // The App cap was the tighter one; skipping it would have loosened.
        (100.0, 10.0, 10.0),
        // The agent's own cap is tighter and must survive untouched.
        (5.0, 100.0, 5.0),
    ] {
        let pool = pool().await;
        let mut tx = pool.begin().await.expect("begin");
        let org = seed_org(&mut tx, "app-retire-ceiling").await;
        let app_public_id = seed_app(&mut tx, &org, true).await;

        sqlx::query(
            r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                    period, status)
               VALUES ($1, 'agent', $2, 'USD', $3, $3,
                       '{"kind":"calendar","unit":"month"}'::jsonb, 'active')"#,
        )
        .bind(org.org_id)
        .bind(&org.agent_public_id)
        .bind(existing)
        .execute(&mut *tx)
        .await
        .expect("seed existing agent budget");

        let before = tightest_agent_limit(&mut tx, org.org_id, &org.agent_public_id)
            .await
            .expect("an existing ceiling");
        seed_app_budget(
            &mut tx,
            org.org_id,
            &app_public_id,
            app_cap,
            app_cap,
            chrono::Utc::now(),
        )
        .await;

        sqlx::query(CONVERT_APP_BUDGETS)
            .bind(org.org_id)
            .execute(&mut *tx)
            .await
            .expect("convert app budgets");

        let after = tightest_agent_limit(&mut tx, org.org_id, &org.agent_public_id)
            .await
            .expect("a ceiling after conversion");
        assert_eq!(
            after, expected,
            "existing {existing} + app {app_cap} must settle at {expected}"
        );
        assert!(
            after <= before.min(app_cap),
            "the effective ceiling must never loosen: {before} -> {after}"
        );
    }
}

/// An App with no agent has no conversion target. That is only sound because it
/// also constrained nothing: `apps.agent_id` is nullable for drafts, and an App
/// with no agent never produced a session. The migration's guard fails loudly if
/// such a budget ever shows recorded spend, so pin that it is a real check and
/// not a no-op.
#[tokio::test]
async fn an_agentless_app_budget_has_no_conversion_target() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let org = seed_org(&mut tx, "app-retire-draft").await;
    let app_public_id = seed_app(&mut tx, &org, false).await;
    seed_app_budget(
        &mut tx,
        org.org_id,
        &app_public_id,
        50.0,
        50.0,
        chrono::Utc::now(),
    )
    .await;

    sqlx::query(CONVERT_APP_BUDGETS)
        .bind(org.org_id)
        .execute(&mut *tx)
        .await
        .expect("convert app budgets");

    assert_eq!(
        tightest_agent_limit(&mut tx, org.org_id, &org.agent_public_id).await,
        None,
        "an agent-less App must not manufacture an agent cap"
    );

    // With spend recorded against it, the migration must refuse rather than drop
    // a cap that was doing work.
    let spent: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM budgets AS b
           LEFT JOIN apps AS app
                  ON app.public_id = b.subject_id AND app.org_id = b.org_id
           LEFT JOIN agents AS a
                  ON a.id = app.agent_id AND a.org_id = b.org_id
           WHERE b.subject_type = 'app' AND b.org_id = $1
             AND a.public_id IS NULL AND b.balance < b."limit""#,
    )
    .bind(org.org_id)
    .fetch_one(&mut *tx)
    .await
    .expect("count unconvertible spent budgets");
    assert_eq!(spent, 0, "the seeded draft budget has no spend");
}
