//! The control plane's half of the event tap (data plane v2, step 11.5).
//!
//! A v2 org's session events live on its data plane. Observers and
//! trace exports work from events, so the org's policy names the
//! types they need (`TenantPolicy::events`), the data plane copies those
//! onto the feed (`cell/src/tap.rs`), and feed ingest hands each one here.
//!
//! Decisions:
//! - The policy asks only for what the org uses: observer types while it
//!   has an active observer, trace types while it has a destination, and
//!   every type while the warehouse export is on (`warehouse.rs`). It is set again whenever one of
//!   those changes (`sync`), and an equal policy is no change.
//! - The listeners are the ones v1 runs (`LISTENERS`), so a v2 org's
//!   observers and exports behave as a v1 org's. Reports are deprecated and
//!   get nothing from v2.
//! - The feed delivers each record once, by position. A listener's own
//!   failure is logged and never fails the record: retrying it would hand
//!   the event to every other listener a second time.
//! - THREAT[TM-TENANT]: an event is delivered only for a session of the
//!   org the record names; anything else is dropped.

use std::collections::BTreeSet;
use std::sync::Arc;

use everruns_contracts::dataplane::{ConfigChange, TenantPolicy};
use everruns_contracts::typed_id::SessionId;
use everruns_core::{Event, EventListener};
use serde_json::Value;

use super::{ControlPlane, put_policy};
use crate::storage::StorageBackend;

/// The control plane listeners, by name, that a v2 org's events go to.
pub const LISTENERS: &[&str] = &[
    "ObserverMatchListener",
    "CustomerTracing",
    crate::domains::agents::view_publication::LISTENER,
];

/// The event types `org_id`'s data plane should copy onto the feed, under
/// its current `policy`: every type while the warehouse export is on.
pub async fn wanted(
    db: &StorageBackend,
    org_id: i64,
    policy: &TenantPolicy,
) -> anyhow::Result<Vec<String>> {
    if super::warehouse::enabled(policy) {
        return Ok(vec!["*".into()]);
    }
    let mut types = BTreeSet::new();
    // Uncached: called right after an observer changed.
    if db.count_active_observers(org_id).await? > 0 {
        types.extend(crate::domains::observers::OBSERVED_EVENT_TYPES);
    }
    if crate::domains::observers::system::policy(db, org_id)
        .await?
        .enabled
    {
        types.extend(
            crate::domains::observers::system::EVENT_TYPES
                .iter()
                .copied(),
        );
    }
    if !db.tracing_export_destinations(org_id).await?.is_empty() {
        types.extend(crate::domains::trace_exports::router::TRACED_EVENT_TYPES);
    }
    if db.has_agent_views(org_id).await? {
        types.extend(crate::domains::agents::view_publication::EVENT_TYPES);
    }
    Ok(types.into_iter().map(str::to_owned).collect())
}

/// `org_id`'s policy as last put (the default before the first).
pub async fn policy(db: &StorageBackend, org_id: i64) -> anyhow::Result<TenantPolicy> {
    Ok(db
        .dp_item(org_id, "policy", "policy")
        .await?
        .and_then(|item| item.body)
        .and_then(|body| match ConfigChange::parse(&body) {
            Some(ConfigChange::Policy(policy)) => Some(policy),
            _ => None,
        })
        .unwrap_or_default())
}

/// Ask `org_id`'s data plane for the events it now needs; nothing for an
/// org the platform's workers run.
pub async fn sync(db: &StorageBackend, org_id: i64) -> anyhow::Result<()> {
    if db.dp_org(org_id).await?.is_none() {
        return Ok(());
    }
    let mut policy = policy(db, org_id).await?;
    policy.events = wanted(db, org_id, &policy).await?;
    put_policy(db, org_id, &policy).await
}

/// [`sync`] for a caller that has already made its change: a failure is
/// logged, and the next change (or the org's next enable) sets it again.
pub async fn resync(db: &StorageBackend, org_id: i64) {
    if let Err(error) = sync(db, org_id).await {
        tracing::warn!(org_id, %error, "data plane event types not updated");
    }
}

/// Hand one tapped event of `org_id` to the listeners that take its type.
pub async fn deliver(
    cp: &ControlPlane,
    listeners: &[Arc<dyn EventListener>],
    org_id: i64,
    session_id: &str,
    event: &Value,
) -> anyhow::Result<()> {
    let event = match serde_json::from_value::<Event>(event.clone()) {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(org_id, session_id, %error, "tapped event not readable; dropped");
            return Ok(());
        }
    };
    let session = session_id.parse::<SessionId>().ok();
    if session != Some(event.session_id)
        || cp.db.session_org_id(event.session_id).await? != Some(org_id)
    {
        tracing::warn!(
            org_id,
            session_id,
            "tapped event of another org's session; dropped"
        );
        return Ok(());
    }
    let kind = event.event_type.as_str();
    // Persist system matching before handing the record to best-effort user
    // listeners. A storage failure retries this feed position, idempotently.
    crate::domains::observers::system::enqueue_event(&cp.db, &event).await?;
    for listener in listeners {
        if LISTENERS.contains(&listener.name())
            && listener
                .event_types()
                .is_none_or(|types| types.contains(&kind))
        {
            listener.on_event(&event).await;
        }
    }
    Ok(())
}
