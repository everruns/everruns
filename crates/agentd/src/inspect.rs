//! The shard inspector: where a session is and what it is doing, and how each
//! shard is keeping up. An operator's first look in an incident (runbooks:
//! leader loss, a stuck move, a drain).
//!
//! ```text
//! app inspect --topology cluster.json shards
//! app inspect --topology cluster.json session <id>
//! app inspect --topology cluster.json moves
//! ```
//!
//! Decisions:
//! - Read only, and from the nodes' own HTTP endpoints, so it works with no
//!   control plane and against a shard that has no leader (it then says so
//!   instead of failing).
//! - The shard list is the topology file's plus the directory shard's
//!   registry: shards and nodes added later by `--join` are not in the file.
//! - A move is "stuck" when a session sits frozen or prepared; the inspector
//!   lists them with their generation and target, and the directory's view,
//!   which is what decides how the move ends.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use everruns::SessionId;
use serde::{Deserialize, Serialize};

use crate::cluster::Topology;
use crate::node::Node;
use crate::state::{Placed, Placement, ShardId};
use crate::types::NodeId;

/// Moves listed per shard, at most.
const MOVES_SHOWN: usize = 50;

/// Events of a session's tail the inspector shows.
const TAIL: usize = 8;

/// One node's view of its shard.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ShardReport {
    pub shard: ShardId,
    pub node: NodeId,
    pub leader: Option<NodeId>,
    pub incarnation: u64,
    pub applied: u64,
    /// Highest index this node shipped, if it leads and has shipped.
    pub shipped: Option<u64>,
    /// How long the oldest unshipped entry has waited, in milliseconds.
    pub unshipped_ms: Option<u64>,
    pub last_snapshot_s: u64,
    pub draining: bool,
    pub sessions: usize,
    pub active_turns: usize,
    pub queued_inputs: usize,
    pub feed_pending: usize,
    pub rss_mb: u64,
    /// Of `rss_mb`: the estimate of resident events, and the Raft log
    /// entries held in memory until a snapshot purges them. The rest is
    /// session metadata, buffers and allocator slack.
    #[serde(default)]
    pub resident_mb: u64,
    #[serde(default)]
    pub log_mb: u64,
    /// Object store writes this node's process made since it started, by
    /// kind (`store::PUT_KINDS`): what a run costs in requests.
    #[serde(default)]
    pub store_puts: std::collections::BTreeMap<String, u64>,
    pub moves_total: usize,
    pub moves: Vec<MoveView>,
}

/// A session part way through a move.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MoveView {
    pub session: SessionId,
    pub placement: Placement,
    pub generation: u64,
    pub moving_to: Option<ShardId>,
}

/// What a session is doing, from the node that answered.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionReport {
    pub shard: ShardId,
    pub node: NodeId,
    pub agent: String,
    pub tenant: Option<String>,
    pub placement: Placement,
    pub generation: u64,
    pub moving_to: Option<ShardId>,
    pub branch: u64,
    pub last_sequence: i32,
    pub turns_completed: u64,
    pub turn_running: bool,
    pub inbox: usize,
    pub holder: Option<crate::authority::Holder>,
    /// Why the session is or is not handed to an executor (`schedule`).
    #[serde(default)]
    pub work: WorkReport,
    /// Events only in the object store, and segments holding them.
    pub offloaded: i32,
    pub segments: usize,
    pub tail: Vec<TailEvent>,
}

/// The authority's scheduling state: a turn that never finishes shows here
/// why nothing runs it (not owed, suspended, a holder that never expires).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct WorkReport {
    pub owed: bool,
    pub runnable: bool,
    pub suspended: bool,
    pub wake_at: Option<i64>,
    pub attempts: u32,
    pub fence: u64,
    pub open: Option<String>,
    pub inbox: i64,
    pub consumed: i64,
    pub finalizing: usize,
    pub now_ms: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TailEvent {
    pub sequence: Option<i64>,
    #[serde(rename = "type")]
    pub kind: String,
    pub ts: Option<String>,
}

/// `GET /shard/inspect`.
pub(crate) async fn shard(State(node): State<Arc<Node>>) -> Json<ShardReport> {
    // Before the state lock: `resident` takes it too.
    let resident_mb = node.shared.resident().0 >> 20;
    let log_mb = node.wal.held_bytes() >> 20;
    let state = node.shared.state.read();
    let mut moves: Vec<MoveView> = state
        .sessions
        .iter()
        .filter(|(_, s)| s.placement != Placement::Live)
        .map(|(id, s)| MoveView {
            session: *id,
            placement: s.placement,
            generation: s.generation,
            moving_to: s.moving_to,
        })
        .collect();
    let moves_total = moves.len();
    moves.sort_by_key(|m| m.session.to_string());
    moves.truncate(MOVES_SHOWN);
    Json(ShardReport {
        shard: node.cfg.shard,
        node: node.cfg.node_id,
        leader: node.leader(),
        incarnation: state.incarnation,
        applied: *node.shared.applied.borrow(),
        shipped: node.shipper.shipped(),
        unshipped_ms: node
            .shipper
            .waiting_since()
            .map(|since| since.elapsed().as_millis() as u64),
        last_snapshot_s: node.shared.last_snapshot_age().as_secs(),
        draining: state.draining.contains(&node.cfg.shard),
        sessions: state.sessions.len(),
        active_turns: state.sessions.values().filter(|s| s.turn_running()).count(),
        queued_inputs: state.sessions.values().map(|s| s.queued()).sum(),
        feed_pending: state.feed.records.len(),
        rss_mb: crate::api::rss_mb(),
        store_puts: crate::store::puts()
            .into_iter()
            .map(|(kind, n)| (kind.to_owned(), n))
            .collect(),
        resident_mb,
        log_mb,
        moves_total,
        moves,
    })
}

/// `GET /shard/sessions/{session}/sandbox`: the session's managed sandbox as
/// its state records it, and the session's work, for the control plane's
/// sandbox diagnosis (`sandboxes diagnose`). Only non-secret fields leave
/// the shard: the state is stored as a session secret, and its provider state
/// and lease credential stay here.
pub(crate) async fn sandbox(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<crate::api::Lookup<serde_json::Value>>, (StatusCode, String)> {
    let id = SessionId::parse(&session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(crate::api::lookup(&node, id, sandbox_report)))
}

fn sandbox_report(s: &crate::state::SessionState) -> serde_json::Value {
    use everruns_contracts::session_sandbox::{SESSION_SANDBOX_SECRET_NAME, SessionSandboxState};
    let a = &s.authority;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let running = s.turn_running();
    let queued = s.queued();
    let phase = if running {
        "running"
    } else if queued > 0 {
        "queued"
    } else {
        "idle"
    };
    let sandbox = a
        .secrets
        .get(SESSION_SANDBOX_SECRET_NAME)
        .and_then(|raw| serde_json::from_str::<SessionSandboxState>(raw).ok())
        .map(|state| {
            serde_json::json!({
                "provider": state.provider,
                "external_id": state.instance.external_id,
                "status": state.status,
                "workspace_path": state.instance.workspace_path,
                "created_at": state.created_at,
                "updated_at": state.updated_at,
                "init_completed_at": state.init_completed_at,
                "last_init_error": state
                    .last_init_error
                    .map(|e| everruns_contracts::secret_scrub::scrub_secrets(&e)),
                "last_sync": state.instance.metadata.get("everruns_fs"),
            })
        });
    serde_json::json!({
        "work": {
            "phase": phase,
            "turn_running": running,
            "inbox": queued,
            "claimed": a.holder.is_some(),
            "attempts": a.attempts,
            "runnable": a.runnable,
            "suspended": a.suspended,
            "owed": a.owed(now_ms),
        },
        "sandbox": sandbox,
    })
}

/// `GET /shard/sessions/{session}/inspect`.
pub(crate) async fn session(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<crate::api::Lookup<SessionReport>>, (StatusCode, String)> {
    let id = SessionId::parse(&session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(crate::api::lookup(&node, id, |s| SessionReport {
        shard: node.cfg.shard,
        node: node.cfg.node_id,
        agent: s.agent.clone(),
        tenant: s.tenant.clone(),
        placement: s.placement,
        generation: s.generation,
        moving_to: s.moving_to,
        branch: s.branch,
        last_sequence: s.last_sequence().unwrap_or(0),
        turns_completed: s.turns_completed(),
        turn_running: s.turn_running(),
        inbox: s.queued(),
        holder: s.authority.holder.clone(),
        work: {
            let a = &s.authority;
            let now_ms = chrono::Utc::now().timestamp_millis();
            WorkReport {
                owed: a.owed(now_ms),
                runnable: a.runnable,
                suspended: a.suspended,
                wake_at: a.wake_at,
                attempts: a.attempts,
                fence: a.fence,
                open: a.open.map(|t| t.to_string()),
                inbox: a.inbox,
                consumed: a.consumed,
                finalizing: a.finalizing.len(),
                now_ms,
            }
        },
        offloaded: s.offloaded,
        segments: s.segments.len(),
        tail: s
            .events
            .iter()
            .rev()
            .take(TAIL)
            .rev()
            .map(|event| {
                let value = serde_json::to_value(event).unwrap_or_default();
                TailEvent {
                    sequence: value["sequence"].as_i64(),
                    kind: value["type"].as_str().unwrap_or("?").to_string(),
                    ts: value["ts"].as_str().map(str::to_string),
                }
            })
            .collect(),
    })))
}

/// What the inspector reads from the fleet.
pub struct Inspector {
    http: reqwest::Client,
    topology: Topology,
}

impl Inspector {
    pub fn new(topology: Topology) -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?,
            topology,
        })
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, addr: &str, path: &str) -> Option<T> {
        let response = self
            .http
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json().await.ok()
    }

    /// Every shard and its nodes: the topology file's, plus what the
    /// directory shard's registry adds (shards that joined later, nodes added
    /// since), less retired shards.
    pub async fn shards(&self) -> BTreeMap<ShardId, BTreeMap<NodeId, String>> {
        let registry = self.registry().await;
        self.merged(registry.as_ref())
    }

    /// The directory shard's registry: shards added since the topology file
    /// was written, retired shards, and which shard holds each directory
    /// slot.
    async fn registry(&self) -> Option<crate::api::Registry> {
        for addr in self.topology.nodes(self.topology.directory_shard) {
            if let Some(registry) = self.get(&addr, "/shard/registry").await {
                return Some(registry);
            }
        }
        None
    }

    fn merged(
        &self,
        registry: Option<&crate::api::Registry>,
    ) -> BTreeMap<ShardId, BTreeMap<NodeId, String>> {
        let mut shards = self.topology.shards.clone();
        if let Some(registry) = registry {
            for (shard, nodes) in &registry.shards {
                shards.entry(*shard).or_default().extend(nodes.clone());
            }
            for retired in &registry.retired {
                shards.remove(retired);
            }
        }
        shards
    }

    /// Each node's report, `None` for a node that did not answer.
    pub async fn reports(&self) -> BTreeMap<ShardId, Vec<(NodeId, Option<ShardReport>)>> {
        let mut out = BTreeMap::new();
        for (shard, nodes) in self.shards().await {
            let mut reports = Vec::new();
            for (node, addr) in nodes {
                reports.push((node, self.get(&addr, "/shard/inspect").await));
            }
            out.insert(shard, reports);
        }
        out
    }

    /// Where the directory places `session`.
    pub async fn placed(&self, session: SessionId) -> Option<Placed> {
        // The directory spreads its slots over shards: ask the shard that holds
        // this session's slot (the directory shard until slots spread).
        let registry = self.registry().await;
        let owner = registry
            .as_ref()
            .and_then(|r| r.slots.get(&crate::state::slot_of(session)).copied())
            .unwrap_or(self.topology.directory_shard);
        let shards = self.merged(registry.as_ref());
        for addr in shards.get(&owner)?.values() {
            if let Some(placed) = self
                .get::<Option<Placed>>(addr, &format!("/shard/directory/{session}"))
                .await
            {
                return placed;
            }
        }
        None
    }

    /// The session as its shard's leader (else any node) sees it.
    pub async fn session(&self, shard: ShardId, session: SessionId) -> Option<SessionReport> {
        let shards = self.shards().await;
        let nodes = shards.get(&shard)?;
        let mut answers = Vec::new();
        for addr in nodes.values() {
            if let Some(crate::api::Lookup::Here(report)) = self
                .get::<crate::api::Lookup<SessionReport>>(
                    addr,
                    &format!("/shard/sessions/{session}/inspect"),
                )
                .await
            {
                answers.push(report);
            }
        }
        let leader = self
            .reports()
            .await
            .get(&shard)
            .and_then(|nodes| nodes.iter().find_map(|(_, r)| r.as_ref()?.leader));
        let pick = answers
            .iter()
            .position(|r| Some(r.node) == leader)
            .unwrap_or(0);
        (!answers.is_empty()).then(|| answers.swap_remove(pick))
    }
}

/// The text the `shards` subcommand prints.
pub fn render_shards(reports: &BTreeMap<ShardId, Vec<(NodeId, Option<ShardReport>)>>) -> String {
    let mut out = String::new();
    for (shard, nodes) in reports {
        let answered: Vec<&ShardReport> = nodes.iter().filter_map(|(_, r)| r.as_ref()).collect();
        let leader = answered.iter().find_map(|r| r.leader);
        let lead = leader.and_then(|l| answered.iter().find(|r| r.node == l));
        out.push_str(&format!(
            "shard {shard}: leader {}, {} of {} nodes answer{}\n",
            leader.map_or("NONE".to_string(), |l| l.to_string()),
            answered.len(),
            nodes.len(),
            if answered.iter().any(|r| r.draining) {
                ", DRAINING"
            } else {
                ""
            },
        ));
        if let Some(r) = lead {
            out.push_str(&format!(
                "  incarnation {}, applied {}, shipped {}{}, snapshot {}s ago\n",
                r.incarnation,
                r.applied,
                r.shipped.map_or("-".to_string(), |s| s.to_string()),
                r.unshipped_ms
                    .map_or(String::new(), |ms| format!(" (oldest unshipped {ms} ms)")),
                r.last_snapshot_s,
            ));
            out.push_str(&format!(
                "  {} sessions, {} turns running, {} inputs queued, {} feed records unacknowledged\n",
                r.sessions, r.active_turns, r.queued_inputs, r.feed_pending
            ));
            if r.moves_total > 0 {
                out.push_str(&format!("  {} sessions mid-move\n", r.moves_total));
            }
        }
        for (node, report) in nodes {
            match report {
                Some(r) => out.push_str(&format!(
                    "  node {node}: applied {} ({} behind), {} MB ({} MB events, {} MB log)\n",
                    r.applied,
                    lead.map_or(0, |l| l.applied.saturating_sub(r.applied)),
                    r.rss_mb,
                    r.resident_mb,
                    r.log_mb
                )),
                None => out.push_str(&format!("  node {node}: NO ANSWER\n")),
            }
        }
    }
    out
}

/// The text the `moves` subcommand prints.
pub fn render_moves(
    reports: &BTreeMap<ShardId, Vec<(NodeId, Option<ShardReport>)>>,
    placed: &std::collections::HashMap<SessionId, Option<Placed>>,
) -> String {
    let mut out = String::new();
    for (shard, nodes) in reports {
        let Some(r) = nodes
            .iter()
            .filter_map(|(_, r)| r.as_ref())
            .find(|r| r.leader == Some(r.node))
        else {
            continue;
        };
        for m in &r.moves {
            out.push_str(&format!(
                "{} in shard {shard}: {:?} at generation {}, to {}; directory says {}\n",
                m.session,
                m.placement,
                m.generation,
                m.moving_to.map_or("-".to_string(), |c| c.to_string()),
                match placed.get(&m.session).copied().flatten() {
                    Some(p) => format!("shard {} at generation {}", p.shard, p.generation),
                    None => "nothing".to_string(),
                }
            ));
        }
        if r.moves_total > r.moves.len() {
            out.push_str(&format!(
                "... and {} more in shard {shard}\n",
                r.moves_total - r.moves.len()
            ));
        }
    }
    if out.is_empty() {
        out.push_str("no session is mid-move\n");
    }
    out
}

/// The text the `session` subcommand prints.
pub fn render_session(
    session: SessionId,
    placed: Option<Placed>,
    report: Option<&SessionReport>,
) -> String {
    let mut out = format!("session {session}\n");
    match placed {
        Some(p) => out.push_str(&format!(
            "  directory: shard {} at generation {}\n",
            p.shard, p.generation
        )),
        None => out.push_str(
            "  directory: not placed (unknown session, or the directory shard did not answer)\n",
        ),
    }
    let Some(r) = report else {
        out.push_str("  no node of that shard holds it\n");
        return out;
    };
    out.push_str(&format!(
        "  held by shard {} (answered by node {}), {:?} at generation {}{}, branch {}\n",
        r.shard,
        r.node,
        r.placement,
        r.generation,
        r.moving_to
            .map_or(String::new(), |c| format!(", moving to shard {c}")),
        r.branch
    ));
    out.push_str(&format!(
        "  agent {}, org {}\n",
        r.agent,
        r.tenant.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "  {} turns done, turn running: {}, {} inputs waiting, last event {}\n",
        r.turns_completed, r.turn_running, r.inbox, r.last_sequence
    ));
    match &r.holder {
        Some(h) => out.push_str(&format!(
            "  claimed by {} on node {}, fence {}, until {}\n",
            h.name, h.node, h.fence, h.expires_ms
        )),
        None => out.push_str("  not claimed\n"),
    }
    let w = &r.work;
    out.push_str(&format!(
        "  owed {}, runnable {}, suspended {}, wake at {}, attempts {}, open turn {}, inbox {} consumed {}, finalizing {} (now {})\n",
        w.owed,
        w.runnable,
        w.suspended,
        w.wake_at.map_or("-".to_string(), |t| t.to_string()),
        w.attempts,
        w.open.as_deref().unwrap_or("-"),
        w.inbox,
        w.consumed,
        w.finalizing,
        w.now_ms
    ));
    out.push_str(&format!(
        "  events 1..={} only in the store ({} segments)\n",
        r.offloaded, r.segments
    ));
    for e in &r.tail {
        out.push_str(&format!(
            "    {:>6} {} {}\n",
            e.sequence.map_or("?".to_string(), |s| s.to_string()),
            e.ts.as_deref().unwrap_or(""),
            e.kind
        ));
    }
    out
}
