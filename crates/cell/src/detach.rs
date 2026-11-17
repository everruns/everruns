//! Detached tool calls on a cell (data plane v2 step 11.4): a long
//! server-side call leaves its turn, so the turn parks and its executor slot,
//! epoch and memory are free while the call runs; the call's record keeps the
//! session waiting for it. The same contract as the worker's
//! (knowledge/execution/detached-tool-calls.md), with the records in the
//! session's cell state instead of the control plane.
//!
//! Decisions:
//! - A call detaches after `CELL_TOOL_DETACH_AFTER_MS` (300 s, the worker's
//!   default; 0 turns detaching off).
//! - Its record lives in the session's authority ([`DetachedCall`]). While a
//!   record is live the session owes no work, so nothing claims it: input
//!   that arrives meanwhile waits for the call. The call's events append
//!   under the record, not under an epoch.
//! - The executor renews its records every third of their TTL (`ttl`). One that dies
//!   stops renewing; once a record expires the session owes work, the next
//!   claim drops the records and resumes the turn, which reruns or cuts off
//!   each unsettled call as after a crash mid-tool. A cancel drops the
//!   records at once, and the executor stops the calls at its next renewal.
//! - A call's events wait for its turn to park with its own events stored
//!   (the gate), so its `tool.completed` never lands before its
//!   `tool.started`. A turn that ends any other way drops its calls.
//! - Settling the last record of a session makes it runnable: the turn
//!   resumes with every call settled and goes on to its next reason.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{EventId, SessionId, TurnId};
use everruns_core::engine::tool_detach::{Detached, DetachedToolCall, DetachedWork, ToolDetacher};
use everruns_core::events::{Event, EventContext, EventRequest};
use everruns_core::host::HostBackends;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::watch;
use tokio::task::AbortHandle;

use crate::authority::Authority;
use crate::hosting::Port;
use crate::state::{Change, Command, Response, SessionState};

/// Life of a record between renewals: `CELL_TOOL_DETACH_TTL_MS`, 30 s
/// by default (the worker's).
fn ttl() -> Duration {
    let ms = std::env::var("CELL_TOOL_DETACH_TTL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000);
    Duration::from_millis(ms)
}

/// A call an executor finishes after its turn parked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachedCall {
    pub turn: TurnId,
    /// The executor's runner that holds it.
    pub holder: String,
    pub expires_ms: i64,
}

/// Whether a detached call of `a` still runs at `now_ms`.
pub fn live(a: &Authority, now_ms: i64) -> bool {
    a.detached.values().any(|call| call.expires_ms > now_ms)
}

/// A change to a session's detached calls ([`crate::platform::Op::Call`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "call_op", rename_all = "snake_case")]
pub enum CallOp {
    /// Record `call` of `turn` as detached by `holder`.
    Detach {
        call: String,
        turn: TurnId,
        holder: String,
        ttl_ms: i64,
    },
    /// Extend `holder`'s `calls`; the reply lists those still recorded.
    Renew {
        holder: String,
        calls: Vec<String>,
        ttl_ms: i64,
    },
    /// Append one of `call`'s events, once its turn parked.
    Event {
        call: String,
        holder: String,
        event_id: EventId,
        request: Box<EventRequest>,
    },
    /// `call` finished: drop its record; the last one makes the session
    /// runnable.
    Settle { call: String, holder: String },
    /// Drop `calls` without resuming (their turn did not park).
    Drop { holder: String, calls: Vec<String> },
}

/// Apply `op` to `state`.
pub(crate) fn apply(
    state: &mut SessionState,
    session: SessionId,
    op: CallOp,
    now_ms: i64,
    changes: &mut Vec<Change>,
) -> Response {
    let a = &mut state.authority;
    let value = match op {
        CallOp::Detach {
            call,
            turn,
            holder,
            ttl_ms,
        } => {
            if !a.executing(now_ms) {
                return Response::Rejected {
                    reason: "a call detaches only while its turn runs".into(),
                };
            }
            a.detached.insert(
                call,
                DetachedCall {
                    turn,
                    holder,
                    expires_ms: now_ms + ttl_ms,
                },
            );
            json!({ "detached": true })
        }
        CallOp::Renew {
            holder,
            calls,
            ttl_ms,
        } => {
            let mut held = Vec::new();
            for id in calls {
                if let Some(call) = a.detached.get_mut(&id)
                    && call.holder == holder
                {
                    call.expires_ms = call.expires_ms.max(now_ms + ttl_ms);
                    held.push(id);
                }
            }
            json!({ "held": held })
        }
        CallOp::Event {
            call,
            holder,
            event_id,
            request,
        } => {
            // A retry finds its event at the tail.
            if let Some(event) = state
                .events
                .iter()
                .rev()
                .take(32)
                .find(|e| e.id == event_id)
            {
                return Response::Value {
                    value: json!(event),
                };
            }
            let a = &state.authority;
            if a.detached.get(&call).is_none_or(|c| c.holder != holder) {
                return Response::Rejected {
                    reason: "the detached call's record is gone".into(),
                };
            }
            if a.executing(now_ms) {
                return Response::Rejected {
                    reason: "the detached call's turn has not parked".into(),
                };
            }
            let event = state.journal(*request, event_id);
            let value = json!(event);
            changes.push(Change::Event(session, Arc::new(event)));
            value
        }
        CallOp::Settle { call, holder } => {
            let settled = a.detached.get(&call).is_some_and(|c| c.holder == holder);
            if settled {
                a.detached.remove(&call);
                if a.detached.is_empty() {
                    a.resume_generation += 1;
                    a.runnable = true;
                }
            }
            json!({ "settled": settled, "runnable": settled && a.detached.is_empty() })
        }
        CallOp::Drop { holder, calls } => {
            for id in calls {
                if a.detached.get(&id).is_some_and(|c| c.holder == holder) {
                    a.detached.remove(&id);
                }
            }
            json!({ "dropped": true })
        }
    };
    changes.push(Change::Session(session));
    Response::Value { value }
}

/// Milliseconds a call runs inline before it detaches; 0: never.
fn detach_after() -> Duration {
    let ms = std::env::var("CELL_TOOL_DETACH_AFTER_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300_000);
    Duration::from_millis(ms)
}

type Key = (SessionId, String);

/// The calls one executor (or node) finishes after their turns parked.
pub(crate) struct Runner {
    port: Port,
    holder: String,
    after: Duration,
    ttl: Duration,
    /// Calls with a record; `None` until the call's task started.
    calls: parking_lot::Mutex<HashMap<Key, Option<AbortHandle>>>,
}

impl Runner {
    /// A runner, renewing its records in the background for as long as it
    /// lives.
    pub(crate) fn start(port: Port) -> Arc<Self> {
        let runner = Arc::new(Self {
            port,
            holder: format!("calls-{}", uuid::Uuid::new_v4()),
            after: detach_after(),
            ttl: ttl(),
            calls: parking_lot::Mutex::default(),
        });
        let weak = Arc::downgrade(&runner);
        tokio::spawn(renew_loop(weak));
        runner
    }

    async fn propose(&self, session: SessionId, op: CallOp) -> Result<serde_json::Value> {
        self.port
            .propose(
                session,
                Command::Platform {
                    session,
                    action: crate::platform::Op::Call(op),
                    now_ms: crate::node::now_ms(),
                    seed: EventId::new(),
                },
            )
            .await
    }

    fn abort(&self, keys: &[Key]) {
        let mut calls = self.calls.lock();
        for key in keys {
            if let Some(Some(task)) = calls.remove(key) {
                task.abort();
            }
        }
    }

    /// Renew every record; stop the calls whose record is gone.
    async fn renew(&self) {
        let mut by_session: HashMap<SessionId, Vec<String>> = HashMap::new();
        for (session, call) in self.calls.lock().keys() {
            by_session.entry(*session).or_default().push(call.clone());
        }
        for (session, calls) in by_session {
            let op = CallOp::Renew {
                holder: self.holder.clone(),
                calls: calls.clone(),
                ttl_ms: self.ttl.as_millis() as i64,
            };
            let held: Vec<String> = match self.propose(session, op).await {
                Ok(value) => serde_json::from_value(value["held"].clone()).unwrap_or_default(),
                // The session is gone or moved: its records went with it.
                Err(AgentLoopError::SessionNotFound(_)) => Vec::new(),
                Err(error) => {
                    tracing::warn!(%session, %error, "renewing detached tool calls failed");
                    continue;
                }
            };
            let gone: Vec<Key> = calls
                .into_iter()
                .filter(|call| !held.contains(call))
                .map(|call| (session, call))
                .collect();
            if !gone.is_empty() {
                tracing::debug!(%session, stopped = gone.len(), "detached calls' records are gone");
                self.abort(&gone);
            }
        }
    }

    /// Run `work` once `gate` says its turn parked, then settle the call.
    fn run(self: Arc<Self>, key: Key, mut gate: watch::Receiver<Option<bool>>, work: DetachedWork) {
        let runner = self.clone();
        let task_key = key.clone();
        let task = tokio::spawn(async move {
            work.await;
            let parked = gate
                .wait_for(Option::is_some)
                .await
                .map(|parked| *parked == Some(true))
                .unwrap_or(false);
            if parked {
                let (session, call) = task_key.clone();
                let op = CallOp::Settle {
                    call,
                    holder: runner.holder.clone(),
                };
                if let Err(error) = runner.propose(session, op).await {
                    tracing::warn!(%session, %error, "settling a detached call failed; its record expires and the turn resumes");
                }
            }
            runner.calls.lock().remove(&task_key);
        });
        let mut calls = self.calls.lock();
        match calls.get_mut(&key) {
            Some(slot) => *slot = Some(task.abort_handle()),
            // Dropped before it started (its turn did not park).
            None => task.abort(),
        }
    }

    /// Stop `calls` of `session` here and drop their records.
    fn drop_calls(self: &Arc<Self>, session: SessionId, calls: Vec<String>) {
        let keys: Vec<Key> = calls.iter().map(|call| (session, call.clone())).collect();
        self.abort(&keys);
        let runner = self.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let op = CallOp::Drop {
                    holder: runner.holder.clone(),
                    calls,
                };
                if let Err(error) = runner.propose(session, op).await {
                    tracing::debug!(%session, %error, "dropping detached calls failed; they expire");
                }
            });
        }
    }

    /// The detacher of one activation of `session`.
    pub(crate) fn attach(
        self: &Arc<Self>,
        session: SessionId,
        mut backends: HostBackends,
    ) -> HostBackends {
        if !self.after.is_zero() {
            backends.extensions.hooks.tool_detacher = Some(Arc::new(ActivationDetacher {
                runner: self.clone(),
                session,
                gates: parking_lot::Mutex::default(),
            }));
        }
        backends
    }
}

async fn renew_loop(runner: Weak<Runner>) {
    loop {
        let Some(every) = runner.upgrade().map(|r| r.ttl / 3) else {
            return;
        };
        tokio::time::sleep(every).await;
        let Some(runner) = runner.upgrade() else {
            return;
        };
        runner.renew().await;
    }
}

/// One activation's view of the runner: gates its calls on how their turn
/// ends.
struct ActivationDetacher {
    runner: Arc<Runner>,
    session: SessionId,
    gates: parking_lot::Mutex<Vec<(String, watch::Sender<Option<bool>>)>>,
}

impl ActivationDetacher {
    fn end(&self, parked: bool) {
        let gates = std::mem::take(&mut *self.gates.lock());
        let mut dropped = Vec::new();
        for (call, gate) in gates {
            gate.send_replace(Some(parked));
            if !parked {
                dropped.push(call);
            }
        }
        if !dropped.is_empty() {
            self.runner.drop_calls(self.session, dropped);
        }
    }
}

impl Drop for ActivationDetacher {
    fn drop(&mut self) {
        self.end(false);
    }
}

#[async_trait]
impl ToolDetacher for ActivationDetacher {
    fn detach_after(&self) -> Duration {
        self.runner.after
    }

    async fn detach(&self, call: &DetachedToolCall) -> Option<Detached> {
        if call.session_id != self.session {
            return None;
        }
        let op = CallOp::Detach {
            call: call.tool_call_id.clone(),
            turn: call.turn_id,
            holder: self.runner.holder.clone(),
            ttl_ms: self.runner.ttl.as_millis() as i64,
        };
        if let Err(error) = self.runner.propose(self.session, op).await {
            tracing::warn!(session = %self.session, %error, "a tool call could not detach; it stays with its turn");
            return None;
        }
        // Held from here, so renewals cover it; only after its record
        // exists, so a renewal cannot miss it and stop it.
        let key = (self.session, call.tool_call_id.clone());
        self.runner.calls.lock().insert(key.clone(), None);
        let (gate, parked) = watch::channel(None);
        self.gates.lock().push((call.tool_call_id.clone(), gate));
        let emitter = Arc::new(CallEmitter {
            runner: self.runner.clone(),
            session: self.session,
            call: call.tool_call_id.clone(),
            parked: parked.clone(),
        });
        let runner = self.runner.clone();
        Some(Detached {
            emitter,
            run: Box::new(move |work| runner.run(key, parked, work)),
        })
    }

    fn turn_ended(&self, parked: bool) {
        self.end(parked);
    }
}

/// A detached call's events: held until its turn parked, refused when the
/// turn did not, then appended under the call's record.
struct CallEmitter {
    runner: Arc<Runner>,
    session: SessionId,
    call: String,
    parked: watch::Receiver<Option<bool>>,
}

#[async_trait]
impl everruns_core::event_emitter::EventEmitter for CallEmitter {
    fn turn_causal_context(&self, _context: &EventContext) -> everruns_core::causal::CausalContext {
        Default::default()
    }

    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let mut parked = self.parked.clone();
        let parked = parked
            .wait_for(Option::is_some)
            .await
            .map(|parked| *parked == Some(true))
            .unwrap_or(false);
        if !parked {
            return Err(AgentLoopError::store(
                "the detached tool call's turn did not park",
            ));
        }
        let op = CallOp::Event {
            call: self.call.clone(),
            holder: self.runner.holder.clone(),
            event_id: EventId::new(),
            request: Box::new(request),
        };
        let value = self.runner.propose(self.session, op).await?;
        serde_json::from_value(value).map_err(|e| AgentLoopError::store(e.to_string()))
    }
}

#[cfg(test)]
#[path = "detach_tests.rs"]
mod tests;
