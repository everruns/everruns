//! The cell inspector: where a session is and what it is doing, and how each
//! cell is keeping up. An operator's first look in an incident (runbooks:
//! leader loss, a stuck move, a drain).
//!
//! ```text
//! app inspect --topology cluster.json cells
//! app inspect --topology cluster.json session <id>
//! app inspect --topology cluster.json moves
//! ```
//!
//! Decisions:
//! - Read only, and from the nodes' own HTTP endpoints, so it works with no
//!   control plane and against a cell that has no leader (it then says so
//!   instead of failing).
//! - The cell list is the topology file's plus the directory cell's
//!   registry: cells and nodes added later by `--join` are not in the file.
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
use crate::state::{CellId, Placed, Placement};
use crate::types::NodeId;

/// Moves listed per cell, at most.
const MOVES_SHOWN: usize = 50;

/// Events of a session's tail the inspector shows.
const TAIL: usize = 8;

/// One node's view of its cell.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CellReport {
    pub cell: CellId,
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
    pub moves_total: usize,
    pub moves: Vec<MoveView>,
}

/// A session part way through a move.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MoveView {
    pub session: SessionId,
    pub placement: Placement,
    pub generation: u64,
    pub moving_to: Option<CellId>,
}

/// What a session is doing, from the node that answered.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionReport {
    pub cell: CellId,
    pub node: NodeId,
    pub agent: String,
    pub tenant: Option<String>,
    pub placement: Placement,
    pub generation: u64,
    pub moving_to: Option<CellId>,
    pub branch: u64,
    pub last_sequence: i32,
    pub turns_completed: u64,
    pub turn_running: bool,
    pub inbox: usize,
    pub holder: Option<crate::authority::Holder>,
    /// Events only in the object store, and segments holding them.
    pub offloaded: i32,
    pub segments: usize,
    pub tail: Vec<TailEvent>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TailEvent {
    pub sequence: Option<i64>,
    #[serde(rename = "type")]
    pub kind: String,
    pub ts: Option<String>,
}

/// `GET /cell/inspect`.
pub(crate) async fn cell(State(node): State<Arc<Node>>) -> Json<CellReport> {
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
    Json(CellReport {
        cell: node.cfg.cell,
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
        draining: state.draining.contains(&node.cfg.cell),
        sessions: state.sessions.len(),
        active_turns: state.sessions.values().filter(|s| s.turn_running()).count(),
        queued_inputs: state.sessions.values().map(|s| s.queued()).sum(),
        feed_pending: state.feed.records.len(),
        rss_mb: crate::api::rss_mb(),
        moves_total,
        moves,
    })
}

/// `GET /cell/sessions/{session}/inspect`.
pub(crate) async fn session(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<crate::api::Lookup<SessionReport>>, (StatusCode, String)> {
    let id = SessionId::parse(&session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(crate::api::lookup(&node, id, |s| SessionReport {
        cell: node.cfg.cell,
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

    /// Every cell and its nodes: the topology file's, plus what the
    /// directory cell's registry adds (cells that joined later, nodes added
    /// since), less retired cells.
    pub async fn cells(&self) -> BTreeMap<CellId, BTreeMap<NodeId, String>> {
        let registry = self.registry().await;
        self.merged(registry.as_ref())
    }

    /// The directory cell's registry: cells added since the topology file
    /// was written, retired cells, and which cell holds each directory
    /// shard.
    async fn registry(&self) -> Option<crate::api::Registry> {
        for addr in self.topology.nodes(self.topology.directory_cell) {
            if let Some(registry) = self.get(&addr, "/cell/registry").await {
                return Some(registry);
            }
        }
        None
    }

    fn merged(
        &self,
        registry: Option<&crate::api::Registry>,
    ) -> BTreeMap<CellId, BTreeMap<NodeId, String>> {
        let mut cells = self.topology.cells.clone();
        if let Some(registry) = registry {
            for (cell, nodes) in &registry.cells {
                cells.entry(*cell).or_default().extend(nodes.clone());
            }
            for retired in &registry.retired {
                cells.remove(retired);
            }
        }
        cells
    }

    /// Each node's report, `None` for a node that did not answer.
    pub async fn reports(&self) -> BTreeMap<CellId, Vec<(NodeId, Option<CellReport>)>> {
        let mut out = BTreeMap::new();
        for (cell, nodes) in self.cells().await {
            let mut reports = Vec::new();
            for (node, addr) in nodes {
                reports.push((node, self.get(&addr, "/cell/inspect").await));
            }
            out.insert(cell, reports);
        }
        out
    }

    /// Where the directory places `session`.
    pub async fn placed(&self, session: SessionId) -> Option<Placed> {
        // The directory is sharded across cells: ask the cell that holds
        // this session's shard (the directory cell until shards spread).
        let registry = self.registry().await;
        let owner = registry
            .as_ref()
            .and_then(|r| r.shards.get(&crate::state::shard_of(session)).copied())
            .unwrap_or(self.topology.directory_cell);
        let cells = self.merged(registry.as_ref());
        for addr in cells.get(&owner)?.values() {
            if let Some(placed) = self
                .get::<Option<Placed>>(addr, &format!("/cell/directory/{session}"))
                .await
            {
                return placed;
            }
        }
        None
    }

    /// The session as its cell's leader (else any node) sees it.
    pub async fn session(&self, cell: CellId, session: SessionId) -> Option<SessionReport> {
        let cells = self.cells().await;
        let nodes = cells.get(&cell)?;
        let mut answers = Vec::new();
        for addr in nodes.values() {
            if let Some(crate::api::Lookup::Here(report)) = self
                .get::<crate::api::Lookup<SessionReport>>(
                    addr,
                    &format!("/cell/sessions/{session}/inspect"),
                )
                .await
            {
                answers.push(report);
            }
        }
        let leader = self
            .reports()
            .await
            .get(&cell)
            .and_then(|nodes| nodes.iter().find_map(|(_, r)| r.as_ref()?.leader));
        let pick = answers
            .iter()
            .position(|r| Some(r.node) == leader)
            .unwrap_or(0);
        (!answers.is_empty()).then(|| answers.swap_remove(pick))
    }
}

/// The text the `cells` subcommand prints.
pub fn render_cells(reports: &BTreeMap<CellId, Vec<(NodeId, Option<CellReport>)>>) -> String {
    let mut out = String::new();
    for (cell, nodes) in reports {
        let answered: Vec<&CellReport> = nodes.iter().filter_map(|(_, r)| r.as_ref()).collect();
        let leader = answered.iter().find_map(|r| r.leader);
        let lead = leader.and_then(|l| answered.iter().find(|r| r.node == l));
        out.push_str(&format!(
            "cell {cell}: leader {}, {} of {} nodes answer{}\n",
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
                    "  node {node}: applied {} ({} behind), {} MB\n",
                    r.applied,
                    lead.map_or(0, |l| l.applied.saturating_sub(r.applied)),
                    r.rss_mb
                )),
                None => out.push_str(&format!("  node {node}: NO ANSWER\n")),
            }
        }
    }
    out
}

/// The text the `moves` subcommand prints.
pub fn render_moves(
    reports: &BTreeMap<CellId, Vec<(NodeId, Option<CellReport>)>>,
    placed: &std::collections::HashMap<SessionId, Option<Placed>>,
) -> String {
    let mut out = String::new();
    for (cell, nodes) in reports {
        let Some(r) = nodes
            .iter()
            .filter_map(|(_, r)| r.as_ref())
            .find(|r| r.leader == Some(r.node))
        else {
            continue;
        };
        for m in &r.moves {
            out.push_str(&format!(
                "{} in cell {cell}: {:?} at generation {}, to {}; directory says {}\n",
                m.session,
                m.placement,
                m.generation,
                m.moving_to.map_or("-".to_string(), |c| c.to_string()),
                match placed.get(&m.session).copied().flatten() {
                    Some(p) => format!("cell {} at generation {}", p.cell, p.generation),
                    None => "nothing".to_string(),
                }
            ));
        }
        if r.moves_total > r.moves.len() {
            out.push_str(&format!(
                "... and {} more in cell {cell}\n",
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
            "  directory: cell {} at generation {}\n",
            p.cell, p.generation
        )),
        None => out.push_str(
            "  directory: not placed (unknown session, or the directory cell did not answer)\n",
        ),
    }
    let Some(r) = report else {
        out.push_str("  no node of that cell holds it\n");
        return out;
    };
    out.push_str(&format!(
        "  held by cell {} (answered by node {}), {:?} at generation {}{}, branch {}\n",
        r.cell,
        r.node,
        r.placement,
        r.generation,
        r.moving_to
            .map_or(String::new(), |c| format!(", moving to cell {c}")),
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
