//! A node's HTTP surface: the internal shard API the gateway uses and a
//! per-node event stream. Raft RPC is `network::router`.

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
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::node::{Node, Proposed};
use crate::state::{Change, Command, Placement, Response, SessionState, ShardId};
use crate::types::NodeId;

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/shard/propose", post(propose))
        .route("/shard/activate/{session}", post(activate))
        .route("/shard/stats", get(stats))
        .route("/shard/usage", get(usage))
        .route("/shard/inspect", get(crate::inspect::shard))
        .route(
            "/shard/sessions/{session}/inspect",
            get(crate::inspect::session),
        )
        .route(
            "/shard/sessions/{session}/sandbox",
            get(crate::inspect::sandbox),
        )
        .route("/metrics", get(metrics))
        .route("/shard/metrics/reset", post(reset_metrics))
        .route("/shard/snapshot", post(trigger_snapshot))
        .route("/shard/registry", get(registry))
        .route("/shard/load", get(load))
        .route("/shard/pressure", get(crate::pressure::reading))
        .route("/shard/tools", get(tools))
        .route("/shard/movable", get(movable))
        .route("/shard/members", get(members).post(add_member))
        .route(
            "/shard/members/{node}",
            axum::routing::delete(remove_member),
        )
        .route("/shard/directory/{session}", get(directory))
        .route("/shard/tenants/{tenant}/grants", post(grant))
        .route("/shard/allowances", get(allowances))
        .route("/shard/slots/{slot}/handoff", post(handoff_slot))
        .route("/shard/sessions/{session}", get(session))
        .route("/shard/sessions/{session}/events", get(events))
        .route("/shard/sessions/{session}/agent", get(agent))
        .route("/shard/sessions/{session}/image", get(image))
        .route("/shard/sessions/{session}/stream", get(stream))
        .route("/shard/sessions/{session}/wait", get(wait))
        .route("/shard/sessions/{session}/create", post(create_session))
        .route("/shard/sessions/{session}/messages", post(send_message))
        .route("/shard/sessions/{session}/cancel", post(cancel))
        // Raft batches and session images can be large; the shard is a
        // trusted network.
        .layer(axum::extract::DefaultBodyLimit::disable())
        .with_state(node.clone())
        .merge(crate::event_previews::router(node))
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
    pub shard: ShardId,
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
pub(crate) fn rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages * 4096 / (1024 * 1024))
        .unwrap_or(0)
}

/// One node's resources and latencies, for its leader's minute report
/// (`GET /shard/usage`, `node_control::report_minute`).
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct NodeUsage {
    pub rss_bytes: u64,
    /// The container's memory limit; 0 when there is none or it is unknown.
    pub memory_limit_bytes: u64,
    /// CPU used since the previous reading, in cores.
    pub cpu_cores: f32,
    /// The container's CPU limit in cores; 0 when unknown.
    pub cpu_limit_cores: f32,
    pub admit_p99_ms: u32,
    pub ship_lag_p99_ms: u32,
}

pub(crate) fn node_usage(node: &Node) -> NodeUsage {
    let (cpu_cores, cpu_limit_cores) = cpu_usage();
    NodeUsage {
        rss_bytes: rss_mb() << 20,
        memory_limit_bytes: memory_limit_bytes(),
        cpu_cores,
        cpu_limit_cores,
        admit_p99_ms: node
            .gateway
            .get()
            .map_or(0, |g| (g.admit_window.p99() * 1000.0).round() as u32),
        ship_lag_p99_ms: (node.prom.ship_lag_window.p99() * 1000.0).round() as u32,
    }
}

async fn usage(State(node): State<Arc<Node>>) -> Json<NodeUsage> {
    Json(node_usage(&node))
}

/// The cgroup v2 memory limit (`memory.max`); 0 for `max` or outside one.
fn memory_limit_bytes() -> u64 {
    std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// CPU used since the reading before, and the cgroup v2 limit
/// (`cpu.max`), both in cores; `(0, 0)` outside Linux.
///
/// Decision: the rate is measured from a baseline that moves only once it
/// is 10 seconds old, so a reader polling often (the inspector) still sees
/// a rate over a useful span rather than over its own short interval.
/// Clock ticks are taken as 100 a second, Linux's `USER_HZ` everywhere it
/// matters here.
fn cpu_usage() -> (f32, f32) {
    static BASELINE: parking_lot::Mutex<Option<(std::time::Instant, u64, f32)>> =
        parking_lot::Mutex::new(None);
    let limit = std::fs::read_to_string("/sys/fs/cgroup/cpu.max")
        .ok()
        .and_then(|s| {
            let mut parts = s.split_whitespace();
            let quota: f32 = parts.next()?.parse().ok()?;
            let period: f32 = parts.next()?.parse().ok()?;
            (period > 0.0).then_some(quota / period)
        })
        .unwrap_or(0.0);
    let Some(ticks) = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|s| {
            // Fields after the command name, which may hold spaces: utime and
            // stime are the 12th and 13th.
            let rest = &s[s.rfind(')')? + 2..];
            let fields: Vec<&str> = rest.split_whitespace().collect();
            Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
        })
    else {
        return (0.0, limit);
    };
    let now = std::time::Instant::now();
    let mut baseline = BASELINE.lock();
    let cores = match *baseline {
        Some((at, before, last)) => {
            let secs = now.duration_since(at).as_secs_f32();
            let rate = if secs > 0.0 {
                ticks.saturating_sub(before) as f32 / 100.0 / secs
            } else {
                last
            };
            if secs >= 10.0 {
                *baseline = Some((now, ticks, rate));
            }
            rate
        }
        None => {
            *baseline = Some((now, ticks, 0.0));
            0.0
        }
    };
    ((cores * 100.0).round() / 100.0, limit)
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
        shard: node.cfg.shard,
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
/// are per node and which per shard.
async fn metrics(State(node): State<Arc<Node>>) -> impl IntoResponse {
    use crate::prom::{Writer, render};
    let shard = node.cfg.shard.to_string();
    let id = node.cfg.node_id.to_string();
    let mut w = Writer::new(&[("shard", &shard), ("node", &id)]);
    let p = &node.prom;

    // Per node: what this process did.
    w.counters(
        "agentd_shard_turns_executed_total",
        "Turn executions on this node, by outcome.",
        &p.turns_executed,
    );
    w.histograms(
        "agentd_shard_turn_duration_seconds",
        "Wall time of turn executions on this node.",
        &p.turn_seconds,
    );
    w.gauges(
        "agentd_shard_turns_running",
        "Turns executing on this node now.",
        &p.turns_running,
    );
    w.counters(
        "agentd_shard_events_appended_total",
        "Durable events appended by this node's executors, by type.",
        &p.events_appended,
    );
    w.counters(
        "agentd_shard_model_calls_total",
        "Model calls made by this node's turns, by model and success.",
        &p.model_calls,
    );
    w.histograms(
        "agentd_shard_model_call_duration_seconds",
        "Model call duration, by model.",
        &p.model_seconds,
    );
    w.histograms(
        "agentd_shard_model_ttft_seconds",
        "Model time to first token, by model.",
        &p.model_ttft_seconds,
    );
    w.counters(
        "agentd_shard_tool_calls_total",
        "Tool calls completed on this node, by tool and status.",
        &p.tool_calls,
    );
    w.histograms(
        "agentd_shard_tool_call_duration_seconds",
        "Tool call duration, by tool.",
        &p.tool_seconds,
    );
    w.histograms(
        "agentd_shard_append_duration_seconds",
        "Executor append: propose, quorum commit, apply.",
        &p.append_seconds,
    );
    w.histograms(
        "agentd_shard_propose_duration_seconds",
        "Proposals from this node, end to end.",
        &p.propose_seconds,
    );
    w.histograms(
        "agentd_shard_ship_first_duration_seconds",
        "Writes shipped to the object store before they were answered.",
        &p.ship_first_seconds,
    );
    w.histograms(
        "agentd_shard_ship_lag_seconds",
        "How long the oldest unshipped entry waited, per shipping round (upper bound).",
        &p.ship_lag_seconds,
    );
    w.counters(
        "agentd_shard_ship_first_total",
        "Ship-first writes by path: replicas (every voter held them) or store (shipped first).",
        &p.ship_first_total,
    );
    let puts = crate::prom::Counters::default();
    for (kind, n) in crate::store::puts() {
        puts.add(&[("kind", kind)], n);
    }
    w.counters(
        "agentd_store_puts_total",
        "Object store writes from this process, by kind, retries included.",
        &puts,
    );
    w.counters(
        "agentd_shard_schedules_fired_total",
        "Schedules this node fired as its shard's leader.",
        &p.schedules_fired,
    );
    w.histograms(
        "agentd_shard_schedule_fire_lag_seconds",
        "From a schedule's time to its occurrence committed.",
        &p.schedule_lag_seconds,
    );
    if let Some(gateway) = node.gateway.get() {
        w.counters(
            "agentd_shard_messages_admitted_total",
            "Messages admitted through this node's gateway, by status.",
            &gateway.admitted,
        );
        w.counters(
            "agentd_shard_sessions_created_total",
            "Sessions created through this node's gateway.",
            &gateway.created,
        );
        w.counters(
            "agentd_shard_rebalance_moves_total",
            "Moves the rebalancer started from this node, by kind (busy, idle, drain, drain-idle).",
            &gateway.rebalanced,
        );
    }
    // One-minute p99s, the SLO dashboard's gauges (`prom::Window`).
    w.gauge(
        "agentd_shard_slo_turn_p99_seconds",
        "p99 turn time this shard finished in the last minute, turn.started to its end.",
        node.shared.turn_window.p99(),
    );
    w.gauge(
        "agentd_shard_slo_ship_lag_p99_seconds",
        "p99 shipping lag in the last minute (budget 15 s).",
        p.ship_lag_window.p99(),
    );
    w.gauge(
        "agentd_shard_slo_ship_first_p99_seconds",
        "p99 wait for the object store before answering, in the last minute.",
        p.ship_first_window.p99(),
    );
    if let Some(gateway) = node.gateway.get() {
        w.gauge(
            "agentd_shard_slo_admit_p99_seconds",
            "p99 admission through this node's gateway in the last minute.",
            gateway.admit_window.p99(),
        );
    }
    let leader = node.leader();
    w.gauge(
        "agentd_shard_is_leader",
        "1 if this node leads its shard.",
        f64::from(u8::from(leader == Some(node.cfg.node_id))),
    );
    w.gauge(
        "agentd_shard_members",
        "Members of this node's shard, as this node knows them.",
        node.members().len() as f64,
    );
    let raft = node.raft.metrics().borrow().clone();
    w.gauge(
        "agentd_shard_raft_term",
        "Current Raft term.",
        raft.current_term as f64,
    );
    w.gauge(
        "agentd_shard_raft_applied_index",
        "Last log index applied here.",
        raft.last_applied.map(|l| l.index).unwrap_or(0) as f64,
    );
    let (resident_bytes, resident_events) = node.shared.resident();
    w.gauge(
        "agentd_shard_resident_events",
        "Session events held in memory.",
        resident_events as f64,
    );
    w.gauge(
        "agentd_shard_resident_bytes",
        "Estimated bytes of session events held in memory.",
        resident_bytes as f64,
    );
    w.gauge(
        "agentd_shard_wal_held_bytes",
        "Raft log bytes held in memory (not yet purged).",
        node.wal.held_bytes() as f64,
    );
    w.gauge(
        "agentd_shard_memory_budget_bytes",
        "Memory budget for resident events.",
        node.cfg.memory_budget as f64,
    );
    w.gauge(
        "agentd_shard_rss_bytes",
        "Process resident set size.",
        (rss_mb() << 20) as f64,
    );
    crate::pressure::write_metrics(&node.pressure, &mut w);
    w.gauge(
        "agentd_shard_catching_up",
        "1 while this follower is far behind its leader and steps aside (no elections, no session requests).",
        f64::from(u8::from(node.catchup.behind())),
    );
    let wal = node.wal.stats();
    w.counter(
        "agentd_shard_wal_fsyncs_total",
        "WAL fsyncs.",
        wal.fsyncs as f64,
    );
    w.counter(
        "agentd_shard_wal_entries_total",
        "Raft entries written to the WAL.",
        wal.entries as f64,
    );
    w.counter(
        "agentd_shard_unloaded_events_total",
        "Events dropped from memory by unloading idle sessions.",
        node.metrics.report().unloaded_events as f64,
    );

    // Per shard, from the replicated state: only the leader reports, so a
    // sum over the fleet counts each shard once.
    if leader == Some(node.cfg.node_id) {
        let base = render(&[("shard", &shard)]);
        // Totals only: an agent label would grow with the fleet (see
        // `node::Running`).
        let mut row = [0u64; 5];
        let (mut unloaded, mut frozen) = (0u64, 0u64);
        let state = node.shared.state.read();
        for s in state.sessions.values() {
            row[0] += 1;
            row[1] += u64::from(s.turn_running());
            row[2] += s.queued() as u64;
            row[3] += s.turns_completed();
            row[4] += u64::from(s.turn_running() && s.authority.attempts > 1);
            unloaded += u64::from(s.events.is_empty() && s.offloaded > 0);
            frozen += u64::from(s.placement == Placement::Frozen);
        }
        for (i, (name, kind, help)) in [
            ("agentd_shard_sessions", "gauge", "Sessions in the shard."),
            (
                "agentd_shard_active_turns",
                "gauge",
                "Sessions with a turn claimed (running or about to).",
            ),
            (
                "agentd_shard_queued_inputs",
                "gauge",
                "Messages waiting for a turn.",
            ),
            (
                "agentd_shard_turns_completed_total",
                "counter",
                "Turns completed in the shard's sessions.",
            ),
            (
                "agentd_shard_resumed_turns",
                "gauge",
                "Active turns on a second or later attempt (resumed after a failure).",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            w.sample_raw(name, kind, help, &base, row[i] as f64);
        }
        w.sample_raw(
            "agentd_shard_unloaded_sessions",
            "gauge",
            "Sessions whose events are only in the object store (leader's view).",
            &base,
            unloaded as f64,
        );
        w.sample_raw(
            "agentd_shard_frozen_sessions",
            "gauge",
            "Sessions frozen for a move.",
            &base,
            frozen as f64,
        );
        w.sample_raw(
            "agentd_shard_forwarded_sessions",
            "gauge",
            "Sessions this shard handed to another shard.",
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

/// How busy this shard is, for placement and rebalancing. Every replica
/// holds the same replicated state, so any node answers for the shard.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct ShardLoad {
    pub sessions: u64,
    /// Turns claimed (running, or parked mid-move).
    pub active: u64,
    /// Of `active`, turns that wrote an event in the last
    /// [`WAITING_AFTER`]: stepping, not waiting.
    #[serde(default)]
    pub busy: u64,
    pub queued: u64,
    /// Sessions frozen for a move out of this shard.
    pub moving: u64,
    /// The worst of the shard's nodes' memory use, in permille of its limit.
    #[serde(default)]
    pub pressure: u64,
    /// Short of memory: place no new sessions here (see `pressure.rs`).
    #[serde(default)]
    pub closed: bool,
}

/// A running turn that wrote nothing for this long is waiting (on a slow
/// tool, a person, another session), not working.
pub const WAITING_AFTER: chrono::Duration = chrono::Duration::seconds(30);

/// An idle session whose schedule falls due within this is not moved.
/// Decision: no shard fires a schedule while its session moves (the source
/// is frozen, the target's copy only prepared), so a move that straddles
/// the due time makes it late by the move. Under load a move takes seconds:
/// main CI saw schedules fire 6.5 s late in `shard-testkit`'s 10 000
/// schedules test while the rebalancer evened out idle sessions. Idle
/// sessions are only moved to even out counts, so waiting costs nothing.
pub const SCHEDULE_CLEAR: std::time::Duration = std::time::Duration::from_secs(30);

impl ShardLoad {
    /// What the shard's turns cost, in turn slots.
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

pub fn shard_load(state: &crate::state::ShardState) -> ShardLoad {
    let mut load = ShardLoad::default();
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

async fn load(State(node): State<Arc<Node>>) -> Json<ShardLoad> {
    let mut load = shard_load(&node.shared.state.read());
    load.pressure = node.pressure.shard_permille();
    load.closed = node.pressure.effective() >= crate::pressure::Level::Full;
    Json(load)
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
    let soon = crate::node::now_ms() + SCHEDULE_CLEAR.as_millis() as i64;
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
            (false, "idle")
                if s.queued() == 0 && !s.authority.runnable && !s.platform.due_before(soon) =>
            {
                Some(((false, 0), *id))
            }
            _ => None,
        })
        .collect();
    found.sort_by_key(|(key, _)| *key);
    Json(found.into_iter().take(q.n).map(|(_, id)| id).collect())
}

/// The fleet's shards as the directory knows them (meaningful on the
/// directory shard).
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Registry {
    pub shards: std::collections::BTreeMap<ShardId, std::collections::BTreeMap<NodeId, String>>,
    pub draining: std::collections::BTreeSet<ShardId>,
    pub retired: std::collections::BTreeSet<ShardId>,
    /// Directory slots held outside the directory shard.
    #[serde(default)]
    pub slots: std::collections::BTreeMap<u16, ShardId>,
}

async fn registry(State(node): State<Arc<Node>>) -> Json<Registry> {
    let state = node.shared.state.read();
    Json(Registry {
        shards: state.shards.clone(),
        draining: state.draining.clone(),
        retired: state.retired.clone(),
        slots: state.slots.clone(),
    })
}

async fn members(State(node): State<Arc<Node>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "shard": node.cfg.shard,
        "leader": node.leader(),
        "members": node.members(),
    }))
}

#[derive(Deserialize)]
pub struct NewMember {
    pub node: NodeId,
    pub addr: String,
}

/// Add a node to this shard (leader only; others answer 421 so a caller
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
    Moved { shard: ShardId },
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

/// `POST /shard/tenants/{tenant}/grants`: take a spend allowance grant (the
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

/// `GET /shard/allowances`: every tenant's grants, usage and open
/// reservations, as this replica applied them.
async fn allowances(State(node): State<Arc<Node>>) -> Json<crate::allowance::Allowances> {
    Json(node.shared.state.read().allowances.clone())
}

#[derive(Deserialize)]
struct Handoff {
    to: ShardId,
}

/// `POST /shard/slots/{slot}/handoff?to=B`: hand this shard's directory
/// entries of `slot` to shard `B`. Seals the slot first, so no entry
/// changes here while it is copied; `B` keeps any entry it already has
/// (written after the seal). The directory shard then assigns the slot.
async fn handoff_slot(
    State(node): State<Arc<Node>>,
    Path(slot): Path<u16>,
    Query(q): Query<Handoff>,
) -> Result<Json<usize>, (StatusCode, String)> {
    let error = |e: String| (StatusCode::SERVICE_UNAVAILABLE, e);
    let gateway = node
        .gateway
        .get()
        .ok_or_else(|| error("no gateway".into()))?;
    let (_, index) = node
        .propose(Command::SealSlot { slot, to: q.to })
        .await
        .map_err(|e| error(e.to_string()))?;
    node.shared.wait_applied(index).await;
    let entries: Vec<(SessionId, crate::state::Placed)> = node
        .shared
        .state
        .read()
        .directory
        .iter()
        .filter(|(session, _)| crate::state::slot_of(**session) == slot)
        .map(|(session, placed)| (*session, *placed))
        .collect();
    let n = entries.len();
    // At least one Adopt, so the new owner takes the slot even when empty.
    for chunk in entries
        .chunks(2000)
        .chain(entries.is_empty().then_some(&[][..]))
    {
        let command = Command::Adopt {
            slot,
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
    /// Tool calls finishing after their turn parked (`detach.rs`).
    #[serde(default)]
    pub detached: usize,
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
            detached: s.authority.detached.len(),
        }
    }
}

pub(crate) fn lookup<T>(
    node: &Node,
    session: SessionId,
    f: impl FnOnce(&SessionState) -> T,
) -> Lookup<T> {
    let state = node.shared.state.read();
    match state.sessions.get(&session) {
        Some(s) => Lookup::Here(f(s)),
        None => match state.forwarded.get(&session) {
            Some(left) => Lookup::Moved { shard: left.shard },
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

/// Durable events after `after`, replayed, then live from this replica's
/// apply. Deltas are on the session's stream node (`stream.rs`).
pub fn subscribe(node: Arc<Node>, session: SessionId, after: i32) -> mpsc::Receiver<Arc<Event>> {
    let (tx, rx) = mpsc::channel(1024);
    tokio::spawn(async move {
        // Subscribe before replaying, so nothing falls between the two.
        let mut changes = node.shared.watch(session);
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
pub enum ShardReply {
    Ok {
        value: serde_json::Value,
    },
    /// The session lives in another shard now.
    Moved {
        shard: ShardId,
    },
    /// Frozen for a move: retry shortly.
    Frozen,
    /// Short of memory: no new work now, retry after this long.
    Busy {
        retry_after_ms: u64,
    },
    NotFound,
    Refused {
        reason: String,
    },
}

pub(crate) fn reply_of(error: impl std::fmt::Display) -> ShardReply {
    let text = error.to_string();
    if let Some(at) = text.find(crate::hosting::MOVED) {
        let rest = &text[at + crate::hosting::MOVED.len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(shard) = digits.parse() {
            return ShardReply::Moved { shard };
        }
    }
    if text.contains(crate::hosting::FROZEN) {
        return ShardReply::Frozen;
    }
    if let Some(at) = text.find(crate::hosting::BUSY) {
        let rest = &text[at + crate::hosting::BUSY.len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return ShardReply::Busy {
            retry_after_ms: digits.parse().unwrap_or(crate::pressure::RETRY_AFTER_MS),
        };
    }
    if text.contains("not found") || text.contains("NotFound") {
        return ShardReply::NotFound;
    }
    ShardReply::Refused { reason: text }
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub definition: serde_json::Value,
    /// The tenant the session's spend is billed to, if metered.
    #[serde(default)]
    pub tenant: Option<String>,
    /// The control plane's key of the agent it runs, when the control
    /// plane created it ([`crate::platform::Platform::agent_ref`]).
    #[serde(default)]
    pub agent_ref: Option<String>,
    /// The control plane's Memory mounts it has
    /// ([`crate::platform::Platform::memory`]).
    #[serde(default)]
    pub memory: Vec<String>,
    /// A moved v1 session the control plane imports: closed to messages
    /// until it opens it (`api_session::activate`).
    #[serde(default)]
    pub importing: bool,
    /// ([`crate::platform::Platform::servers`]).
    #[serde(default)]
    pub servers: crate::platform::Servers,
    /// The session that started it, when it is a subagent
    /// ([`crate::platform::Platform::origin`]).
    #[serde(default)]
    pub subagent: Option<everruns_contracts::dataplane::SubagentOrigin>,
    /// The person who owns it ([`crate::platform::Platform::owner`]).
    #[serde(default)]
    pub owner: Option<String>,
    /// Its labels (agentd's public API), set with the tenant.
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

/// Create the session through Engine: its binding pins the definition's key
/// and this host's build; the definition rides along in the same command.
async fn create_session(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<CreateBody>,
) -> Result<Json<ShardReply>, (StatusCode, String)> {
    let id = parse(&session)?;
    // Marked before it exists, so no message reaches it before its import.
    if body.importing {
        let (marked, _) = node
            .propose(Command::Importing {
                session: id,
                on: true,
            })
            .await
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
        if !matches!(marked, Response::Ok) {
            return Ok(Json(match marked {
                Response::Moved { shard } => ShardReply::Moved { shard },
                other => ShardReply::Refused {
                    reason: format!("{other:?}"),
                },
            }));
        }
    }
    // Held while creating: Engine resolves the build from the interned copy.
    let definition = crate::state::definitions::intern(body.definition);
    let build = crate::hosting::build_for(&definition, crate::hosting::BUILD_ID);
    let reply = match node.engine.create_registered(id, &build).await {
        Ok(_) => ShardReply::Ok {
            value: serde_json::json!({ "id": id }),
        },
        Err(error) => reply_of(error),
    };
    drop(definition);
    // Billed before the reply, so no input reaches the session unmetered.
    if let (ShardReply::Ok { .. }, Some(tenant)) = (&reply, body.tenant) {
        let (assigned, _) = node
            .propose(Command::Assign {
                session: id,
                tenant,
                labels: body.labels,
            })
            .await
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
        if let Response::Rejected { reason } = assigned {
            return Ok(Json(ShardReply::Refused { reason }));
        }
    }
    if let (ShardReply::Ok { .. }, Some(agent_ref)) = (&reply, body.agent_ref) {
        let action = crate::platform::Op::Agent {
            agent_ref,
            memory: body.memory,
            servers: body.servers,
            owner: body.owner,
        };
        match crate::api_session::platform(&node, id, action).await? {
            ShardReply::Ok { .. } => {}
            other => return Ok(Json(other)),
        }
    }
    if let (ShardReply::Ok { .. }, Some(origin)) = (&reply, body.subagent) {
        let action = crate::platform::Op::Subagent { origin };
        match crate::api_session::platform(&node, id, action).await? {
            ShardReply::Ok { .. } => {}
            other => return Ok(Json(other)),
        }
    }
    Ok(Json(reply))
}

#[derive(Deserialize)]
pub struct MessageBody {
    /// The message's text, when it is only text.
    #[serde(default)]
    pub text: String,
    /// Or its parts (text, images, files), when it has any but text
    /// (step 11.6). An image or file part refers to an attachment already
    /// in the session's files (`/attachments/<id>/`).
    #[serde(default)]
    pub parts: Vec<everruns_core::ContentPart>,
    pub id: everruns_contracts::typed_id::MessageId,
    /// Who wrote it, when not the person (an agent trigger, step 11.4).
    #[serde(default)]
    pub origin: Option<everruns_contracts::message_origin::MessageOrigin>,
    /// The message's own metadata (a channel's thread and platform ids,
    /// step 11.5).
    #[serde(default)]
    pub metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// The channel participant who wrote it (step 11.5).
    #[serde(default)]
    pub external_actor: Option<everruns_core::ExternalActor>,
    /// The person who sent it (a user id), from a trusted admission path:
    /// the gateway's verified org token or the control plane
    /// (`senders.rs`). Recorded before the message is admitted.
    #[serde(default)]
    pub sender: Option<String>,
}

/// Why a message to a session still being imported is refused.
pub(crate) const IMPORTING: &str =
    "The session is moving to the data plane; send the message again in a moment";

/// Admit a user message through Engine (`Session::send_with_id`): a retry
/// with the same id finds the first admission.
async fn send_message(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(body): Json<MessageBody>,
) -> Result<Json<ShardReply>, (StatusCode, String)> {
    let id = parse(&session)?;
    let handle = match node.engine.resume(id).await {
        Ok(handle) => handle,
        Err(error) => {
            // Engine folds authority errors into `Unavailable`; ask the
            // replica why.
            let reply = match lookup(&node, id, |s| s.placement) {
                Lookup::Moved { shard } => ShardReply::Moved { shard },
                Lookup::Missing => ShardReply::NotFound,
                Lookup::Here(Placement::Frozen | Placement::Prepared) => ShardReply::Frozen,
                Lookup::Here(_) => reply_of(error),
            };
            return Ok(Json(reply));
        }
    };
    if matches!(lookup(&node, id, |s| s.importing), Lookup::Here(true)) {
        return Ok(Json(ShardReply::Refused {
            reason: IMPORTING.into(),
        }));
    }
    if let Some(reason) = missing_attachment(&node, id, &body.parts).await {
        return Ok(Json(ShardReply::Refused { reason }));
    }
    if let Err(reply) = crate::api_session::prepare_initial_model(&node, id, &handle).await {
        return Ok(Json(reply));
    }
    if let Some(user) = body.sender.clone() {
        let action = crate::platform::Op::Sender {
            message: body.id,
            user,
        };
        match crate::api_session::platform(&node, id, action).await? {
            ShardReply::Ok { .. } => {}
            other => return Ok(Json(other)),
        }
    }
    let mut input = everruns::InputMessage::user(body.text);
    if !body.parts.is_empty() {
        input.content = body.parts;
    }
    input.metadata = body.metadata;
    input.external_actor = body.external_actor;
    if let Some(origin) = &body.origin {
        origin.write(&mut input.metadata);
    }
    let reply = match handle.send_with_id(body.id, input).await {
        Ok(sent) => {
            node.prom.messages_admitted.inc(&[("status", "accepted")]);
            // A channel delivers a started turn's reply itself; a steered
            // message's reply comes in the turn already running.
            let disposition = match sent.disposition {
                everruns::SendDisposition::Started => "started",
                everruns::SendDisposition::Steered => "steered",
                _ => "pending",
            };
            ShardReply::Ok {
                value: serde_json::json!({
                    "id": body.id,
                    "session_id": id,
                    "status": "accepted",
                    "disposition": disposition,
                }),
            }
        }
        Err(error) => reply_of(error),
    };
    Ok(Json(reply))
}

/// Why `parts` cannot be admitted: an image or file part whose attachment
/// is not in the session's files. The executor reads an attachment's bytes
/// from there when it builds a request (Engine's session attachments), so
/// a message must never name one the session does not hold. The control
/// plane copies uploads there before it sends such a message (step 11.6).
async fn missing_attachment(
    node: &Node,
    session: SessionId,
    parts: &[everruns_core::ContentPart],
) -> Option<String> {
    use everruns_core::ContentPart;
    use everruns_core::session_files::SessionFileSystem as _;
    let files = node.hosting.workspace_api();
    let mut synced = false;
    for part in parts {
        let id = match part {
            ContentPart::ImageFile(image) => image.image_id.to_string(),
            ContentPart::File(file) => file.file_id.to_string(),
            _ => continue,
        };
        let dir = format!(
            "{}/{id}",
            everruns_contracts::runtime::attachments::ATTACHMENTS_ROOT
        );
        loop {
            let held = files
                .list_directory(session, &dir)
                .await
                .is_ok_and(|entries| entries.iter().any(|entry| !entry.is_directory));
            if held {
                break;
            }
            // The copy may have committed after this replica last applied.
            if !synced && crate::api_files::read_barrier(node).await.is_ok() {
                synced = true;
                continue;
            }
            return Some(format!(
                "attachment {id} is not in the session's files; send a message with uploads \
                 through the control plane, which copies them there"
            ));
        }
    }
    None
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
) -> Result<Json<ShardReply>, (StatusCode, String)> {
    use everruns::hosting::SessionHosting as _;
    let id = parse(&session)?;
    let reply = match node.hosting.cancel(id, body.id, body.input).await {
        Ok(()) => ShardReply::Ok {
            value: serde_json::json!({ "cancelled": body.input }),
        },
        Err(error) => reply_of(error),
    };
    Ok(Json(reply))
}

#[cfg(test)]
mod load_tests {
    use super::ShardLoad;

    #[test]
    fn waiting_turns_weigh_a_tenth_of_stepping_ones() {
        let waiting = ShardLoad {
            active: 100,
            busy: 0,
            ..Default::default()
        };
        let stepping = ShardLoad {
            active: 10,
            busy: 10,
            ..Default::default()
        };
        assert_eq!(waiting.work(), stepping.work());
        let queued = ShardLoad {
            queued: 3,
            ..Default::default()
        };
        assert_eq!(queued.work(), 3.0);
    }
}
