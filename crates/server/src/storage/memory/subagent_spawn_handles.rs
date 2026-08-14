// In-memory SubagentSpawnStore implementation for dev mode (EVE-535).

use async_trait::async_trait;
use everruns_core::error::AgentLoopError;
use everruns_core::typed_id::SessionId;
use everruns_core::{
    delegation_services::SpawnClaimResult, delegation_services::SubagentSpawnStore,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone)]
struct SpawnHandleEntry {
    spawn_handle_id: Uuid,
    child_session_id: Option<SessionId>,
    claim_token: Uuid,
    terminal_status: Option<String>,
    terminal_result: Option<String>,
    status: SpawnStatus,
}

#[derive(Clone, PartialEq)]
enum SpawnStatus {
    Pending,
    Running,
    Settled,
}

/// In-memory SubagentSpawnStore — mirrors `subagent_spawn_handles` table logic.
///
/// Thread-safe via `Mutex`. CAS semantics are identical to the Postgres impl:
/// first `try_claim_spawn` call for a given `(parent, tool_call_id)` pair wins;
/// subsequent calls return the appropriate reattach variant.
pub struct InMemorySubagentSpawnStore {
    handles: Mutex<HashMap<(SessionId, String), SpawnHandleEntry>>,
}

impl InMemorySubagentSpawnStore {
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemorySubagentSpawnStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SubagentSpawnStore for InMemorySubagentSpawnStore {
    async fn try_claim_spawn(
        &self,
        parent_session_id: SessionId,
        tool_call_id: &str,
        claim_token: Uuid,
    ) -> Result<SpawnClaimResult, AgentLoopError> {
        let key = (parent_session_id, tool_call_id.to_string());
        let mut handles = self.handles.lock();

        if let Some(entry) = handles.get(&key) {
            return Ok(match &entry.status {
                SpawnStatus::Pending => SpawnClaimResult::ClaimedPendingChild {
                    spawn_handle_id: entry.spawn_handle_id,
                    claim_token: entry.claim_token,
                },
                SpawnStatus::Running => SpawnClaimResult::AlreadyRunning {
                    child_session_id: entry
                        .child_session_id
                        .expect("running entry must have child"),
                    claim_token: entry.claim_token,
                },
                SpawnStatus::Settled => SpawnClaimResult::AlreadySettled {
                    child_session_id: entry
                        .child_session_id
                        .expect("settled entry must have child"),
                    terminal_status: entry
                        .terminal_status
                        .clone()
                        .unwrap_or_else(|| "idle".to_string()),
                    terminal_result: entry.terminal_result.clone().unwrap_or_default(),
                },
            });
        }

        let spawn_handle_id = Uuid::new_v4();
        handles.insert(
            key,
            SpawnHandleEntry {
                spawn_handle_id,
                child_session_id: None,
                claim_token,
                terminal_status: None,
                terminal_result: None,
                status: SpawnStatus::Pending,
            },
        );

        Ok(SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        })
    }

    async fn register_child_session(
        &self,
        spawn_handle_id: Uuid,
        claim_token: Uuid,
        child_session_id: SessionId,
    ) -> Result<(), AgentLoopError> {
        let mut handles = self.handles.lock();
        if let Some(entry) = handles
            .values_mut()
            .find(|e| e.spawn_handle_id == spawn_handle_id && e.claim_token == claim_token)
            && entry.status == SpawnStatus::Pending
        {
            entry.child_session_id = Some(child_session_id);
            entry.status = SpawnStatus::Running;
        }
        Ok(())
    }

    async fn settle_spawn(
        &self,
        parent_session_id: SessionId,
        tool_call_id: &str,
        claim_token: Uuid,
        terminal_status: &str,
        terminal_result: &str,
    ) -> Result<(), AgentLoopError> {
        let key = (parent_session_id, tool_call_id.to_string());
        let mut handles = self.handles.lock();

        if let Some(entry) = handles.get_mut(&key)
            && entry.claim_token == claim_token
            && entry.status == SpawnStatus::Running
        {
            entry.status = SpawnStatus::Settled;
            entry.terminal_status = Some(terminal_status.to_string());
            entry.terminal_result = Some(terminal_result.to_string());
        }

        Ok(())
    }
}
