//! Lets a host take a long tool call off its turn, so the turn parks instead
//! of holding its execution slot, lease and memory while the tool works.
//!
//! Decision: a worker runs a session's whole turn on one lease, and a
//! server-side call held all of it until the tool returned. A 20-minute
//! sandbox build kept one of the worker's slots, a lease renewal every few
//! seconds and the whole turn in memory, so 50 such waits filled a worker.
//! A call still running after [`ToolDetacher::detach_after`] is handed to the
//! host instead: the call keeps running there, the act parks the turn
//! ([`parked`]), and the host records the call's `tool.completed` when it
//! finishes and makes the session runnable. The turn resumes from the log
//! like a turn parked on client tools: every call settled, so it goes on to
//! its next reason.
//!
//! Decision: hosts opt in through task-locals, like [`super::model_wait`]. A
//! worker installs its detacher around each leased turn ([`provide`]); only
//! the act of that turn's own loop may use it ([`for_act`]), and tools the act
//! runs inline do not see it ([`without`]). So a child turn a tool runs inline
//! (a subagent) never detaches into its parent's park, and other callers of
//! the act (native async tools, the Agents API backend) keep running calls to
//! their end.
//!
//! Decision: a call detaches only when nothing else in its turn depends on it
//! running inline: it runs on the server, is not `cpu_bound` (an in-process
//! interpreter really uses the worker's CPU, which the slot cap must keep
//! counting), does not shape the turn (`stays_direct`, native-only tools such
//! as `spawn_agent` and `ask_user`), and its act has no client-side calls.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::tool_surface::NATIVE_ONLY_TOOL_NAMES;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};

use crate::engine::event_emitter::EventEmitter;
use crate::engine::events::{Event, EventContext, EventRequest};
use crate::engine::tool_types::{ToolDefinition, ToolPolicy};

/// A call the act hands to its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedToolCall {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub input_message_id: MessageId,
    pub tool_call_id: String,
    pub tool_name: String,
}

/// The rest of a detached call: it finishes the tool, runs the post-tool
/// hooks and emits the call's `tool.completed` through [`Detached::emitter`].
pub type DetachedWork = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// A call the host took over.
pub struct Detached {
    /// Where the call's events go once it is detached: its turn no longer
    /// holds the lease its host appends under.
    pub emitter: Arc<dyn EventEmitter>,
    /// Runs the rest of the call to its end, then settles it with the host.
    pub run: Box<dyn FnOnce(DetachedWork) + Send>,
}

/// A host that can finish a long call after its turn parks.
#[async_trait]
pub trait ToolDetacher: Send + Sync {
    /// How long a call runs inline before it detaches.
    fn detach_after(&self) -> Duration;

    /// Take `call` over: record it so the turn waits for it and a crash is
    /// noticed. `None` keeps it inline (the host is at its cap, or the record
    /// failed).
    async fn detach(&self, call: &DetachedToolCall) -> Option<Detached>;

    /// The turn whose act detached calls ended. `parked`: it parked and its
    /// own events are stored, so the detached calls may record theirs after
    /// them. Otherwise the turn did not keep its calls (it was cancelled, lost
    /// its lease, or failed): drop them, and the next holder resumes the turn
    /// from the log as after a crash.
    fn turn_ended(&self, _parked: bool) {}
}

/// The status an act reports for a call its host took over.
pub const DETACHED_STATUS: &str = "detached";

type Slot = Option<Arc<dyn ToolDetacher>>;

tokio::task_local! {
    static PROVIDED: Slot;
    static ACTIVE: Slot;
}

/// Runs `fut` (a leased turn) with `detacher` available to its loop's act.
pub async fn provide<F: Future>(detacher: Arc<dyn ToolDetacher>, fut: F) -> F::Output {
    PROVIDED.scope(Some(detacher), fut).await
}

/// Runs `fut` (the act a turn's own loop runs) with the provided detacher.
pub async fn for_act<F: Future>(fut: F) -> F::Output {
    let detacher = PROVIDED.try_with(Clone::clone).ok().flatten();
    ACTIVE.scope(detacher, fut).await
}

/// Runs `fut` (a tool the act runs inline) without a detacher, for its own
/// calls and for any turn it runs.
pub async fn without<F: Future>(fut: F) -> F::Output {
    PROVIDED.scope(None, ACTIVE.scope(None, fut)).await
}

/// The detacher of the act running on this task, if its host allows one.
pub fn active() -> Option<Arc<dyn ToolDetacher>> {
    ACTIVE.try_with(Clone::clone).ok().flatten()
}

/// Tell the host's detacher that the leased turn running on this task ended
/// (see [`ToolDetacher::turn_ended`]).
pub fn turn_ended(parked: bool) {
    if let Some(detacher) = PROVIDED.try_with(Clone::clone).ok().flatten() {
        detacher.turn_ended(parked);
    }
}

/// The error an act returns once it detached a call: the turn parks until
/// the host makes the session runnable again.
pub fn parked() -> AgentLoopError {
    everruns_contracts::durable_retry::parked()
}

/// Whether `definition`'s calls may detach (see the module notes).
pub fn may_detach(definition: &ToolDefinition) -> bool {
    !matches!(definition, ToolDefinition::ClientSide(_))
        && *definition.policy() != ToolPolicy::ClientSide
        && !definition.is_cpu_bound()
        && definition.hints().stays_direct != Some(true)
        && !NATIVE_ONLY_TOOL_NAMES.contains(&definition.name())
}

/// Forwards to one emitter until [`redirect`](Self::redirect) points it at
/// another: a call's events go to its turn's host until the call detaches,
/// then to the host's detached-call emitter. The turn's trace context stays
/// the turn host's, which keeps it in memory.
pub struct SwitchEmitter {
    turn: Arc<dyn EventEmitter>,
    target: RwLock<Arc<dyn EventEmitter>>,
}

impl SwitchEmitter {
    pub fn new(target: Arc<dyn EventEmitter>) -> Self {
        Self {
            turn: target.clone(),
            target: RwLock::new(target),
        }
    }

    pub fn redirect(&self, target: Arc<dyn EventEmitter>) {
        *self
            .target
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = target;
    }

    fn current(&self) -> Arc<dyn EventEmitter> {
        self.target
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl EventEmitter for SwitchEmitter {
    fn turn_causal_context(&self, context: &EventContext) -> crate::causal::CausalContext {
        self.turn.turn_causal_context(context)
    }

    fn restore_turn_context(&self, context: &EventContext, metadata: Option<&serde_json::Value>) {
        self.turn.restore_turn_context(context, metadata);
    }

    async fn emit(&self, request: EventRequest) -> Result<Event> {
        self.current().emit(request).await
    }
}

#[cfg(test)]
#[path = "tool_detach_tests.rs"]
mod tests;
