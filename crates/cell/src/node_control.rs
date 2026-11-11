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
//! - Delivered feed records are archived in the object store before the
//!   acknowledgement leaves state (`feed/<source>/<incarnation>/...`), so a
//!   restored control plane can ask for them again (`POST
//!   /dp/v1/feed/replay`). An archive that fails keeps the records in
//!   state; they are sent again and the control plane deduplicates them.
//! - Both directions of the link carry the data plane's token (`Authorization:
//!   Bearer`, `CELL_DP_TOKEN`) when one is set; mTLS replaces it with the
//!   EKS deployment (step 10).
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
    /// The link's shared secret (`CELL_DP_TOKEN`), sent as a bearer token
    /// and required on the data plane's `/dp/v1`.
    pub token: Option<String>,
    /// Where the control plane's org token keys are (`CELL_JWKS_URL`);
    /// set, the gateway requires org tokens (`org_token`).
    pub jwks_url: Option<String>,
}

impl ControlPlane {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            data_plane: std::env::var("CELL_DATA_PLANE")
                .ok()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "dp-local".into()),
            incarnation: 1,
            want_micros: 1_000_000,
            per_call_micros: 10_000,
            token: link_token(),
            jwks_url: std::env::var("CELL_JWKS_URL")
                .ok()
                .filter(|u| !u.is_empty()),
        }
    }

    /// A request to the control plane at `path`, with the link's token.
    pub fn post(&self, http: &reqwest::Client, path: &str) -> reqwest::RequestBuilder {
        let request = http.post(format!("{}{path}", self.url));
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

/// The link's shared secret, from `CELL_DP_TOKEN` (empty: none).
pub fn link_token() -> Option<String> {
    std::env::var("CELL_DP_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

/// The link's token as the routes that check it hold it (none: open).
pub type LinkToken = Option<Arc<str>>;

/// Refuse a `/dp/v1` call that lacks the link's token, when one is set.
pub async fn require_link_token(
    axum::extract::State(token): axum::extract::State<LinkToken>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Some(token) = token {
        let sent = crate::org_token::bearer(request.headers());
        if !sent.is_some_and(|sent| same_secret(sent.as_bytes(), token.as_bytes())) {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                "the data plane token is missing or wrong",
            )
                .into_response();
        }
    }
    next.run(request).await
}

/// Compare two secrets in time that doesn't depend on where they differ.
pub fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
        let ack: FeedAck = cp
            .post(&self.http, "/dp/v1/feed")
            .json(&batch)
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let delivered: Vec<_> = batch
            .records
            .iter()
            .filter(|r| (r.index, r.ordinal) <= ack.stored_through)
            .cloned()
            .collect();
        self.cfg
            .store
            .put_feed(&batch.source, batch.incarnation, &delivered)
            .await?;
        self.propose(Command::FeedAcked {
            through: ack.stored_through,
        })
        .await?;
        Ok(())
    }

    pub(crate) fn source(&self) -> String {
        format!("cell-{}", self.cfg.cell)
    }

    /// Resend `source`'s archived feed from `from_index` (feed replay).
    pub async fn replay_feed(
        &self,
        source: &str,
        incarnation: u64,
        from_index: u64,
    ) -> anyhow::Result<usize> {
        let cp = self
            .cfg
            .control_plane
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no control plane configured"))?;
        let records = self
            .cfg
            .store
            .feed_since(source, incarnation, from_index)
            .await?;
        let total = records.len();
        for chunk in records.chunks(FEED_BATCH) {
            let batch = FeedBatch {
                data_plane: cp.data_plane.clone(),
                source: source.to_owned(),
                incarnation,
                records: chunk.to_vec(),
            };
            cp.post(&self.http, "/dp/v1/feed")
                .json(&batch)
                .timeout(Duration::from_secs(10))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(total)
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
        let grant: AllowanceGrant = cp
            .post(&self.http, &format!("/dp/v1/tenants/{tenant}/allowances"))
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
