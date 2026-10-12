// Orphaned managed sandbox scan.
//
// Decision: records can lose track of a provider sandbox (a create whose state
// save failed, a delete that never reached the provider, a crash between the
// two), and a lost sandbox bills until someone notices. Leases only cover what
// was leased, so this job asks each managed provider (Daytona, E2B, Modal)
// directly, by provider-side label, which sandboxes Everruns created, and
// cleans up the ones no live record tracks.
//
// Decision: only sandboxes labelled `everruns.managed=true` (managed Session
// Sandboxes) are candidates, re-checked client-side together with
// `everruns=true` and the organization label. Sandboxes an agent created with
// a provider tool carry `everruns=true` but not the managed label, so they are
// never touched. Managed sandboxes created before the label existed are
// invisible to the scan; that is the safe direction.
//
// Decision: an orphan is a candidate with no live `sandbox_instances` row
// (`retired_at IS NULL` on a Sandbox that is not deleted). Every hosted
// managed create persists through the PostgreSQL state store, so a missing
// row really means untracked. Two guards cover the window between a provider
// create and its state save: candidates younger than `MIN_AGE` are skipped, as
// are ones an unreleased `leased_resources` row still owns (lease cleanup is
// that row's job).
//
// Decision: two-phase action. The first sighting stops the sandbox (pause on
// E2B, terminate on Modal, which cannot pause from the control plane) and
// records it in `sandbox_orphan_sightings`; a sighting at least
// `DELETE_AFTER` later deletes it. A sandbox that becomes tracked again in
// between loses its sighting. At most `MAX_ACTIONS_PER_RUN` provider calls
// per run, so a bad listing cannot mass-delete; the rest waits an hour.
//
// Decision: credentials come from the accounts managed Sandboxes actually
// pinned (`list_sandbox_orphan_scan_accounts`), decrypted through the
// cleanup-only `get_connection_token_for_connection`. The job is therefore
// disabled without an encryption key (no connection resolver). Identical
// tokens are listed once per organization.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use everruns_contracts::session_sandbox::{
    LABEL_EVERRUNS, LABEL_MANAGED, LABEL_ORG_ID, OwnedSandbox, SandboxAdminError,
    SessionSandboxProvider,
};
use everruns_core::connection_services::UserConnectionResolver;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use crate::background::cluster_jobs::ClusterJob;
use crate::storage::StorageBackend;

/// Durable schedule name of the orphaned managed sandbox scan.
pub const SANDBOX_ORPHAN_SCAN_SCHEDULE: &str = "sandbox-orphan-scan";
/// Activity type the orphaned managed sandbox scan schedule enqueues.
pub const SANDBOX_ORPHAN_SCAN_ACTIVITY: &str = "sandbox_orphan_scan";

/// Providers asked for their managed sandboxes.
pub const SCAN_PROVIDERS: &[&str] = &["daytona", "e2b", "modal"];
/// Candidates younger than this may be a create whose state save is in flight.
pub const MIN_AGE: chrono::Duration = chrono::Duration::minutes(30);
/// A stopped orphan seen again this long after its first sighting is deleted.
pub const DELETE_AFTER: chrono::Duration = chrono::Duration::hours(24);
/// Provider stop/delete calls allowed per run.
pub const MAX_ACTIONS_PER_RUN: usize = 20;
/// Sightings not seen for this long are forgotten.
const SIGHTING_RETENTION_DAYS: i32 = 7;

/// Provider lookup, injectable for tests.
pub type ProviderLookup = dyn Fn(&str) -> Option<Box<dyn SessionSandboxProvider>> + Send + Sync;

/// The hourly scan as a cluster-once job (`cluster_jobs.rs`). Disabled
/// without a connection resolver, which only exists with an encryption key.
pub fn orphan_scan_job(
    db: Arc<StorageBackend>,
    resolver: Option<Arc<dyn UserConnectionResolver>>,
) -> ClusterJob {
    let Some(resolver) = resolver else {
        info!("Orphaned managed sandbox scan disabled (no SECRETS_ENCRYPTION_KEY)");
        return ClusterJob::disabled(SANDBOX_ORPHAN_SCAN_SCHEDULE, SANDBOX_ORPHAN_SCAN_ACTIVITY);
    };
    ClusterJob::every(
        SANDBOX_ORPHAN_SCAN_SCHEDULE,
        SANDBOX_ORPHAN_SCAN_ACTIVITY,
        "Stops, then deletes, managed provider sandboxes no Everruns record tracks.",
        Duration::from_secs(3600),
        move || {
            let db = db.clone();
            let resolver = resolver.clone();
            Box::pin(async move {
                let lookup: &ProviderLookup =
                    &everruns_capabilities::session_sandbox::create_session_sandbox_provider;
                match run_scan(&db, resolver.as_ref(), lookup, Utc::now()).await {
                    Ok(report) if report.stopped + report.deleted > 0 => info!(
                        accounts = report.accounts,
                        candidates = report.candidates,
                        stopped = report.stopped,
                        deleted = report.deleted,
                        "Orphaned managed sandbox scan acted"
                    ),
                    Ok(report) => debug!(
                        accounts = report.accounts,
                        candidates = report.candidates,
                        "Orphaned managed sandbox scan found nothing to do"
                    ),
                    Err(e) => warn!(error = %e, "Orphaned managed sandbox scan failed"),
                }
            })
        },
    )
}

/// What a run did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScanReport {
    /// Distinct (provider, organization, token) listings made.
    pub accounts: usize,
    /// Listed sandboxes that passed the label re-check.
    pub candidates: usize,
    pub stopped: usize,
    pub deleted: usize,
}

/// A previous sighting of an untracked sandbox.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Sighting {
    pub first_seen_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Why a listed sandbox is left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    /// A live `sandbox_instances` row tracks it.
    Tracked,
    /// An unreleased lease owns its cleanup.
    Leased,
    /// The provider reported no creation time.
    UnknownAge,
    /// Created less than `MIN_AGE` ago.
    TooYoung,
}

/// What the scan does with one listed sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanDecision {
    Keep(KeepReason),
    /// First phase: stop it and record the sighting.
    Stop,
    /// Stopped and recorded; not yet old enough to delete.
    Wait,
    /// Second phase: delete it.
    Delete,
}

/// The listing filter: managed sandboxes of one organization.
pub fn scan_labels(org_public_id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (LABEL_EVERRUNS.to_string(), "true".to_string()),
        (LABEL_MANAGED.to_string(), "true".to_string()),
        (LABEL_ORG_ID.to_string(), org_public_id.to_string()),
    ])
}

/// Client-side re-check: providers' label filters are not trusted to be exact.
pub fn labels_match(sandbox: &OwnedSandbox, org_public_id: &str) -> bool {
    scan_labels(org_public_id)
        .iter()
        .all(|(key, value)| sandbox.labels.get(key) == Some(value))
}

/// Decide one candidate. Pure: the caller supplies what the database knows.
pub fn decide(
    now: DateTime<Utc>,
    created_at: Option<DateTime<Utc>>,
    tracked: bool,
    leased: bool,
    sighting: Option<&Sighting>,
) -> OrphanDecision {
    if tracked {
        return OrphanDecision::Keep(KeepReason::Tracked);
    }
    if leased {
        return OrphanDecision::Keep(KeepReason::Leased);
    }
    let Some(created_at) = created_at else {
        return OrphanDecision::Keep(KeepReason::UnknownAge);
    };
    if now - created_at < MIN_AGE {
        return OrphanDecision::Keep(KeepReason::TooYoung);
    }
    match sighting {
        None => OrphanDecision::Stop,
        Some(sighting) if now - sighting.first_seen_at >= DELETE_AFTER => OrphanDecision::Delete,
        Some(sighting) if sighting.stopped_at.is_none() => OrphanDecision::Stop,
        Some(_) => OrphanDecision::Wait,
    }
}

/// Whether a live record tracks `external_id` at `provider`.
pub async fn is_tracked(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM sandbox_instances i
               JOIN sandboxes s ON s.id = i.sandbox_id
               WHERE i.external_id = $2 AND i.retired_at IS NULL
                 AND s.provider = $1
                 AND s.deleted_at IS NULL AND s.desired_state <> 'deleted'
           )"#,
    )
    .bind(provider)
    .bind(external_id)
    .fetch_one(pool)
    .await
}

/// Whether an unreleased lease still owns `external_id`'s cleanup.
pub async fn is_leased(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM leased_resources
               WHERE provider = $1 AND external_id = $2 AND status <> 'released'
           )"#,
    )
    .bind(provider)
    .bind(external_id)
    .fetch_one(pool)
    .await
}

pub async fn get_sighting(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Option<Sighting>, sqlx::Error> {
    sqlx::query_as(
        "SELECT first_seen_at, stopped_at, deleted_at FROM sandbox_orphan_sightings \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(pool)
    .await
}

/// Record (or refresh) a sighting; `stopped` stamps `stopped_at` once.
pub async fn record_sighting(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    org_id: i64,
    stopped: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO sandbox_orphan_sightings (provider, external_id, org_id, stopped_at)
           VALUES ($1, $2, $3, CASE WHEN $4 THEN now() END)
           ON CONFLICT (provider, external_id) DO UPDATE SET
               last_seen_at = now(),
               stopped_at = COALESCE(sandbox_orphan_sightings.stopped_at, EXCLUDED.stopped_at)"#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(org_id)
    .bind(stopped)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_deleted(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sandbox_orphan_sightings SET deleted_at = now(), last_seen_at = now() \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Forget a sighting: the sandbox is tracked again.
pub async fn clear_sighting(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sandbox_orphan_sightings WHERE provider = $1 AND external_id = $2")
        .bind(provider)
        .bind(external_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn prune_sightings(pool: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "DELETE FROM sandbox_orphan_sightings \
         WHERE last_seen_at < now() - make_interval(days => $1)",
    )
    .bind(SIGHTING_RETENTION_DAYS)
    .execute(pool)
    .await?
    .rows_affected())
}

/// One full scan. `lookup` maps a provider id to its implementation.
pub async fn run_scan(
    db: &StorageBackend,
    resolver: &dyn UserConnectionResolver,
    lookup: &ProviderLookup,
    now: DateTime<Utc>,
) -> anyhow::Result<ScanReport> {
    let pool = db.pool();
    let providers: Vec<String> = SCAN_PROVIDERS.iter().map(|p| p.to_string()).collect();
    let accounts = db.list_sandbox_orphan_scan_accounts(&providers).await?;
    let mut report = ScanReport::default();
    let mut budget = MAX_ACTIONS_PER_RUN;
    let mut listed: HashSet<(String, i64, String)> = HashSet::new();

    for account in accounts {
        if budget == 0 {
            break;
        }
        let token = match resolver
            .get_connection_token_for_connection(
                account.connection_id,
                account.virtual_user_id,
                &account.provider,
            )
            .await
        {
            Ok(Some(token)) if !token.trim().is_empty() => token,
            Ok(_) => continue,
            Err(e) => {
                warn!(
                    error = %e,
                    provider = %account.provider,
                    connection_id = %account.connection_id,
                    "Orphan scan could not decrypt a sandbox provider connection"
                );
                continue;
            }
        };
        if !listed.insert((account.provider.clone(), account.org_id, token.clone())) {
            continue;
        }
        let Some(provider) = lookup(&account.provider) else {
            continue;
        };
        report.accounts += 1;
        let org = everruns_core::org_public_id_from_internal(account.org_id);
        let sandboxes = match provider.list_owned(&token, &scan_labels(&org)).await {
            Ok(sandboxes) => sandboxes,
            Err(SandboxAdminError::Unsupported) => continue,
            Err(e) => {
                warn!(
                    error = %e,
                    provider = %account.provider,
                    org_id = account.org_id,
                    "Orphan scan could not list provider sandboxes"
                );
                continue;
            }
        };
        for sandbox in sandboxes {
            if budget == 0 {
                break;
            }
            if !labels_match(&sandbox, &org) {
                continue;
            }
            report.candidates += 1;
            let acted = handle_candidate(
                pool,
                provider.as_ref(),
                &token,
                account.org_id,
                &sandbox,
                now,
            )
            .await?;
            match acted {
                Some(OrphanDecision::Stop) => {
                    report.stopped += 1;
                    budget -= 1;
                }
                Some(OrphanDecision::Delete) => {
                    report.deleted += 1;
                    budget -= 1;
                }
                _ => {}
            }
        }
    }

    prune_sightings(pool).await?;
    Ok(report)
}

/// Decide and act on one candidate. Returns the decision when a provider
/// call was made (so it counts against the budget).
async fn handle_candidate(
    pool: &PgPool,
    provider: &dyn SessionSandboxProvider,
    token: &str,
    org_id: i64,
    sandbox: &OwnedSandbox,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<OrphanDecision>> {
    let provider_id = provider.id();
    let external_id = sandbox.external_id.as_str();
    let tracked = is_tracked(pool, provider_id, external_id).await?;
    let leased = !tracked && is_leased(pool, provider_id, external_id).await?;
    let sighting = get_sighting(pool, provider_id, external_id).await?;
    let decision = decide(now, sandbox.created_at, tracked, leased, sighting.as_ref());
    match decision {
        OrphanDecision::Keep(KeepReason::Tracked) => {
            if sighting.is_some() {
                clear_sighting(pool, provider_id, external_id).await?;
            }
            Ok(None)
        }
        OrphanDecision::Keep(_) => Ok(None),
        OrphanDecision::Wait => {
            record_sighting(pool, provider_id, external_id, org_id, false).await?;
            Ok(None)
        }
        OrphanDecision::Stop => {
            // An already-stopped sandbox only needs the sighting recorded.
            if !sandbox.active {
                record_sighting(pool, provider_id, external_id, org_id, true).await?;
                return Ok(None);
            }
            let stopped = match provider.stop_by_id(token, external_id).await {
                Ok(()) => {
                    info!(
                        provider = provider_id,
                        external_id,
                        org_id,
                        state = %sandbox.state,
                        "Stopped orphaned managed sandbox"
                    );
                    true
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        provider = provider_id,
                        external_id,
                        org_id,
                        "Failed to stop orphaned managed sandbox"
                    );
                    false
                }
            };
            record_sighting(pool, provider_id, external_id, org_id, stopped).await?;
            Ok(Some(OrphanDecision::Stop))
        }
        OrphanDecision::Delete => {
            match provider.delete_by_id(token, external_id).await {
                Ok(()) => {
                    info!(
                        provider = provider_id,
                        external_id, org_id, "Deleted orphaned managed sandbox"
                    );
                    mark_deleted(pool, provider_id, external_id).await?;
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        provider = provider_id,
                        external_id,
                        org_id,
                        "Failed to delete orphaned managed sandbox"
                    );
                    record_sighting(pool, provider_id, external_id, org_id, false).await?;
                }
            }
            Ok(Some(OrphanDecision::Delete))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes_ago: i64, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        Some(now - chrono::Duration::minutes(minutes_ago))
    }

    fn sighting(hours_ago: i64, stopped: bool, now: DateTime<Utc>) -> Sighting {
        let first = now - chrono::Duration::hours(hours_ago);
        Sighting {
            first_seen_at: first,
            stopped_at: stopped.then_some(first),
            deleted_at: None,
        }
    }

    #[test]
    fn tracked_and_leased_sandboxes_are_kept_whatever_their_age() {
        let now = Utc::now();
        let old = at(60 * 48, now);
        let seen = sighting(48, true, now);
        assert_eq!(
            decide(now, old, true, false, Some(&seen)),
            OrphanDecision::Keep(KeepReason::Tracked)
        );
        assert_eq!(
            decide(now, old, false, true, Some(&seen)),
            OrphanDecision::Keep(KeepReason::Leased)
        );
    }

    #[test]
    fn young_or_undated_sandboxes_are_kept() {
        let now = Utc::now();
        assert_eq!(
            decide(now, at(29, now), false, false, None),
            OrphanDecision::Keep(KeepReason::TooYoung)
        );
        assert_eq!(
            decide(now, None, false, false, None),
            OrphanDecision::Keep(KeepReason::UnknownAge)
        );
    }

    #[test]
    fn first_sighting_stops_and_a_day_later_deletes() {
        let now = Utc::now();
        let created = at(31, now);
        assert_eq!(
            decide(now, created, false, false, None),
            OrphanDecision::Stop
        );
        assert_eq!(
            decide(now, created, false, false, Some(&sighting(1, true, now))),
            OrphanDecision::Wait
        );
        assert_eq!(
            decide(now, created, false, false, Some(&sighting(23, false, now))),
            OrphanDecision::Stop,
            "a stop that failed is retried"
        );
        assert_eq!(
            decide(now, created, false, false, Some(&sighting(24, true, now))),
            OrphanDecision::Delete
        );
    }

    #[test]
    fn label_recheck_requires_managed_label_and_org() {
        let mut sandbox = OwnedSandbox {
            external_id: "sb".to_string(),
            labels: scan_labels("org_a"),
            state: "started".to_string(),
            active: true,
            created_at: None,
        };
        assert!(labels_match(&sandbox, "org_a"));
        assert!(!labels_match(&sandbox, "org_b"));
        sandbox.labels.remove(LABEL_MANAGED);
        assert!(!labels_match(&sandbox, "org_a"), "tool-created sandbox");
    }
}
