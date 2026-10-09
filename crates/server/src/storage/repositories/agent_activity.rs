// Org-wide agent activity for the Agents home page: what each agent is doing
// now, how its last day of runs went, and how much traffic each channel took.
//
// Decision: a "run" is a turn. Turns are what the active-turn limit counts and
// what fails visibly, so the run bars and the failure counts read the same
// `turn.started` / `turn.failed` events the session counters are built from.
//
// Decision: the 24-hour window is read through sessions touched in that window
// (`sessions.updated_at`, bumped by the event counter trigger on every append)
// and then each session's turn events through `idx_events_turns`. That keeps
// the read proportional to recent activity instead of the org's whole history.

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};

/// Hours covered by the per-agent run buckets.
pub const AGENT_ACTIVITY_HOURS: i32 = 24;
/// Days covered by the per-channel session buckets.
pub const CHANNEL_ACTIVITY_DAYS: i32 = 7;

/// One agent's current load and most recent turn.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentLoadRow {
    pub agent_id: String,
    /// Sessions executing a turn right now (`status = 'active'`, the same
    /// definition the org active-turn limit uses).
    pub running_sessions: i64,
    pub last_turn_at: Option<DateTime<Utc>>,
}

/// Turns one agent started, and turns that failed, in one hour of the window.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentRunBucketRow {
    pub agent_id: String,
    /// 0 is the hour ending at `now`, 23 the oldest.
    pub hours_ago: i32,
    pub runs: i64,
    pub failed: i64,
}

/// Sessions one channel created in one day of the window.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChannelSessionBucketRow {
    pub channel_id: String,
    /// 0 is the day ending at `now`, 6 the oldest.
    pub days_ago: i32,
    pub sessions: i64,
    /// Newest session the channel started inside this bucket.
    pub last_session_at: DateTime<Utc>,
}

/// One live trigger: a way the agent starts itself.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentTriggerSummaryRow {
    pub agent_id: String,
    pub trigger_type: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AgentActivityRows {
    pub loads: Vec<AgentLoadRow>,
    pub triggers: Vec<AgentTriggerSummaryRow>,
    pub run_buckets: Vec<AgentRunBucketRow>,
    pub channel_buckets: Vec<ChannelSessionBucketRow>,
}

impl Database {
    /// Activity for every agent and channel in `org_id`, as seen at `now`.
    pub async fn agent_activity(
        &self,
        org_id: i64,
        now: DateTime<Utc>,
    ) -> Result<AgentActivityRows> {
        let loads = sqlx::query_as::<_, AgentLoadRow>(
            r#"
            SELECT a.public_id AS agent_id,
                   COUNT(*) FILTER (WHERE s.status = 'active')::bigint AS running_sessions,
                   MAX(s.last_turn_at) AS last_turn_at
            FROM sessions s
            JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            WHERE s.org_id = $1
            GROUP BY a.public_id
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        let run_buckets = sqlx::query_as::<_, AgentRunBucketRow>(
            r#"
            SELECT a.public_id AS agent_id,
                   LEAST($3 - 1, FLOOR(EXTRACT(EPOCH FROM ($2 - e.ts)) / 3600))::int AS hours_ago,
                   COUNT(*) FILTER (WHERE e.event_type = 'turn.started')::bigint AS runs,
                   COUNT(*) FILTER (WHERE e.event_type = 'turn.failed')::bigint AS failed
            FROM sessions s
            JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
            JOIN events e ON e.session_id = s.id
            WHERE s.org_id = $1
              AND s.updated_at > $2 - make_interval(hours => $3)
              AND e.event_type IN ('turn.started', 'turn.failed')
              AND e.ts > $2 - make_interval(hours => $3)
              AND e.ts <= $2
            GROUP BY 1, 2
            "#,
        )
        .bind(org_id)
        .bind(now)
        .bind(AGENT_ACTIVITY_HOURS)
        .fetch_all(&self.pool)
        .await?;

        let channel_buckets = sqlx::query_as::<_, ChannelSessionBucketRow>(
            r#"
            SELECT c.public_id AS channel_id,
                   LEAST($3 - 1, FLOOR(EXTRACT(EPOCH FROM ($2 - s.created_at)) / 86400))::int AS days_ago,
                   COUNT(*)::bigint AS sessions,
                   MAX(s.created_at) AS last_session_at
            FROM sessions s
            JOIN agent_channels c ON c.id = s.channel_id
            WHERE s.org_id = $1
              AND s.created_at > $2 - make_interval(days => $3)
              AND s.created_at <= $2
            GROUP BY 1, 2
            "#,
        )
        .bind(org_id)
        .bind(now)
        .bind(CHANNEL_ACTIVITY_DAYS)
        .fetch_all(&self.pool)
        .await?;

        let triggers = sqlx::query_as::<_, AgentTriggerSummaryRow>(
            r#"
            SELECT a.public_id AS agent_id, t.trigger_type, t.enabled
            FROM agent_triggers t
            JOIN agents a ON a.id = t.agent_id AND a.org_id = t.org_id
            WHERE t.org_id = $1 AND t.status = 'active'
            ORDER BY t.created_at, t.id
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(AgentActivityRows {
            loads,
            triggers,
            run_buckets,
            channel_buckets,
        })
    }
}
