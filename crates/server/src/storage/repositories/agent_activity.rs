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
//
// Decision: "people reached" counts distinct end-user principals behind a
// channel's sessions: the user participants a channel records (Slack senders),
// minus the session owner that every session lists (on a channel that is the
// channel's own identity), and the virtual user stamped on each `input.message`
// (public chat, signed-in AG-UI, PACT A2A callers). Webhook, API, FCP and
// anonymous AG-UI or plain A2A traffic carries no person, so those channels
// report no identified sessions and the page shows sessions only rather than
// guessing a head count.
//
// Decision: "first reply" is the time from a session's first `input.message` to
// its first `output.message.completed`, the persisted message pair the events
// table indexes. The channel reports the median over its sessions in the window.

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

/// Channel sessions in the window and the end-user principals behind them.
/// Binds: $1 org id, $2 now, $3 window days.
const CHANNEL_PEOPLE_CTE: &str = r#"
WITH win AS (
    SELECT s.id, s.channel_id, s.owner_principal_id
    FROM sessions s
    WHERE s.org_id = $1
      AND s.channel_id IS NOT NULL
      AND s.created_at > $2 - make_interval(days => $3)
      AND s.created_at <= $2
),
people AS (
    SELECT w.channel_id, w.id AS session_id, p.principal_id::text AS person
    FROM win w
    JOIN session_participants p ON p.session_id = w.id AND p.kind = 'user'
    -- Every session records its owner as a user participant; on a channel
    -- that is the channel's own identity, not someone it reached.
    WHERE p.principal_id IS DISTINCT FROM w.owner_principal_id
    UNION
    SELECT w.channel_id, w.id, e.metadata->>'principal_id'
    FROM win w
    JOIN events e ON e.session_id = w.id
    WHERE e.event_type = 'input.message'
      AND e.metadata->>'type' = 'virtual_user'
      AND e.metadata->>'principal_id' IS NOT NULL
)
"#;

/// Who a channel reached and how fast it answered over the whole window.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChannelAudienceRow {
    pub channel_id: String,
    /// Distinct end-user principals across the channel's sessions.
    pub people: i64,
    /// Sessions that carried any end-user identity at all.
    pub identified_sessions: i64,
    /// Median milliseconds from first user message to first agent reply.
    pub median_first_reply_ms: Option<f64>,
    /// Sessions with both a user message and a completed reply.
    pub replied_sessions: i64,
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
    pub channel_audience: Vec<ChannelAudienceRow>,
    /// Distinct end users across every channel in the window.
    pub people_reached: i64,
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

        // Only the constant CTE text is interpolated; every value is bound.
        let audience_sql = format!(
            r#"
            {CHANNEL_PEOPLE_CTE},
            reach AS (
                SELECT channel_id,
                       COUNT(DISTINCT person)::bigint AS people,
                       COUNT(DISTINCT session_id)::bigint AS identified_sessions
                FROM people
                GROUP BY channel_id
            ),
            firsts AS (
                SELECT w.channel_id,
                       MIN(e.ts) FILTER (WHERE e.event_type = 'input.message') AS first_in,
                       MIN(e.ts) FILTER (WHERE e.event_type = 'output.message.completed') AS first_out
                FROM win w
                JOIN events e ON e.session_id = w.id
                WHERE e.event_type IN ('input.message', 'output.message.completed')
                GROUP BY w.id, w.channel_id
            ),
            speed AS (
                SELECT channel_id,
                       (percentile_cont(0.5) WITHIN GROUP (
                           ORDER BY EXTRACT(EPOCH FROM (first_out - first_in)) * 1000
                       ))::float8 AS median_first_reply_ms,
                       COUNT(*)::bigint AS replied_sessions
                FROM firsts
                WHERE first_in IS NOT NULL AND first_out > first_in
                GROUP BY channel_id
            )
            SELECT c.public_id AS channel_id,
                   COALESCE(r.people, 0) AS people,
                   COALESCE(r.identified_sessions, 0) AS identified_sessions,
                   sp.median_first_reply_ms,
                   COALESCE(sp.replied_sessions, 0) AS replied_sessions
            FROM agent_channels c
            LEFT JOIN reach r ON r.channel_id = c.id
            LEFT JOIN speed sp ON sp.channel_id = c.id
            WHERE r.channel_id IS NOT NULL OR sp.channel_id IS NOT NULL
            "#
        );
        let channel_audience =
            sqlx::query_as::<_, ChannelAudienceRow>(sqlx::AssertSqlSafe(audience_sql.as_str()))
                .bind(org_id)
                .bind(now)
                .bind(CHANNEL_ACTIVITY_DAYS)
                .fetch_all(&self.pool)
                .await?;

        // People can reach the org through several channels; the org figure
        // counts each of them once.
        let people_sql =
            format!("{CHANNEL_PEOPLE_CTE} SELECT COUNT(DISTINCT person)::bigint FROM people");
        let people_reached: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(people_sql.as_str()))
            .bind(org_id)
            .bind(now)
            .bind(CHANNEL_ACTIVITY_DAYS)
            .fetch_one(&self.pool)
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
            channel_audience,
            people_reached,
        })
    }
}
