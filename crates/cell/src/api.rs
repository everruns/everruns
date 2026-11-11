//! A node's HTTP surface: Raft RPC, the internal cell API the gateway uses,
//! and a per-node event stream.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response as HttpResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use futures::Stream;
use openraft::raft::{AppendEntriesRequest, InstallSnapshotRequest, VoteRequest};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::node::{Node, Proposed};
use crate::state::{CellId, Change, Command, Placement, Response, SessionState};
use crate::types::{NodeId, TypeConfig};

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/raft/append", post(raft_append))
        .route("/raft/vote", post(raft_vote))
        .route("/raft/snapshot", post(raft_snapshot))
        .route("/cell/propose", post(propose))
        .route("/cell/activate/{session}", post(activate))
        .route("/cell/stats", get(stats))
        .route("/metrics", get(metrics))
        .route("/cell/metrics/reset", post(reset_metrics))
        .route("/cell/snapshot", post(trigger_snapshot))
        .route("/cell/registry", get(registry))
        .route("/cell/load", get(load))
        .route("/cell/tools", get(tools))
        .route("/cell/movable", get(movable))
        .route("/cell/members", get(members).post(add_member))
        .route("/cell/members/{node}", axum::routing::delete(remove_member))
        .route("/cell/directory/{session}", get(directory))
        .route("/cell/tenants/{tenant}/grants", post(grant))
        .route("/cell/allowances", get(allowances))
        .route("/cell/shards/{shard}/handoff", post(handoff_shard))
        .route("/cell/sessions/{session}", get(session))
        .route("/cell/sessions/{session}/events", get(events))
        .route("/cell/sessions/{session}/agent", get(agent))
        .route("/cell/sessions/{session}/image", get(image))
        .route("/cell/sessions/{session}/stream", get(stream))
        .route("/cell/sessions/{session}/wait", get(wait))
        .route("/cell/sessions/{session}/create", post(create_session))
        .route("/cell/sessions/{session}/messages", post(send_message))
        .route("/cell/sessions/{session}/cancel", post(cancel))
        // Raft batches and session images can be large; the cell is a
        // trusted network.
        .layer(axum::extract::DefaultBodyLimit::disable())
        .with_state(node)
}

async fn raft_append(
    State(node): State<Arc<Node>>,
    Json(rpc): Json<AppendEntriesRequest<TypeConfig>>,
) -> impl IntoResponse {
    Json(node.raft.append_entries(rpc).await)
}

async fn raft_vote(
    State(node): State<Arc<Node>>,
    Json(rpc): Json<VoteRequest<NodeId>>,
) -> impl IntoResponse {
    Json(node.raft.vote(rpc).await)
}

async fn raft_snapshot(
    State(node): State<Arc<Node>>,
    headers: axum::http::HeaderMap,
    data: axum::body::Bytes,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let chunk: crate::network::SnapshotChunk = headers
        .get(crate::network::SNAPSHOT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| serde_json::from_str(value).ok())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "missing snapshot chunk header".into(),
            )
        })?;
    let rpc = InstallSnapshotRequest::<TypeConfig> {
        vote: chunk.vote,
        meta: chunk.meta,
        offset: chunk.offset,
        data: data.to_vec(),
        done: chunk.done,
    };
    Ok(Json(node.raft.install_snapshot(rpc).await))
}

async fn propose(
    State(node): State<Arc<Node>>,
    Json(command): Json<Command>,
) -> Result<Json<Proposed>, (StatusCode, String)> {
    let (response, index) = node
        .propose(command)
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    Ok(Json(Proposed { response, index }))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Stats {
    pub cell: CellId,
    pub node: NodeId,
    pub leader: Option<NodeId>,
    pub applied: u64,
    pub sessions: usize,
    pub active_turns: usize,
    pub running_here: usize,
    pub queued_inputs: usize,
    pub turns_completed: u64,
    /// Sessions whose events are all in the object store, none in memory.
    #[serde(default)]
    pub unloaded_sessions: usize,
    #[serde(default)]
    pub resident_events: usize,
    #[serde(default)]
    pub resident_mb: u64,
    #[serde(default)]
    pub rss_mb: u64,
    pub wal: crate::wal::WalStats,
    /// Raft log held in memory (not yet purged by a snapshot).
    #[serde(default)]
    pub wal_held_mb: u64,
    /// Feed records the control plane has not acknowledged yet.
    #[serde(default)]
    pub feed_pending: usize,
    pub metrics: crate::metrics::MetricsReport,
}

/// This process's resident set size, from `/proc` (0 elsewhere).
fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages * 4096 / (1024 * 1024))
        .unwrap_or(0)
}

async fn stats(State(node): State<Arc<Node>>) -> Json<Stats> {
    let (sessions, active_turns, queued_inputs, turns_completed, unloaded_sessions) = {
        let state = node.shared.state.read();
        (
            state.sessions.len(),
            state.sessions.values().filter(|s| s.turn_running()).count(),
            state.sessions.values().map(|s| s.queued()).sum(),
            state.sessions.values().map(|s| s.turns_completed()).sum(),
            state
                .sessions
                .values()
                .filter(|s| s.events.is_empty() && s.offloaded > 0)
                .count(),
        )
    };
    let (resident_bytes, resident_events) = node.shared.resident();
    Json(Stats {
        cell: node.cfg.cell,
        node: node.cfg.node_id,
        leader: node.leader(),
        applied: *node.shared.applied.borrow(),
        sessions,
        active_turns,
        running_here: node.running_count(),
        queued_inputs,
        turns_completed,
        unloaded_sessions,
        resident_events,
        resident_mb: resident_bytes / (1024 * 1024),
        rss_mb: rss_mb(),
        wal: node.wal.stats(),
        wal_held_mb: node.wal.held_bytes() >> 20,
        feed_pending: node.shared.state.read().feed.records.len(),
        metrics: node.metrics.report(),
    })
}

/// `GET /metrics`: Prometheus text format. See `prom.rs` for which series
/// are per node and which per cell.
async fn metrics(State(node): State<Arc<Node>>) -> impl IntoResponse {
    use crate::prom::{Writer, render};
    let cell = node.cfg.cell.to_string();
    let id = node.cfg.node_id.to_string();
    let mut w = Writer::new(&[("cell", &cell), ("node", &id)]);
    let p = &node.prom;

    // Per node: what this process did.
    w.counters(
        "cell_turns_executed_total",
        "Turn executions on this node, by agent and outcome.",
        &p.turns_executed,
    );
    w.histograms(
        "cell_turn_duration_seconds",
        "Wall time of turn executions on this node, by agent.",
        &p.turn_seconds,
    );
    w.gauges(
        "cell_turns_running",
        "Turns executing on this node now, by agent.",
        &p.turns_running,
    );
    w.counters(
        "cell_events_appended_total",
        "Durable events appended by this node's executors, by type.",
        &p.events_appended,
    );
    w.counters(
        "cell_model_calls_total",
        "Model calls made by this node's turns, by model and success.",
        &p.model_calls,
    );
    w.histograms(
        "cell_model_call_duration_seconds",
        "Model call duration, by model.",
        &p.model_seconds,
    );
    w.histograms(
        "cell_model_ttft_seconds",
        "Model time to first token, by model.",
        &p.model_ttft_seconds,
    );
    w.counters(
        "cell_tool_calls_total",
        "Tool calls completed on this node, by tool and status.",
        &p.tool_calls,
    );
    w.histograms(
        "cell_tool_call_duration_seconds",
        "Tool call duration, by tool.",
        &p.tool_seconds,
    );
    w.histograms(
        "cell_append_duration_seconds",
        "Executor append: propose, quorum commit, apply.",
        &p.append_seconds,
    );
    w.histograms(
        "cell_propose_duration_seconds",
        "Proposals from this node, end to end.",
        &p.propose_seconds,
    );
    if let Some(gateway) = node.gateway.get() {
        w.counters(
            "cell_messages_admitted_total",
            "Messages admitted through this node's gateway, by status.",
            &gateway.admitted,
        );
        w.counters(
            "cell_sessions_created_total",
            "Sessions created through this node's gateway, by agent.",
            &gateway.created,
        );
        w.counters(
            "cell_rebalance_moves_total",
            "Moves the rebalancer started from this node, by kind (busy, idle, drain, drain-idle).",
            &gateway.rebalanced,
        );
    }
    let leader = node.leader();
    w.gauge(
        "cell_is_leader",
        "1 if this node leads its cell.",
        f64::from(u8::from(leader == Some(node.cfg.node_id))),
    );
    w.gauge(
        "cell_cell_members",
        "Members of this node's cell, as this node knows them.",
        node.members().len() as f64,
    );
    let raft = node.raft.metrics().borrow().clone();
    w.gauge(
        "cell_raft_term",
        "Current Raft term.",
        raft.current_term as f64,
    );
    w.gauge(
        "cell_raft_applied_index",
        "Last log index applied here.",
        raft.last_applied.map(|l| l.index).unwrap_or(0) as f64,
    );
    let (resident_bytes, resident_events) = node.shared.resident();
    w.gauge(
        "cell_resident_events",
        "Session events held in memory.",
        resident_events as f64,
    );
    w.gauge(
        "cell_resident_bytes",
        "Estimated bytes of session events held in memory.",
        resident_bytes as f64,
    );
    w.gauge(
        "cell_wal_held_bytes",
        "Raft log bytes held in memory (not yet purged).",
        node.wal.held_bytes() as f64,
    );
    w.gauge(
        "cell_memory_budget_bytes",
        "Memory budget for resident events.",
        node.cfg.memory_budget as f64,
    );
    w.gauge(
        "cell_rss_bytes",
        "Process resident set size.",
        (rss_mb() << 20) as f64,
    );
    let wal = node.wal.stats();
    w.counter("cell_wal_fsyncs_total", "WAL fsyncs.", wal.fsyncs as f64);
    w.counter(
        "cell_wal_entries_total",
        "Raft entries written to the WAL.",
        wal.entries as f64,
    );
    w.counter(
        "cell_unloaded_events_total",
        "Events dropped from memory by unloading idle sessions.",
        node.metrics.report().unloaded_events as f64,
    );

    // Per cell, from the replicated state: only the leader reports, so a
    // sum over the fleet counts each cell once.
    if leader == Some(node.cfg.node_id) {
        let base = render(&[("cell", &cell)]);
        let mut by_agent: std::collections::BTreeMap<&str, [u64; 5]> = Default::default();
        let (mut unloaded, mut frozen) = (0u64, 0u64);
        let state = node.shared.state.read();
        for s in state.sessions.values() {
            let row = by_agent.entry(s.agent.as_str()).or_default();
            row[0] += 1;
            row[1] += u64::from(s.turn_running());
            row[2] += s.queued() as u64;
            row[3] += s.turns_completed();
            row[4] += u64::from(s.turn_running() && s.authority.attempts > 1);
            unloaded += u64::from(s.events.is_empty() && s.offloaded > 0);
            frozen += u64::from(s.placement == Placement::Frozen);
        }
        let rows = |i: usize| {
            by_agent
                .iter()
                .map(move |(agent, row)| (render(&[("agent", agent)]), row[i] as f64))
                .collect::<Vec<_>>()
        };
        w.family(
            "cell_sessions",
            "gauge",
            "Sessions in the cell, by agent.",
            &base,
            rows(0),
        );
        w.family(
            "cell_active_turns",
            "gauge",
            "Sessions with a turn claimed (running or about to), by agent.",
            &base,
            rows(1),
        );
        w.family(
            "cell_queued_inputs",
            "gauge",
            "Messages waiting for a turn, by agent.",
            &base,
            rows(2),
        );
        w.family(
            "cell_turns_completed_total",
            "counter",
            "Turns completed in the cell's sessions, by agent.",
            &base,
            rows(3),
        );
        w.family(
            "cell_resumed_turns",
            "gauge",
            "Active turns on a second or later attempt (resumed after a failure).",
            &base,
            rows(4),
        );
        w.sample_raw(
            "cell_unloaded_sessions",
            "gauge",
            "Sessions whose events are only in the object store (leader's view).",
            &base,
            unloaded as f64,
        );
        w.sample_raw(
            "cell_frozen_sessions",
            "gauge",
            "Sessions frozen for a move.",
            &base,
            frozen as f64,
        );
        w.sample_raw(
            "cell_forwarded_sessions",
            "gauge",
            "Sessions this cell handed to another cell.",
            &base,
            state.forwarded.len() as f64,
        );
    }
    ([("content-type", "text/plain; version=0.0.4")], w.out)
}

async fn reset_metrics(State(node): State<Arc<Node>>) -> StatusCode {
    node.metrics.reset();
    StatusCode::NO_CONTENT
}

async fn trigger_snapshot(State(node): State<Arc<Node>>) -> StatusCode {
    match node.raft.trigger().snapshot().await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// How busy this cell is, for placement and rebalancing. Every replica
/// holds the same replicated state, so any node answers for the cell.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct CellLoad {
    pub sessions: u64,
    /// Turns claimed (running, or parked mid-move).
    pub active: u64,
    /// Of `active`, turns that wrote an event in the last
    /// [`WAITING_AFTER`]: stepping, not waiting.
    #[serde(default)]
    pub busy: u64,
    pub queued: u64,
    /// Sessions frozen for a move out of this cell.
    pub moving: u64,
}

/// A running turn that wrote nothing for this long is waiting (on a slow
/// tool, a person, another session), not working.
pub const WAITING_AFTER: chrono::Duration = chrono::Duration::seconds(30);

impl CellLoad {
    /// What the cell's turns cost, in turn slots.
    ///
    /// Decision: a turn's cost is what it takes from the nodes now, not
    /// that it exists. Turns may run for days, mostly waiting; one that
    /// wrote nothing for [`WAITING_AFTER`] holds a slot but little else, so
    /// it weighs a tenth of a turn that is stepping. Queued input is work
    /// that will start as soon as it can.
    pub fn work(&self) -> f64 {
        let waiting = self.active.saturating_sub(self.busy);
        (self.busy + self.queued) as f64 + waiting as f64 * 0.1
    }

    /// Work, with idle sessions weighing a little as future work.
    pub fn score(&self) -> f64 {
        self.work() + self.sessions as f64 * 0.01
    }
}

pub fn cell_load(state: &crate::state::CellState) -> CellLoad {
    let mut load = CellLoad::default();
    let since = chrono::Utc::now() - WAITING_AFTER;
    for s in state.sessions.values() {
        load.sessions += 1;
        if s.turn_running() {
            load.active += 1;
            load.busy += u64::from(s.events.last().is_none_or(|e| e.ts > since));
        }
        load.queued += s.queued() as u64;
        load.moving += u64::from(s.moving_to.is_some());
    }
    load
}

async fn tools(State(node): State<Arc<Node>>) -> Json<Vec<serde_json::Value>> {
    Json(node.host.tools())
}

async fn load(State(node): State<Arc<Node>>) -> Json<CellLoad> {
    Json(cell_load(&node.shared.state.read()))
}

#[derive(Deserialize)]
struct Movable {
    n: usize,
    /// `busy`: sessions with a running turn; `idle`: sessions without one.
    kind: String,
}

/// Up to `n` sessions a rebalancer may move: live, not already moving.
/// Busy ones come longest-running first, measured by the events their
/// current turn has written.
async fn movable(State(node): State<Arc<Node>>, Query(q): Query<Movable>) -> Json<Vec<SessionId>> {
    let since = chrono::Utc::now() - WAITING_AFTER;
    let state = node.shared.state.read();
    let mut found: Vec<((bool, i64), SessionId)> = state
        .sessions
        .iter()
        .filter(|(_, s)| s.placement == Placement::Live && s.moving_to.is_none())
        .filter_map(|(id, s)| match (s.turn_running(), q.kind.as_str()) {
            (true, "busy") => {
                // Stepping turns first (moving those moves cost), each
                // group longest-running first.
                let progress = s.last_sequence().unwrap_or(0) - s.turn_start;
                let waiting = s.events.last().is_some_and(|e| e.ts <= since);
                Some(((waiting, -(progress as i64)), *id))
            }
            (false, "idle") if s.queued() == 0 && !s.authority.runnable => Some(((false, 0), *id)),
            _ => None,
        })
        .collect();
    found.sort_by_key(|(key, _)| *key);
    Json(found.into_iter().take(q.n).map(|(_, id)| id).collect())
}

/// The fleet's cells as the directory knows them (meaningful on the
/// directory cell).
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Registry {
    pub cells: std::collections::BTreeMap<CellId, std::collections::BTreeMap<NodeId, String>>,
    pub draining: std::collections::BTreeSet<CellId>,
    pub retired: std::collections::BTreeSet<CellId>,
    /// Directory shards held outside the directory cell.
    #[serde(default)]
    pub shards: std::collections::BTreeMap<u16, CellId>,
}

async fn registry(State(node): State<Arc<Node>>) -> Json<Registry> {
    let state = node.shared.state.read();
    Json(Registry {
        cells: state.cells.clone(),
        draining: state.draining.clone(),
        retired: state.retired.clone(),
        shards: state.shards.clone(),
    })
}

async fn members(State(node): State<Arc<Node>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "cell": node.cfg.cell,
        "leader": node.leader(),
        "members": node.members(),
    }))
}

#[derive(Deserialize)]
pub struct NewMember {
    pub node: NodeId,
    pub addr: String,
}

/// Add a node to this cell (leader only; others answer 421 so a caller
/// tries the next node).
async fn add_member(
    State(node): State<Arc<Node>>,
    Json(body): Json<NewMember>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !node.is_leader() {
        return Err((StatusCode::MISDIRECTED_REQUEST, "not the leader".into()));
    }
    node.add_member(body.node, body.addr)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    Ok(Json(serde_json::json!({ "members": node.members() })))
}

async fn remove_member(
    State(node): State<Arc<Node>>,
    Path(id): Path<NodeId>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !node.is_leader() {
        return Err((StatusCode::MISDIRECTED_REQUEST, "not the leader".into()));
    }
    node.remove_member(id)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    Ok(Json(serde_json::json!({ "members": node.members() })))
}

fn parse(session: &str) -> Result<SessionId, (StatusCode, String)> {
    SessionId::parse(session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "found", content = "value", rename_all = "snake_case")]
pub enum Lookup<T> {
    Here(T),
    Moved { cell: CellId },
    Missing,
}

async fn directory(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<Option<crate::state::Placed>>, (StatusCode, String)> {
    let session = parse(&session)?;
    Ok(Json(
        node.shared.state.read().directory.get(&session).copied(),
    ))
}

#[derive(Deserialize)]
struct GrantBody {
    grant_id: String,
    #[serde(flatten)]
    grant: crate::allowance::Grant,
}

/// `POST /cell/tenants/{tenant}/grants`: take a spend allowance grant (the
/// control plane's side of `allowance`). Idempotent by `grant_id`.
async fn grant(
    State(node): State<Arc<Node>>,
    Path(tenant): Path<String>,
    Json(body): Json<GrantBody>,
) -> Result<Json<Response>, (StatusCode, String)> {
    let (response, _) = node
        .propose(Command::Grant {
            tenant,
            grant_id: body.grant_id,
            grant: body.grant,
        })
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    Ok(Json(response))
}

/// `GET /cell/allowances`: every tenant's grants, usage and open
/// reservations, as this replica applied them.
async fn allowances(State(node): State<Arc<Node>>) -> Json<crate::allowance::Allowances> {
    Json(node.shared.state.read().allowances.clone())
}

#[derive(Deserialize)]
struct Handoff {
    to: CellId,
}

/// `POST /cell/shards/{shard}/handoff?to=B`: hand this cell's directory
/// entries of `shard` to cell `B`. Seals the shard first, so no entry
/// changes here while it is copied; `B` keeps any entry it already has
/// (written after the seal). The directory cell then assigns the shard.
async fn handoff_shard(
    State(node): State<Arc<Node>>,
    Path(shard): Path<u16>,
    Query(q): Query<Handoff>,
) -> Result<Json<usize>, (StatusCode, String)> {
    let error = |e: String| (StatusCode::SERVICE_UNAVAILABLE, e);
    let gateway = node
        .gateway
        .get()
        .ok_or_else(|| error("no gateway".into()))?;
    let (_, index) = node
        .propose(Command::SealShard { shard, to: q.to })
        .await
        .map_err(|e| error(e.to_string()))?;
    node.shared.wait_applied(index).await;
    let entries: Vec<(SessionId, crate::state::Placed)> = node
        .shared
        .state
        .read()
        .directory
        .iter()
        .filter(|(session, _)| crate::state::shard_of(**session) == shard)
        .map(|(session, placed)| (*session, *placed))
        .collect();
    let n = entries.len();
    // At least one Adopt, so the new owner takes the shard even when empty.
    for chunk in entries
        .chunks(2000)
        .chain(entries.is_empty().then_some(&[][..]))
    {
        let command = Command::Adopt {
            shard,
            entries: chunk.to_vec(),
        };
        gateway
            .propose(q.to, &command)
            .await
            .map_err(|(_, e)| error(e))?;
    }
    Ok(Json(n))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionView {
    pub id: SessionId,
    pub agent: String,
    pub last_sequence: i32,
    /// Admitted input no turn has consumed yet.
    pub inbox: usize,
    /// The execution epoch holding the session, if any.
    pub active: Option<crate::authority::Holder>,
    pub turns_completed: u64,
    pub placement: Placement,
    /// The org the session is billed to.
    #[serde(default)]
    pub tenant: Option<String>,
}

impl SessionView {
    fn of(id: SessionId, s: &SessionState) -> Self {
        Self {
            id,
            agent: s.agent.clone(),
            last_sequence: s.last_sequence().unwrap_or(0),
            inbox: s.queued(),
            active: s.authority.holder.clone(),
            turns_completed: s.turns_completed(),
            placement: s.placement,
            tenant: s.tenant.clone(),
        }
    }
}

fn lookup<T>(node: &Node, session: SessionId, f: impl FnOnce(&SessionState) -> T) -> Lookup<T> {
    let state = node.shared.state.read();
    match state.sessions.get(&session) {
        Some(s) => Lookup::Here(f(s)),
        None => match state.forwarded.get(&session) {
            Some(left) => Lookup::Moved { cell: left.cell },
            None => Lookup::Missing,
        },
    }
}

async fn session(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<Lookup<SessionView>>, (StatusCode, String)> {
    let id = parse(&session)?;
    Ok(Json(lookup(&node, id, |s| SessionView::of(id, s))))
}

/// The session's agent definition, as its client sent it.
async fn agent(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
) -> Result<Json<Lookup<serde_json::Value>>, (StatusCode, String)> {
    let id = parse(&session)?;
    Ok(Json(lookup(&node, id, |s| (*s.definition).clone())))
}

#[derive(Deserialize)]
pub struct After {
    #[serde(default)]
    pub after: i32,
}

async fn events(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Query(after): Query<After>,
) -> Result<Json<Lookup<Vec<Event>>>, (StatusCode, String)> {
    let id = parse(&session)?;
    match node.shared.read_events(id, after.after.max(0)).await {
        Some(Ok(events)) => Ok(Json(Lookup::Here(events))),
        Some(Err(error)) => Err((StatusCode::SERVICE_UNAVAILABLE, error.to_string())),
        None => Ok(Json(lookup(&node, id, |_| Vec::new()))),
    }
}

#[derive(Deserialize)]
pub struct ImageQuery {
    #[serde(default)]
    pub after: i32,
    #[serde(default)]
    pub stub: bool,
}

/// The session's state; with `after`, only the events after that sequence
/// (an executor already holds the prefix from its own replica); with `stub`,
/// history stays in the object store and only the segment list travels.
async fn image(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Query(query): Query<ImageQuery>,
) -> Result<Json<Lookup<SessionState>>, (StatusCode, String)> {
    let id = parse(&session)?;
    match node.shared.image(id, query.after.max(0), query.stub).await {
        Some(Ok(image)) => Ok(Json(Lookup::Here(image))),
        Some(Err(error)) => Err((StatusCode::SERVICE_UNAVAILABLE, error.to_string())),
        None => Ok(Json(lookup(&node, id, |_| SessionState::default()))),
    }
}

/// One event on a node stream: durable (with a sequence) or ephemeral.
pub fn sse_event(event: &Event) -> SseEvent {
    let data = serde_json::to_string(event).unwrap_or_default();
    let sse = SseEvent::default()
        .event(event.event_type.clone())
        .data(data);
    if event.sequence.is_some() {
        sse.id(event.id.to_string())
    } else {
        sse
    }
}

async fn stream(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Query(after): Query<After>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, String)> {
    let id = parse(&session)?;
    let rx = subscribe(node, id, after.after);
    let stream = ReceiverStream::new(rx).map(|event| Ok(sse_event(&event)));
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

/// Durable events after `after` (replayed, then live from this replica's
/// apply) interleaved with this node's ephemeral events for the session.
pub fn subscribe(node: Arc<Node>, session: SessionId, after: i32) -> mpsc::Receiver<Arc<Event>> {
    let (tx, rx) = mpsc::channel(1024);
    tokio::spawn(async move {
        // Subscribe before replaying, so nothing falls between the two.
        let mut changes = node.shared.watch(session);
        let mut live = node.live.subscribe();
        let mut last = after;
        let replay = async |last: &mut i32| -> Vec<Arc<Event>> {
            let events: Vec<Arc<Event>> = match node.shared.read_events(session, *last).await {
                Some(Ok(events)) => events.into_iter().map(Arc::new).collect(),
                _ => Vec::new(),
            };
            if let Some(seq) = events.last().and_then(|e| e.sequence) {
                *last = seq;
            }
            events
        };
        for event in replay(&mut last).await {
            if tx.send(event).await.is_err() {
                return;
            }
        }
        loop {
            tokio::select! {
                change = changes.recv() => match change {
                    Ok(Change::Event(s, event)) if s == session => {
                        let seq = event.sequence.unwrap_or(0);
                        if seq == last + 1 {
                            last = seq;
                            if tx.send(event).await.is_err() { return; }
                        } else if seq > last + 1 {
                            for event in replay(&mut last).await {
                                if tx.send(event).await.is_err() { return; }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for event in replay(&mut last).await {
                            if tx.send(event).await.is_err() { return; }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                item = live.recv() => match item {
                    Ok((s, event)) if s == session => {
                        if tx.send(event).await.is_err() { return; }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = tx.closed() => return,
            }
        }
    });
    rx
}

pub fn not_found(message: &str) -> HttpResponse {
    (StatusCode::NOT_FOUND, message.to_string()).into_response()
}

#[derive(Deserialize)]
pub struct WaitQuery {
    pub turns: u64,
    #[serde(default = "default_wait_ms")]
    pub timeout_ms: u64,
}

fn default_wait_ms() -> u64 {
    30_000
}

/// Long-poll until the session has completed `turns` turns (or the timeout
/// passes); answers with the session view either way.
async fn wait(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Query(query): Query<WaitQuery>,
) -> Result<Json<Lookup<SessionView>>, (StatusCode, String)> {
    let id = parse(&session)?;
    let mut changes = node.shared.watch(id);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(query.timeout_ms);
    loop {
        let view = lookup(&node, id, |s| SessionView::of(id, s));
        let done = match &view {
            Lookup::Here(v) => v.turns_completed >= query.turns,
            _ => true,
        };
        if done || tokio::time::Instant::now() >= deadline {
            return Ok(Json(view));
        }
        loop {
            match tokio::time::timeout_at(deadline, changes.recv()).await {
                Err(_) => break,
                Ok(Ok(Change::Session(s))) if s == id => break,
                Ok(Ok(_)) => continue,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => break,
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
            }
        }
    }
}

/// The leader asking this node to run Engine's activation of a session.
async fn activate(State(node): State<Arc<Node>>, Path(session): Path<String>) -> StatusCode {
    match parse(&session) {
        Ok(id) => {
            node.activate(id);
            StatusCode::ACCEPTED
        }
        Err((status, _)) => status,
    }
}

/// What a session command through this node's Engine came to.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum CellReply {
    Ok {
        value: serde_json::Value,
    },
    /// The session lives in another cell now.
    Moved {
        cell: CellId,
    },
    /// Frozen for a move: retry shortly.
    Frozen,
    NotFound,
    Refused {
        reason: String,
    },
}

fn reply_of(error: impl std::fmt::Display) -> CellReply {
    let text = error.to_string();
    if let Some(at) = text.find(crate::hosting::MOVED) {
        let rest = &text[at + crate::hosting::MOVED.len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(cell) = digits.parse() {
            return CellReply::Moved { cell };
        }
    }
    if text.contains(crate::hosting::FROZEN) {
        return CellReply::Frozen;
    }
    if text.contains("not found") || text.contains("NotFound") {
        return CellReply::NotFound;
    }
    CellReply::Refused { reason: text }
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub definition: serde_json::Value,
    /// The tenant the session's spend is billed to, if metered.
    #[serde(default)]
    pub tenant: Option<String>,
}

/// Create the session through Engine: its binding pins the definition's key
/// and this host's build; the definition rides along in the same command.
async fn create_session(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<CreateBody>,
) -> Result<Json<CellReply>, (StatusCode, String)> {
    let id = parse(&session)?;
    // Held while creating: Engine resolves the build from the interned copy.
    let definition = crate::state::definitions::intern(body.definition);
    let build = crate::hosting::build_for(&definition, crate::hosting::BUILD_ID);
    let reply = match node.engine.create_registered(id, &build).await {
        Ok(_) => CellReply::Ok {
            value: serde_json::json!({ "id": id }),
        },
        Err(error) => reply_of(error),
    };
    drop(definition);
    // Billed before the reply, so no input reaches the session unmetered.
    if let (CellReply::Ok { .. }, Some(tenant)) = (&reply, body.tenant) {
        let (assigned, _) = node
            .propose(Command::Assign {
                session: id,
                tenant,
            })
            .await
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
        if let Response::Rejected { reason } = assigned {
            return Ok(Json(CellReply::Refused { reason }));
        }
    }
    Ok(Json(reply))
}

#[derive(Deserialize)]
pub struct MessageBody {
    pub text: String,
    pub id: everruns_contracts::typed_id::MessageId,
}

/// Admit a user message through Engine (`Session::send_with_id`): a retry
/// with the same id finds the first admission.
async fn send_message(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<MessageBody>,
) -> Result<Json<CellReply>, (StatusCode, String)> {
    let id = parse(&session)?;
    let handle = match node.engine.resume(id).await {
        Ok(handle) => handle,
        Err(error) => {
            // Engine folds authority errors into `Unavailable`; ask the
            // replica why.
            let reply = match lookup(&node, id, |s| s.placement) {
                Lookup::Moved { cell } => CellReply::Moved { cell },
                Lookup::Missing => CellReply::NotFound,
                Lookup::Here(Placement::Frozen | Placement::Prepared) => CellReply::Frozen,
                Lookup::Here(_) => reply_of(error),
            };
            return Ok(Json(reply));
        }
    };
    let reply = match handle
        .send_with_id(body.id, everruns::InputMessage::user(body.text))
        .await
    {
        Ok(_) => {
            node.prom.messages_admitted.inc(&[("status", "accepted")]);
            CellReply::Ok {
                value: serde_json::json!({ "id": body.id, "session_id": id, "status": "accepted" }),
            }
        }
        Err(error) => reply_of(error),
    };
    Ok(Json(reply))
}

#[derive(Deserialize)]
pub struct CancelBody {
    /// The admitted message whose turn to cancel.
    pub input: everruns_contracts::typed_id::MessageId,
    /// The cancel command's own id (retry-safe).
    pub id: uuid::Uuid,
}

/// Cancel the turn of an admitted message through Engine's command reducer.
async fn cancel(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<CancelBody>,
) -> Result<Json<CellReply>, (StatusCode, String)> {
    use everruns::hosting::SessionHosting as _;
    let id = parse(&session)?;
    let reply = match node.hosting.cancel(id, body.id, body.input).await {
        Ok(()) => CellReply::Ok {
            value: serde_json::json!({ "cancelled": body.input }),
        },
        Err(error) => reply_of(error),
    };
    Ok(Json(reply))
}

#[cfg(test)]
mod load_tests {
    use super::CellLoad;

    #[test]
    fn waiting_turns_weigh_a_tenth_of_stepping_ones() {
        let waiting = CellLoad {
            active: 100,
            busy: 0,
            ..Default::default()
        };
        let stepping = CellLoad {
            active: 10,
            busy: 10,
            ..Default::default()
        };
        assert_eq!(waiting.work(), stepping.work());
        let queued = CellLoad {
            queued: 3,
            ..Default::default()
        };
        assert_eq!(queued.work(), 3.0);
    }
}
