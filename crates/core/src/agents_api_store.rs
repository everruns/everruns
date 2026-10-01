//! Durable orchestration state for the opt-in OpenAI Agents API runtime backend.
//!
//! OpenAI owns the agent loop; Everruns owns the record. One checkpoint per
//! Everruns session holds the provider session id, the current turn's stream
//! cursor, provider-to-local id correlations, and the input and tool-result
//! outboxes. A worker writes it ahead of every provider call and every local
//! effect, so a replacement worker can reconcile instead of repeating work.
//! Design: `knowledge/execution/openai-agents-api-runtime.md`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Maximum time a worker may own a session's provider loop without renewal.
pub const AGENTS_API_LEASE_SECONDS: i64 = 60;
/// Checkpoint size bound below the internal RPC transport limit.
pub const MAX_AGENTS_API_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;

/// Exclusive ownership of one Everruns session's provider loop.
///
/// The lease is per session, not per turn: an Agents API session is durable
/// across turns, and only one worker may submit inputs or tool results to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentsApiLease {
    /// Owning tenant.
    pub org_id: i64,
    /// Everruns session whose provider loop is owned.
    pub session_id: SessionId,
    /// Fencing token for this acquisition; never reused by a new owner.
    pub owner: Uuid,
}

/// Durable state for one Everruns session bound to one Agents API session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentsApiCheckpoint {
    /// Provider session id, saved before any event of that session is applied.
    #[serde(default)]
    pub provider_session_id: Option<String>,
    /// Set before `POST /agents/sessions`. The create call is not idempotent
    /// at the provider, so recovery adopts a session whose metadata carries
    /// this attempt instead of creating a second one.
    #[serde(default)]
    pub create_attempt: Option<String>,
    /// Digest of the agent definition the provider session was created with.
    /// A changed definition starts a new provider session.
    #[serde(default)]
    pub agent_fingerprint: Option<String>,
    /// The turn currently or most recently driven through the provider.
    #[serde(default)]
    pub turn: Option<AgentsApiTurnCheckpoint>,
}

impl AgentsApiCheckpoint {
    /// The turn checkpoint for `turn_id`, replacing any earlier turn's state.
    /// Earlier turns are finished: their effects are already in the event log.
    pub fn turn_mut(
        &mut self,
        turn_id: TurnId,
        input_message_id: MessageId,
    ) -> &mut AgentsApiTurnCheckpoint {
        if self
            .turn
            .as_ref()
            .is_none_or(|turn| turn.turn_id != turn_id)
        {
            self.turn = Some(AgentsApiTurnCheckpoint::new(turn_id, input_message_id));
        }
        self.turn.as_mut().expect("turn initialized above")
    }
}

/// Durable state of one Everruns turn driven by the provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentsApiTurnCheckpoint {
    /// Everruns turn.
    pub turn_id: TurnId,
    /// Input message that started the turn.
    pub input_message_id: MessageId,
    /// Input outbox. `None` until the turn's user input is staged.
    #[serde(default)]
    pub input: Option<InputOutbox>,
    /// Root provider turns that existed before this turn's input was sent.
    /// A root turn outside this set is the one the input created.
    #[serde(default)]
    pub prior_provider_turns: Vec<String>,
    /// Provider root turn this Everruns turn maps to, once observed.
    #[serde(default)]
    pub provider_turn_id: Option<String>,
    /// Stream cursor: last provider event id applied to this checkpoint.
    /// Provider streams cannot resume from it; reconciliation uses saved
    /// items. It records how far the live projection got.
    #[serde(default)]
    pub last_event_id: Option<String>,
    /// Provider item id to the local record it produced.
    #[serde(default)]
    pub items: BTreeMap<String, ItemCorrelation>,
    /// Client function call id to its tool-result outbox entry.
    #[serde(default)]
    pub tool_results: BTreeMap<String, ToolResultOutbox>,
    /// Final host outcome, retained so a replayed activity returns it without
    /// another provider call.
    #[serde(default)]
    pub outcome: Option<Value>,
}

impl AgentsApiTurnCheckpoint {
    /// Empty state for a turn that has not reached the provider yet.
    pub fn new(turn_id: TurnId, input_message_id: MessageId) -> Self {
        Self {
            turn_id,
            input_message_id,
            input: None,
            prior_provider_turns: Vec::new(),
            provider_turn_id: None,
            last_event_id: None,
            items: BTreeMap::new(),
            tool_results: BTreeMap::new(),
            outcome: None,
        }
    }
}

/// Input outbox entry for the turn's user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputOutbox {
    /// Sent as `Idempotency-Key`; the provider deduplicates repeated input
    /// events carrying the same key.
    pub idempotency_key: String,
    /// Delivery state.
    pub state: OutboxState,
}

/// Delivery state shared by the input and tool-result outboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    /// Staged; the provider may or may not have received it.
    Pending,
    /// The provider acknowledged it, or reconciliation observed its effect.
    Delivered,
}

/// What one provider item produced locally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemCorrelation {
    /// What the item is.
    pub kind: ItemKind,
    /// Local message id (messages) or tool call id (tools).
    pub local_id: String,
    /// How far its local record got.
    pub state: ItemState,
}

/// Kind of provider item correlated with a local record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// Assistant text message.
    Message,
    /// Client function call, executed by Everruns.
    FunctionCall,
    /// Remote MCP call, executed by the provider.
    McpCall,
}

/// Write-ahead state of an item's local record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    /// Seen; start events emitted at most once.
    Open,
    /// The completing event is about to be emitted. After a crash the host
    /// checks the event log before emitting it again.
    Completing,
    /// The completing event is in the event log.
    Completed,
}

/// Tool-result outbox entry for one client function call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultOutbox {
    /// Provider turn the call belongs to.
    pub provider_turn_id: String,
    /// Provider call id, also the local tool call id.
    pub call_id: String,
    /// Function name.
    pub name: String,
    /// Function arguments as the provider sent them.
    pub arguments: Value,
    /// Execution and delivery state.
    pub state: ToolResultState,
}

/// Execution and delivery state of one client function call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ToolResultState {
    /// Claimed for local execution. A replacement worker re-enters the
    /// ordinary tool pipeline, whose durable per-call claim decides whether
    /// the tool may run again.
    Executing,
    /// Result recorded locally, not yet acknowledged by the provider.
    Ready {
        /// Whether the call succeeded.
        success: bool,
        /// Output, or the error text when it failed.
        output: String,
    },
    /// The provider acknowledged the result or already holds its output item.
    Submitted {
        /// Whether the call succeeded.
        success: bool,
        /// Output, or the error text when it failed.
        output: String,
    },
}

/// Durable, tenant-scoped storage with exclusive expiring ownership.
#[async_trait]
pub trait AgentsApiStore: Send + Sync {
    /// Acquire an absent or expired lease, keeping the previous owner's
    /// checkpoint. Re-acquiring with the same live token is idempotent.
    async fn acquire(&self, lease: AgentsApiLease) -> Result<AgentsApiCheckpoint>;
    /// Renew the live lease; an expired owner cannot resurrect itself.
    async fn renew(&self, lease: AgentsApiLease) -> Result<()>;
    /// Atomically persist the checkpoint and renew the live lease.
    async fn save(&self, lease: AgentsApiLease, checkpoint: &AgentsApiCheckpoint) -> Result<()>;
    /// Release this owner's live lease; the checkpoint stays.
    async fn release(&self, lease: AgentsApiLease) -> Result<()>;
}

fn fence_error() -> AgentLoopError {
    AgentLoopError::store("agents api ownership fence lost or unavailable")
}

struct MemoryRow {
    org_id: i64,
    owner: Uuid,
    lease_until: Instant,
    checkpoint: AgentsApiCheckpoint,
}

/// Process-local store for tests and single-process hosts. It enforces the
/// same ownership fence as the PostgreSQL store but does not survive a
/// process restart.
#[derive(Default)]
pub struct InMemoryAgentsApiStore {
    rows: Mutex<HashMap<SessionId, MemoryRow>>,
}

impl InMemoryAgentsApiStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current checkpoint regardless of ownership (test inspection).
    pub fn snapshot(&self, session_id: SessionId) -> Option<AgentsApiCheckpoint> {
        self.rows
            .lock()
            .ok()?
            .get(&session_id)
            .map(|row| row.checkpoint.clone())
    }

    /// Expire the live lease, as if its worker died (test support).
    pub fn expire(&self, session_id: SessionId) {
        if let Ok(mut rows) = self.rows.lock()
            && let Some(row) = rows.get_mut(&session_id)
        {
            row.lease_until = Instant::now();
        }
    }

    fn live<'a>(
        rows: &'a mut HashMap<SessionId, MemoryRow>,
        lease: &AgentsApiLease,
    ) -> Result<&'a mut MemoryRow> {
        rows.get_mut(&lease.session_id)
            .filter(|row| {
                row.org_id == lease.org_id
                    && row.owner == lease.owner
                    && row.lease_until > Instant::now()
            })
            .ok_or_else(fence_error)
    }
}

fn lease_duration() -> Duration {
    Duration::from_secs(AGENTS_API_LEASE_SECONDS as u64)
}

#[async_trait]
impl AgentsApiStore for InMemoryAgentsApiStore {
    async fn acquire(&self, lease: AgentsApiLease) -> Result<AgentsApiCheckpoint> {
        let mut rows = self.rows.lock().map_err(|_| fence_error())?;
        let now = Instant::now();
        let row = rows.entry(lease.session_id).or_insert_with(|| MemoryRow {
            org_id: lease.org_id,
            owner: lease.owner,
            lease_until: now,
            checkpoint: AgentsApiCheckpoint::default(),
        });
        if row.org_id != lease.org_id || (row.owner != lease.owner && row.lease_until > now) {
            return Err(fence_error());
        }
        row.owner = lease.owner;
        row.lease_until = now + lease_duration();
        Ok(row.checkpoint.clone())
    }

    async fn renew(&self, lease: AgentsApiLease) -> Result<()> {
        let mut rows = self.rows.lock().map_err(|_| fence_error())?;
        Self::live(&mut rows, &lease)?.lease_until = Instant::now() + lease_duration();
        Ok(())
    }

    async fn save(&self, lease: AgentsApiLease, checkpoint: &AgentsApiCheckpoint) -> Result<()> {
        let mut rows = self.rows.lock().map_err(|_| fence_error())?;
        let row = Self::live(&mut rows, &lease)?;
        row.checkpoint = checkpoint.clone();
        row.lease_until = Instant::now() + lease_duration();
        Ok(())
    }

    async fn release(&self, lease: AgentsApiLease) -> Result<()> {
        let mut rows = self.rows.lock().map_err(|_| fence_error())?;
        Self::live(&mut rows, &lease)?.lease_until = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(owner: Uuid) -> AgentsApiLease {
        AgentsApiLease {
            org_id: 7,
            session_id: SessionId::from_seed(1),
            owner,
        }
    }

    #[tokio::test]
    async fn memory_store_fences_competing_owners_and_keeps_checkpoint() {
        let store = InMemoryAgentsApiStore::new();
        let first = lease(Uuid::new_v4());
        let second = lease(Uuid::new_v4());
        let mut checkpoint = store.acquire(first).await.unwrap();
        checkpoint.provider_session_id = Some("sess_1".into());
        store.save(first, &checkpoint).await.unwrap();
        assert!(
            store.acquire(second).await.is_err(),
            "live lease is exclusive"
        );

        store.expire(first.session_id);
        assert!(
            store.save(first, &checkpoint).await.is_err(),
            "expired owner is fenced"
        );
        let recovered = store.acquire(second).await.unwrap();
        assert_eq!(recovered.provider_session_id.as_deref(), Some("sess_1"));
        assert!(store.renew(first).await.is_err());

        let other_org = AgentsApiLease {
            org_id: 8,
            ..second
        };
        assert!(
            store.acquire(other_org).await.is_err(),
            "tenant is part of the fence"
        );
    }

    #[test]
    fn a_new_turn_replaces_the_previous_turn_state() {
        let mut checkpoint = AgentsApiCheckpoint::default();
        let first = TurnId::from_seed(1);
        checkpoint
            .turn_mut(first, MessageId::from_seed(1))
            .provider_turn_id = Some("t1".into());
        assert_eq!(
            checkpoint
                .turn_mut(first, MessageId::from_seed(1))
                .provider_turn_id
                .as_deref(),
            Some("t1")
        );
        let second = checkpoint.turn_mut(TurnId::from_seed(2), MessageId::from_seed(2));
        assert!(second.provider_turn_id.is_none());
    }
}
