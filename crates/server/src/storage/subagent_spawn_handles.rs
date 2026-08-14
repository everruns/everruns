// PostgreSQL implementation of SubagentSpawnStore (EVE-535).

use async_trait::async_trait;
use everruns_core::error::AgentLoopError;
use everruns_core::typed_id::SessionId;
use everruns_core::{
    delegation_services::SpawnClaimResult, delegation_services::SubagentSpawnStore,
};
use sqlx::PgPool;
use uuid::Uuid;

/// PostgreSQL-backed durable subagent spawn handle store.
///
/// Uses the `subagent_spawn_handles` table (migration 052) to record
/// `(parent_session_id, tool_call_id) → child_session_id` mappings so that a
/// parent worker can reattach to an existing child on reclaim instead of
/// spawning a duplicate (EVE-535).
///
/// Lifecycle: try_claim_spawn → register_child_session → settle_spawn.
#[derive(Clone)]
pub struct PgSubagentSpawnStore {
    pool: PgPool,
}

impl PgSubagentSpawnStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SubagentSpawnStore for PgSubagentSpawnStore {
    async fn try_claim_spawn(
        &self,
        parent_session_id: SessionId,
        tool_call_id: &str,
        claim_token: Uuid,
    ) -> Result<SpawnClaimResult, AgentLoopError> {
        let parent_uuid: Uuid = parent_session_id.into();

        // Insert a 'pending' row (child_session_id NULL). On conflict the row already exists.
        let inserted = sqlx::query(
            r#"
            INSERT INTO subagent_spawn_handles
                (parent_session_id, tool_call_id, claim_token)
            VALUES ($1, $2, $3)
            ON CONFLICT (parent_session_id, tool_call_id) DO NOTHING
            "#,
        )
        .bind(parent_uuid)
        .bind(tool_call_id)
        .bind(claim_token)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles claim: {e}")))?
        .rows_affected();

        if inserted == 1 {
            // We inserted a new row — fetch its generated id.
            let spawn_handle_id: Uuid = sqlx::query_scalar(
                r#"
                SELECT id FROM subagent_spawn_handles
                WHERE parent_session_id = $1 AND tool_call_id = $2
                "#,
            )
            .bind(parent_uuid)
            .bind(tool_call_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles id lookup: {e}")))?;

            return Ok(SpawnClaimResult::Claimed {
                spawn_handle_id,
                claim_token,
            });
        }

        // Row already exists — read its current state.
        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                Option<Uuid>,
                String,
                Option<String>,
                Option<String>,
                Uuid,
            ),
        >(
            r#"
            SELECT id, child_session_id, status, terminal_status, terminal_result, claim_token
            FROM subagent_spawn_handles
            WHERE parent_session_id = $1 AND tool_call_id = $2
            "#,
        )
        .bind(parent_uuid)
        .bind(tool_call_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles read: {e}")))?;

        let (
            spawn_handle_id,
            child_uuid_opt,
            status,
            terminal_status,
            terminal_result,
            stored_token,
        ) = row;

        match status.as_str() {
            "pending" => Ok(SpawnClaimResult::ClaimedPendingChild {
                spawn_handle_id,
                claim_token: stored_token,
            }),
            "running" => {
                let child_id = child_uuid_opt.map(SessionId::from).ok_or_else(|| {
                    AgentLoopError::tool("spawn handle 'running' but child_session_id is NULL")
                })?;
                Ok(SpawnClaimResult::AlreadyRunning {
                    child_session_id: child_id,
                    claim_token: stored_token,
                })
            }
            "settled" => {
                let child_id = child_uuid_opt.map(SessionId::from).ok_or_else(|| {
                    AgentLoopError::tool("spawn handle 'settled' but child_session_id is NULL")
                })?;
                Ok(SpawnClaimResult::AlreadySettled {
                    child_session_id: child_id,
                    terminal_status: terminal_status.unwrap_or_else(|| "idle".to_string()),
                    terminal_result: terminal_result.unwrap_or_default(),
                })
            }
            other => Err(AgentLoopError::tool(format!(
                "subagent_spawn_handles: unknown status '{other}'"
            ))),
        }
    }

    async fn register_child_session(
        &self,
        spawn_handle_id: Uuid,
        claim_token: Uuid,
        child_session_id: SessionId,
    ) -> Result<(), AgentLoopError> {
        let child_uuid: Uuid = child_session_id.into();

        sqlx::query(
            r#"
            UPDATE subagent_spawn_handles
            SET child_session_id = $3,
                status = 'running'
            WHERE id = $1
              AND claim_token = $2
              AND status = 'pending'
            "#,
        )
        .bind(spawn_handle_id)
        .bind(claim_token)
        .bind(child_uuid)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles register_child: {e}")))?;

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
        let parent_uuid: Uuid = parent_session_id.into();

        sqlx::query(
            r#"
            UPDATE subagent_spawn_handles
            SET status = 'settled',
                terminal_status = $4,
                terminal_result = $5,
                settled_at = NOW()
            WHERE parent_session_id = $1
              AND tool_call_id = $2
              AND claim_token = $3
              AND status = 'running'
            "#,
        )
        .bind(parent_uuid)
        .bind(tool_call_id)
        .bind(claim_token)
        .bind(terminal_status)
        .bind(terminal_result)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles settle: {e}")))?;

        Ok(())
    }
}
