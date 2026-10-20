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
use everruns_core::session_commands::history::{self, HistoryBranch};
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
    /// Imported conversation branches, the active one included; `0` for a
    /// linear import.
    pub branches: usize,
}

/// A compaction checkpoint produced on one branch of an imported lineage.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchCheckpoint {
    /// Branch whose context the checkpoint summarized.
    pub branch: String,
    /// The checkpoint; its source sequence must lie on that branch.
    pub checkpoint: CompactionCheckpoint,
}

/// Portable conversation lineage for a cutover (EVE-1305).
///
/// The source journal is imported whole; this value says which events form
/// each branch and which one is active. A downstream product parses its own
/// timeline into it: Everruns knows no product file names, checkpoint ids or
/// workspace restore policy. Branches are made of whole turns and identified
/// by opaque ids. Each checkpoint stays eligible only on branches that share
/// the lineage prefix it summarized. Stability: experimental.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversationLineage {
    /// Branch new input extends after the cutover.
    pub active: String,
    /// Every branch to retain, the active one included.
    pub branches: Vec<HistoryBranch>,
    /// Compaction checkpoints with the branch each one was produced on.
    pub checkpoints: Vec<BranchCheckpoint>,
}

impl ConversationLineage {
    /// Lineage with `active` among `branches` and no checkpoints.
    pub fn new(active: impl Into<String>, branches: Vec<HistoryBranch>) -> Self {
        Self {
            active: active.into(),
            branches,
            checkpoints: Vec::new(),
        }
    }

    /// Add a checkpoint produced on `branch`.
    pub fn with_checkpoint(
        mut self,
        branch: impl Into<String>,
        checkpoint: CompactionCheckpoint,
    ) -> Self {
        self.checkpoints.push(BranchCheckpoint {
            branch: branch.into(),
            checkpoint,
        });
        self
    }

    /// Retry identity: a repeated cutover must name the same lineage.
    pub(crate) fn fingerprint(&self) -> serde_json::Value {
        let value = serde_json::json!({
            "active": self.active,
            "branches": self.branches,
            "checkpoints": self.checkpoints.iter().map(|entry| serde_json::json!({
                "branch": entry.branch,
                "id": entry.checkpoint.id,
                "source_sequence": entry.checkpoint.source_sequence,
                "provider_type": entry.checkpoint.provider_type,
                "model": entry.checkpoint.model,
                "format_version": entry.checkpoint.format_version,
            })).collect::<Vec<_>>(),
        });
        serde_json::Value::String(history::json_digest(&value))
    }
}

/// Options for [`Engine::cut_over_legacy_session_with_options`](crate::Engine::cut_over_legacy_session_with_options).
/// Stability: experimental.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct CutoverOptions {
    /// Creation metadata recorded on the authoritative binding.
    pub metadata: serde_json::Value,
    /// Branch structure of the imported journal; `None` imports it linearly.
    pub lineage: Option<ConversationLineage>,
}

impl CutoverOptions {
    /// Linear import without metadata.
    pub fn new() -> Self {
        Self::default()
    }
    /// Record `metadata` on the binding, as session creation does.
    pub fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
    /// Import the journal with this branch structure.
    pub fn lineage(mut self, lineage: ConversationLineage) -> Self {
        self.lineage = Some(lineage);
        self
    }
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
    /// Branch structure of `events`; `None` is a linear journal.
    pub(crate) lineage: Option<ConversationLineage>,
}

#[cfg_attr(not(feature = "local"), allow(dead_code))]
impl LegacySessionImage {
    /// The committed decision. `decided_at` is not part of a retry's identity.
    pub(crate) fn record(&self) -> serde_json::Value {
        let mut record = serde_json::json!({
            "owner": "authority",
            "source": self.source,
            "binding": self.binding,
        });
        if let Some(lineage) = &self.lineage {
            record["lineage"] = lineage.fingerprint();
        }
        record
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
