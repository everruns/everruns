//! What the Eval Builder reads and records (`domains/evals/builder.rs`).
//!
//! Decisions:
//! - One run at a time per agent: a new run is refused while another started
//!   within the stale window. A run whose server went away mid-call stays
//!   `running` in the table and reads as interrupted after that window, so a
//!   later request can start again.
//! - Proposals are kept after a decision. Accepted ones record the case they
//!   became; dismissed ones keep the note. Both are offered back to the
//!   builder as ideas not to repeat.
//! - Sessions are sampled from the agent's own production traffic in the
//!   org: never eval-run sessions, never another org's.
use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One request to propose cases for an agent.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EvalBuilderRunRow {
    pub id: Uuid,
    pub org_id: i64,
    pub agent_id: Uuid,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub sessions_read: i32,
    pub proposals: i32,
    pub error: Option<String>,
}

/// One suggested case.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EvalCaseProposalRow {
    pub id: Uuid,
    pub org_id: i64,
    pub agent_id: Uuid,
    pub run_id: Uuid,
    pub status: String,
    pub name: String,
    pub category: String,
    pub reason: String,
    pub source_session_ids: Vec<Uuid>,
    pub case: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub eval_id: Option<Uuid>,
    pub eval_case_id: Option<Uuid>,
    pub dismiss_note: Option<String>,
}

/// A proposal about to be stored.
#[derive(Debug, Clone)]
pub struct NewEvalCaseProposal<'a> {
    pub name: &'a str,
    pub category: &'a str,
    pub reason: &'a str,
    pub source_session_ids: &'a [Uuid],
    pub case: &'a serde_json::Value,
}

/// A recent session of the agent and what makes it worth reading.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BuilderSessionRow {
    pub id: Uuid,
    pub title: Option<String>,
    /// A detector or classifier opened a finding on it.
    pub has_finding: bool,
    /// An observer scored one of its turns as failing.
    pub low_score: bool,
}

/// How a run ended.
#[derive(Debug, Clone, Default)]
pub struct FinishedBuilderRun<'a> {
    pub status: &'a str,
    pub model: Option<&'a str>,
    pub cost_usd: Option<f64>,
    pub sessions_read: i32,
    pub proposals: i32,
    pub error: Option<&'a str>,
}

impl StorageBackend {
    /// Start a run unless one for the agent started in the last
    /// `stale_secs`. `None` when another run is still going.
    pub async fn start_eval_builder_run(
        &self,
        org_id: i64,
        agent_id: Uuid,
        requested_by: Option<Uuid>,
        stale_secs: i64,
    ) -> Result<Option<EvalBuilderRunRow>> {
        Ok(sqlx::query_as(
            "INSERT INTO eval_builder_runs (org_id, agent_id, requested_by)
             SELECT $1, $2, $3
             WHERE NOT EXISTS (
                SELECT 1 FROM eval_builder_runs
                WHERE agent_id = $2 AND status = 'running'
                  AND started_at > now() - make_interval(secs => $4)
             )
             RETURNING id, org_id, agent_id, status, started_at, completed_at, model,
                       cost_usd, sessions_read, proposals, error",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(requested_by)
        .bind(stale_secs as f64)
        .fetch_optional(self.database().pool())
        .await?)
    }

    pub async fn finish_eval_builder_run(
        &self,
        run_id: Uuid,
        finished: FinishedBuilderRun<'_>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE eval_builder_runs
             SET status = $2, model = $3, cost_usd = $4, sessions_read = $5,
                 proposals = $6, error = $7, completed_at = now()
             WHERE id = $1",
        )
        .bind(run_id)
        .bind(finished.status)
        .bind(finished.model)
        .bind(finished.cost_usd)
        .bind(finished.sessions_read)
        .bind(finished.proposals)
        .bind(finished.error)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// The agent's most recent run.
    pub async fn latest_eval_builder_run(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<Option<EvalBuilderRunRow>> {
        Ok(sqlx::query_as(
            "SELECT id, org_id, agent_id, status, started_at, completed_at, model,
                    cost_usd, sessions_read, proposals, error
             FROM eval_builder_runs WHERE org_id = $1 AND agent_id = $2
             ORDER BY started_at DESC LIMIT 1",
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    pub async fn insert_eval_case_proposal(
        &self,
        org_id: i64,
        agent_id: Uuid,
        run_id: Uuid,
        proposal: NewEvalCaseProposal<'_>,
    ) -> Result<Uuid> {
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO eval_case_proposals
                (org_id, agent_id, run_id, name, category, reason, source_session_ids, \"case\")
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(run_id)
        .bind(proposal.name)
        .bind(proposal.category)
        .bind(proposal.reason)
        .bind(proposal.source_session_ids)
        .bind(proposal.case)
        .fetch_one(self.database().pool())
        .await?;
        Ok(id)
    }

    /// The agent's proposals in `status`, newest first.
    pub async fn list_eval_case_proposals(
        &self,
        org_id: i64,
        agent_id: Uuid,
        status: &str,
        limit: i64,
    ) -> Result<Vec<EvalCaseProposalRow>> {
        Ok(sqlx::query_as(
            "SELECT id, org_id, agent_id, run_id, status, name, category, reason,
                    source_session_ids, \"case\", created_at, decided_at, eval_id,
                    eval_case_id, dismiss_note
             FROM eval_case_proposals
             WHERE org_id = $1 AND agent_id = $2 AND status = $3
             ORDER BY created_at DESC, id LIMIT $4",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(status)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Names of the agent's earlier proposals with how each ended, so the
    /// builder does not suggest them again.
    pub async fn eval_proposal_history(
        &self,
        org_id: i64,
        agent_id: Uuid,
        limit: i64,
    ) -> Result<Vec<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT name, status FROM eval_case_proposals
             WHERE org_id = $1 AND agent_id = $2
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    pub async fn get_eval_case_proposal(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<EvalCaseProposalRow>> {
        Ok(sqlx::query_as(
            "SELECT id, org_id, agent_id, run_id, status, name, category, reason,
                    source_session_ids, \"case\", created_at, decided_at, eval_id,
                    eval_case_id, dismiss_note
             FROM eval_case_proposals WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// Record a decision on a proposal still open. `false` when someone
    /// decided it first.
    #[allow(clippy::too_many_arguments)]
    pub async fn decide_eval_case_proposal(
        &self,
        org_id: i64,
        id: Uuid,
        status: &str,
        decided_by: Option<Uuid>,
        eval_id: Option<Uuid>,
        eval_case_id: Option<Uuid>,
        dismiss_note: Option<&str>,
    ) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE eval_case_proposals
             SET status = $3, decided_at = now(), decided_by = $4, eval_id = $5,
                 eval_case_id = $6, dismiss_note = $7
             WHERE org_id = $1 AND id = $2 AND status = 'proposed'",
        )
        .bind(org_id)
        .bind(id)
        .bind(status)
        .bind(decided_by)
        .bind(eval_id)
        .bind(eval_case_id)
        .bind(dismiss_note)
        .execute(self.database().pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// The agent's recent production sessions worth reading: those with
    /// findings first, then those an observer failed, then the newest.
    pub async fn eval_builder_sessions(
        &self,
        org_id: i64,
        agent_id: Uuid,
        days: i32,
        limit: i64,
    ) -> Result<Vec<BuilderSessionRow>> {
        Ok(sqlx::query_as(
            "SELECT s.id, s.title::text AS title,
                EXISTS (SELECT 1 FROM health_issues h
                        WHERE h.org_id = s.org_id AND h.session_id = s.id) AS has_finding,
                EXISTS (SELECT 1 FROM trace_scores t
                        WHERE t.org_id = s.org_id AND t.session_id = s.id
                          AND t.status = 'completed' AND t.pass = false) AS low_score
             FROM sessions s
             WHERE s.org_id = $1 AND s.agent_id = $2
               AND s.source <> 'eval' AND s.parent_session_id IS NULL
               AND s.turn_count > 0
               AND s.created_at > now() - make_interval(days => $3)
             ORDER BY has_finding DESC, low_score DESC, s.updated_at DESC
             LIMIT $4",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(days)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Case names in the datasets that target the agent.
    pub async fn agent_dataset_case_names(
        &self,
        org_id: i64,
        agent_id: Uuid,
        limit: i64,
    ) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT c.name FROM eval_cases c
             JOIN evals e ON e.id = c.eval_id
             JOIN agents a ON a.id = $2 AND e.target->>'agent_id' = a.public_id
             WHERE e.org_id = $1 AND e.status NOT IN ('archived', 'deleted')
             ORDER BY c.created_at DESC LIMIT $3",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?;
        Ok(rows.into_iter().map(|(name,)| name).collect())
    }
}
