//! A follower that fell far behind its leader steps aside until it has
//! caught up.
//!
//! A node that rejoins after a kill can be tens of thousands of entries
//! behind, and on a loaded host it takes minutes to catch up. In EKS run
//! 43803384547 such a follower:
//!
//! - answered "not found" from its stale copy for sessions created since,
//!   and 6,790 live sessions lost their first message;
//! - streamed sessions it had not heard of, and failed those streams;
//! - kept calling elections it could not win while it waited (term 244 by
//!   the end). Each one moved the cell to a new term, unseated its leader
//!   and restarted the leader's replication to every follower.
//!
//! (It waited for good because its snapshot chunks were refused for their
//! size, see `network::router`; a follower far behind still takes minutes.)
//!
//! So while a follower is behind ([`BEHIND_AT`] entries or more short of
//! the commit index its leader last sent), it does not call elections and
//! answers no session request: `/health` and the session routes say 503 and
//! 421, and the gateway and the cell's other members use another member.
//! It still votes and still takes the leader's entries. It counts as caught
//! up again at [`CAUGHT_UP_AT`].
//!
//! Stepping aside never costs the cell a leader: a log that short of the
//! commit index cannot win an election, so its candidacy only disturbs.
//! When the leader has gone quiet for [`QUIET`], the node campaigns again
//! whatever it last heard, so a cell whose other members all went away
//! still elects once they return.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Behind by this many committed entries or more: step aside.
pub const BEHIND_AT: u64 = 2_048;
/// Back within this many: take part again.
pub const CAUGHT_UP_AT: u64 = 256;
/// No word from a leader for this long: campaign whatever it last said.
pub const QUIET: Duration = Duration::from_secs(30);

/// What this node last heard from its leader, and whether it is behind.
#[derive(Debug)]
pub struct CatchUp {
    leader_commit: AtomicU64,
    heard: Mutex<Option<Instant>>,
    behind: AtomicBool,
}

impl Default for CatchUp {
    fn default() -> Self {
        Self {
            leader_commit: AtomicU64::new(0),
            heard: Mutex::new(None),
            behind: AtomicBool::new(false),
        }
    }
}

impl CatchUp {
    /// A leader's append or snapshot said `index` is committed.
    pub fn heard(&self, index: u64) {
        self.heard_at(index, Instant::now());
    }

    fn heard_at(&self, index: u64, now: Instant) {
        self.leader_commit.fetch_max(index, Ordering::Relaxed);
        *self.heard.lock() = Some(now);
    }

    /// Whether this node is behind and stepping aside.
    pub fn behind(&self) -> bool {
        self.behind.load(Ordering::Relaxed)
    }

    /// The commit index the leader last sent.
    pub fn leader_commit(&self) -> u64 {
        self.leader_commit.load(Ordering::Relaxed)
    }

    /// Re-judge from this node's role and applied index; `Some(behind)`
    /// when that changed.
    pub fn update(&self, leader: bool, applied: u64) -> Option<bool> {
        self.update_at(leader, applied, Instant::now())
    }

    fn update_at(&self, leader: bool, applied: u64, now: Instant) -> Option<bool> {
        let was = self.behind();
        let quiet = self
            .heard
            .lock()
            .is_none_or(|heard| now.duration_since(heard) >= QUIET);
        let gap = self.leader_commit().saturating_sub(applied);
        let behind = !leader
            && !quiet
            && match was {
                true => gap > CAUGHT_UP_AT,
                false => gap >= BEHIND_AT,
            };
        self.behind.store(behind, Ordering::Relaxed);
        (behind != was).then_some(behind)
    }
}

impl crate::node::Node {
    /// Whether this node's copy has `session`, here or as moved away.
    pub fn knows(&self, session: everruns_contracts::typed_id::SessionId) -> bool {
        let state = self.shared.state.read();
        state.sessions.contains_key(&session) || state.forwarded.contains_key(&session)
    }

    /// Step aside while behind: no elections, no session requests.
    pub(crate) async fn catchup_loop(self: std::sync::Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            let (leader, applied) = {
                let metrics = self.raft.metrics().borrow().clone();
                (
                    metrics.state == openraft::ServerState::Leader,
                    metrics.last_applied.map_or(0, |id| id.index),
                )
            };
            if let Some(behind) = self.catchup.update(leader, applied) {
                self.raft.runtime_config().elect(!behind);
                tracing::warn!(
                    cell = self.cfg.cell,
                    node = self.cfg.node_id,
                    behind,
                    applied,
                    leader_commit = self.catchup.leader_commit(),
                    "follower catch-up changed"
                );
            }
        }
    }
}

/// Turn a node's requests away while it cannot answer them: while it is
/// behind, `/health` (so its peers stop counting on it) and every session
/// route; on any follower, a session it has not seen.
///
/// Decision: only the leader's "no such session" is an answer. A follower
/// answers from its own copy, which lags the leader's (by milliseconds in a
/// healthy cell, by minutes while it catches up), and the gateway took its
/// "not found" as final. A follower that has not seen the session says 421
/// and the gateway asks the next member.
pub async fn guard(
    axum::extract::State(node): axum::extract::State<std::sync::Arc<crate::node::Node>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let path = request.uri().path();
    if path == "/health" && node.catchup.behind() {
        return (StatusCode::SERVICE_UNAVAILABLE, "catching up").into_response();
    }
    if let Some(rest) = path.strip_prefix("/cell/sessions/") {
        if node.catchup.behind() {
            return (
                StatusCode::MISDIRECTED_REQUEST,
                "this member is catching up; ask another",
            )
                .into_response();
        }
        let (session, command) = rest.split_once('/').unwrap_or((rest, ""));
        if command != "create"
            && !node.is_leader()
            && let Ok(id) = everruns_contracts::typed_id::SessionId::parse(session)
            && !node.knows(id)
        {
            return (
                StatusCode::MISDIRECTED_REQUEST,
                format!("this member has not seen session {id}; ask another"),
            )
                .into_response();
        }
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_follower_far_behind_steps_aside_until_it_catches_up() {
        let c = CatchUp::default();
        let t = Instant::now();
        assert_eq!(c.update_at(false, 0, t), None, "nothing heard yet");
        c.heard_at(60_000, t);
        assert_eq!(c.update_at(false, 59_000, t), None, "a normal lag");
        assert_eq!(c.update_at(false, 40_000, t), Some(true));
        assert!(c.behind());
        c.heard_at(90_000, t);
        assert_eq!(c.update_at(false, 89_000, t), None, "still short");
        assert_eq!(c.update_at(false, 89_800, t), Some(false));
        assert!(!c.behind());
    }

    #[test]
    fn a_leader_or_a_follower_whose_leader_went_quiet_never_steps_aside() {
        let c = CatchUp::default();
        let t = Instant::now();
        c.heard_at(60_000, t);
        assert_eq!(c.update_at(true, 0, t), None, "the leader is never behind");
        assert_eq!(c.update_at(false, 0, t), Some(true));
        assert_eq!(c.update_at(false, 0, t + QUIET), Some(false), "quiet");
        c.heard_at(60_000, t + QUIET);
        assert_eq!(c.update_at(false, 0, t + QUIET), Some(true));
        assert_eq!(c.update_at(true, 0, t + QUIET), Some(false), "elected");
    }
}
