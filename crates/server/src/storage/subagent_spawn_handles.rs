// PostgreSQL implementation of SubagentSpawnStore (EVE-535).

use async_trait::async_trait;
use everruns_core::error::AgentLoopError;
use everruns_core::traits::{SpawnClaimResult, SubagentSpawnStore};
use everruns_core::typed_id::SessionId;
use sqlx::PgPool;
use uuid::Uuid;

/// PostgreSQL-backed durable subagent spawn handle store.
///
/// Uses the `subagent_spawn_handles` table (migration 051) to record
/// `(parent_session_id, tool_call_id) → child_session_id` mappings so that a
/// parent worker can reattach to an existing child on reclaim instead of
/// spawning a duplicate (EVE-535).
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
        child_session_id: SessionId,
        subagent_name: &str,
        subagent_task: &str,
        claim_token: Uuid,
    ) -> Result<SpawnClaimResult, AgentLoopError> {
        let parent_uuid: Uuid = parent_session_id.into();
        let child_uuid: Uuid = child_session_id.into();

        // Try to INSERT a running claim. On unique conflict the row already exists.
        let inserted = sqlx::query(
            r#"
            INSERT INTO subagent_spawn_handles
                (parent_session_id, tool_call_id, child_session_id,
                 subagent_name, subagent_task, claim_token)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (parent_session_id, tool_call_id) DO NOTHING
            "#,
        )
        .bind(parent_uuid)
        .bind(tool_call_id)
        .bind(child_uuid)
        .bind(subagent_name)
        .bind(subagent_task)
        .bind(claim_token)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles claim: {e}")))?
        .rows_affected();

        if inserted == 1 {
            // We inserted a new row — fetch its generated id for the caller.
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

        // Row already exists — read its current state to detect reattach.
        let row = sqlx::query_as::<_, (Uuid, String, Option<String>)>(
            r#"
            SELECT child_session_id, status, terminal_result
            FROM subagent_spawn_handles
            WHERE parent_session_id = $1 AND tool_call_id = $2
            "#,
        )
        .bind(parent_uuid)
        .bind(tool_call_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles read: {e}")))?;

        let (existing_child_uuid, status, terminal_result) = row;
        let existing_child_id = SessionId::from(existing_child_uuid);

        match status.as_str() {
            "settled" => Ok(SpawnClaimResult::AlreadySpawned {
                child_session_id: existing_child_id,
                terminal_result,
            }),
            "running" => Ok(SpawnClaimResult::AlreadySpawned {
                child_session_id: existing_child_id,
                terminal_result: None,
            }),
            other => Err(AgentLoopError::tool(format!(
                "subagent_spawn_handles: unknown status '{other}'"
            ))),
        }
    }

    async fn settle_spawn(
        &self,
        parent_session_id: SessionId,
        tool_call_id: &str,
        claim_token: Uuid,
        terminal_result: &str,
    ) -> Result<(), AgentLoopError> {
        let parent_uuid: Uuid = parent_session_id.into();

        sqlx::query(
            r#"
            UPDATE subagent_spawn_handles
            SET status = 'settled',
                terminal_result = $4,
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
        .bind(terminal_result)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("subagent_spawn_handles settle: {e}")))?;

        Ok(())
    }
}
