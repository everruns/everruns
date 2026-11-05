//! Experimental: the in-process runtime for an [`Agent`], over host-supplied
//! backends, for hosts that schedule and persist turns themselves.
//!
//! [`SessionHosting`](super::SessionHosting) is the supported way to host
//! sessions under [`Engine`](crate::Engine): Engine's actor driver claims a
//! session and runs its turns through the profile's ports. Some hosts own
//! scheduling, ownership and replication outright (the funout experiment
//! replicates every session's canonical log with Raft and decides itself which
//! process runs a turn). They need the Agent's turn semantics without Engine's
//! driver: build the runtime here, give it a [`HostBackends`] whose
//! [`EventLog`](everruns_core::host::EventLog) is the host's durable log, and
//! call [`InProcessRuntime::run_stored_turn`] or
//! [`InProcessRuntime::resume_interrupted_turn`].
//!
//! Decision: this exposes the existing composition rather than a new step API.
//! The runtime holds no durable state of its own; everything a resume needs is
//! in the canonical log (`TurnLogFold`), so a host that replicates the log can
//! drop the runtime at any point and rebuild it elsewhere. No compatibility
//! promise: outside [API Stability](https://docs.everruns.com).

use std::sync::Arc;

pub use everruns_core::host::{
    AcceptedTurnInput, EventSink, HostBackends, InProcessRuntime, TurnResult, TurnSteering,
    in_process_internal_org_id,
};

pub use everruns_contracts::typed_id::{MessageId, TurnId};

use crate::{Agent, SessionId};

impl Agent {
    /// Build this Agent's in-process runtime for `session_id` over `backends`.
    ///
    /// The session record and agent definition are re-seeded from the Agent on
    /// every build, so a host only has to make `backends.event_log` durable.
    /// Post-commit canonical events and ephemeral deltas reach
    /// `backends.event_sink`. The Agent's own environment and workspace
    /// configuration is not applied: the session runs without a bound
    /// environment unless the backends supply one.
    ///
    /// # Errors
    ///
    /// Configuration failures from composing the runtime (an unknown host
    /// provider, an unreadable memory directory, a plugin that fails to load).
    pub async fn external_runtime(
        &self,
        backends: HostBackends,
        session_id: SessionId,
    ) -> Result<InProcessRuntime, everruns_contracts::error::AgentLoopError> {
        self.build_external_runtime(backends, session_id, None)
            .await
    }

    /// As [`external_runtime`](Self::external_runtime), routing finalized
    /// events through `sink` as well as the backends' own sink.
    ///
    /// # Errors
    ///
    /// As [`external_runtime`](Self::external_runtime).
    pub async fn external_runtime_with_sink(
        &self,
        backends: HostBackends,
        session_id: SessionId,
        sink: Arc<dyn EventSink>,
    ) -> Result<InProcessRuntime, everruns_contracts::error::AgentLoopError> {
        self.build_external_runtime(backends, session_id, Some(sink))
            .await
    }
}
