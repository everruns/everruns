//! Consent, managed observer configuration and atomic monitoring admission.
use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

use super::{ObserverRow, StorageBackend};

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, utoipa::ToSchema)]
pub struct MonitoringCheckTotals {
    pub check: String,
    pub passed: i64,
    pub failed: i64,
    pub insufficient_evidence: i64,
    pub pending: i64,
}

#[derive(Debug, Clone, Default, sqlx::FromRow, serde::Serialize, utoipa::ToSchema)]
pub struct MonitoringSpend {
    pub reserved_usd: f64,
    pub reported_usd: f64,
}

impl StorageBackend {
    pub async fn activate_monitoring_continuations(
        &self,
        org_id: i64,
        session: everruns_contracts::typed_id::SessionId,
        sequence: i32,
        turn: everruns_contracts::typed_id::TurnId,
    ) -> Result<u64> {
        Ok(sqlx::query("UPDATE trace_scores SET evidence_ref = jsonb_set(evidence_ref,'{continuation_sequence}',to_jsonb($3::integer)), not_before = NOW() WHERE id IN (SELECT t.id FROM trace_scores t JOIN observers o ON o.id=t.observer_id WHERE t.org_id=$1 AND t.session_id=$2 AND o.org_id=$1 AND o.system_kind='compaction' AND t.scorer_key='compaction_continuity_v1' AND t.status='pending' AND NOT (t.evidence_ref ? 'continuation_sequence') AND (t.evidence_ref->>'event_sequence')::bigint < $3 AND t.evidence_ref->>'turn_id'=$4 ORDER BY t.created_at LIMIT 32)")
            .bind(org_id).bind(session).bind(sequence).bind(turn.to_string()).execute(self.database().tx_pool()).await?.rows_affected())
    }

    pub async fn get_trace_score_session(
        &self,
        id: Uuid,
        org_id: i64,
    ) -> Result<Option<everruns_contracts::typed_id::SessionId>> {
        Ok(
            sqlx::query_scalar("SELECT session_id FROM trace_scores WHERE id=$1 AND org_id=$2")
                .bind(id)
                .bind(org_id)
                .fetch_optional(self.database().tx_pool())
                .await?,
        )
    }
    pub async fn monitoring_previews(
        &self,
        session: everruns_contracts::typed_id::SessionId,
        from: i32,
        to: i32,
        chars: usize,
    ) -> Result<Vec<everruns_core::compaction_observation::CompactionEvidenceExcerpt>> {
        anyhow::ensure!(
            from >= 1
                && to >= from
                && i64::from(to) - i64::from(from) < 64
                && (1..=2048).contains(&chars),
            "Invalid monitoring evidence range"
        );
        let rows: Vec<(Uuid, i32, String, String, bool)> = sqlx::query_as("SELECT id, sequence, event_type, left(data::text,$4), length(data::text) > $4 FROM events WHERE session_id=$1 AND sequence BETWEEN $2 AND $3 AND event_type IN ('input.message','output.message.completed','tool.completed','tool.started','conversation.message') ORDER BY sequence LIMIT 16")
            .bind(session).bind(from).bind(to).bind(chars as i32).fetch_all(self.database().tx_pool()).await?;
        Ok(rows
            .into_iter()
            .map(|(id, sequence, event_type, text, truncated)| {
                everruns_core::compaction_observation::CompactionEvidenceExcerpt {
                    id: everruns_contracts::typed_id::EventId::from_uuid(id).to_string(),
                    sequence,
                    event_type,
                    text,
                    truncated,
                }
            })
            .collect())
    }

    pub async fn finish_monitoring_score(
        &self,
        score: &super::TraceScoreRow,
        result: super::CompleteTraceScoreRow,
    ) -> Result<bool> {
        let changed = sqlx::query("UPDATE trace_scores SET status=$4,pass=$5,value=$6,label=$7,reason=$8,judge_input_tokens=$9,judge_output_tokens=$10,judge_cost_usd=$11,error_message=$12,updated_at=NOW() WHERE org_id=$1 AND id=$2 AND attempts=$3 AND status='scoring'")
            .bind(score.org_id).bind(score.id).bind(score.attempts).bind(result.status).bind(result.pass).bind(result.value).bind(result.label).bind(result.reason).bind(result.judge_input_tokens).bind(result.judge_output_tokens).bind(result.judge_cost_usd).bind(result.error_message)
            .execute(self.database().tx_pool()).await?.rows_affected();
        Ok(changed == 1)
    }

    /// System scores are derived data: endless sessions do not keep an
    /// unlimited monitoring archive. Existing user-observer retention is intact.
    pub async fn prune_monitoring_scores(&self) -> Result<u64> {
        Ok(sqlx::query("DELETE FROM trace_scores WHERE id IN (SELECT t.id FROM trace_scores t JOIN observers o ON o.id=t.observer_id WHERE o.system_kind='compaction' AND t.created_at < NOW() - INTERVAL '30 days' ORDER BY t.created_at LIMIT 500)")
            .execute(self.database().tx_pool()).await?.rows_affected())
    }
    pub async fn org_quality_monitoring(&self, org_id: i64) -> Result<Option<Value>> {
        let value: Option<Option<Value>> = sqlx::query_scalar(
            "SELECT quality_monitoring FROM organization_settings WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_optional(self.database().tx_pool())
        .await?;
        Ok(value.flatten())
    }

    /// Consent and its managed observer change in one transaction. Existing
    /// queued jobs still check current consent before accessing content.
    pub async fn set_org_quality_monitoring(
        &self,
        org_id: i64,
        policy: &Value,
        scorers: &Value,
        enabled: bool,
    ) -> Result<()> {
        let mut tx = self.database().tx_pool().begin().await?;
        sqlx::query("INSERT INTO organization_settings (org_id, quality_monitoring) VALUES ($1,$2) ON CONFLICT (org_id) DO UPDATE SET quality_monitoring = EXCLUDED.quality_monitoring, updated_at = NOW()")
            .bind(org_id).bind(policy).execute(&mut *tx).await?;
        let public_id = everruns_contracts::typed_id::ObserverId::new().to_string();
        sqlx::query("INSERT INTO observers (org_id, public_id, name, description, match_config, sampling_rate, scorers, status, system_kind, alert) VALUES ($1,$2,'Compaction monitoring','System-managed operational and continuity assessments','{}',1,$3,$4,'compaction','{\"min_pass_rate\":0.95,\"window_hours\":24,\"min_scored\":5}') ON CONFLICT (org_id, system_kind) WHERE system_kind IS NOT NULL DO UPDATE SET scorers = EXCLUDED.scorers, status = EXCLUDED.status, updated_at = NOW()")
            .bind(org_id).bind(public_id).bind(scorers).bind(if enabled { "active" } else { "paused" }).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn system_compaction_observer(&self, org_id: i64) -> Result<Option<ObserverRow>> {
        let rows = self.list_observers(org_id, false).await?;
        Ok(rows
            .into_iter()
            .find(|row| row.system_kind.as_deref() == Some("compaction")))
    }

    /// Serializing on the org settings row makes concurrent admission obey one
    /// cap and one consent snapshot. Duplicate attempts never dispatch twice.
    pub async fn reserve_monitoring_budget(
        &self,
        org_id: i64,
        score_id: Uuid,
        attempt: i32,
        expected_policy: &Value,
        hold_micros: i64,
    ) -> Result<bool> {
        if hold_micros <= 0 {
            return Ok(false);
        }
        let mut tx = self.database().tx_pool().begin().await?;
        let current: Option<Option<Value>> = sqlx::query_scalar(
            "SELECT quality_monitoring FROM organization_settings WHERE org_id = $1 FOR UPDATE",
        )
        .bind(org_id)
        .fetch_optional(&mut *tx)
        .await?;
        if current.flatten().as_ref() != Some(expected_policy)
            || expected_policy.get("enabled") != Some(&Value::Bool(true))
        {
            return Ok(false);
        }
        let daily_micros = expected_policy
            .get("daily_budget_usd")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 100.0)
            .map(|value| (value * 1_000_000.0).floor() as i64)
            .unwrap_or(0);
        let spent: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(GREATEST(reserved_micros, COALESCE(actual_micros,0))),0)::bigint FROM observer_budget_reservations WHERE org_id = $1 AND day = (now() AT TIME ZONE 'UTC')::date")
            .bind(org_id).fetch_one(&mut *tx).await?;
        if spent.saturating_add(hold_micros) > daily_micros {
            return Ok(false);
        }
        let inserted = sqlx::query("INSERT INTO observer_budget_reservations (org_id, score_id, attempt, reserved_micros) SELECT $1,$2,$3,$4 FROM trace_scores t JOIN observers o ON o.id=t.observer_id AND o.org_id=t.org_id WHERE t.id = $2 AND t.org_id = $1 AND t.status = 'scoring' AND t.attempts = $3 AND o.status='active' AND o.system_kind='compaction' AND (t.evidence_ref->>'consent_version')::timestamptz=o.updated_at ON CONFLICT DO NOTHING")
            .bind(org_id).bind(score_id).bind(attempt).bind(hold_micros).execute(&mut *tx).await?.rows_affected();
        if inserted == 1 {
            // Evidence reads may have consumed most of the original lease.
            // Fence and renew before dispatch so the bounded judge call gets
            // a full lease; a concurrently reclaimed attempt rolls back its hold.
            let renewed = sqlx::query("UPDATE trace_scores SET updated_at=NOW() WHERE id=$1 AND org_id=$2 AND attempts=$3 AND status='scoring'")
                .bind(score_id).bind(org_id).bind(attempt).execute(&mut *tx).await?.rows_affected();
            if renewed != 1 {
                return Ok(false);
            }
        }
        tx.commit().await?;
        Ok(inserted == 1)
    }

    /// Record provider-reported cost. The maximum allowance remains held:
    /// transport retry spend may be absent even from a successful response.
    pub async fn settle_monitoring_budget(
        &self,
        org_id: i64,
        score_id: Uuid,
        attempt: i32,
        actual_micros: Option<i64>,
    ) -> Result<()> {
        if let Some(actual) = actual_micros {
            sqlx::query("UPDATE observer_budget_reservations SET actual_micros = $4 WHERE org_id = $1 AND score_id = $2 AND attempt = $3")
                .bind(org_id).bind(score_id).bind(attempt).bind(actual.max(0)).execute(self.database().tx_pool()).await?;
        }
        Ok(())
    }

    pub async fn monitoring_spend(&self, org_id: i64) -> Result<MonitoringSpend> {
        Ok(sqlx::query_as("SELECT COALESCE(SUM(GREATEST(reserved_micros, COALESCE(actual_micros,0))),0)::float8 / 1000000 AS reserved_usd, COALESCE(SUM(actual_micros),0)::float8 / 1000000 AS reported_usd FROM observer_budget_reservations WHERE org_id = $1 AND day = (now() AT TIME ZONE 'UTC')::date")
            .bind(org_id).fetch_one(self.database().tx_pool()).await?)
    }

    pub async fn compaction_monitoring_totals(
        &self,
        org_id: i64,
    ) -> Result<Vec<MonitoringCheckTotals>> {
        self.compaction_monitoring_window(org_id, chrono::Utc::now() - chrono::Duration::days(7))
            .await
    }

    pub async fn compaction_monitoring_window(
        &self,
        org_id: i64,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<MonitoringCheckTotals>> {
        Ok(sqlx::query_as("SELECT t.scorer_key AS check, COUNT(*) FILTER (WHERE t.status = 'completed' AND t.pass IS TRUE) AS passed, COUNT(*) FILTER (WHERE t.status = 'completed' AND t.pass IS FALSE) AS failed, COUNT(*) FILTER (WHERE (t.status = 'completed' AND t.pass IS NULL) OR t.status IN ('skipped','errored')) AS insufficient_evidence, COUNT(*) FILTER (WHERE t.status IN ('pending','scoring')) AS pending FROM trace_scores t JOIN observers o ON o.id=t.observer_id WHERE t.org_id = $1 AND o.org_id = $1 AND o.system_kind='compaction' AND t.created_at >= $2 GROUP BY t.scorer_key ORDER BY t.scorer_key")
            .bind(org_id).bind(since).fetch_all(self.database().tx_pool()).await?)
    }
}
