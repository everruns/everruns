//! Storage for detecting unattended background work that goes wrong: how a
//! schedule's runs end, health issues about one schedule, agent or session,
//! who hears about them, and the numbers the detection sweep reads.
//! See knowledge/operations/background-limits.md ("Detection").
use super::StorageBackend;
use crate::storage::HealthIssueRow;
use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A health issue about one schedule, agent or session, as a detector sees it.
pub struct SubjectHealthIssue<'a> {
    pub org_id: i64,
    pub code: &'a str,
    pub subject_id: &'a str,
    pub agent_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    /// `open` or `resolved`.
    pub status: &'a str,
    pub evidence: serde_json::Value,
    pub checked_at: DateTime<Utc>,
}

/// What recording an observation changed.
pub struct ObservedIssue {
    pub id: Uuid,
    pub episode_id: Uuid,
    pub status: String,
    /// The issue was not open before this observation: a new episode whose
    /// people are told now.
    pub opened: bool,
}

/// An agent's unattended spend over the last day against its own recent days.
#[derive(Debug, sqlx::FromRow)]
pub struct UnattendedSpendRow {
    pub org_id: i64,
    pub agent_id: Uuid,
    pub last_day_usd: f64,
    /// Average spend per day over the six days before the last one.
    pub baseline_daily_usd: f64,
}

/// A session's recent model calls: prompt size and how much of it was cached.
#[derive(Debug, sqlx::FromRow)]
pub struct ContextSizeRow {
    pub org_id: i64,
    pub session_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub calls: i64,
    pub avg_prompt_tokens: f64,
    pub cached_ratio: f64,
}

impl StorageBackend {
    /// Count a finished run's outcome against its schedule. Returns how many
    /// runs in a row have now ended with `fingerprint`; a run already counted
    /// (same `input_message_id`) leaves the count as it is.
    pub async fn record_schedule_outcome(
        &self,
        schedule_id: Uuid,
        input_message_id: Uuid,
        fingerprint: &str,
    ) -> Result<i32> {
        let runs: i32 = sqlx::query_scalar(
            "UPDATE session_schedules SET
                same_outcome_runs = CASE
                    WHEN outcome_input_message_id IS NOT DISTINCT FROM $2 THEN same_outcome_runs
                    WHEN outcome_fingerprint = $3 THEN same_outcome_runs + 1
                    ELSE 1 END,
                outcome_fingerprint = $3,
                outcome_input_message_id = $2
             WHERE id = $1
             RETURNING same_outcome_runs",
        )
        .bind(schedule_id)
        .bind(input_message_id)
        .bind(fingerprint)
        .fetch_one(self.database().pool())
        .await?;
        Ok(runs)
    }

    /// Forget a schedule's repeated outcomes, when a person resumes it.
    pub async fn reset_schedule_outcomes(&self, schedule_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE session_schedules SET same_outcome_runs = 0, outcome_fingerprint = NULL
             WHERE id = $1",
        )
        .bind(schedule_id)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// Keep what a probe monitor saw at this fire in its spec, for the next
    /// fire's wake decision (`everruns_contracts::runtime::monitor_probe`).
    pub async fn set_monitor_probe_state(
        &self,
        session_id: Uuid,
        task_id: &str,
        state: serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE session_tasks SET spec = spec || jsonb_build_object('probe_state', $3::jsonb)
             WHERE session_id = $1 AND id = $2",
        )
        .bind(session_id)
        .bind(task_id)
        .bind(state)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    /// How many runs in a row the schedule has ended the same way.
    pub async fn schedule_same_outcome_runs(&self, schedule_id: Uuid) -> Result<i32> {
        Ok(sqlx::query_scalar::<_, i32>(
            "SELECT same_outcome_runs FROM session_schedules WHERE id = $1",
        )
        .bind(schedule_id)
        .fetch_optional(self.database().pool())
        .await?
        .unwrap_or(0))
    }

    /// The input message of the schedule's latest fire.
    pub async fn latest_schedule_fire(&self, schedule_id: Uuid) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT input_message_id FROM runtime_invocations WHERE schedule_id = $1
             ORDER BY input_message_id DESC LIMIT 1",
        )
        .bind(schedule_id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// The final answer of the turn that `input_message_id` started, if that
    /// turn has finished one: the message content, as stored.
    pub async fn turn_final_output(
        &self,
        session_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<Option<serde_json::Value>> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(data->'message'->'content', data->'data'->'message'->'content')
             FROM events
             WHERE session_id = $1 AND event_type = 'output.message.completed'
               AND context->>'input_message_id' = $2
             ORDER BY sequence DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(input_message_id.to_string())
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// Record a detector's observation of one schedule, agent or session.
    /// Reopening a resolved issue starts a new episode, so people are told
    /// again; a still-open issue keeps its episode and takes the new evidence.
    pub async fn observe_subject_health_issue(
        &self,
        issue: SubjectHealthIssue<'_>,
    ) -> Result<Option<ObservedIssue>> {
        let row = sqlx::query_as::<_, (Uuid, Uuid, String, bool)>(
            "WITH before AS (
                SELECT status FROM health_issues
                WHERE org_id = $1 AND code = $2 AND subject_id = $3
             )
             INSERT INTO health_issues (org_id, code, subject_id, agent_id, session_id, status,
                                        evidence, last_checked_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (org_id, code, subject_id) WHERE subject_id IS NOT NULL DO UPDATE SET
                episode_id = CASE WHEN health_issues.status IN ('resolved', 'inapplicable')
                    AND EXCLUDED.status = 'open' THEN uuidv7()
                    ELSE health_issues.episode_id END,
                first_detected_at = CASE WHEN health_issues.status IN ('resolved', 'inapplicable')
                    AND EXCLUDED.status = 'open' THEN EXCLUDED.last_checked_at
                    ELSE health_issues.first_detected_at END,
                status = EXCLUDED.status,
                agent_id = COALESCE(EXCLUDED.agent_id, health_issues.agent_id),
                session_id = COALESCE(EXCLUDED.session_id, health_issues.session_id),
                evidence = COALESCE(EXCLUDED.evidence, health_issues.evidence),
                last_checked_at = EXCLUDED.last_checked_at
             WHERE health_issues.last_checked_at <= EXCLUDED.last_checked_at
             RETURNING id, episode_id, status,
                 $6 = 'open' AND COALESCE((SELECT status FROM before), 'resolved')
                     NOT IN ('open', 'needs_check')",
        )
        .bind(issue.org_id)
        .bind(issue.code)
        .bind(issue.subject_id)
        .bind(issue.agent_id)
        .bind(issue.session_id)
        .bind(issue.status)
        .bind(&issue.evidence)
        .bind(issue.checked_at)
        .fetch_optional(self.database().pool())
        .await?;
        let Some((id, episode_id, status, opened)) = row else {
            return Ok(None);
        };
        if status == "resolved" {
            // Announcements of a resolved episode are read; nobody has to act.
            sqlx::query(
                "UPDATE notifications SET viewed_at = COALESCE(viewed_at, NOW()),
                    payload = payload || jsonb_build_object('status', 'resolved')
                 WHERE org_id = $1 AND kind = 'health.issue' AND target_id = $2
                   AND dedupe_key = $3",
            )
            .bind(issue.org_id)
            .bind(id.to_string())
            .bind(format!("health:{id}:{episode_id}"))
            .execute(self.database().pool())
            .await?;
        }
        Ok(Some(ObservedIssue {
            id,
            episode_id,
            status,
            opened,
        }))
    }

    /// Open issues of `code` that are about one subject, for re-evaluation.
    pub async fn open_subject_health_issues(&self, code: &str) -> Result<Vec<HealthIssueRow>> {
        Ok(sqlx::query_as(
            "SELECT h.*, NULL::text AS channel_public_id, a.public_id AS agent_public_id,
                    COALESCE(a.display_name, a.name) AS agent_name
             FROM health_issues h LEFT JOIN agents a ON a.id = h.agent_id
             WHERE h.code = $1 AND h.subject_id IS NOT NULL
               AND h.status IN ('open', 'needs_check')",
        )
        .bind(code)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Tell `users` about an issue episode in their notifications. With
    /// `resurface`, an announcement a user already read is updated and shown
    /// again, for an issue that got worse (a warned schedule that stopped).
    #[allow(clippy::too_many_arguments)]
    pub async fn announce_health_issue(
        &self,
        org_id: i64,
        issue: &ObservedIssue,
        users: &[Uuid],
        title: &str,
        body: &str,
        href: &str,
        agent: Option<(&str, &str)>,
        resurface: bool,
    ) -> Result<()> {
        let dedupe_key = format!("health:{}:{}", issue.id, issue.episode_id);
        let payload = serde_json::json!({"issue_id": issue.id, "episode_id": issue.episode_id});
        let (source_type, source_id, source_name) = match agent {
            Some((id, name)) => (Some("agent"), Some(id), Some(name)),
            None => (None, None, None),
        };
        for user in users {
            sqlx::query(
                "INSERT INTO notifications (org_id, user_id, kind, title, body, target_type,
                    target_id, href, payload, dedupe_key, source_type, source_id, source_name)
                 VALUES ($1, $2, 'health.issue', $3, $4, 'health_issue', $5, $6, $7, $8, $9, $10, $11)
                 ON CONFLICT DO NOTHING",
            )
            .bind(org_id)
            .bind(user)
            .bind(title)
            .bind(body)
            .bind(issue.id.to_string())
            .bind(href)
            .bind(&payload)
            .bind(&dedupe_key)
            .bind(source_type)
            .bind(source_id)
            .bind(source_name)
            .execute(self.database().pool())
            .await?;
            if resurface {
                sqlx::query(
                    "UPDATE notifications SET title = $4, body = $5, viewed_at = NULL,
                        updated_at = NOW()
                     WHERE org_id = $1 AND user_id = $2 AND kind = 'health.issue'
                       AND dedupe_key = $3",
                )
                .bind(org_id)
                .bind(user)
                .bind(&dedupe_key)
                .bind(title)
                .bind(body)
                .execute(self.database().pool())
                .await?;
            }
        }
        Ok(())
    }

    /// People who changed the agent and are still members of its org: the
    /// agent's authors, who hear about what its unattended work does.
    pub async fn agent_author_user_ids(&self, org_id: i64, agent_id: Uuid) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT c.actor_user_id FROM entity_changes c
             JOIN agents a ON a.org_id = c.org_id AND c.entity_kind = 'agent'
                AND c.entity_ref = a.public_id
             JOIN organization_members m ON m.org_id = c.org_id AND m.user_id = c.actor_user_id
             WHERE c.org_id = $1 AND a.id = $2 AND c.actor_user_id IS NOT NULL",
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// The org's owners.
    pub async fn org_owner_user_ids(&self, org_id: i64) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT user_id FROM organization_members WHERE org_id = $1 AND role = 'owner'",
        )
        .bind(org_id)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Whether the user is still a member of the org.
    pub async fn is_org_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM organization_members WHERE org_id = $1 AND user_id = $2)",
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_one(self.database().pool())
        .await?)
    }

    /// Orgs whose unattended budget for the current day is spent.
    pub async fn orgs_with_spent_unattended_budget(&self) -> Result<Vec<(i64, f64)>> {
        Ok(sqlx::query_as(
            r#"SELECT org_id, "limit" FROM budgets
               WHERE subject_type = 'org_unattended' AND status <> 'disabled'
                 AND (status = 'exhausted' OR balance <= 0)
                 AND period_started_at >= date_trunc('day', NOW() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'"#,
        )
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Agents whose unattended spend over the last day passed `floor_usd` and
    /// `factor` times their average day over the six days before it. Spend is
    /// what the agents' turns charged the orgs' `org_unattended` budgets.
    pub async fn unattended_spend_spikes(
        &self,
        floor_usd: f64,
        factor: f64,
    ) -> Result<Vec<UnattendedSpendRow>> {
        Ok(sqlx::query_as(
            r#"WITH spend AS (
                SELECT b.org_id, COALESCE(l.agent_id, s.agent_id) AS agent_id,
                    SUM(l.amount) FILTER (WHERE l.created_at >= NOW() - INTERVAL '1 day') AS last_day,
                    SUM(l.amount) FILTER (WHERE l.created_at < NOW() - INTERVAL '1 day') AS before
                FROM budgets b
                JOIN usage_ledger l ON l.budget_id = b.id
                LEFT JOIN sessions s ON s.id = l.session_id
                WHERE b.subject_type = 'org_unattended' AND l.amount > 0
                  AND l.created_at >= NOW() - INTERVAL '7 days'
                GROUP BY b.org_id, COALESCE(l.agent_id, s.agent_id)
            )
            SELECT org_id, agent_id, COALESCE(last_day, 0)::float8 AS last_day_usd,
                   (COALESCE(before, 0) / 6.0)::float8 AS baseline_daily_usd
            FROM spend
            WHERE agent_id IS NOT NULL AND COALESCE(last_day, 0) > $1
              AND COALESCE(last_day, 0) > $2 * COALESCE(before, 0) / 6.0"#,
        )
        .bind(floor_usd)
        .bind(factor)
        .fetch_all(self.database().pool())
        .await?)
    }

    /// Sessions with model calls in the last hour whose last `calls` calls
    /// averaged more than `min_prompt_tokens` prompt tokens with less than
    /// `max_cached_ratio` of them cached.
    pub async fn bloated_sessions(
        &self,
        calls: i64,
        min_prompt_tokens: f64,
        max_cached_ratio: f64,
    ) -> Result<Vec<ContextSizeRow>> {
        Ok(sqlx::query_as(
            "WITH active AS (
                SELECT DISTINCT session_id FROM llm_generations
                WHERE created_at >= NOW() - INTERVAL '1 hour' AND session_id IS NOT NULL
             ), recent AS (
                SELECT g.org_id, g.session_id, g.input_tokens, g.cache_read_tokens
                FROM active
                CROSS JOIN LATERAL (
                    SELECT org_id, session_id, input_tokens, cache_read_tokens
                    FROM llm_generations
                    WHERE session_id = active.session_id
                      AND created_at >= NOW() - INTERVAL '1 day'
                    ORDER BY created_at DESC LIMIT $1
                ) g
             ), sized AS (
                SELECT org_id, session_id, COUNT(*) AS calls,
                    AVG(input_tokens + cache_read_tokens)::float8 AS avg_prompt_tokens,
                    (SUM(cache_read_tokens)::float8
                        / NULLIF(SUM(input_tokens + cache_read_tokens), 0))::float8 AS cached_ratio
                FROM recent GROUP BY org_id, session_id
             )
             SELECT sized.org_id, sized.session_id, s.agent_id, sized.calls,
                    sized.avg_prompt_tokens, COALESCE(sized.cached_ratio, 0) AS cached_ratio
             FROM sized JOIN sessions s ON s.id = sized.session_id
             WHERE sized.calls >= $1 AND sized.avg_prompt_tokens > $2
               AND COALESCE(sized.cached_ratio, 0) < $3",
        )
        .bind(calls)
        .bind(min_prompt_tokens)
        .bind(max_cached_ratio)
        .fetch_all(self.database().pool())
        .await?)
    }
}

/// One schedule as the detectors see it.
#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleWatchRow {
    pub id: Uuid,
    pub public_id: String,
    pub org_id: i64,
    pub session_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub agent_public_id: Option<String>,
    pub agent_name: Option<String>,
    pub owner_user_id: Option<Uuid>,
    pub description: String,
    pub enabled: bool,
    pub stop_reason: Option<String>,
    pub deleted: bool,
    pub trigger_count: i32,
    pub same_outcome_runs: i32,
}

impl StorageBackend {
    /// The schedule with this public id, with its session's agent.
    pub async fn schedule_watch(&self, public_id: &str) -> Result<Option<ScheduleWatchRow>> {
        Ok(sqlx::query_as(
            "SELECT sc.id, sc.public_id, sc.org_id, sc.session_id, s.agent_id,
                    a.public_id AS agent_public_id, COALESCE(a.display_name, a.name) AS agent_name,
                    sc.resolved_owner_user_id AS owner_user_id, sc.description, sc.enabled,
                    sc.stop_reason, sc.deleted_at IS NOT NULL AS deleted, sc.trigger_count,
                    sc.same_outcome_runs
             FROM session_schedules sc
             JOIN sessions s ON s.id = sc.session_id
             LEFT JOIN agents a ON a.id = s.agent_id
             WHERE sc.public_id = $1",
        )
        .bind(public_id)
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// An agent's public id and display name.
    pub async fn agent_label(&self, agent_id: Uuid) -> Result<Option<(String, String)>> {
        Ok(sqlx::query_as(
            "SELECT public_id, COALESCE(display_name, name) FROM agents WHERE id = $1",
        )
        .bind(agent_id)
        .fetch_optional(self.database().pool())
        .await?)
    }
}
