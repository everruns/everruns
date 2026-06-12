use crate::domains::common::{CommandError, Ctx};
use crate::storage::{
    DbSessionScheduleStore, StorageBackend, create_db_session_storage_store,
    create_db_session_storage_store_without_encryption, session_task_store::DbSessionTaskRegistry,
};
use everruns_core::session_task::SessionTaskRegistry;
use everruns_core::traits::{SessionScheduleStore, SessionStorageStore, ToolContext};
use everruns_core::{SessionId, SessionTask};
use std::sync::Arc;

pub fn parse_session_id(input: &str) -> Result<SessionId, CommandError> {
    input
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))
}

/// Build the registry, attaching the ctx event service when present so API
/// mutations emit task.* events for live UIs.
pub fn registry_for_ctx(ctx: &Ctx) -> DbSessionTaskRegistry {
    let mut registry = DbSessionTaskRegistry::new(ctx.db.clone());
    if let Some(event_service) = &ctx.event_service {
        registry = registry.with_event_emitter(event_service.clone());
    }
    registry
}

/// Build a ToolContext for executor calls from an API command context.
///
/// Contract: populate only what the built-in executors actually read —
/// - `session_task_registry`: all executors use this for state updates.
/// - `storage_store`: ExternalAgentTaskExecutor reads/writes the AgentRunRecord
///   (KV key `agent_run:{run_id}`) from session storage. The encrypted store is
///   used when an encryption service is configured (matching the worker path);
///   plain KV works either way, only secrets operations need encryption.
/// - `schedule_store`: MonitorTaskExecutor.cancel disables the linked schedule.
/// - `egress_service`: ExternalAgentTaskExecutor rebuilds the A2A client for
///   network calls to the remote agent.
///
/// `platform_store` is not populated — SubagentTaskExecutor needs it but
/// it is not cheaply available from Ctx; deliver/cancel for subagents will
/// return an error that the caller logs as a warn (best-effort, not a failure).
pub fn tool_context_for_ctx(ctx: &Ctx, session_id: SessionId) -> ToolContext {
    let registry: Arc<dyn SessionTaskRegistry> = Arc::new(registry_for_ctx(ctx));

    let storage_store: Arc<dyn SessionStorageStore> = match ctx.db.as_ref() {
        StorageBackend::InMemory(mem_db) => mem_db.clone() as Arc<dyn SessionStorageStore>,
        StorageBackend::Postgres(db) => {
            if let Some(enc) = &ctx.encryption {
                Arc::new(create_db_session_storage_store(
                    db.clone(),
                    enc.as_ref().clone(),
                )) as Arc<dyn SessionStorageStore>
            } else {
                Arc::new(create_db_session_storage_store_without_encryption(
                    db.clone(),
                )) as Arc<dyn SessionStorageStore>
            }
        }
    };

    let schedule_store: Arc<dyn SessionScheduleStore> =
        Arc::new(DbSessionScheduleStore::new(ctx.db.clone(), ctx.org_id()));

    let mut tool_ctx = ToolContext::new(session_id)
        .with_session_task_registry(registry)
        .with_storage_store_arc(storage_store)
        .with_schedule_store(schedule_store);

    if let Some(egress) = &ctx.egress_service {
        tool_ctx = tool_ctx.with_egress_service(egress.clone());
    }

    tool_ctx
}

/// Validate org ownership of the session. Returns false when the session
/// does not exist in `org_id`.
pub async fn session_in_org(
    db: &Arc<StorageBackend>,
    org_id: i64,
    session_id: SessionId,
) -> anyhow::Result<bool> {
    Ok(db.get_session(org_id, session_id).await?.is_some())
}

/// Look up one task after validating org ownership.
pub async fn get_task_in_org(
    ctx: &Ctx,
    org_id: i64,
    session_id: SessionId,
    task_id: &str,
) -> Result<Option<SessionTask>, CommandError> {
    if !session_in_org(&ctx.db, org_id, session_id)
        .await
        .map_err(crate::domains::common::classify_anyhow)?
    {
        return Ok(None);
    }
    registry_for_ctx(ctx)
        .get(session_id, task_id)
        .await
        .map_err(|e| CommandError::internal(anyhow::anyhow!(e)))
}
