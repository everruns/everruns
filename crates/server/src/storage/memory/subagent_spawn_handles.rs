// In-memory SubagentSpawnStore implementation for dev mode (EVE-535).

use async_trait::async_trait;
use everruns_core::error::AgentLoopError;
use everruns_core::traits::{SpawnClaimResult, SubagentSpawnStore};
use everruns_core::typed_id::SessionId;
use parking_lot::Mutex;
use std::collections::HashMap;
use uuid::Uuid;

/// Entry stored in the in-memory spawn handles map.
#[derive(Clone)]
struct SpawnHandleEntry {
    child_session_id: SessionId,
    claim_token: Uuid,
    terminal_result: Option<String>,
    settled: bool,
}

/// In-memory SubagentSpawnStore — mirrors `subagent_spawn_handles` table logic.
///
/// Thread-safe via `Mutex`. CAS semantics are identical to the Postgres impl:
/// first `try_claim_spawn` call for a given `(parent, tool_call_id)` pair wins;
/// subsequent calls return `AlreadySpawned`.
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
        child_session_id: SessionId,
        _subagent_name: &str,
        _subagent_task: &str,
        claim_token: Uuid,
    ) -> Result<SpawnClaimResult, AgentLoopError> {
        let key = (parent_session_id, tool_call_id.to_string());
        let mut handles = self.handles.lock();

        if let Some(entry) = handles.get(&key) {
            let terminal_result = if entry.settled {
                entry.terminal_result.clone()
            } else {
                None
            };
            return Ok(SpawnClaimResult::AlreadySpawned {
                child_session_id: entry.child_session_id,
                terminal_result,
            });
        }

        let spawn_handle_id = Uuid::new_v4();
        handles.insert(
            key,
            SpawnHandleEntry {
                child_session_id,
                claim_token,
                terminal_result: None,
                settled: false,
            },
        );

        Ok(SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        })
    }

    async fn settle_spawn(
        &self,
        parent_session_id: SessionId,
        tool_call_id: &str,
        claim_token: Uuid,
        terminal_result: &str,
    ) -> Result<(), AgentLoopError> {
        let key = (parent_session_id, tool_call_id.to_string());
        let mut handles = self.handles.lock();

        if let Some(entry) = handles.get_mut(&key) {
            if entry.claim_token == claim_token && !entry.settled {
                entry.settled = true;
                entry.terminal_result = Some(terminal_result.to_string());
            }
        }

        Ok(())
    }
}
