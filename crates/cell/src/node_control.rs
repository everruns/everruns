//! The leader's link to the control plane (cp-dp-protocol.md): it delivers
//! the cell's feed and asks for its tenants' spend allowances.
//!
//! Decisions:
//! - Everything the loop sends comes from committed state and every answer
//!   goes back through Raft, so a leader change loses nothing and repeats
//!   nothing: the next leader resends unacknowledged feed records under
//!   their ids, and an unanswered allowance request under its id (the
//!   tenant's answered-request count names it), which the control plane
//!   answers once.
//! - A metered session's turn waits while its tenant has no money here and
//!   the control plane has not refused it: with the control plane down,
//!   turns run until the allowances are spent, then new turns wait. A
//!   refusal (`budget_exhausted`, ...) lets the turn start and end
//!   exhausted, and the loop keeps asking now and then.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::dp::{AllowanceGrant, AllowanceRequest, FeedAck, FeedBatch};
use crate::node::{Node, now_ms};
use crate::state::{CellState, Command, SessionState};

/// Where a cell's control plane is, and what to ask it for.
#[derive(Debug, Clone)]
pub struct ControlPlane {
    /// Base URL serving `/dp/v1`.
    pub url: String,
    /// This data plane's id in the control plane.
    pub data_plane: String,
    /// Bumped when the cell is rebuilt from a restore.
    pub incarnation: u64,
    /// What one allowance request asks for.
    pub want_micros: i64,
    /// The most one provider call may cost, when a grant does not say.
    pub per_call_micros: i64,
}

impl ControlPlane {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            data_plane: "dp-local".into(),
            incarnation: 1,
            want_micros: 1_000_000,
            per_call_micros: 10_000,
        }
    }
}

/// Feed records per batch.
const FEED_BATCH: usize = 256;
/// How often a refused tenant asks again.
const ASK_AGAIN: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct ControlLink {
    /// Tenants with a request in flight, and when refused ones last asked.
    asking: Mutex<BTreeSet<String>>,
    refused_at: Mutex<BTreeMap<String, Instant>>,
}

impl Node {
    /// Whether `s` may start a turn as far as money goes.
    pub(crate) fn allowance_ready(&self, state: &CellState, s: &SessionState, now: i64) -> bool {
        if self.cfg.control_plane.is_none() {
            return true;
        }
        let Some(tenant) = s.tenant.as_deref() else {
            return true;
        };
        state
            .allowances
            .tenants
            .get(tenant)
            .is_some_and(|a| a.spendable(now) > 0 || a.refused.is_some())
    }

    pub(crate) async fn control_loop(self: Arc<Self>) {
        let Some(cp) = self.cfg.control_plane.clone() else {
            return;
        };
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        loop {
            tick.tick().await;
            if !self.is_leader() {
                continue;
            }
            if let Err(error) = self.deliver_feed(&cp).await {
                tracing::debug!(%error, "feed delivery failed");
            }
            for tenant in self.tenants_to_ask(&cp) {
                let node = self.clone();
                let cp = cp.clone();
                tokio::spawn(async move {
                    if let Err(error) = node.ask(&cp, &tenant).await {
                        tracing::debug!(%tenant, %error, "allowance request failed");
                    }
                    node.control.asking.lock().remove(&tenant);
                });
            }
        }
    }

    async fn deliver_feed(&self, cp: &ControlPlane) -> anyhow::Result<()> {
        let records: Vec<_> = {
            let state = self.shared.state.read();
            state
                .feed
                .records
                .iter()
                .take(FEED_BATCH)
                .cloned()
                .collect()
        };
        if records.is_empty() {
            return Ok(());
        }
        let batch = FeedBatch {
            data_plane: cp.data_plane.clone(),
            source: self.source(),
            incarnation: cp.incarnation,
            records,
        };
        let ack: FeedAck = self
            .http
            .post(format!("{}/dp/v1/feed", cp.url))
            .json(&batch)
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        self.propose(Command::FeedAcked {
            through: ack.stored_through,
        })
        .await?;
        Ok(())
    }

    fn source(&self) -> String {
        format!("cell-{}", self.cfg.cell)
    }

    /// Tenants running or owing turns here that are low on money, or hold
    /// grants to settle as closed; marked as asking.
    fn tenants_to_ask(&self, cp: &ControlPlane) -> Vec<String> {
        let now = now_ms();
        let state = self.shared.state.read();
        let busy: BTreeSet<&str> = state
            .sessions
            .values()
            .filter(|s| s.turn_running() || s.authority.owed(now))
            .filter_map(|s| s.tenant.as_deref())
            .collect();
        let mut asking = self.control.asking.lock();
        let mut refused_at = self.control.refused_at.lock();
        let mut out = Vec::new();
        let tenants: BTreeSet<&str> = busy
            .iter()
            .copied()
            .chain(state.allowances.tenants.keys().map(String::as_str))
            .collect();
        for tenant in tenants {
            if asking.contains(tenant) {
                continue;
            }
            let allowance = state.allowances.tenants.get(tenant);
            let closable = allowance.is_some_and(|a| a.settlements(now).iter().any(|s| s.closed));
            let low = busy.contains(tenant)
                && allowance.is_none_or(|a| a.spendable(now) < cp.want_micros / 4);
            let refused = allowance.is_some_and(|a| a.refused.is_some());
            if refused
                && !closable
                && refused_at
                    .get(tenant)
                    .is_some_and(|at| at.elapsed() < ASK_AGAIN)
            {
                continue;
            }
            if low || closable {
                if refused {
                    refused_at.insert(tenant.to_owned(), Instant::now());
                }
                asking.insert(tenant.to_owned());
                out.push(tenant.to_owned());
            }
        }
        out
    }

    /// Ask the control plane for `tenant`'s allowance, settling its grants,
    /// and commit the answer.
    async fn ask(&self, cp: &ControlPlane, tenant: &str) -> anyhow::Result<()> {
        let now = now_ms();
        let (request, settlements, busy) = {
            let state = self.shared.state.read();
            let allowance = state.allowances.tenants.get(tenant);
            let busy = state.sessions.values().any(|s| {
                s.tenant.as_deref() == Some(tenant) && (s.turn_running() || s.authority.owed(now))
            });
            (
                allowance.map_or(0, |a| a.requests),
                allowance.map(|a| a.settlements(now)).unwrap_or_default(),
                busy,
            )
        };
        let body = AllowanceRequest {
            request_id: format!("{}/{}/{tenant}/{request}", self.source(), cp.incarnation),
            cell: self.source(),
            incarnation: cp.incarnation,
            want_micros: if busy { cp.want_micros } else { 0 },
            settlements: settlements.clone(),
        };
        let grant: AllowanceGrant = self
            .http
            .post(format!("{}/dp/v1/tenants/{tenant}/allowances", cp.url))
            .json(&body)
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let granted = (grant.granted_micros > 0).then(|| {
            (
                grant.grant_id.clone(),
                crate::allowance::Grant {
                    granted_micros: grant.granted_micros,
                    per_call_micros: grant.per_call_micros.unwrap_or(cp.per_call_micros),
                    spend_until_ms: grant.spend_until_unix_ms,
                    used_micros: 0,
                    overrun_micros: 0,
                },
            )
        });
        let refused = match (&granted, body.want_micros) {
            (None, want) if want > 0 => Some(grant.reason.unwrap_or_else(|| "exhausted".into())),
            _ => None,
        };
        self.propose(Command::Allowance {
            tenant: tenant.to_owned(),
            request,
            grant: granted,
            refused,
            closed: settlements
                .into_iter()
                .filter(|s| s.closed)
                .map(|s| s.grant_id)
                .collect(),
        })
        .await?;
        Ok(())
    }
}
