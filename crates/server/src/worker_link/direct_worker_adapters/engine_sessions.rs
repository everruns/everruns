//! Engine sessions for the in-process worker (DEV_MODE): the same claim and
//! repository calls the gRPC handlers in `grpc_service/worker/engine_sessions.rs`
//! make, without the wire.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_worker::engine_sessions::{EngineBinding, EngineClaim, EngineFence, EngineSessions};
use serde_json::Value;

use super::store_error;
use crate::domains::agent_definitions::sessions;
use crate::storage::StorageBackend;

/// Most receipts one prefix read returns (as over gRPC).
const MAX_ENGINE_RECORDS: i64 = 1000;

fn fence(fence: &EngineFence) -> Result<i64> {
    i64::try_from(fence.fence)
        .map_err(|_| AgentLoopError::store(format!("fence {} out of range", fence.fence)))
}

/// [`EngineSessions`] on this process's storage backend.
pub(crate) struct DirectEngineSessions {
    pub(crate) db: Arc<StorageBackend>,
}

#[async_trait]
impl EngineSessions for DirectEngineSessions {
    async fn claim(&self, session: SessionId, lease: &EngineFence) -> Result<EngineClaim> {
        let claimed = sessions::claim(&self.db, session, &lease.holder, fence(lease)?)
            .await
            .map_err(|e| store_error(format!("Failed to claim Engine session: {e}")))?;
        Ok(match claimed {
            sessions::EngineClaim::Lost => EngineClaim::Lost,
            sessions::EngineClaim::Legacy => EngineClaim::Legacy,
            sessions::EngineClaim::Bound(session) => EngineClaim::Bound(Box::new(EngineBinding {
                agent_key: session.binding.agent_key,
                revision: session.binding.revision,
                runtime: u32::try_from(session.binding.runtime).unwrap_or(0),
                suspended_reason: session.binding.suspended_reason,
                wake_at_ms: session.binding.wake_at.map(|at| at.timestamp_millis()),
                configuration: session.configuration,
                created_at: session.created_at.to_rfc3339(),
            })),
        })
    }

    async fn definition(&self, agent_key: &str, revision: &str) -> Result<Option<Value>> {
        self.db
            .engine_agent_definition(agent_key, revision)
            .await
            .map_err(|e| store_error(format!("Failed to read Engine agent definition: {e}")))
    }

    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>> {
        self.db
            .engine_session_record(session, key)
            .await
            .map_err(|e| store_error(format!("Failed to read Engine receipt: {e}")))
    }

    async fn records(&self, session: SessionId, prefix: &str) -> Result<Vec<(String, Value)>> {
        self.db
            .engine_session_records(session, prefix, MAX_ENGINE_RECORDS)
            .await
            .map_err(|e| store_error(format!("Failed to read Engine receipts: {e}")))
    }

    async fn commit(
        &self,
        session: SessionId,
        lease: &EngineFence,
        key: &str,
        value: &Value,
        absent: bool,
    ) -> Result<Option<bool>> {
        self.db
            .commit_engine_session_record(session, &lease.holder, fence(lease)?, key, value, absent)
            .await
            .map_err(|e| store_error(format!("Failed to write Engine receipt: {e}")))
    }

    async fn suspend(&self, session: SessionId, lease: &EngineFence, reason: &str) -> Result<bool> {
        self.db
            .suspend_engine_session(session, &lease.holder, fence(lease)?, reason)
            .await
            .map_err(|e| store_error(format!("Failed to suspend Engine session: {e}")))
    }

    async fn schedule(
        &self,
        session: SessionId,
        lease: &EngineFence,
        wake_at_ms: Option<i64>,
    ) -> Result<bool> {
        let wake_at = wake_at_ms
            .map(|ms| {
                chrono::DateTime::from_timestamp_millis(ms)
                    .ok_or_else(|| AgentLoopError::store("invalid wake time"))
            })
            .transpose()?;
        self.db
            .schedule_engine_session(session, &lease.holder, fence(lease)?, wake_at)
            .await
            .map_err(|e| store_error(format!("Failed to schedule Engine session: {e}")))
    }
}
