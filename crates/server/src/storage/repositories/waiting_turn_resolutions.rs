// PostgreSQL repository: durable parked-turn resolution claims

use super::Database;
use crate::storage::{
    ClaimWaitingTurnResult, RESOLVING_TOOL_RESULTS_STATUS, WaitingTurnResolutionClaim,
    WaitingTurnResolutionPlan, waiting_turn_claim_lease_expires_at,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SessionId;
use uuid::Uuid;

type WaitingTurnResolutionRow = (
    String,
    Option<Uuid>,
    Option<DateTime<Utc>>,
    Option<serde_json::Value>,
);

impl Database {
    pub async fn claim_waiting_turn(
        &self,
        org_id: i64,
        session_id: SessionId,
        resolution_plan: WaitingTurnResolutionPlan,
    ) -> Result<ClaimWaitingTurnResult> {
        self.claim_waiting_turn_inner(org_id, session_id, resolution_plan, false, None)
            .await
    }

    /// Shared Rust validates client results against canonical pending calls
    /// while the same row lock protects admission of their durable resolution.
    pub async fn claim_tool_result_command(
        &self,
        org_id: i64,
        session_id: SessionId,
        results: Vec<everruns_core::events::ToolCompletedData>,
    ) -> Result<ClaimWaitingTurnResult> {
        let plan = WaitingTurnResolutionPlan {
            kind: "tool_results".into(),
            events: Vec::new(),
            session_values: Vec::new(),
            response: serde_json::json!({}),
        };
        self.claim_waiting_turn_inner(org_id, session_id, plan, false, Some(results))
            .await
    }

    pub async fn recover_waiting_turn(
        &self,
        org_id: i64,
        session_id: SessionId,
        resolution_plan: WaitingTurnResolutionPlan,
    ) -> Result<ClaimWaitingTurnResult> {
        self.claim_waiting_turn_inner(org_id, session_id, resolution_plan, true, None)
            .await
    }

    async fn claim_waiting_turn_inner(
        &self,
        org_id: i64,
        session_id: SessionId,
        resolution_plan: WaitingTurnResolutionPlan,
        recover_existing: bool,
        command_results: Option<Vec<everruns_core::events::ToolCompletedData>>,
    ) -> Result<ClaimWaitingTurnResult> {
        let mut tx = self.pool.begin().await?;
        let row: Option<WaitingTurnResolutionRow> = sqlx::query_as(
            "SELECT status, turn_resolution_id, turn_resolution_lease_expires_at, \
             turn_resolution_plan FROM sessions WHERE org_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((status, existing_resolution_id, lease_expires_at, existing_plan)) = row else {
            tx.commit().await?;
            return Ok(ClaimWaitingTurnResult::SessionNotFound);
        };

        if status == "waiting_for_tool_results" {
            let resolution_plan = if let Some(results) = command_results {
                super::session_command::prepare_tool_results(&mut tx, session_id, results).await?
            } else {
                resolution_plan
            };
            let resolution_id = Uuid::now_v7();
            let claim_token = Uuid::now_v7();
            sqlx::query(
                "UPDATE sessions SET status = $3, turn_resolution_id = $4, \
                 turn_resolution_claim_token = $5, turn_resolution_lease_expires_at = $6, \
                 turn_resolution_plan = $7, updated_at = NOW() WHERE org_id = $1 AND id = $2",
            )
            .bind(org_id)
            .bind(session_id)
            .bind(RESOLVING_TOOL_RESULTS_STATUS)
            .bind(resolution_id)
            .bind(claim_token)
            .bind(waiting_turn_claim_lease_expires_at())
            .bind(serde_json::to_value(&resolution_plan)?)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(ClaimWaitingTurnResult::Claimed(
                WaitingTurnResolutionClaim {
                    resolution_id,
                    claim_token,
                    plan: resolution_plan,
                    recovered: false,
                },
            ));
        }

        if status == RESOLVING_TOOL_RESULTS_STATUS
            && lease_expires_at.is_some_and(|expires_at| expires_at <= Utc::now())
            && let (Some(resolution_id), Some(plan_json)) = (existing_resolution_id, existing_plan)
        {
            let plan: WaitingTurnResolutionPlan = serde_json::from_value(plan_json)?;
            if !recover_existing && plan.kind != resolution_plan.kind {
                tx.commit().await?;
                return Ok(ClaimWaitingTurnResult::Conflict {
                    current_status: "waiting_for_tool_results".to_string(),
                });
            }
            let claim_token = Uuid::now_v7();
            sqlx::query(
                "UPDATE sessions SET turn_resolution_claim_token = $3, \
                 turn_resolution_lease_expires_at = $4, updated_at = NOW() \
                 WHERE org_id = $1 AND id = $2",
            )
            .bind(org_id)
            .bind(session_id)
            .bind(claim_token)
            .bind(waiting_turn_claim_lease_expires_at())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(ClaimWaitingTurnResult::Claimed(
                WaitingTurnResolutionClaim {
                    resolution_id,
                    claim_token,
                    plan,
                    recovered: true,
                },
            ));
        }

        tx.commit().await?;
        Ok(ClaimWaitingTurnResult::Conflict {
            current_status: if status == RESOLVING_TOOL_RESULTS_STATUS {
                "waiting_for_tool_results".to_string()
            } else {
                status
            },
        })
    }

    pub async fn complete_waiting_turn_claim(
        &self,
        org_id: i64,
        session_id: SessionId,
        resolution_id: Uuid,
        claim_token: Uuid,
    ) -> Result<bool> {
        self.complete_waiting_turn_claim_inner(org_id, session_id, resolution_id, claim_token, true)
            .await
    }

    pub async fn park_waiting_turn_claim(
        &self,
        org_id: i64,
        session_id: SessionId,
        resolution_id: Uuid,
        claim_token: Uuid,
    ) -> Result<bool> {
        self.complete_waiting_turn_claim_inner(
            org_id,
            session_id,
            resolution_id,
            claim_token,
            false,
        )
        .await
    }

    async fn complete_waiting_turn_claim_inner(
        &self,
        org_id: i64,
        session_id: SessionId,
        resolution_id: Uuid,
        claim_token: Uuid,
        resume: bool,
    ) -> Result<bool> {
        let completed = sqlx::query(
            "UPDATE sessions SET \
                 status = CASE WHEN status = $3 OR ($6 = 'waiting_for_tool_results' AND status = 'active') THEN $6 ELSE status END, \
                 turn_resolution_id = NULL, \
                 turn_resolution_claim_token = NULL, turn_resolution_lease_expires_at = NULL, \
                 turn_resolution_plan = NULL, updated_at = NOW() \
             WHERE org_id = $1 AND id = $2 \
               AND status IN ($3, 'active', 'idle', 'paused') \
               AND turn_resolution_id = $4 AND turn_resolution_claim_token = $5 \
               AND (turn_resolution_lease_expires_at > NOW() \
                    OR status IN ('active', 'idle', 'paused'))",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(RESOLVING_TOOL_RESULTS_STATUS)
        .bind(resolution_id)
        .bind(claim_token)
        .bind(if resume {
            "active"
        } else {
            "waiting_for_tool_results"
        })
        .execute(&self.pool)
        .await?;
        Ok(completed.rows_affected() == 1)
    }

    pub async fn abandon_waiting_turn_claim(
        &self,
        org_id: i64,
        session_id: SessionId,
        resolution_id: Uuid,
        claim_token: Uuid,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET turn_resolution_lease_expires_at = NOW(), updated_at = NOW() \
             WHERE org_id = $1 AND id = $2 AND status = $3 \
               AND turn_resolution_id = $4 AND turn_resolution_claim_token = $5",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(RESOLVING_TOOL_RESULTS_STATUS)
        .bind(resolution_id)
        .bind(claim_token)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
