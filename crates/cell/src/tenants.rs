//! What a cell leader and an executor know of the tenants they serve: the
//! orgs that may not start new turns (suspended or deleted), as the
//! directory cell holds them (cp-dp-protocol.md, "Freshness").
//!
//! Decisions:
//! - The directory cell is the data plane's change stream: every cell
//!   leader and every executor polls its `/cell/tenants` read every
//!   [`POLL`]. A suspension therefore stops new turns within a poll
//!   everywhere that can reach the directory.
//! - Freshness bounds a partition: a leader or executor that hasn't heard
//!   from the directory within [`FRESH`] starts no new turn at all (a
//!   leader offers nothing, an executor pulls nothing). Running turns go
//!   on: suspension refuses new work, it doesn't kill turns.
//! - Two guards, not one: the cell leader doesn't offer a stopped tenant's
//!   sessions, and the executor doesn't run one it was offered anyway (a
//!   leader whose view is a poll older than the executor's).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::node::Node;

/// How often the view is refreshed.
pub const POLL: Duration = Duration::from_secs(2);
/// How old a view may be before no new turn starts.
pub const FRESH: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct TenantView {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    stopped: BTreeMap<String, String>,
    heard: Option<Instant>,
}

/// Whether a turn of a tenant may start now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admit {
    Yes,
    /// The view is older than [`FRESH`]: nothing starts.
    Stale,
    /// The tenant is suspended or deleted (the reason).
    Stopped(String),
}

impl TenantView {
    /// A view that has heard just now (a single-cell test fleet with no
    /// directory to poll, and unit tests).
    pub fn fresh() -> Self {
        let view = Self::default();
        view.heard(BTreeMap::new());
        view
    }

    /// The directory answered: these tenants are stopped.
    pub fn heard(&self, stopped: BTreeMap<String, String>) -> Vec<String> {
        let mut inner = self.inner.lock();
        let resumed = inner
            .stopped
            .keys()
            .filter(|t| !stopped.contains_key(*t))
            .cloned()
            .collect();
        inner.stopped = stopped;
        inner.heard = Some(Instant::now());
        resumed
    }

    pub fn is_fresh(&self) -> bool {
        self.inner
            .lock()
            .heard
            .is_some_and(|at| at.elapsed() < FRESH)
    }

    /// May a turn of `tenant` (none: an unmetered, tenantless session)
    /// start now?
    pub fn admit(&self, tenant: Option<&str>) -> Admit {
        let inner = self.inner.lock();
        if !inner.heard.is_some_and(|at| at.elapsed() < FRESH) {
            return Admit::Stale;
        }
        match tenant.and_then(|t| inner.stopped.get(t)) {
            Some(why) => Admit::Stopped(why.clone()),
            None => Admit::Yes,
        }
    }

    #[cfg(test)]
    fn age(&self, by: Duration) {
        let mut inner = self.inner.lock();
        inner.heard = inner.heard.and_then(|at| at.checked_sub(by));
    }
}

/// The directory's answer to `/cell/tenants`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Tenants {
    /// Org id → why it is stopped (`suspended`, `deleted`, a reason).
    pub stopped: BTreeMap<String, String>,
}

impl Node {
    /// While this node leads its cell: keep [`Node::tenants`] current.
    pub async fn tenants_loop(self: std::sync::Arc<Self>) {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if !self.is_leader() {
                continue;
            }
            match self.read_tenants().await {
                Ok(tenants) => {
                    self.tenants.heard(tenants.stopped);
                }
                Err(error) => tracing::debug!(cell = self.cfg.cell, %error, "tenants not read"),
            }
        }
    }

    async fn read_tenants(&self) -> anyhow::Result<Tenants> {
        let gateway = self
            .gateway
            .get()
            .ok_or_else(|| anyhow::anyhow!("no gateway yet"))?;
        let directory = gateway.directory_cell().map_err(|(_, e)| anyhow::anyhow!(e))?;
        if directory == self.cfg.cell {
            let state = self.shared.state.read();
            return Ok(Tenants {
                stopped: state.orgs.stopped(),
            });
        }
        gateway
            .call(directory, "/cell/tenants", None)
            .await
            .map_err(|(_, e)| anyhow::anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_tenant_waits_and_a_stale_view_stops_everything() {
        let view = TenantView::default();
        assert_eq!(view.admit(Some("o")), Admit::Stale, "never heard");
        view.heard(BTreeMap::from([("o".into(), "billing".into())]));
        assert_eq!(view.admit(Some("o")), Admit::Stopped("billing".into()));
        assert_eq!(view.admit(Some("p")), Admit::Yes);
        assert_eq!(view.admit(None), Admit::Yes);
        assert_eq!(view.heard(BTreeMap::new()), vec!["o".to_owned()], "resumed");
        assert_eq!(view.admit(Some("o")), Admit::Yes);
        view.age(FRESH);
        assert_eq!(view.admit(None), Admit::Stale);
    }
}
