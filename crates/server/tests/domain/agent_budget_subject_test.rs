//! Evidence for re-keying the `agent` budget subject onto the agent's public id
//! (EVE-1136, migration 150).
//!
//! Budget subjects are the identifiers the API exposes, because those are the
//! only identifiers a caller can put in a budget. The `agent` level did not
//! obey that: the hierarchy resolver rendered the typed `AgentId` taken off
//! `sessions.agent_id` — an FK to `agents(id)` — which spells the *internal*
//! uuid, while `POST /v1/budgets` stores the caller's `agents.public_id`. The
//! two columns are generated independently, so the subject the resolver looked
//! for never existed and agent-scoped budgets were stored, listed and displayed
//! but never evaluated.
//!
//! The risk the migration carries is not that it misses a row — an unconverted
//! row was already inert. It is the opposite: that a row which *does* bind
//! today stops binding, or that converting one onto an agent that already has a
//! budget quietly raises that agent's ceiling. These tests pin both.
//!
//! The re-key statement is re-executed here against freshly seeded
//! pre-migration-shaped rows rather than asserted over whatever the migration
//! already processed, because the test database is migrated before the test
//! runs. It is mirrored from
//! `crates/server/migrations/150_agent_budget_subject_public_id.sql`; change
//! them together.
//!
//! Run with: cargo test -p everruns-server --test domain agent_budget_subject_test:: -- --test-threads=1

use crate::test_harness;

use sqlx::PgPool;
use test_harness::get_database_url;
use uuid::Uuid;

/// Mirrors migration 150.
const REKEY_AGENT_SUBJECTS: &str = r#"
    UPDATE budgets AS b
    SET subject_id = a.public_id,
        updated_at = NOW()
    FROM agents AS a
    WHERE b.subject_type = 'agent'
      AND b.org_id = a.org_id
      AND b.subject_id = 'agent_' || replace(a.id::text, '-', '')
      AND b.subject_id IS DISTINCT FROM a.public_id
"#;

async fn pool() -> PgPool {
    PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL")
}

fn hex32() -> String {
    Uuid::new_v4().simple().to_string()
}

struct Agent {
    org_id: i64,
    internal_id: Uuid,
    public_id: String,
}

impl Agent {
    /// The spelling the resolver used to look for, and the only spelling a
    /// pre-migration row could carry.
    fn internal_subject_id(&self) -> String {
        format!("agent_{}", self.internal_id.simple())
    }
}

async fn seed_agent(pool: &PgPool, label: &str) -> Agent {
    let org_id: i64 = sqlx::query_scalar(
        "INSERT INTO organizations (public_id, name) VALUES ($1, $2) RETURNING org_id",
    )
    .bind(format!("org_{}", hex32()))
    .bind(format!("{label}-{}", hex32()))
    .fetch_one(pool)
    .await
    .expect("seed organization");

    let harness_id = Uuid::now_v7();
    sqlx::query("INSERT INTO harnesses (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(harness_id)
        .bind(org_id)
        .bind(format!("harness-{}", hex32()))
        .execute(pool)
        .await
        .expect("seed harness");

    // `id` and `public_id` are independent by construction, exactly as the
    // production insert leaves them.
    let internal_id = Uuid::now_v7();
    let public_id = format!("agent_{}", hex32());
    sqlx::query(
        "INSERT INTO agents (id, org_id, public_id, name, system_prompt, harness_id)
         VALUES ($1, $2, $3, $4, '', $5)",
    )
    .bind(internal_id)
    .bind(org_id)
    .bind(&public_id)
    .bind(format!("agent-{}", hex32()))
    .bind(harness_id)
    .execute(pool)
    .await
    .expect("seed agent");

    assert_ne!(
        public_id,
        format!("agent_{}", internal_id.simple()),
        "fixture must exercise the id/public_id gap"
    );

    Agent {
        org_id,
        internal_id,
        public_id,
    }
}

async fn seed_budget(pool: &PgPool, org_id: i64, subject_id: &str, limit: f64) -> Uuid {
    sqlx::query_scalar(
        r#"INSERT INTO budgets (org_id, subject_type, subject_id, currency, "limit", balance,
                                status)
           VALUES ($1, 'agent', $2, 'usd', $3, $3, 'active')
           RETURNING id"#,
    )
    .bind(org_id)
    .bind(subject_id)
    .bind(limit)
    .fetch_one(pool)
    .await
    .expect("seed budget")
}

async fn subject_of(pool: &PgPool, budget_id: Uuid) -> String {
    sqlx::query_scalar("SELECT subject_id FROM budgets WHERE id = $1")
        .bind(budget_id)
        .fetch_one(pool)
        .await
        .expect("read budget subject")
}

/// The point of the migration: a row written in the internal spelling is the
/// only kind that binds today, so it is the only kind that could regress.
#[tokio::test]
async fn internal_spelling_is_rekeyed_onto_the_public_id() {
    let pool = pool().await;
    let agent = seed_agent(&pool, "rekey-internal").await;
    let budget = seed_budget(&pool, agent.org_id, &agent.internal_subject_id(), 10.0).await;

    sqlx::query(REKEY_AGENT_SUBJECTS)
        .execute(&pool)
        .await
        .expect("re-key");

    assert_eq!(subject_of(&pool, budget).await, agent.public_id);
}

/// A row already keyed by the public id is what the API produces. It must come
/// through untouched — re-keying it onto itself would be a no-op, but matching
/// it at all would mean the predicate is wrong.
#[tokio::test]
async fn public_spelling_is_left_alone() {
    let pool = pool().await;
    let agent = seed_agent(&pool, "rekey-public").await;
    let budget = seed_budget(&pool, agent.org_id, &agent.public_id, 20.0).await;

    let affected = sqlx::query(REKEY_AGENT_SUBJECTS)
        .execute(&pool)
        .await
        .expect("re-key")
        .rows_affected();

    assert_eq!(affected, 0, "an already-public subject must not be matched");
    assert_eq!(subject_of(&pool, budget).await, agent.public_id);
}

/// Converting onto an agent that already has a budget must not raise that
/// agent's ceiling. Both rows survive and both bind; `check_budgets_for_session`
/// keeps the most restrictive, so the effective ceiling becomes the tighter of
/// the two. Merging them would have to pick a limit, and picking the looser one
/// is the silent loosening this whole change exists to avoid.
#[tokio::test]
async fn converting_onto_an_agent_that_already_has_a_budget_keeps_the_tighter_ceiling() {
    let pool = pool().await;
    let agent = seed_agent(&pool, "rekey-collision").await;
    let existing = seed_budget(&pool, agent.org_id, &agent.public_id, 100.0).await;
    let converted = seed_budget(&pool, agent.org_id, &agent.internal_subject_id(), 10.0).await;

    sqlx::query(REKEY_AGENT_SUBJECTS)
        .execute(&pool)
        .await
        .expect("re-key");

    assert_eq!(subject_of(&pool, existing).await, agent.public_id);
    assert_eq!(subject_of(&pool, converted).await, agent.public_id);

    let tightest: f64 = sqlx::query_scalar(
        r#"SELECT MIN("limit") FROM budgets
           WHERE org_id = $1 AND subject_type = 'agent' AND subject_id = $2"#,
    )
    .bind(agent.org_id)
    .bind(&agent.public_id)
    .fetch_one(&pool)
    .await
    .expect("read effective ceiling");
    assert_eq!(tightest, 10.0, "the effective ceiling must not loosen");
}

/// A subject that matches no agent in the same org is not ours to touch. The
/// org predicate matters: two orgs' agents could otherwise cross-key.
#[tokio::test]
async fn subjects_without_a_matching_agent_are_untouched() {
    let pool = pool().await;
    let agent = seed_agent(&pool, "rekey-unmatched").await;
    let other = seed_agent(&pool, "rekey-unmatched-other").await;

    let orphan = seed_budget(&pool, agent.org_id, &format!("agent_{}", hex32()), 30.0).await;
    // The internal spelling of an agent that lives in a *different* org.
    let cross_org = seed_budget(&pool, agent.org_id, &other.internal_subject_id(), 40.0).await;
    let orphan_subject = subject_of(&pool, orphan).await;
    let cross_org_subject = subject_of(&pool, cross_org).await;

    sqlx::query(REKEY_AGENT_SUBJECTS)
        .execute(&pool)
        .await
        .expect("re-key");

    assert_eq!(subject_of(&pool, orphan).await, orphan_subject);
    assert_eq!(
        subject_of(&pool, cross_org).await,
        cross_org_subject,
        "an agent in another org must not be matched"
    );
}
