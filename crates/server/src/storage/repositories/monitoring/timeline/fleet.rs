//! Fleet history for Monitoring → Timeline: what the 10-second sampler
//! cannot see (open sessions and agents in total, server replicas) and what
//! data plane v2 cells report through the feed (`monitoring_cell_minutes`
//! and the `dp_*` columns of `monitoring_minutes`).
//!
//! Decisions:
//! - Open sessions and defined agents are counted once a minute and written
//!   only to organization rows that already exist for that minute, plus the
//!   platform row. An idle organization gets no row; the reader carries the
//!   last known value forward instead, so history costs nothing for
//!   organizations that do nothing.
//! - A cell's minute report is applied once: the first report stored for a
//!   (cell, minute) wins, because a new leader may send the same minute
//!   again. Its population is added to the platform row and to the rows of
//!   organizations with sessions on the cell.
//! - Turn and spend counters from cells are added per feed batch, after its
//!   records are stored, and never recomputed: the closing job reads v1
//!   tables only. A batch delivered twice after a failed acknowledgement can
//!   count twice; monitoring accepts that rather than a lookup per record.

use super::{Database, PLATFORM_ORG, timeline_minute};
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use std::collections::BTreeMap;

/// How recently a server replica must have reported to count as running
/// (matches the doctor's live window).
const SERVER_LIVE_SECS: i64 = 90;

/// One cell's minute report, as stored.
#[derive(Debug, Clone, Default)]
pub struct CellMinuteReport {
    pub cell: String,
    pub data_plane: String,
    pub minute: DateTime<Utc>,
    pub leader: String,
    pub nodes: i32,
    pub nodes_answering: i32,
    pub sessions: i64,
    pub running: i64,
    pub queued: i64,
    pub resumed: i64,
    pub rss_peak_bytes: i64,
    pub memory_limit_bytes: i64,
    pub cpu_peak_cores: f32,
    pub cpu_limit_cores: f32,
    pub admit_p99_ms: i32,
    pub ship_lag_p99_ms: i32,
    pub raft_term: i64,
    pub forwarded: i64,
    /// (internal org id, sessions, running, queued, agents working)
    pub orgs: Vec<(i64, i64, i64, i64, i64)>,
}

/// Turn and model counters from cell feed records for one minute.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellCounters {
    pub turns_completed: i32,
    pub turns_failed: i32,
    pub turns_cancelled: i32,
    pub model_calls: i32,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

impl CellCounters {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn add(&mut self, other: &CellCounters) {
        self.turns_completed += other.turns_completed;
        self.turns_failed += other.turns_failed;
        self.turns_cancelled += other.turns_cancelled;
        self.model_calls += other.model_calls;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cost_usd += other.cost_usd;
    }
}

/// Counters of one feed batch: per (organization, minute), and per minute
/// for the cell that sent it.
#[derive(Debug, Clone, Default)]
pub struct CellBatchCounters {
    pub orgs: BTreeMap<(i64, DateTime<Utc>), CellCounters>,
}

impl CellBatchCounters {
    pub fn entry(&mut self, org: i64, at: DateTime<Utc>) -> &mut CellCounters {
        self.orgs.entry((org, timeline_minute(at))).or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.orgs.values().all(CellCounters::is_empty)
    }

    /// Per-minute totals across organizations (platform row and cell row).
    pub fn minutes(&self) -> BTreeMap<DateTime<Utc>, CellCounters> {
        let mut out: BTreeMap<DateTime<Utc>, CellCounters> = BTreeMap::new();
        for ((_, minute), c) in &self.orgs {
            out.entry(*minute).or_default().add(c);
        }
        out
    }
}

/// Fleet columns of one bucket (`timeline_fleet_buckets`).
#[derive(Debug, Clone, FromRow)]
pub struct TimelineFleetRow {
    pub bucket: DateTime<Utc>,
    pub sessions: Option<i64>,
    pub agents: Option<i32>,
    pub agents_working_sum: i64,
    pub agents_working_peak: i32,
    pub servers: Option<i32>,
    pub dp_minutes: i64,
    pub dp_reports: i32,
    pub dp_sessions_sum: i64,
    pub dp_sessions_peak: i64,
    pub dp_running_sum: i64,
    pub dp_running_peak: i64,
    pub dp_queued_peak: i64,
    pub dp_agents_working_sum: i64,
    pub dp_agents_working_peak: i64,
    pub dp_turns_completed: i64,
    pub dp_turns_failed: i64,
    pub dp_turns_cancelled: i64,
    pub dp_model_calls: i64,
    pub dp_input_tokens: i64,
    pub dp_output_tokens: i64,
    pub dp_cost_usd: f64,
}

/// Cell-wide columns of one bucket, across all cells (platform scope).
#[derive(Debug, Clone, FromRow)]
pub struct TimelineCellsRow {
    pub bucket: DateTime<Utc>,
    pub cells: i64,
    pub nodes: i64,
    pub nodes_answering: i64,
    /// Highest node memory use as a share of its limit, 0..1.
    pub memory_peak: Option<f64>,
    /// Highest node CPU use as a share of its limit, 0..1.
    pub cpu_peak: Option<f64>,
    pub admit_p99_ms: Option<i32>,
    pub ship_lag_p99_ms: Option<i32>,
    pub elections: i64,
    pub moves: i64,
    pub resumed_peak: i64,
}

/// One cell over one bucket.
#[derive(Debug, Clone, FromRow)]
pub struct TimelineCellBucketRow {
    pub cell: String,
    pub bucket: DateTime<Utc>,
    pub reports: i64,
    pub nodes: Option<i32>,
    pub nodes_answering: Option<i32>,
    pub sessions_peak: Option<i64>,
    pub running_peak: Option<i64>,
    pub queued_peak: Option<i64>,
    pub rss_peak_bytes: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    pub cpu_peak_cores: Option<f32>,
    pub cpu_limit_cores: Option<f32>,
    pub admit_p99_ms: Option<i32>,
    pub ship_lag_p99_ms: Option<i32>,
    pub elections: i64,
    pub moves: i64,
    pub turns_completed: i64,
    pub turns_failed: i64,
    pub turns_cancelled: i64,
}

/// A cell's newest stored report.
#[derive(Debug, Clone, FromRow)]
pub struct CellLatestRow {
    pub cell: String,
    pub data_plane: String,
    pub minute: DateTime<Utc>,
    pub leader: Option<String>,
    pub nodes: Option<i32>,
    pub nodes_answering: Option<i32>,
    pub sessions: Option<i64>,
    pub running: Option<i64>,
    pub queued: Option<i64>,
    pub resumed: Option<i64>,
    pub rss_peak_bytes: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    pub cpu_peak_cores: Option<f32>,
    pub cpu_limit_cores: Option<f32>,
    pub admit_p99_ms: Option<i32>,
    pub ship_lag_p99_ms: Option<i32>,
    pub raft_term: Option<i64>,
}

// Per cell and minute, with term and forwarded differences from the cell's
// previous reported minute. Rows from a little before $1 seed the first
// difference; callers filter to [$1, $2).
macro_rules! cell_minutes {
    () => {
        r#"
    SELECT c.*,
        GREATEST(0, COALESCE(c.raft_term - LAG(c.raft_term) OVER w, 0)) AS elections,
        GREATEST(0, COALESCE(c.forwarded - LAG(c.forwarded) OVER w, 0)) AS moves
    FROM monitoring_cell_minutes c
    WHERE c.minute >= $1 - interval '15 minutes' AND c.minute < $2 AND c.reported
    WINDOW w AS (PARTITION BY c.cell ORDER BY c.minute)
"#
    };
}

impl Database {
    /// Count open sessions and defined agents per organization, and running
    /// server replicas, into the minute containing `at`.
    pub async fn timeline_record_minute_counts(&self, at: DateTime<Utc>) -> Result<()> {
        let minute = timeline_minute(at);
        sqlx::query(
            r#"WITH s AS (
                SELECT org_id, COUNT(*)::bigint AS n FROM sessions
                WHERE archived_at IS NULL AND deleted_at IS NULL GROUP BY org_id
            ), a AS (
                SELECT org_id, COUNT(*)::integer AS n FROM agents
                WHERE archived_at IS NULL AND deleted_at IS NULL GROUP BY org_id
            ), per_org AS (
                SELECT COALESCE(s.org_id, a.org_id) AS org_id, COALESCE(s.n, 0) AS sessions,
                    COALESCE(a.n, 0) AS agents
                FROM s FULL JOIN a ON a.org_id = s.org_id
            ), orgs AS (
                UPDATE monitoring_minutes m SET sessions = p.sessions, agents = p.agents
                FROM per_org p WHERE m.org_id = p.org_id AND m.minute = $1 AND m.org_id <> 0
            )
            INSERT INTO monitoring_minutes AS m (org_id, minute, sessions, agents, servers)
            SELECT 0, $1, COALESCE(SUM(sessions), 0)::bigint, COALESCE(SUM(agents), 0)::integer,
                (SELECT COUNT(*)::integer FROM system_instances
                 WHERE kind = 'server' AND last_seen_at > $2)
            FROM per_org
            ON CONFLICT (org_id, minute) DO UPDATE SET sessions = EXCLUDED.sessions,
                agents = EXCLUDED.agents, servers = EXCLUDED.servers"#,
        )
        .bind(minute)
        .bind(at - chrono::Duration::seconds(SERVER_LIVE_SECS))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Store a cell's minute report. Returns false when the minute was
    /// already reported (a repeat from a new leader), which changes nothing.
    pub async fn monitoring_record_cell_report(&self, r: &CellMinuteReport) -> Result<bool> {
        let minute = timeline_minute(r.minute);
        let mut tx = self.pool.begin().await?;
        let first: Option<String> = sqlx::query_scalar(
            r#"INSERT INTO monitoring_cell_minutes AS c (cell, minute, data_plane, reported, leader,
                nodes, nodes_answering, sessions, running, queued, resumed, rss_peak_bytes,
                memory_limit_bytes, cpu_peak_cores, cpu_limit_cores, admit_p99_ms, ship_lag_p99_ms,
                raft_term, forwarded)
            VALUES ($1, $2, $3, TRUE, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
            ON CONFLICT (cell, minute) DO UPDATE SET data_plane = EXCLUDED.data_plane, reported = TRUE,
                leader = EXCLUDED.leader, nodes = EXCLUDED.nodes,
                nodes_answering = EXCLUDED.nodes_answering, sessions = EXCLUDED.sessions,
                running = EXCLUDED.running, queued = EXCLUDED.queued, resumed = EXCLUDED.resumed,
                rss_peak_bytes = EXCLUDED.rss_peak_bytes,
                memory_limit_bytes = EXCLUDED.memory_limit_bytes,
                cpu_peak_cores = EXCLUDED.cpu_peak_cores, cpu_limit_cores = EXCLUDED.cpu_limit_cores,
                admit_p99_ms = EXCLUDED.admit_p99_ms, ship_lag_p99_ms = EXCLUDED.ship_lag_p99_ms,
                raft_term = EXCLUDED.raft_term, forwarded = EXCLUDED.forwarded
            WHERE NOT c.reported
            RETURNING cell"#,
        )
        .bind(&r.cell)
        .bind(minute)
        .bind(&r.data_plane)
        .bind(&r.leader)
        .bind(r.nodes)
        .bind(r.nodes_answering)
        .bind(r.sessions)
        .bind(r.running)
        .bind(r.queued)
        .bind(r.resumed)
        .bind(r.rss_peak_bytes)
        .bind(r.memory_limit_bytes)
        .bind(r.cpu_peak_cores)
        .bind(r.cpu_limit_cores)
        .bind(r.admit_p99_ms)
        .bind(r.ship_lag_p99_ms)
        .bind(r.raft_term)
        .bind(r.forwarded)
        .fetch_optional(&mut *tx)
        .await?;
        if first.is_none() {
            return Ok(false);
        }
        let working: i64 = r.orgs.iter().map(|o| o.4).sum();
        let mut ids = vec![PLATFORM_ORG];
        let mut sessions = vec![r.sessions];
        let mut running = vec![r.running];
        let mut queued = vec![r.queued];
        let mut agents = vec![working];
        for &(org, s, run, q, a) in r.orgs.iter().filter(|o| o.0 != PLATFORM_ORG && o.1 > 0) {
            ids.push(org);
            sessions.push(s);
            running.push(run);
            queued.push(q);
            agents.push(a);
        }
        sqlx::query(
            r#"INSERT INTO monitoring_minutes AS m (org_id, minute, dp_reports, dp_sessions, dp_running,
                dp_queued, dp_agents_working)
            SELECT u.org_id, $1, 1, u.sessions, u.running, u.queued, u.agents
            FROM UNNEST($2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[], $6::bigint[])
                AS u(org_id, sessions, running, queued, agents)
            ON CONFLICT (org_id, minute) DO UPDATE SET dp_reports = m.dp_reports + 1,
                dp_sessions = m.dp_sessions + EXCLUDED.dp_sessions,
                dp_running = m.dp_running + EXCLUDED.dp_running,
                dp_queued = m.dp_queued + EXCLUDED.dp_queued,
                dp_agents_working = m.dp_agents_working + EXCLUDED.dp_agents_working"#,
        )
        .bind(minute)
        .bind(ids)
        .bind(sessions)
        .bind(running)
        .bind(queued)
        .bind(agents)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Add one feed batch's counters to its organizations, the platform, and
    /// the cell that sent it.
    pub async fn monitoring_add_cell_counters(
        &self,
        cell: &str,
        data_plane: &str,
        counters: &CellBatchCounters,
    ) -> Result<()> {
        if counters.is_empty() {
            return Ok(());
        }
        let minutes = counters.minutes();
        let mut rows: Vec<(i64, DateTime<Utc>, CellCounters)> = counters
            .orgs
            .iter()
            .filter(|(_, c)| !c.is_empty())
            .map(|((org, minute), c)| (*org, *minute, *c))
            .collect();
        rows.extend(minutes.iter().map(|(m, c)| (PLATFORM_ORG, *m, *c)));
        let ints = |f: fn(&CellCounters) -> i32| rows.iter().map(|r| f(&r.2)).collect::<Vec<_>>();
        let longs = |f: fn(&CellCounters) -> i64| rows.iter().map(|r| f(&r.2)).collect::<Vec<_>>();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO monitoring_minutes AS m (org_id, minute, dp_turns_completed, dp_turns_failed,
                dp_turns_cancelled, dp_model_calls, dp_input_tokens, dp_output_tokens, dp_cost_usd)
            SELECT * FROM UNNEST($1::bigint[], $2::timestamptz[], $3::integer[], $4::integer[],
                $5::integer[], $6::integer[], $7::bigint[], $8::bigint[], $9::float8[])
            ON CONFLICT (org_id, minute) DO UPDATE SET
                dp_turns_completed = m.dp_turns_completed + EXCLUDED.dp_turns_completed,
                dp_turns_failed = m.dp_turns_failed + EXCLUDED.dp_turns_failed,
                dp_turns_cancelled = m.dp_turns_cancelled + EXCLUDED.dp_turns_cancelled,
                dp_model_calls = m.dp_model_calls + EXCLUDED.dp_model_calls,
                dp_input_tokens = m.dp_input_tokens + EXCLUDED.dp_input_tokens,
                dp_output_tokens = m.dp_output_tokens + EXCLUDED.dp_output_tokens,
                dp_cost_usd = m.dp_cost_usd + EXCLUDED.dp_cost_usd"#,
        )
        .bind(rows.iter().map(|r| r.0).collect::<Vec<_>>())
        .bind(rows.iter().map(|r| r.1).collect::<Vec<_>>())
        .bind(ints(|c| c.turns_completed))
        .bind(ints(|c| c.turns_failed))
        .bind(ints(|c| c.turns_cancelled))
        .bind(ints(|c| c.model_calls))
        .bind(longs(|c| c.input_tokens))
        .bind(longs(|c| c.output_tokens))
        .bind(rows.iter().map(|r| r.2.cost_usd).collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
        let cell_minutes: Vec<(&DateTime<Utc>, &CellCounters)> = minutes
            .iter()
            .filter(|(_, c)| c.turns_completed + c.turns_failed + c.turns_cancelled > 0)
            .collect();
        if !cell_minutes.is_empty() {
            sqlx::query(
                r#"INSERT INTO monitoring_cell_minutes AS c (cell, minute, data_plane, turns_completed,
                    turns_failed, turns_cancelled)
                SELECT $1, u.minute, $2, u.completed, u.failed, u.cancelled
                FROM UNNEST($3::timestamptz[], $4::integer[], $5::integer[], $6::integer[])
                    AS u(minute, completed, failed, cancelled)
                ON CONFLICT (cell, minute) DO UPDATE SET
                    turns_completed = c.turns_completed + EXCLUDED.turns_completed,
                    turns_failed = c.turns_failed + EXCLUDED.turns_failed,
                    turns_cancelled = c.turns_cancelled + EXCLUDED.turns_cancelled"#,
            )
            .bind(cell)
            .bind(data_plane)
            .bind(cell_minutes.iter().map(|m| *m.0).collect::<Vec<_>>())
            .bind(cell_minutes.iter().map(|m| m.1.turns_completed).collect::<Vec<_>>())
            .bind(cell_minutes.iter().map(|m| m.1.turns_failed).collect::<Vec<_>>())
            .bind(cell_minutes.iter().map(|m| m.1.turns_cancelled).collect::<Vec<_>>())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Fleet columns aggregated into buckets for one organization or the
    /// platform. Open sessions and agents are the bucket's newest count.
    pub async fn timeline_fleet_buckets(
        &self,
        org: i64,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        bucket_secs: i64,
    ) -> Result<Vec<TimelineFleetRow>> {
        Ok(sqlx::query_as(
            r#"SELECT to_timestamp(floor(extract(epoch FROM minute) / $4) * $4) AS bucket,
                (array_agg(sessions ORDER BY minute DESC) FILTER (WHERE sessions IS NOT NULL))[1] AS sessions,
                (array_agg(agents ORDER BY minute DESC) FILTER (WHERE agents IS NOT NULL))[1] AS agents,
                COALESCE(SUM(agents_working_sum), 0)::bigint AS agents_working_sum,
                COALESCE(MAX(agents_working_peak), 0) AS agents_working_peak,
                MAX(servers) AS servers,
                COUNT(*) FILTER (WHERE dp_reports > 0) AS dp_minutes,
                COALESCE(MAX(dp_reports), 0) AS dp_reports,
                COALESCE(SUM(dp_sessions), 0)::bigint AS dp_sessions_sum,
                COALESCE(MAX(dp_sessions), 0)::bigint AS dp_sessions_peak,
                COALESCE(SUM(dp_running), 0)::bigint AS dp_running_sum,
                COALESCE(MAX(dp_running), 0)::bigint AS dp_running_peak,
                COALESCE(MAX(dp_queued), 0)::bigint AS dp_queued_peak,
                COALESCE(SUM(dp_agents_working), 0)::bigint AS dp_agents_working_sum,
                COALESCE(MAX(dp_agents_working), 0)::bigint AS dp_agents_working_peak,
                COALESCE(SUM(dp_turns_completed), 0)::bigint AS dp_turns_completed,
                COALESCE(SUM(dp_turns_failed), 0)::bigint AS dp_turns_failed,
                COALESCE(SUM(dp_turns_cancelled), 0)::bigint AS dp_turns_cancelled,
                COALESCE(SUM(dp_model_calls), 0)::bigint AS dp_model_calls,
                COALESCE(SUM(dp_input_tokens), 0)::bigint AS dp_input_tokens,
                COALESCE(SUM(dp_output_tokens), 0)::bigint AS dp_output_tokens,
                COALESCE(SUM(dp_cost_usd), 0)::float8 AS dp_cost_usd
            FROM monitoring_minutes WHERE org_id = $1 AND minute >= $2 AND minute < $3
            GROUP BY 1 ORDER BY 1"#,
        )
        .bind(org)
        .bind(from)
        .bind(to)
        .bind(bucket_secs)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Newest open-session and agent counts before `before`: where a range
    /// whose first buckets have no row starts from.
    pub async fn timeline_counts_before(
        &self,
        org: i64,
        before: DateTime<Utc>,
    ) -> Result<(Option<i64>, Option<i32>)> {
        Ok(sqlx::query_as(
            r#"SELECT sessions, agents FROM monitoring_minutes
            WHERE org_id = $1 AND minute < $2 AND sessions IS NOT NULL
            ORDER BY minute DESC LIMIT 1"#,
        )
        .bind(org)
        .bind(before)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or((None, None)))
    }

    /// Cells, their nodes and the fullest node, per bucket (platform).
    pub async fn timeline_cells_buckets(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        bucket_secs: i64,
    ) -> Result<Vec<TimelineCellsRow>> {
        let sql = concat!(
            r#"WITH c AS ("#,
            cell_minutes!(),
            r#"), per_minute AS (
                SELECT minute, COUNT(*) AS cells, SUM(nodes)::bigint AS nodes,
                    SUM(nodes_answering)::bigint AS nodes_answering,
                    MAX(rss_peak_bytes::float8 / NULLIF(memory_limit_bytes, 0)) AS memory_peak,
                    MAX(cpu_peak_cores::float8 / NULLIF(cpu_limit_cores, 0)) AS cpu_peak,
                    MAX(admit_p99_ms) AS admit_p99_ms, MAX(ship_lag_p99_ms) AS ship_lag_p99_ms,
                    SUM(elections)::bigint AS elections, SUM(moves)::bigint AS moves,
                    SUM(resumed)::bigint AS resumed
                FROM c WHERE minute >= $1 GROUP BY minute
            )
            SELECT to_timestamp(floor(extract(epoch FROM minute) / $3) * $3) AS bucket,
                MAX(cells)::bigint AS cells, MAX(nodes)::bigint AS nodes,
                MIN(nodes_answering)::bigint AS nodes_answering,
                MAX(memory_peak) AS memory_peak, MAX(cpu_peak) AS cpu_peak,
                MAX(admit_p99_ms) AS admit_p99_ms, MAX(ship_lag_p99_ms) AS ship_lag_p99_ms,
                COALESCE(SUM(elections), 0)::bigint AS elections,
                COALESCE(SUM(moves), 0)::bigint AS moves,
                COALESCE(MAX(resumed), 0)::bigint AS resumed_peak
            FROM per_minute GROUP BY 1 ORDER BY 1"#
        );
        Ok(sqlx::query_as(sql)
            .bind(from)
            .bind(to)
            .bind(bucket_secs)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Every cell's history over the range, per bucket.
    pub async fn timeline_cell_buckets(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        bucket_secs: i64,
    ) -> Result<Vec<TimelineCellBucketRow>> {
        let sql = concat!(
            r#"WITH r AS ("#,
            cell_minutes!(),
            r#"), t AS (
                SELECT cell, minute, turns_completed, turns_failed, turns_cancelled
                FROM monitoring_cell_minutes WHERE minute >= $1 AND minute < $2
            ), j AS (
                SELECT t.cell, t.minute, t.turns_completed, t.turns_failed, t.turns_cancelled,
                    r.nodes, r.nodes_answering, r.sessions, r.running, r.queued, r.rss_peak_bytes,
                    r.memory_limit_bytes, r.cpu_peak_cores, r.cpu_limit_cores, r.admit_p99_ms,
                    r.ship_lag_p99_ms, r.elections, r.moves, r.cell IS NOT NULL AS reported
                FROM t LEFT JOIN r ON r.cell = t.cell AND r.minute = t.minute
            )
            SELECT cell, to_timestamp(floor(extract(epoch FROM minute) / $3) * $3) AS bucket,
                COUNT(*) FILTER (WHERE reported) AS reports,
                MAX(nodes) AS nodes, MIN(nodes_answering) AS nodes_answering,
                MAX(sessions) AS sessions_peak, MAX(running) AS running_peak,
                MAX(queued) AS queued_peak, MAX(rss_peak_bytes) AS rss_peak_bytes,
                MAX(memory_limit_bytes) AS memory_limit_bytes, MAX(cpu_peak_cores) AS cpu_peak_cores,
                MAX(cpu_limit_cores) AS cpu_limit_cores, MAX(admit_p99_ms) AS admit_p99_ms,
                MAX(ship_lag_p99_ms) AS ship_lag_p99_ms,
                COALESCE(SUM(elections), 0)::bigint AS elections,
                COALESCE(SUM(moves), 0)::bigint AS moves,
                SUM(turns_completed)::bigint AS turns_completed,
                SUM(turns_failed)::bigint AS turns_failed,
                SUM(turns_cancelled)::bigint AS turns_cancelled
            FROM j GROUP BY 1, 2 ORDER BY 1, 2"#
        );
        Ok(sqlx::query_as(sql)
            .bind(from)
            .bind(to)
            .bind(bucket_secs)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Oldest and newest minute of the platform's recorded history, for the
    /// system doctor. Both `None` before the first sample.
    pub async fn monitoring_history_extent(
        &self,
    ) -> Result<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> {
        Ok(sqlx::query_as(
            "SELECT min(minute), max(minute) FROM monitoring_minutes WHERE org_id = $1",
        )
        .bind(PLATFORM_ORG)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Each cell's newest report from the last week.
    pub async fn monitoring_cells_latest(&self) -> Result<Vec<CellLatestRow>> {
        Ok(sqlx::query_as(
            r#"SELECT DISTINCT ON (cell) cell, data_plane, minute, leader, nodes, nodes_answering,
                sessions, running, queued, resumed, rss_peak_bytes, memory_limit_bytes,
                cpu_peak_cores, cpu_limit_cores, admit_p99_ms, ship_lag_p99_ms, raft_term
            FROM monitoring_cell_minutes WHERE reported AND minute > now() - interval '7 days'
            ORDER BY cell, minute DESC"#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Leader changes of cells inside the range, as timeline markers.
    pub async fn timeline_cell_markers(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<super::TimelineMarkerRow>> {
        let sql = concat!(
            r#"WITH c AS ("#,
            cell_minutes!(),
            r#")
            SELECT minute AS at, 'leader_change' AS kind, cell AS label FROM c
            WHERE minute >= $1 AND elections > 0 ORDER BY minute LIMIT 200"#
        );
        Ok(sqlx::query_as(sql)
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?)
    }
}
