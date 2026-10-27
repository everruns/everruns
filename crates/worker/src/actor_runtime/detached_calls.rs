//! The worker's detached tool calls: long server-side calls taken off their
//! turn, so the turn parks and frees its execution slot, lease and memory
//! while the call runs (knowledge/execution/detached-tool-calls.md).
//!
//! Decisions:
//! - A call detaches after `WORKER_TOOL_DETACH_AFTER_SECS` (30 s; 0 turns
//!   detaching off). The threshold is high because each detach costs a
//!   re-lease and a turn setup when the turn resumes; a held slot for 30 s
//!   costs more than that.
//! - `MAX_DETACHED_TOOL_CALLS` (1000) bounds the futures one worker holds.
//!   Past it a call stays with its turn, as a model wait past its cap does.
//! - Each detached call has a control-plane record. The worker renews all of
//!   its records in one call every third of the TTL, and early on a cancel
//!   notification; a record missing from the reply (cancelled, expired) stops
//!   its call. A worker that dies stops renewing, its records expire and the
//!   next lease resumes the turns, rerunning or settling each call as after a
//!   crash mid-tool.
//! - A detached call's events wait for its turn to park with its own events
//!   stored (the gate), so the call's `tool.completed` never lands before its
//!   `tool.started`. A turn that ends any other way (cancel, lost lease,
//!   failure) drops its detached calls and expires their records: the next
//!   holder resumes the turn from the log.
//! - Shutdown stops every detached call once the running turns had their
//!   grace, and expires their records at once, so another worker resumes those
//!   turns without waiting out a TTL.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Notify, watch};
use tokio::task::AbortHandle;
use tracing::{debug, warn};

use crate::actor_lease_store::{ActorFence, ActorLeaseStore, DetachedCallKey, actor_lease_lost};
use crate::core::event_emitter::EventEmitter;
use crate::core::events::{Event, EventContext, EventRequest};
use crate::engine::tool_detach::{Detached, DetachedToolCall, DetachedWork, ToolDetacher};
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;

/// Default for `WORKER_TOOL_DETACH_AFTER_SECS`.
pub const DEFAULT_TOOL_DETACH_AFTER: Duration = Duration::from_secs(30);
/// Default for `MAX_DETACHED_TOOL_CALLS`, per worker.
pub const DEFAULT_MAX_DETACHED_TOOL_CALLS: usize = 1000;
/// Life of a detached call's record between renewals.
pub const DETACHED_CALL_TTL: Duration = Duration::from_secs(30);

/// Builds the emitter a detached call's events go through, appending under
/// the call's record (`ActorFence::detached_call`).
pub type DetachedEmitterFactory = Arc<dyn Fn(ActorFence) -> Arc<dyn EventEmitter> + Send + Sync>;

/// Settings of a worker's detached calls.
#[derive(Debug, Clone, Copy)]
pub struct DetachedCallsConfig {
    pub detach_after: Duration,
    pub max_calls: usize,
    pub ttl: Duration,
}

/// The calls one worker finishes after their turns parked.
pub struct DetachedCalls {
    store: Arc<dyn ActorLeaseStore>,
    holder: String,
    config: DetachedCallsConfig,
    emitter_for: DetachedEmitterFactory,
    /// Calls with a record; `None` until the call's task started.
    calls: Mutex<HashMap<DetachedCallKey, Option<AbortHandle>>>,
}

impl DetachedCalls {
    pub fn new(
        store: Arc<dyn ActorLeaseStore>,
        holder: impl Into<String>,
        config: DetachedCallsConfig,
        emitter_for: DetachedEmitterFactory,
    ) -> Self {
        Self {
            store,
            holder: holder.into(),
            config,
            emitter_for,
            calls: Mutex::new(HashMap::new()),
        }
    }

    fn calls(&self) -> std::sync::MutexGuard<'_, HashMap<DetachedCallKey, Option<AbortHandle>>> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Calls this worker holds now.
    pub fn len(&self) -> usize {
        self.calls().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The detacher for one leased actor's turns.
    pub fn for_lease(self: &Arc<Self>, fence: u64, wake: Arc<Notify>) -> Arc<dyn ToolDetacher> {
        Arc::new(LeaseDetacher {
            runner: self.clone(),
            fence,
            wake,
            gates: Mutex::new(Vec::new()),
        })
    }

    /// Run `work` (the rest of the call) once `gate` says its turn parked,
    /// then drop the call's record, waking `wake` when that made the actor
    /// runnable.
    fn start(
        self: Arc<Self>,
        key: DetachedCallKey,
        mut gate: watch::Receiver<Option<bool>>,
        work: DetachedWork,
        wake: Arc<Notify>,
    ) {
        let runner = self.clone();
        let task_key = key.clone();
        let task = tokio::spawn(async move {
            work.await;
            // A call that finished while it was being detached recorded its
            // events through its turn; it still waits for the turn to park.
            let parked = gate
                .wait_for(Option::is_some)
                .await
                .map(|parked| *parked == Some(true))
                .unwrap_or(false);
            if parked {
                match runner
                    .store
                    .complete_detached_tool_call(&task_key, &runner.holder)
                    .await
                {
                    Ok(Some(true)) => wake.notify_one(),
                    Ok(Some(false)) => {}
                    Ok(None) => debug!(
                        session_id = %task_key.session_id,
                        tool_call_id = %task_key.tool_call_id,
                        "Detached tool call's record was gone at completion (cancelled or expired)"
                    ),
                    Err(error) => warn!(
                        session_id = %task_key.session_id,
                        tool_call_id = %task_key.tool_call_id,
                        error = %error,
                        "Failed to complete a detached tool call; its record expires and the turn resumes"
                    ),
                }
            }
            runner.calls().remove(&task_key);
        });
        let mut calls = self.calls();
        match calls.get_mut(&key) {
            Some(slot) => *slot = Some(task.abort_handle()),
            // Dropped before it started (its turn did not park).
            None => task.abort(),
        }
    }

    /// Stop `keys` here and expire their records.
    fn drop_calls(self: &Arc<Self>, keys: Vec<DetachedCallKey>) {
        self.abort(&keys);
        let runner = self.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = runner
                    .store
                    .expire_detached_tool_calls(&runner.holder, &keys)
                    .await
                {
                    warn!(error = %error, "Failed to expire dropped detached tool calls; they expire on their own");
                }
            });
        }
    }

    fn abort(&self, keys: &[DetachedCallKey]) {
        let mut calls = self.calls();
        for key in keys {
            if let Some(Some(task)) = calls.remove(key) {
                task.abort();
            }
        }
    }

    /// Renew every record; stop the calls whose record is gone.
    async fn renew(&self) {
        let keys: Vec<DetachedCallKey> = self.calls().keys().cloned().collect();
        if keys.is_empty() {
            return;
        }
        match self
            .store
            .renew_detached_tool_calls(&self.holder, self.config.ttl, &keys)
            .await
        {
            Ok(live) => {
                let gone: Vec<_> = keys.into_iter().filter(|key| !live.contains(key)).collect();
                if !gone.is_empty() {
                    debug!(
                        stopped = gone.len(),
                        "Stopping detached tool calls whose record is gone"
                    );
                    self.abort(&gone);
                }
            }
            Err(error) => {
                warn!(error = %error, "Failed to renew detached tool calls");
            }
        }
    }

    /// Renew until `shutdown`, every third of the TTL and on `poke` (a
    /// cancel notification).
    pub async fn run(self: Arc<Self>, poke: Arc<Notify>, mut shutdown: watch::Receiver<bool>) {
        let interval = self.config.ttl / 3;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = poke.notified() => {}
                _ = shutdown.changed() => return,
            }
            self.renew().await;
        }
    }

    /// Stop every call and expire its record (shutdown).
    pub async fn stop(&self) {
        let calls: Vec<_> = self.calls().drain().collect();
        if calls.is_empty() {
            return;
        }
        let keys: Vec<_> = calls
            .into_iter()
            .map(|(key, task)| {
                if let Some(task) = task {
                    task.abort();
                }
                key
            })
            .collect();
        warn!(
            stopped = keys.len(),
            "Stopping detached tool calls for shutdown; their turns resume on another worker"
        );
        if let Err(error) = self
            .store
            .expire_detached_tool_calls(&self.holder, &keys)
            .await
        {
            warn!(error = %error, "Failed to expire detached tool calls at shutdown; they expire on their own");
        }
    }
}

/// One lease's view of the runner: detaches under the lease's fence and
/// gates its calls on how the turn ends.
struct LeaseDetacher {
    runner: Arc<DetachedCalls>,
    fence: u64,
    wake: Arc<Notify>,
    gates: Mutex<Vec<(DetachedCallKey, watch::Sender<Option<bool>>)>>,
}

impl LeaseDetacher {
    fn end(&self, parked: bool) {
        let gates = std::mem::take(
            &mut *self
                .gates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        let mut dropped = Vec::new();
        for (key, gate) in gates {
            gate.send_replace(Some(parked));
            if !parked {
                dropped.push(key);
            }
        }
        if !dropped.is_empty() {
            self.runner.drop_calls(dropped);
        }
    }
}

impl Drop for LeaseDetacher {
    fn drop(&mut self) {
        self.end(false);
    }
}

#[async_trait]
impl ToolDetacher for LeaseDetacher {
    fn detach_after(&self) -> Duration {
        self.runner.config.detach_after
    }

    async fn detach(&self, call: &DetachedToolCall) -> Option<Detached> {
        let runner = &self.runner;
        if runner.len() >= runner.config.max_calls {
            debug!(
                session_id = %call.session_id,
                tool_call_id = %call.tool_call_id,
                "Detached tool call cap reached; the call stays with its turn"
            );
            return None;
        }
        let key = DetachedCallKey {
            session_id: call.session_id,
            tool_call_id: call.tool_call_id.clone(),
        };
        match runner
            .store
            .detach_tool_call(
                &key,
                call.turn_id,
                &runner.holder,
                self.fence,
                runner.config.ttl,
            )
            .await
        {
            Ok(true) => {}
            Ok(false) => return None,
            Err(error) => {
                warn!(
                    session_id = %call.session_id,
                    tool_call_id = %call.tool_call_id,
                    error = %error,
                    "Failed to detach a tool call; it stays with its turn"
                );
                return None;
            }
        }
        // Held from here, so renewals cover it; only after its record exists,
        // so a renewal cannot miss it and stop it.
        runner.calls().insert(key.clone(), None);
        let (gate, parked) = watch::channel(None);
        self.gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((key.clone(), gate));
        let emitter: Arc<dyn EventEmitter> = Arc::new(GatedEmitter {
            session_id: call.session_id,
            parked: parked.clone(),
            inner: (runner.emitter_for)(ActorFence::detached_call(
                call.session_id,
                runner.holder.clone(),
                call.tool_call_id.clone(),
            )),
        });
        let runner = runner.clone();
        let wake = self.wake.clone();
        Some(Detached {
            emitter,
            run: Box::new(move |work| runner.start(key, parked, work, wake)),
        })
    }

    fn turn_ended(&self, parked: bool) {
        self.end(parked);
    }
}

/// A detached call's events: held until its turn parked, refused when the
/// turn did not.
struct GatedEmitter {
    session_id: SessionId,
    parked: watch::Receiver<Option<bool>>,
    inner: Arc<dyn EventEmitter>,
}

#[async_trait]
impl EventEmitter for GatedEmitter {
    fn turn_causal_context(&self, context: &EventContext) -> crate::core::causal::CausalContext {
        self.inner.turn_causal_context(context)
    }

    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let mut parked = self.parked.clone();
        let parked = parked
            .wait_for(Option::is_some)
            .await
            .map(|parked| *parked == Some(true))
            .unwrap_or(false);
        if !parked {
            return Err(actor_lease_lost(
                self.session_id,
                "the detached tool call's turn did not park",
            ));
        }
        self.inner.emit(request).await
    }
}
