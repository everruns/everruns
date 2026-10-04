//! Evidence for giving trigger ingress a structural budget subject
//! (EVE-1138, migration 153).
//!
//! `app_channel` was the last budget subject resolved from a session tag.
//! Migration 137 had converted every one of them to `agent_channel` and
//! asserted none remained; migration 138 then moved App webhooks onto
//! `agent_triggers`, moved exactly those budgets back to `app_channel`, and
//! deleted the endpoint rows they had been keyed on. For a webhook trigger the
//! tag was therefore the only identifier left, and retiring the level without a
//! successor would have deleted a live, enforced cap.
//!
//! Migration 153 adds `sessions.trigger_id` and re-keys those budgets onto an
//! `agent_trigger` subject keyed on the trigger's API id (`trg_<hex>`), not on
//! `agent_triggers.ingress_id`. `ingress_id` would have made the conversion a
//! pure rename, but it is a nullable compatibility column carried only by the
//! webhooks 138 moved: a trigger created today has none, so keying on it would
//! have produced a level no new trigger could ever use. EVE-1136 is the
//! cautionary tale — agent budgets were keyed on an identifier the API does not
//! expose and silently never bound.
//!
//! The risk here is the same one EVE-1129 was written about: not that a row is
//! missed, but that the effective ceiling comes out *looser* than it went in.
//! Every test is a statement about the ceiling or about an actual refusal, not
//! about row counts.
//!
//! The conversion is re-executed here against freshly seeded pre-153 rows
//! rather than asserted over whatever the migration already processed, because
//! the test database is migrated before the test runs and afterwards an
//! `app_channel` budget can no longer be inserted at all. Reconstructing the
//! pre-migration shape means taking the CHECK constraint off, so every test
//! runs inside one transaction that is never committed: Postgres makes DDL
//! transactional, so the constraint is never absent for anyone else and the
//! seeded rows never land. The statements are mirrored from
//! `crates/server/migrations/153_agent_trigger_budget_subject.sql`; change them
//! together.
//!
//! Run with: cargo test -p everruns-server --test domain trigger_budget_subject_test:: -- --test-threads=1

use crate::test_harness;

use sqlx::{PgPool, Postgres, Transaction};
use test_harness::get_database_url;
use uuid::Uuid;

/// Mirrors the re-key in 153.
const REKEY_APP_CHANNEL_BUDGETS: &str = r#"
    UPDATE budgets AS b
    SET subject_type = 'agent_trigger',
        subject_id = 'trg_' || REPLACE(t.id::text, '-', ''),
        metadata = COALESCE(b.metadata, '{}'::jsonb) || jsonb_build_object(
            'converted_from', 'app_channel',
            'converted_from_subject_id', b.subject_id
        ),
        updated_at = NOW()
    FROM agent_triggers AS t
    WHERE b.subject_type = 'app_channel'
      AND t.org_id = b.org_id
      AND t.ingress_id = b.subject_id
      AND b.org_id = $1
"#;

/// Mirrors the session backfill in 153 for the webhook spelling.
const BACKFILL_SESSION_TRIGGER_ID: &str = r#"
    UPDATE sessions AS s
    SET trigger_id = t.id
    FROM agent_triggers AS t
    WHERE s.trigger_id IS NULL
      AND s.org_id = t.org_id
      AND t.ingress_id IS NOT NULL
      AND ('app_channel:' || t.ingress_id) = ANY (s.tags)
      AND s.org_id = $1
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
    harness_id: Uuid,
    workspace_id: Uuid,
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

    let workspace_id = Uuid::now_v7();
    sqlx::query("INSERT INTO workspaces (id, org_id, public_id, name) VALUES ($1, $2, $3, $4)")
        .bind(workspace_id)
        .bind(org_id)
        .bind(format!("wsp_{}", hex32()))
        .bind(format!("workspace-{}", hex32()))
        .execute(&mut **tx)
        .await
        .expect("seed workspace");

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
    .execute(&mut **tx)
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

struct Trigger {
    id: Uuid,
    /// The `appchan_` id migration 138 carried over from the endpoint it
    /// deleted. `None` for a trigger created after 138, which is the case the
    /// subject key has to work for.
    ingress_id: Option<String>,
}

impl Trigger {
    /// The identifier the API exposes, and therefore the only one an operator
    /// can put in a budget.
    fn subject_id(&self) -> String {
        format!("trg_{}", self.id.simple())
    }
}

async fn seed_trigger(
    tx: &mut Transaction<'_, Postgres>,
    org: &Org,
    with_ingress_id: bool,
) -> Trigger {
    let id = Uuid::now_v7();
    let ingress_id = with_ingress_id.then(|| format!("appchan_{}", hex32()));
    sqlx::query(
        "INSERT INTO agent_triggers (id, org_id, agent_id, trigger_type, ingress_id,
                                     execution_harness_id, execution_owner_principal_id)
         VALUES ($1, $2, $3, 'webhook', $4, $5, $6)",
    )
    .bind(id)
    .bind(org.org_id)
    .bind(org.agent_id)
    .bind(ingress_id.as_deref())
    .bind(org.harness_id)
    .bind(org.owner_principal_id)
    .execute(&mut **tx)
    .await
    .expect("seed trigger");
    Trigger { id, ingress_id }
}

/// Reconstruct a pre-153 `app_channel` budget. The CHECK constraint rejects the
/// subject type now, which is the migration working, so it comes off for the
/// insert and goes straight back on.
async fn seed_app_channel_budget(
    tx: &mut Transaction<'_, Postgres>,
    org_id: i64,
    ingress_id: &str,
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
           VALUES ($1, 'app_channel', $2, 'USD', $3, $4,
                   '{"kind":"calendar","unit":"month"}'::jsonb, 'active', $5)"#,
    )
    .bind(org_id)
    .bind(ingress_id)
    .bind(limit)
    .bind(balance)
    .bind(window_start)
    .execute(&mut **tx)
    .await;
    sqlx::query(
        // NOT VALID: re-adding it plain would revalidate every existing row,
        // including the `app_channel` row this helper just seeded. The
        // constraint still rejects new rows, which is all these tests need.
        "ALTER TABLE budgets ADD CONSTRAINT budgets_subject_type_check
         CHECK (subject_type IN ('session', 'agent', 'user', 'org', 'agent_trigger',
                                 'agent_channel'))
         NOT VALID",
    )
    .execute(&mut **tx)
    .await
    .expect("restore check");
    inserted.expect("seed app_channel budget");
}

/// The effective ceiling for a trigger is the tightest cap on it: every
/// matching budget is evaluated and the most restrictive result wins.
async fn tightest_trigger_limit(
    tx: &mut Transaction<'_, Postgres>,
    org_id: i64,
    subject_id: &str,
) -> Option<f64> {
    sqlx::query_scalar(
        r#"SELECT MIN("limit") FROM budgets
           WHERE org_id = $1 AND subject_type = 'agent_trigger' AND subject_id = $2"#,
    )
    .bind(org_id)
    .bind(subject_id)
    .fetch_one(&mut **tx)
    .await
    .expect("read effective ceiling")
}

/// The cap survives the re-key carrying its recorded spend and its in-flight
/// window. Resetting the window would hand back a fresh allowance; dropping the
/// balance would forgive spend the operator has already been charged for.
#[tokio::test]
async fn a_webhook_cap_rekeys_onto_the_trigger_preserving_spend_and_window() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let org = seed_org(&mut tx, "trg-budget-rekey").await;
    let trigger = seed_trigger(&mut tx, &org, true).await;
    let ingress_id = trigger
        .ingress_id
        .clone()
        .expect("seeded with an ingress id");
    let window_start = chrono::Utc::now() - chrono::Duration::days(9);
    seed_app_channel_budget(&mut tx, org.org_id, &ingress_id, 100.0, 40.0, window_start).await;

    sqlx::query(REKEY_APP_CHANNEL_BUDGETS)
        .bind(org.org_id)
        .execute(&mut *tx)
        .await
        .expect("re-key app_channel budgets");

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
           WHERE org_id = $1 AND subject_type = 'agent_trigger' AND subject_id = $2"#,
    )
    .bind(org.org_id)
    .bind(trigger.subject_id())
    .fetch_one(&mut *tx)
    .await
    .expect("the trigger inherits the channel's cap");

    assert_eq!(limit, 100.0, "the cap is preserved, not divided");
    assert_eq!(balance, 40.0, "spend already recorded is carried over");
    assert_eq!(
        started.timestamp(),
        window_start.timestamp(),
        "an in-flight window must not reset to now"
    );
    assert_eq!(converted_from.as_deref(), Some("app_channel"));
    assert_eq!(from_subject.as_deref(), Some(ingress_id.as_str()));

    let leftover: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM budgets WHERE org_id = $1 AND subject_type = 'app_channel'",
    )
    .bind(org.org_id)
    .fetch_one(&mut *tx)
    .await
    .expect("count leftovers");
    assert_eq!(leftover, 0, "nothing may be left on the retired level");
}

/// The acceptance criterion that matters. A trigger that already has an
/// `agent_trigger` cap keeps the tighter of the two, whichever side it came
/// from: the re-key must never raise a ceiling.
#[tokio::test]
async fn rekeying_never_loosens_the_effective_ceiling() {
    for (existing, channel_cap, expected) in [
        // The channel cap was the tighter one; losing it would have loosened.
        (100.0, 10.0, 10.0),
        // The trigger's own cap is tighter and must survive untouched.
        (5.0, 100.0, 5.0),
    ] {
        let pool = pool().await;
        let mut tx = pool.begin().await.expect("begin");
        let org = seed_org(&mut tx, "trg-budget-ceiling").await;
        let trigger = seed_trigger(&mut tx, &org, true).await;
        let ingress_id = trigger
            .ingress_id
            .clone()
            .expect("seeded with an ingress id");

        sqlx::query(
            r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                    period, status)
               VALUES ($1, 'agent_trigger', $2, 'USD', $3, $3,
                       '{"kind":"calendar","unit":"month"}'::jsonb, 'active')"#,
        )
        .bind(org.org_id)
        .bind(trigger.subject_id())
        .bind(existing)
        .execute(&mut *tx)
        .await
        .expect("seed existing trigger budget");

        let before = tightest_trigger_limit(&mut tx, org.org_id, &trigger.subject_id())
            .await
            .expect("an existing ceiling");
        seed_app_channel_budget(
            &mut tx,
            org.org_id,
            &ingress_id,
            channel_cap,
            channel_cap,
            chrono::Utc::now(),
        )
        .await;

        sqlx::query(REKEY_APP_CHANNEL_BUDGETS)
            .bind(org.org_id)
            .execute(&mut *tx)
            .await
            .expect("re-key app_channel budgets");

        let after = tightest_trigger_limit(&mut tx, org.org_id, &trigger.subject_id())
            .await
            .expect("a ceiling after the re-key");
        assert_eq!(
            after, expected,
            "existing {existing} + channel {channel_cap} must settle at {expected}"
        );
        assert!(
            after <= before.min(channel_cap),
            "the effective ceiling must never loosen: {before} -> {after}"
        );
    }
}

/// An `app_channel` budget whose ingress id matches no trigger cannot be
/// re-keyed. The migration's guard exists so that such a row fails the
/// migration loudly instead of being dropped — dropping an enforced ceiling is
/// exactly the harm the conversion exists to avoid. Pin that the guard is a
/// real check by showing the row the re-key leaves behind.
#[tokio::test]
async fn an_unmatched_channel_budget_survives_the_rekey_and_trips_the_guard() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let org = seed_org(&mut tx, "trg-budget-orphan").await;
    seed_app_channel_budget(
        &mut tx,
        org.org_id,
        &format!("appchan_{}", hex32()),
        50.0,
        50.0,
        chrono::Utc::now(),
    )
    .await;

    sqlx::query(REKEY_APP_CHANNEL_BUDGETS)
        .bind(org.org_id)
        .execute(&mut *tx)
        .await
        .expect("re-key app_channel budgets");

    let unconverted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM budgets WHERE org_id = $1 AND subject_type = 'app_channel'",
    )
    .bind(org.org_id)
    .fetch_one(&mut *tx)
    .await
    .expect("count unconverted");
    assert_eq!(
        unconverted, 1,
        "an unmatched cap must remain for the guard to find, not vanish"
    );
}

/// A trigger created after migration 138 has no `ingress_id`, which is why the
/// subject is keyed on the trigger's own API id: keying it on `ingress_id`
/// would have produced a level no new trigger could ever be given a cap on.
#[tokio::test]
async fn a_trigger_without_an_ingress_id_can_still_carry_a_cap() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let org = seed_org(&mut tx, "trg-budget-new").await;
    let trigger = seed_trigger(&mut tx, &org, false).await;
    assert!(trigger.ingress_id.is_none());

    sqlx::query(
        r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                period, status)
           VALUES ($1, 'agent_trigger', $2, 'USD', 25.0, 25.0,
                   '{"kind":"calendar","unit":"month"}'::jsonb, 'active')"#,
    )
    .bind(org.org_id)
    .bind(trigger.subject_id())
    .execute(&mut *tx)
    .await
    .expect("a post-138 trigger accepts a budget");

    assert_eq!(
        tightest_trigger_limit(&mut tx, org.org_id, &trigger.subject_id()).await,
        Some(25.0)
    );
}

/// The session side of the migration. A webhook-trigger session carrying the
/// `app_channel:` tag and no endpoint row — the shape migration 138 left — is
/// backfilled onto `sessions.trigger_id`, which is what the resolver now reads.
#[tokio::test]
async fn a_webhook_session_is_backfilled_onto_the_trigger_column() {
    let pool = pool().await;
    let mut tx = pool.begin().await.expect("begin");
    let org = seed_org(&mut tx, "trg-budget-session").await;
    let trigger = seed_trigger(&mut tx, &org, true).await;
    let ingress_id = trigger
        .ingress_id
        .clone()
        .expect("seeded with an ingress id");

    let session_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sessions (id, org_id, workspace_id, harness_id, agent_id,
                               owner_principal_id, tags, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'webhook')",
    )
    .bind(session_id)
    .bind(org.org_id)
    .bind(org.workspace_id)
    .bind(org.harness_id)
    .bind(org.agent_id)
    .bind(org.owner_principal_id)
    .bind(vec![format!("app_channel:{ingress_id}")])
    .execute(&mut *tx)
    .await
    .expect("seed webhook session");

    sqlx::query(BACKFILL_SESSION_TRIGGER_ID)
        .bind(org.org_id)
        .execute(&mut *tx)
        .await
        .expect("backfill sessions.trigger_id");

    let backfilled: Option<Uuid> =
        sqlx::query_scalar("SELECT trigger_id FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await
            .expect("read back the column");
    assert_eq!(
        backfilled,
        Some(trigger.id),
        "the tag must resolve to the trigger it names"
    );
}
