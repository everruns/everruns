//! What the built-in session classifiers read and record
//! (`domains/health_issues/classifiers.rs`).
//!
//! Decisions:
//! - A session is claimed before it is read: one row per session, taken with
//!   an upsert that only succeeds when the last claim is older than the
//!   recheck interval. Every replica sweeps, and this keeps them from paying
//!   twice for the same session.
//! - The summary a classifier reads is a few recent messages and the names
//!   and targets of recent tool calls, truncated. It is read for the call and
//!   never stored: the row keeps probabilities, not content.
//! - Sampling only reads orgs whose decision checks go to the deployment's
//!   service. An org that answers them on its own provider account pays for
//!   each call, so only sessions a layer 1 detector already flagged are read
//!   there.
use super::StorageBackend;
use anyhow::Result;
use uuid::Uuid;

/// A session the classifiers may read.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClassifyCandidateRow {
    pub org_id: i64,
    pub session_id: Uuid,
    pub agent_id: Option<Uuid>,
}

/// One recent message of a session, for a classifier to read.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClassifyMessageRow {
    /// `user` or `agent`.
    pub role: String,
    pub text: String,
}

/// One tool the session called recently, and on what.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClassifyToolRow {
    pub name: String,
    pub target: Option<String>,
    pub calls: i64,
    pub failed: i64,
}

impl StorageBackend {
    /// Sessions with an open issue of one of `codes` that were not read in
    /// the last `recheck_secs`.
    pub async fn flagged_sessions_to_classify(
        &self,
        codes: &[&str],
        recheck_secs: i64,
        limit: i64,
    ) -> Result<Vec<ClassifyCandidateRow>> {
        let codes: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
        Ok(sqlx::query_as(
            "SELECT DISTINCT ON (h.session_id) h.org_id, h.session_id, s.agent_id
             FROM health_issues h
             JOIN sessions s ON s.id = h.session_id
             LEFT JOIN session_classifications c ON c.session_id = h.session_id
             WHERE h.code = ANY($1) AND h.status IN ('open', 'needs_check')
               AND h.session_id IS NOT NULL AND s.source <> 'eval'
               AND (c.claimed_at IS NULL OR c.claimed_at < NOW() - make_interval(secs => $2))
             ORDER BY h.session_id
             LIMIT $3",
        )
        .bind(codes)
        .bind(recheck_secs as f64)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// A random few of the sessions with model calls in the last hour, in
    /// orgs that use the deployment's decision service, not read in the last
    /// `recheck_secs`.
    pub async fn sampled_sessions_to_classify(
        &self,
        recheck_secs: i64,
        limit: i64,
    ) -> Result<Vec<ClassifyCandidateRow>> {
        Ok(sqlx::query_as(
            "WITH active AS (
                SELECT DISTINCT session_id FROM llm_generations
                WHERE created_at >= NOW() - INTERVAL '1 hour' AND session_id IS NOT NULL
             )
             SELECT s.org_id, s.id AS session_id, s.agent_id
             FROM active a
             JOIN sessions s ON s.id = a.session_id
             LEFT JOIN organization_settings o ON o.org_id = s.org_id
             LEFT JOIN session_classifications c ON c.session_id = s.id
             WHERE s.source <> 'eval'
               AND COALESCE(o.system_decisions, 'deployment') = 'deployment'
               AND (c.claimed_at IS NULL OR c.claimed_at < NOW() - make_interval(secs => $1))
             ORDER BY random()
             LIMIT $2",
        )
        .bind(recheck_secs as f64)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Claim a session for reading. `false` when another replica read it in
    /// the last `recheck_secs`.
    pub async fn claim_session_classification(
        &self,
        org_id: i64,
        session_id: Uuid,
        reason: &str,
        recheck_secs: i64,
    ) -> Result<bool> {
        let claimed: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO session_classifications (session_id, org_id, reason)
             VALUES ($1, $2, $3)
             ON CONFLICT (session_id) DO UPDATE
                SET claimed_at = NOW(), reason = EXCLUDED.reason
                WHERE session_classifications.claimed_at < NOW() - make_interval(secs => $4)
             RETURNING session_id",
        )
        .bind(session_id)
        .bind(org_id)
        .bind(reason)
        .bind(recheck_secs as f64)
        .fetch_optional(self.database().pool())
        .await?;
        Ok(claimed.is_some())
    }

    /// Record what the classifiers concluded about a claimed session, or why
    /// they could not.
    pub async fn complete_session_classification(
        &self,
        session_id: Uuid,
        answers: Option<serde_json::Value>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_classifications
             SET classified_at = NOW(), answers = $2, error = $3
             WHERE session_id = $1",
        )
        .bind(session_id)
        .bind(answers)
        .bind(error)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// The session's last `limit` messages from people and from the agent,
    /// oldest first.
    pub async fn classifier_messages(
        &self,
        session_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ClassifyMessageRow>> {
        // The text of a message's content, whether a string or a list of parts.
        Ok(sqlx::query_as(
            "SELECT role, text FROM (
                SELECT sequence,
                    CASE event_type WHEN 'input.message' THEN 'user' ELSE 'agent' END AS role,
                    CASE jsonb_typeof(data->'message'->'content')
                        WHEN 'string' THEN data->'message'->>'content'
                        WHEN 'array' THEN (SELECT string_agg(part->>'text', ' ')
                            FROM jsonb_array_elements(data->'message'->'content') part
                            WHERE part->>'type' = 'text')
                        ELSE NULL END AS text
                FROM events
                WHERE session_id = $1
                  AND event_type IN ('input.message', 'output.message.completed')
                ORDER BY sequence DESC
                LIMIT $2
             ) recent
             WHERE text IS NOT NULL AND text <> ''
             ORDER BY sequence",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// The tools among the session's last `window` tool calls, with their
    /// targets, most called first.
    pub async fn classifier_tools(
        &self,
        session_id: Uuid,
        window: i64,
    ) -> Result<Vec<ClassifyToolRow>> {
        Ok(sqlx::query_as(
            "SELECT COALESCE(name, 'unknown') AS name, target,
                COUNT(*) AS calls, COUNT(*) FILTER (WHERE status = 'error') AS failed
             FROM (
                SELECT name, target, status FROM session_trace_steps
                WHERE session_id = $1 AND kind = 'tool'
                ORDER BY turn_no DESC, step_no DESC LIMIT $2
             ) recent
             GROUP BY 1, 2
             ORDER BY calls DESC, name
             LIMIT 15",
        )
        .bind(session_id)
        .bind(window)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Which of `sessions` made a model call in the last `hours` hours.
    pub async fn sessions_active_within(&self, sessions: &[Uuid], hours: i32) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT session_id FROM llm_generations
             WHERE session_id = ANY($1)
               AND created_at >= NOW() - make_interval(hours => $2)",
        )
        .bind(sessions)
        .bind(hours)
        .fetch_all(self.database().pool())
        .await?)
    }
}
