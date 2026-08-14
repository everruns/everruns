// PostgreSQL implementation of DurableToolResultStore (EVE-530, EVE-533).

use async_trait::async_trait;
use everruns_core::{
    durability::DurableToolCallStatus, durability::DurableToolResultStore,
    durability::ToolCallClaimResult,
};
use everruns_provider::error::AgentLoopError;
use sqlx::PgPool;
use uuid::Uuid;

/// PostgreSQL-backed durable tool result store.
///
/// Uses the `durable_tool_results` table (migration 050) to persist claim/settle
/// records for per-tool-call idempotency in the durable Act activity (EVE-530).
#[derive(Clone)]
pub struct PgDurableToolResultStore {
    pool: PgPool,
}

impl PgDurableToolResultStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DurableToolResultStore for PgDurableToolResultStore {
    async fn try_claim_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        args_fingerprint: &str,
    ) -> Result<ToolCallClaimResult, AgentLoopError> {
        let claim_token = Uuid::new_v4();

        // Try to INSERT a running claim. On unique conflict the row already exists.
        let inserted = sqlx::query(
            r#"
            INSERT INTO durable_tool_results
                (turn_id, tool_call_id, tool_name, args_fingerprint, status, claim_token)
            VALUES ($1, $2, $3, $4, 'running', $5)
            ON CONFLICT (turn_id, tool_call_id) DO NOTHING
            "#,
        )
        .bind(turn_id)
        .bind(tool_call_id)
        .bind(tool_name)
        .bind(args_fingerprint)
        .bind(claim_token)
        .execute(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("durable_tool_results claim: {e}")))?
        .rows_affected();

        if inserted == 1 {
            return Ok(ToolCallClaimResult::Claimed { claim_token });
        }

        // Row already exists; read its current state.
        let row = sqlx::query_as::<_, (String, Option<serde_json::Value>, String)>(
            r#"
            SELECT status, result_json, args_fingerprint
            FROM durable_tool_results
            WHERE turn_id = $1 AND tool_call_id = $2
            "#,
        )
        .bind(turn_id)
        .bind(tool_call_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("durable_tool_results read: {e}")))?;

        let (status, result_json, stored_fp) = row;

        match status.as_str() {
            "settled" => {
                if stored_fp != args_fingerprint {
                    return Ok(ToolCallClaimResult::DeterminismViolation {
                        stored_fingerprint: stored_fp,
                        current_fingerprint: args_fingerprint.to_string(),
                    });
                }
                Ok(ToolCallClaimResult::AlreadySettled {
                    result_json: result_json.unwrap_or(serde_json::Value::Null),
                    args_fingerprint: stored_fp,
                })
            }
            "running" | "interrupted" => Ok(ToolCallClaimResult::AlreadyRunning {
                args_fingerprint: stored_fp,
            }),
            other => Err(AgentLoopError::tool(format!(
                "durable_tool_results: unknown status '{other}'"
            ))),
        }
    }

    async fn settle_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        result_json: serde_json::Value,
        status: &str,
        claim_token: Uuid,
    ) -> Result<bool, AgentLoopError> {
        // CAS update: only transition from 'running' if the claim token matches.
        // The nil UUID sentinel bypasses the token check for forced interrupts only.
        if claim_token == Uuid::nil() && status != "interrupted" {
            return Err(AgentLoopError::tool(format!(
                "durable_tool_results: nil claim token bypass is restricted to \
                 status='interrupted', got '{status}'"
            )));
        }
        let rows = if claim_token == Uuid::nil() {
            sqlx::query(
                r#"
                UPDATE durable_tool_results
                SET status = $1, result_json = $2, settled_at = now()
                WHERE turn_id = $3 AND tool_call_id = $4 AND status = 'running'
                "#,
            )
            .bind(status)
            .bind(&result_json)
            .bind(turn_id)
            .bind(tool_call_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AgentLoopError::tool(format!("durable_tool_results settle: {e}")))?
            .rows_affected()
        } else {
            sqlx::query(
                r#"
                UPDATE durable_tool_results
                SET status = $1, result_json = $2, settled_at = now()
                WHERE turn_id = $3 AND tool_call_id = $4
                  AND status = 'running'
                  AND claim_token = $5
                "#,
            )
            .bind(status)
            .bind(&result_json)
            .bind(turn_id)
            .bind(tool_call_id)
            .bind(claim_token)
            .execute(&self.pool)
            .await
            .map_err(|e| AgentLoopError::tool(format!("durable_tool_results settle: {e}")))?
            .rows_affected()
        };

        Ok(rows > 0)
    }

    async fn get_tool_call_status(
        &self,
        turn_id: &str,
        tool_call_id: &str,
    ) -> Result<Option<DurableToolCallStatus>, AgentLoopError> {
        let row = sqlx::query_as::<_, (String, Option<serde_json::Value>)>(
            r#"
            SELECT status, result_json
            FROM durable_tool_results
            WHERE turn_id = $1 AND tool_call_id = $2
            "#,
        )
        .bind(turn_id)
        .bind(tool_call_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AgentLoopError::tool(format!("durable_tool_results status lookup: {e}")))?;

        Ok(row.map(|(status, result_json)| match status.as_str() {
            "settled" => DurableToolCallStatus::Settled {
                result_json: result_json.unwrap_or(serde_json::Value::Null),
            },
            "interrupted" => DurableToolCallStatus::Interrupted { result_json },
            _ => DurableToolCallStatus::Running,
        }))
    }
}
