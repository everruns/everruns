//! Migration `20261018212249_github_person_installs_to_org_links` (step 3b of
//! knowledge/integrations/github-model.md): shared App installations held on a
//! virtual user's own `github` row become org links; an agent's row becomes its
//! choice of the link, a person's row goes away, and an installation lands in
//! one org only. Seeds legacy rows in a transaction, runs the migration SQL and
//! rolls back.

use crate::test_harness;

use serde_json::Value;
use sqlx::{Connection, PgConnection, Postgres, Row, Transaction};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../migrations/20261018212249_github_person_installs_to_org_links.sql");

async fn org(tx: &mut Transaction<'_, Postgres>) -> i64 {
    sqlx::query(
        "INSERT INTO organizations (public_id, name) \
         VALUES ('org_' || replace(gen_random_uuid()::text, '-', ''), 'Migration org') \
         RETURNING org_id",
    )
    .fetch_one(&mut **tx)
    .await
    .expect("create org")
    .get(0)
}

async fn virtual_user(tx: &mut Transaction<'_, Postgres>, org_id: i64, usage: &str) -> Uuid {
    sqlx::query(
        "INSERT INTO virtual_users (org_id, name, usage) VALUES ($1, 'vu', $2) RETURNING id",
    )
    .bind(org_id)
    .bind(usage)
    .fetch_one(&mut **tx)
    .await
    .expect("create virtual user")
    .get(0)
}

/// A pre-D3 row: the shared App's installation on the virtual user's own row.
async fn legacy_install(
    tx: &mut Transaction<'_, Postgres>,
    virtual_user_id: Uuid,
    installation_id: i64,
    created_at: &str,
) {
    sqlx::query(
        "INSERT INTO virtual_user_connections \
           (virtual_user_id, provider, connection_type, provider_user_id, provider_username, \
            scopes, installation_id, provider_metadata, created_at) \
         VALUES ($1, 'github', 'oauth', '77', 'acme', 'contents: read', $2, \
                 '{\"account_type\": \"Organization\"}', $3::timestamptz)",
    )
    .bind(virtual_user_id)
    .bind(installation_id)
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .expect("seed legacy row");
}

/// The org link for an installation: its id and org.
async fn link(tx: &mut Transaction<'_, Postgres>, installation_id: i64) -> Option<(Uuid, i64)> {
    sqlx::query(
        "SELECT c.id, v.org_id FROM virtual_user_connections c \
         JOIN virtual_users v ON v.id = c.virtual_user_id \
         WHERE c.provider = 'github' AND c.owner_scope = 'organization' \
           AND c.connection_type = 'everruns_app' AND v.usage = 'organization' \
           AND c.installation_id = $1",
    )
    .bind(installation_id)
    .fetch_optional(&mut **tx)
    .await
    .expect("read link")
    .map(|row| (row.get(0), row.get(1)))
}

/// A virtual user's `github` row: its type and metadata.
async fn github_row(
    tx: &mut Transaction<'_, Postgres>,
    virtual_user_id: Uuid,
) -> Option<(String, Value)> {
    sqlx::query(
        "SELECT connection_type, provider_metadata FROM virtual_user_connections \
         WHERE virtual_user_id = $1 AND provider = 'github'",
    )
    .bind(virtual_user_id)
    .fetch_optional(&mut **tx)
    .await
    .expect("read row")
    .map(|row| (row.get(0), row.get(1)))
}

#[tokio::test]
async fn legacy_installs_become_org_links_and_agent_choices() {
    let mut connection = PgConnection::connect(&test_harness::get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let mut tx = connection.begin().await.expect("begin");
    let installation = 970_000_000_000 + i64::from(rand_suffix());

    let first = org(&mut tx).await;
    let second = org(&mut tx).await;
    let person = virtual_user(&mut tx, first, "end_user").await;
    let agent = virtual_user(&mut tx, first, "service").await;
    let other_agent = virtual_user(&mut tx, second, "service").await;
    legacy_install(&mut tx, person, installation, "2026-01-01T00:00:00Z").await;
    legacy_install(&mut tx, agent, installation, "2026-02-01T00:00:00Z").await;
    // The same installation, attached later in another org.
    legacy_install(&mut tx, other_agent, installation, "2026-03-01T00:00:00Z").await;

    sqlx::raw_sql(MIGRATION)
        .execute(&mut *tx)
        .await
        .expect("run migration");

    let (link_id, link_org) = link(&mut tx, installation).await.expect("org link");
    assert_eq!(link_org, first, "the oldest holder's org keeps it");

    let (kind, metadata) = github_row(&mut tx, agent).await.expect("agent choice");
    assert_eq!(kind, "everruns_app");
    assert_eq!(metadata["org_connection_id"], link_id.to_string());
    assert_eq!(metadata["account_type"], "Organization");

    assert!(
        github_row(&mut tx, person).await.is_none(),
        "person row removed"
    );
    assert!(
        github_row(&mut tx, other_agent).await.is_none(),
        "the other org's agent does not keep an installation it no longer links"
    );

    // Idempotent: nothing legacy is left, so a rerun changes nothing.
    sqlx::raw_sql(MIGRATION)
        .execute(&mut *tx)
        .await
        .expect("re-run migration");
    assert_eq!(link(&mut tx, installation).await, Some((link_id, first)));
    tx.rollback().await.expect("roll back");
}

#[tokio::test]
async fn an_installation_already_linked_keeps_its_link() {
    let mut connection = PgConnection::connect(&test_harness::get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let mut tx = connection.begin().await.expect("begin");
    let installation = 980_000_000_000 + i64::from(rand_suffix());

    let linked = org(&mut tx).await;
    let holder = org(&mut tx).await;
    // An admin of `linked` already linked it (3a).
    let owner = virtual_user(&mut tx, linked, "organization").await;
    let existing: Uuid = sqlx::query(
        "INSERT INTO virtual_user_connections \
           (virtual_user_id, provider, connection_type, owner_scope, name, installation_id) \
         VALUES ($1, 'github', 'everruns_app', 'organization', 'acme', $2) RETURNING id",
    )
    .bind(owner)
    .bind(installation)
    .fetch_one(&mut *tx)
    .await
    .expect("seed link")
    .get(0);
    let same_org_agent = virtual_user(&mut tx, linked, "service").await;
    let other_org_agent = virtual_user(&mut tx, holder, "service").await;
    legacy_install(
        &mut tx,
        same_org_agent,
        installation,
        "2026-01-01T00:00:00Z",
    )
    .await;
    legacy_install(
        &mut tx,
        other_org_agent,
        installation,
        "2026-01-02T00:00:00Z",
    )
    .await;

    sqlx::raw_sql(MIGRATION)
        .execute(&mut *tx)
        .await
        .expect("run migration");

    assert_eq!(link(&mut tx, installation).await, Some((existing, linked)));
    let (kind, metadata) = github_row(&mut tx, same_org_agent)
        .await
        .expect("agent choice");
    assert_eq!(kind, "everruns_app");
    assert_eq!(metadata["org_connection_id"], existing.to_string());
    assert!(github_row(&mut tx, other_org_agent).await.is_none());
    tx.rollback().await.expect("roll back");
}

/// Keeps installation ids distinct across concurrent test runs on one database.
fn rand_suffix() -> u32 {
    Uuid::new_v4().as_u128() as u32 >> 1
}
