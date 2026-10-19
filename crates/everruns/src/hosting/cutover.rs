//! Lossless cutover of a legacy local/custom-backend session into a coherent
//! session authority (EVE-1298).
//!
//! [`Engine::cut_over_legacy_session`](crate::Engine::cut_over_legacy_session)
//! reads a quiescent legacy session and hands the authority one image to
//! commit in a single transaction: the canonical journal verbatim (event ids,
//! sequences, turn and tool-call correlation), admission receipts, turn
//! assignments and outcomes, the session record and overlay, compaction
//! checkpoints, session values, terminal task records and the workspace
//! binding. The authority records the source boundary and ownership decision
//! with it, so a retried cutover replays and a conflicting one fails.

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_core::CompactionCheckpoint;
use everruns_core::events::Event;
use everruns_core::host::WorkspaceBinding;
use everruns_core::session::ExecutionSession;
use everruns_core::{SessionTask, TaskMessage};
use serde::{Deserialize, Serialize};

use super::{SessionBinding, SessionConfiguration, SessionHosting};

/// Authority record key holding the committed cutover decision.
#[cfg_attr(not(feature = "local"), allow(dead_code))]
pub(crate) const CUTOVER_RECORD: &str = "cutover";
/// Authority record key holding the imported legacy workspace binding.
pub(crate) const CUTOVER_WORKSPACE: &str = "cutover/workspace";

/// The last legacy journal position an import covers. The legacy lane is
/// frozen at this boundary: nothing after it belongs to the legacy executor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutoverBoundary {
    /// Highest imported legacy sequence; `0` for a session without events.
    pub last_sequence: i32,
    /// Identity of the event at `last_sequence`, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    /// Number of imported canonical events.
    pub events: usize,
}

/// Outcome of a legacy session cutover. Stability: experimental.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CutoverReport {
    /// Stable session identity, unchanged by the cutover.
    pub session: SessionId,
    /// Retained build the authority now executes the session with.
    pub binding: SessionBinding,
    /// Legacy journal boundary the import covers.
    pub boundary: CutoverBoundary,
    /// `true` when this call confirmed an earlier committed cutover.
    pub replayed: bool,
    /// Imported compaction checkpoints.
    pub checkpoints: usize,
    /// Imported terminal task records.
    pub tasks: usize,
    /// Imported session values.
    pub values: usize,
    /// The legacy session used the Agent's volatile in-memory workspace. Its
    /// content is not durable on either lane and is not migrated; the
    /// authority opens the target build's own workspace.
    pub volatile_workspace: bool,
    /// Imported session secrets, sealed under the authority's secret keys.
    pub secrets: usize,
    /// Imported schedules; the authority's supervisor fires them.
    pub schedules: usize,
}

/// One quiescent legacy session, ready for a single authority transaction.
///
/// Opaque: only the Framework Engine builds it, and only the built-in SQLite
/// authority commits it.
#[doc(hidden)]
#[cfg_attr(not(feature = "local"), allow(dead_code))]
pub struct LegacySessionImage {
    pub(crate) binding: SessionBinding,
    pub(crate) source: serde_json::Value,
    pub(crate) events: Vec<Event>,
    pub(crate) record: Option<ExecutionSession>,
    pub(crate) configuration: SessionConfiguration,
    pub(crate) checkpoints: Vec<CompactionCheckpoint>,
    pub(crate) values: Vec<(String, String)>,
    pub(crate) tasks: Vec<(SessionTask, Vec<TaskMessage>)>,
    /// Plaintext legacy secrets, sealed by the authority before its commit.
    pub(crate) secrets: Vec<(String, String)>,
    /// Legacy schedules with their journal metadata.
    pub(crate) schedules: Vec<(
        everruns_core::session_schedule::SessionSchedule,
        serde_json::Value,
    )>,
    pub(crate) workspace: Option<WorkspaceBinding>,
}

#[cfg_attr(not(feature = "local"), allow(dead_code))]
impl LegacySessionImage {
    /// The committed decision. `decided_at` is not part of a retry's identity.
    pub(crate) fn record(&self) -> serde_json::Value {
        serde_json::json!({
            "owner": "authority",
            "source": self.source,
            "binding": self.binding,
        })
    }
}

/// Reopens the workspace head a legacy session was bound to before cutover,
/// and the Agent's default workspace for every other session.
///
/// Register it as the build's Environment factory with
/// [`Engine::register_agent_with_environment`](crate::Engine::register_agent_with_environment).
/// It reads the imported binding from the authority, so a restarted host
/// reopens the same head instead of provisioning a new one. A binding the
/// Agent cannot reopen fails the activation; it never falls back to a fresh
/// workspace. Stability: experimental.
pub struct ImportedWorkspaceFactory<H> {
    agent: crate::Agent,
    hosting: H,
}

impl<H: SessionHosting> ImportedWorkspaceFactory<H> {
    /// Reopen imported heads with `agent`'s workspace backends, reading the
    /// imported binding from `hosting`.
    pub fn new(agent: crate::Agent, hosting: H) -> Self {
        Self { agent, hosting }
    }
}

#[async_trait]
impl<H: SessionHosting> super::EnvironmentFactory for ImportedWorkspaceFactory<H> {
    async fn open(&self, session: SessionId) -> Result<crate::Environment> {
        let imported = match self.hosting.record(session, CUTOVER_WORKSPACE).await {
            Ok(imported) => imported,
            // Validation before a native session's creation has no record yet.
            Err(AgentLoopError::SessionNotFound(_)) => None,
            Err(error) => return Err(error),
        };
        let Some(imported) = imported else {
            return self
                .agent
                .default_session_environment(session)
                .await
                .map_err(|error| AgentLoopError::config(error.to_string()));
        };
        let binding: WorkspaceBinding = serde_json::from_value(imported)
            .map_err(|_| AgentLoopError::store("imported workspace binding is corrupt"))?;
        reopen_binding(&self.agent, session, binding).await
    }
}

/// Reopen exactly `binding` with `agent`'s workspace backends.
pub(crate) async fn reopen_binding(
    agent: &crate::Agent,
    session: SessionId,
    binding: WorkspaceBinding,
) -> Result<crate::Environment> {
    let store = everruns_core::host::InMemoryEnvironmentBindingStore::default();
    everruns_core::host::EnvironmentBindingStore::bind(&store, session, &binding)
        .await
        .map_err(|error| AgentLoopError::config(error.to_string()))?;
    let environment = agent
        .reopen_session_environment(&store, session)
        .await
        .map_err(|error| AgentLoopError::config(error.to_string()))?
        .ok_or_else(|| AgentLoopError::config("imported workspace binding is unavailable"))?;
    if environment.workspace_head().binding() != &binding {
        return Err(AgentLoopError::config(
            "reopened workspace head differs from the imported binding",
        ));
    }
    Ok(environment)
}
