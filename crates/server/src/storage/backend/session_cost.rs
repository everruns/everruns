//! Reads behind a session's Cost tab: model calls, turns and who started them,
//! tool calls, and sandbox lifecycle.
//!
//! Decision: every read is one aggregate or one narrow row per model call, turn
//! or tool name. Nothing here reads event payloads of model calls, which carry
//! whole requests (megabytes each on a long session); turn origins come from the
//! small `turn.started` and `input.message` events through their partial indexes.

use super::*;
use crate::storage::backend::sandbox_fleet::SandboxTransitionRow;

/// One model call's tokens and cost.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CostGenerationRow {
    pub turn_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub actual_cost_usd: Option<f64>,
    pub estimated_cost_usd: Option<f64>,
}

/// One turn of the trace index.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CostTurnRow {
    pub turn_no: i32,
    pub turn_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub status: String,
}

/// What started a turn, read from its input message.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CostTurnOriginRow {
    /// Public turn id (`turn_...`).
    pub turn_id: String,
    /// `schedule`, `agent_trigger`, ...; `None` for a message a person sent.
    pub kind: Option<String>,
    /// The schedule or trigger behind it, when there is one.
    pub source_id: Option<String>,
}

/// Calls, failures and running time of one tool.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CostToolRow {
    pub name: String,
    pub calls: i64,
    pub failed: i64,
    pub duration_ms: i64,
}

/// A session schedule, deleted ones included, to name turn origins.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CostScheduleRow {
    pub public_id: String,
    pub description: String,
    pub cron_expression: Option<String>,
    pub enabled: bool,
    pub deleted: bool,
}

/// A sandbox the session used.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CostSandboxRow {
    pub id: Uuid,
    pub provider: String,
    pub profile_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl StorageBackend {
    pub async fn session_cost_generations(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Vec<CostGenerationRow>> {
        Ok(sqlx::query_as(
            "SELECT turn_id, created_at, model, provider, input_tokens, output_tokens, \
                    cache_read_tokens, cache_creation_tokens, actual_cost_usd, estimated_cost_usd \
             FROM llm_generations WHERE org_id = $1 AND session_id = $2 ORDER BY created_at",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    pub async fn session_cost_turns(&self, session_id: Uuid) -> Result<Vec<CostTurnRow>> {
        Ok(sqlx::query_as(
            "SELECT turn_no, turn_id, started_at, status FROM session_trace_turns \
             WHERE session_id = $1 ORDER BY turn_no",
        )
        .bind(session_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// The origin of each turn's input message: the event `initiator` written by
    /// `execution_metadata` (schedule, agent trigger, channel), falling back to
    /// the message's own `metadata.source`. A person's message has neither.
    pub async fn session_cost_turn_origins(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<CostTurnOriginRow>> {
        Ok(sqlx::query_as(
            "SELECT t.data->>'turn_id' AS turn_id, \
                    COALESCE(NULLIF(m.metadata->'initiator'->>'type', 'user'), \
                             m.data->'message'->'metadata'->>'source') AS kind, \
                    COALESCE(m.metadata->'initiator'->>'schedule_id', \
                             m.metadata->'initiator'->>'trigger_id', \
                             m.metadata->'initiator'->>'channel_id') AS source_id \
             FROM events t \
             JOIN events m ON m.session_id = t.session_id \
                          AND m.event_type = 'input.message' \
                          AND m.data->'message'->>'id' = t.data->>'input_message_id' \
             WHERE t.session_id = $1 AND t.event_type = 'turn.started'",
        )
        .bind(session_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    pub async fn session_cost_tools(&self, session_id: Uuid) -> Result<Vec<CostToolRow>> {
        Ok(sqlx::query_as(
            "SELECT COALESCE(name, 'unknown') AS name, COUNT(*)::BIGINT AS calls, \
                    COUNT(*) FILTER (WHERE status = 'error')::BIGINT AS failed, \
                    COALESCE(SUM(duration_ms), 0)::BIGINT AS duration_ms \
             FROM session_trace_steps WHERE session_id = $1 AND kind = 'tool' \
             GROUP BY 1 ORDER BY calls DESC, name",
        )
        .bind(session_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    pub async fn session_cost_schedules(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Vec<CostScheduleRow>> {
        Ok(sqlx::query_as(
            "SELECT public_id, description, cron_expression, enabled, \
                    deleted_at IS NOT NULL AS deleted \
             FROM session_schedules WHERE org_id = $1 AND session_id = $2",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    pub async fn session_cost_sandboxes(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Vec<CostSandboxRow>> {
        Ok(sqlx::query_as(
            "SELECT id, provider, profile_name, created_at FROM sandboxes \
             WHERE org_id = $1 AND session_id = $2 ORDER BY created_at",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Lifecycle spans of the session's sandboxes, same shape as the fleet timeline.
    pub async fn session_cost_sandbox_transitions(
        &self,
        org_id: i64,
        session_id: Uuid,
    ) -> Result<Vec<SandboxTransitionRow>> {
        Ok(sqlx::query_as(
            r#"
            WITH t AS (
                SELECT tr.sandbox_id, tr.generation, tr.state, tr.at,
                       lead(tr.at) OVER (PARTITION BY tr.sandbox_id ORDER BY tr.at, tr.id) AS next_at
                FROM sandbox_state_transitions tr
                JOIN sandboxes s ON s.id = tr.sandbox_id
                WHERE tr.org_id = $1 AND s.org_id = $1 AND s.session_id = $2
            )
            SELECT sandbox_id, generation, state,
                   at AS start_at,
                   COALESCE(next_at, now()) AS end_at,
                   next_at IS NULL AS current
            FROM t
            ORDER BY sandbox_id, at
            "#,
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_all(self.database().pool())
        .await?)
    }
}
