//! Coherent session hosting below [`Engine`](crate::Engine).
//!
//! A hosting plugin owns admission, canonical state, execution epochs and the
//! wakeup obligation together. It never implements agent planning. Frontend
//! access is read-only; only a claimed epoch supplies runtime write ports.

/// Reusable host qualification scenarios; never enabled by application defaults.
#[cfg(all(feature = "hosting-conformance", not(target_arch = "wasm32")))]
pub mod conformance;
pub mod services;
pub mod spi;

mod admission;
mod configuration;
pub mod cutover;
#[cfg(not(target_arch = "wasm32"))]
pub use configuration::DirectoryEnvironmentFactory;
#[doc(hidden)]
pub use cutover::LegacySessionImage;
pub use cutover::{CutoverBoundary, CutoverReport, ImportedWorkspaceFactory};
pub use everruns_contracts::execution_api::ToolApprovalDecision;
pub use everruns_core::session_commands::{ApprovalCommit, SessionConfiguration};
pub(crate) mod execution;
mod memory;
mod retry;
pub use everruns_core::session_commands::{
    QuestionAnswersRequest, validate_question_answers, validate_tool_resolutions,
};
pub(crate) mod query;
pub use admission::{Admission, AdmissionState, TurnAssignment};
pub use memory::MemoryHosting;
pub use query::SessionQuerySnapshot;
#[cfg(feature = "local")]
mod sqlite;
#[cfg(feature = "local")]
pub use sqlite::SqliteHosting;
#[cfg(feature = "local")]
pub(crate) use sqlite::task_is_running_elsewhere;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spi::{
    HostBackends, RuntimeMessage,
    actor::{ActorFence, LeasedActor, SessionActorStore},
    error::Result,
    events::ToolCompletedData,
    ids::{MessageId, SessionId, TurnId},
};
use std::{sync::Arc, time::Duration};

/// Trusted host factory for a retained, session-scoped Environment. It may
/// reconnect to durable workspace/compute services; it must be idempotent for
/// the session identity and must not perform agent lifecycle effects.
#[async_trait]
pub trait EnvironmentFactory: Send + Sync + 'static {
    /// Reopen the same workspace and permissions for this session.
    async fn open(&self, session: SessionId) -> Result<crate::Environment>;
}
#[async_trait]
impl<F, Fut> EnvironmentFactory for F
where
    F: Fn(SessionId) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<crate::Environment>> + Send,
{
    async fn open(&self, session: SessionId) -> Result<crate::Environment> {
        self(session).await
    }
}

/// Immutable, secret-free reference to trusted executable behavior.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentBuild {
    /// Application registration key.
    pub agent: String,
    /// Retained Agent behavior/configuration version, independent of deployment
    /// revisions and compatible Engine library upgrades.
    pub build_id: String,
}

/// Persisted Framework runtime format understood by this Engine. This is not
/// a crate version or deployment digest; bump only for incompatible state changes.
pub const SESSION_RUNTIME_VERSION: u32 = 1;

/// Whether this Engine can reconstruct a session's exact retained behavior.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SessionCompatibility {
    /// The runtime format and all immutable registrations are available.
    Available,
    /// Restore this Agent registration; never substitute the latest build.
    MissingBuild {
        /// Exact Agent behavior required by the session.
        required: AgentBuild,
    },
    /// Route to a compatible Engine or perform a separately checked migration.
    UnsupportedRuntimeVersion {
        /// Persisted format requested by the session.
        required: u32,
        /// Format this Engine understands.
        supported: u32,
    },
    /// Restore the registered Harness or Environment definition.
    ConfigurationUnavailable,
}

impl SessionCompatibility {
    /// Whether activation can safely reconstruct the retained registration.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub(crate) fn suspension_reason(&self) -> &'static str {
        match self {
            Self::Available => "compatible",
            Self::MissingBuild { .. } => "retained_build_unavailable",
            Self::UnsupportedRuntimeVersion { .. } => "unsupported_runtime_version",
            Self::ConfigurationUnavailable => "retained_configuration_unavailable",
        }
    }
}

impl AgentBuild {
    /// Name trusted behavior without serializing its closures or secrets.
    pub fn new(agent: impl Into<String>, build_id: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            build_id: build_id.into(),
        }
    }
}

/// Authoritative session binding. All optional configuration is immutable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionBinding {
    /// Persisted Framework runtime format, independent of Agent and deployment
    /// versions. Existing version-one bindings remain readable unchanged.
    pub framework: u32,
    #[serde(flatten)]
    /// Registered executable behavior.
    pub behavior: AgentBuild,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Portable immutable Harness definition.
    pub harness: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Retained Environment factory key, when explicitly selected.
    pub environment: Option<String>,
    /// Optional creation metadata retained for rebuilding host catalogs.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

/// Admission is distinct from consumption and turn assignment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdmissionReceipt {
    /// Caller-retained command identity.
    pub message_id: MessageId,
    /// Canonical input sequence.
    pub sequence: i32,
}

/// A claimed epoch, allocated exclusively by the authority.
pub struct SessionActivation {
    /// Binding read in the claim transaction.
    pub binding: SessionBinding,
    /// Claimed actor and inbox cursors.
    pub actor: LeasedActor,
    /// Capability for exactly this ownership epoch.
    pub epoch: Arc<dyn SessionEpoch>,
}

/// Authority status; suspension is observable without another execution attempt.
#[derive(Clone, Debug)]
pub struct HostingStatus {
    /// An execution epoch is currently leased according to the authority clock.
    pub executing: bool,
    /// Authority clock used for interpreting persisted deadlines.
    pub now_ms: i64,
    /// Next independent activation deadline, even while parked.
    pub wake_at: Option<i64>,
    /// Authority-allocated creation timestamp, stable across retries.
    pub created_at: String,
    /// Authoritative immutable binding.
    pub binding: SessionBinding,
    /// Work remains owed by the authority.
    pub runnable: bool,
    /// Host intervention is required before execution resumes.
    pub suspended: bool,
    /// Durable diagnostic code when host intervention is required.
    pub suspension_reason: Option<String>,
    /// Logical turn still open in the journal, including a parked turn.
    pub open_turn: Option<TurnId>,
    /// Client calls whose durable resolutions remain outstanding.
    pub waiting: Vec<crate::ToolCall>,
}

/// Whether a host needs compute now, at a retained deadline, or only after a
/// new external event. This is derived state, not another scheduling authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeState {
    /// Execution is running or runnable now.
    Active,
    /// No active compute is needed until this authority-clock deadline.
    At(i64),
    /// No scheduled compute; external input or intervention is required.
    Dormant,
}

impl HostingStatus {
    /// Derive the compute requirement without claiming or activating execution.
    pub fn wake(&self) -> WakeState {
        if self.suspended {
            WakeState::Dormant
        } else if self.executing
            || self.runnable
            || self.wake_at.is_some_and(|at| at <= self.now_ms)
        {
            WakeState::Active
        } else if let Some(at) = self.wake_at {
            WakeState::At(at)
        } else {
            WakeState::Dormant
        }
    }

    /// Stable execution API phase, derived from authoritative obligations.
    pub fn phase(&self) -> &'static str {
        if self.suspended {
            "suspended"
        } else if !self.waiting.is_empty() {
            "waitingfortoolresults"
        } else if self.runnable {
            "active"
        } else {
            "idle"
        }
    }
}

/// One coherent authority and its activation policy.
#[async_trait]
pub trait SessionHosting: Send + Sync + 'static {
    /// Execute a semantic command in the authority transaction using the shared
    /// Rust reducer. Remote hosts transport the command, never unchecked effects.
    async fn command(
        &self,
        session: SessionId,
        command: spi::session_commands::SessionCommand,
    ) -> Result<Value>;

    /// Resolve mixed approvals and client answers on one retained turn in one
    /// transaction. This is the command boundary for protocol resume batches.
    async fn resolve_waits(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        target: TurnId,
        approvals: Vec<ApprovalCommit>,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        self.command(
            session,
            spi::session_commands::SessionCommand::ResolveWaits {
                id: command.to_string(),
                turn: target,
                approvals,
                results,
            },
        )
        .await
        .map(|_| ())
    }
    /// Resolve an Execution API approval batch atomically and retain its response.
    async fn submit_approvals(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        turn: TurnId,
        decisions: Vec<crate::execution_api::ToolApprovalAnswer>,
    ) -> Result<crate::execution_api::SubmitToolApprovalsResponse> {
        serde_json::from_value(
            self.command(
                session,
                spi::session_commands::SessionCommand::SubmitApprovals {
                    id: command.to_string(),
                    turn,
                    decisions,
                },
            )
            .await?,
        )
        .map_err(|e| spi::error::AgentLoopError::store(e.to_string()))
    }
    /// Apply an idempotent patch to client configuration under the authority lock.
    async fn configure(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        patch: SessionConfiguration,
    ) -> Result<()> {
        self.command(
            session,
            spi::session_commands::SessionCommand::Configure {
                id: command.to_string(),
                patch,
            },
        )
        .await
        .map(|_| ())
    }
    /// Atomically import prior conversation only while canonical conversation is empty.
    async fn seed_history(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        messages: Vec<RuntimeMessage>,
    ) -> Result<bool> {
        serde_json::from_value(
            self.command(
                session,
                spi::session_commands::SessionCommand::Seed {
                    id: command.to_string(),
                    messages,
                },
            )
            .await?,
        )
        .map_err(|e| spi::error::AgentLoopError::store(e.to_string()))
    }
    /// Capture a read boundary and execution settings in one authority transaction.
    async fn query_snapshot(&self, session: SessionId) -> Result<SessionQuerySnapshot>;
    /// Resolve an opaque event identity without assuming UUID time order.
    /// Persistent plugins should index this lookup. The default pages a
    /// pinned snapshot with bounded memory for small/volatile authorities.
    async fn event_sequence(&self, session: SessionId, event_id: &str) -> Result<Option<i32>> {
        use spi::{EventReadLimit, EventReadRequest};
        let log = self.reader(session).event_log;
        let mut cursor = None;
        loop {
            let mut query = EventReadRequest::new(
                session,
                EventReadLimit::new(256)
                    .map_err(|error| spi::error::AgentLoopError::store(error.to_string()))?,
            );
            if let Some(cursor) = cursor.take() {
                query = query.with_cursor(cursor);
            }
            let page = log
                .read_page(query)
                .await
                .map_err(|error| spi::error::AgentLoopError::store(error.to_string()))?;
            if let Some(event) = page
                .events
                .iter()
                .find(|event| event.id.to_string() == event_id)
            {
                return Ok(event.sequence);
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(None),
            }
        }
    }
    /// Commit a canonical approval decision, correlated answer and wakeup.
    async fn approve_many(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        approvals: Vec<ApprovalCommit>,
    ) -> Result<()> {
        self.command(
            session,
            spi::session_commands::SessionCommand::Approve {
                id: command.to_string(),
                approvals,
            },
        )
        .await
        .map(|_| ())
    }
    /// Same id and binding succeeds; conflicting reuse fails atomically.
    async fn create(&self, session: SessionId, binding: &SessionBinding) -> Result<()> {
        self.command(
            session,
            spi::session_commands::SessionCommand::Create {
                binding: serde_json::to_value(binding)
                    .map_err(|e| spi::error::AgentLoopError::config(e.to_string()))?,
            },
        )
        .await
        .map(|_| ())
    }
    /// Read binding and scheduling state.
    async fn status(&self, session: SessionId) -> Result<HostingStatus>;
    /// Commits the input, deduplicated receipt and wakeup obligation together.
    async fn admit(
        &self,
        session: SessionId,
        message: RuntimeMessage,
        cancelled: bool,
    ) -> Result<AdmissionReceipt> {
        serde_json::from_value(
            self.command(
                session,
                spi::session_commands::SessionCommand::Admit {
                    message: Box::new(message),
                    cancelled,
                },
            )
            .await?,
        )
        .map_err(|e| spi::error::AgentLoopError::store(e.to_string()))
    }
    /// Reopen an accepted command without submitting its payload again.
    async fn receipt(
        &self,
        session: SessionId,
        message: MessageId,
    ) -> Result<Option<AdmissionReceipt>>;
    /// A read-only bundle. Canonical mutations must be refused by these ports.
    fn reader(&self, session: SessionId) -> HostBackends;
    /// Framework receipts share the authority's consistency domain.
    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>>;
    /// Commit matching pending tool results and preserve the wakeup obligation.
    async fn resolve(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        target: TurnId,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        self.command(
            session,
            spi::session_commands::SessionCommand::Resolve {
                id: command.to_string(),
                turn: target,
                results,
            },
        )
        .await
        .map(|_| ())
    }
    /// Target a retained admission, never whichever turn happens to run later.
    async fn cancel(
        &self,
        session: SessionId,
        command: uuid::Uuid,
        input: MessageId,
    ) -> Result<()> {
        self.command(
            session,
            spi::session_commands::SessionCommand::Cancel {
                id: command.to_string(),
                input,
            },
        )
        .await
        .map(|_| ())
    }
    /// Acquire a known session using the authority clock and monotonically increasing epoch.
    async fn claim(
        &self,
        session: SessionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<SessionActivation>>;
    /// Local profiles may also use Engine's process scheduler for latency.
    /// Durable profiles must arrange independent startup/expiry/retry wakeups;
    /// this optimization never supplies their persistence guarantee.
    fn process_scheduler(&self) -> bool {
        false
    }
    /// Optional process-local hint, signalled after this session's receipts or
    /// control state change in this process. It carries no authority: an
    /// [`Admission`] waiter re-reads receipts on every signal and still polls,
    /// so writes from other processes or remote hosts are seen without it.
    /// Without the hint every waiter pays up to one poll interval per turn.
    fn changes(&self, _session: SessionId) -> Option<Arc<tokio::sync::Notify>> {
        None
    }
    /// Attach profile-owned host services (task registry, child sessions,
    /// wakes) to an activation's runtime after reconciling lost executions.
    /// Profiles without such services keep the defaults.
    #[doc(hidden)]
    async fn prepare_activation(
        &self,
        _engine: &crate::Engine,
        _session: SessionId,
        _environment: &crate::Environment,
        backends: HostBackends,
    ) -> Result<HostBackends> {
        Ok(backends)
    }
    /// Commit a quiescent legacy session, its source boundary and ownership
    /// decision in one transaction (EVE-1298). Returns `true` when an
    /// identical cutover was already committed. Only the built-in SQLite
    /// authority supports it; reached through
    /// [`Engine::cut_over_legacy_session`](crate::Engine::cut_over_legacy_session).
    #[doc(hidden)]
    async fn import_legacy(&self, _session: SessionId, _image: LegacySessionImage) -> Result<bool> {
        Err(spi::error::AgentLoopError::config(
            "this hosting profile does not accept legacy session cutover",
        ))
    }
}

/// Per-session [`SessionHosting::changes`] hints for hosts whose sessions have
/// no in-process object to hold one. Entries are weak, so a session nobody
/// waits on costs nothing; a signal with no waiter is dropped, which is safe
/// because waiters read receipts after subscribing.
#[derive(Clone, Default)]
#[cfg_attr(not(feature = "local"), allow(dead_code))]
pub(crate) struct ChangeHints(
    Arc<
        std::sync::Mutex<
            std::collections::HashMap<SessionId, std::sync::Weak<tokio::sync::Notify>>,
        >,
    >,
);

#[cfg_attr(not(feature = "local"), allow(dead_code))]
impl ChangeHints {
    pub(crate) fn subscribe(&self, session: SessionId) -> Arc<tokio::sync::Notify> {
        let mut hints = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(hint) = hints.get(&session).and_then(std::sync::Weak::upgrade) {
            return hint;
        }
        hints.retain(|_, hint| hint.strong_count() > 0);
        let hint = Arc::new(tokio::sync::Notify::new());
        hints.insert(session, Arc::downgrade(&hint));
        hint
    }

    pub(crate) fn notify(&self, session: SessionId) {
        let hint = self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .and_then(std::sync::Weak::upgrade);
        if let Some(hint) = hint {
            hint.notify_waiters();
        }
    }
}

/// Write capability scoped to one unexpired authority epoch.
#[async_trait]
pub trait SessionEpoch: Send + Sync + 'static {
    /// Optional local hint for prompt control observation. Remote ownership
    /// still observes commands on renewal; hints carry no authority.
    fn renew_poke(&self) -> Option<Arc<tokio::sync::Notify>> {
        None
    }
    /// Retain a timer obligation under the current epoch. Its activation is
    /// independent of frontend traffic; only Rust interprets expired waits.
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()>;
    /// Coherent write ports, all fenced at mutation time.
    fn backends(&self) -> HostBackends;
    /// Driver renewal, release and bounded journal access.
    fn actor_store(&self) -> Arc<dyn SessionActorStore>;
    /// Connect refused writes to the driver's ownership-loss signal.
    fn observe_fence(&self, fence: &ActorFence);
    /// Read one lifecycle, assignment or outcome receipt.
    async fn record(&self, key: &str) -> Result<Option<Value>>;
    /// Bounded receipt scan used to recover per-turn lifecycle failures.
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>>;
    /// With `absent`, atomically insert only if the key does not exist.
    /// Returns false on an existing key. Every mutation checks the epoch.
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool>;
    /// Persist a visible suspension instead of retrying incompatible behavior.
    async fn suspend(&self, reason: &str) -> Result<()>;
}
