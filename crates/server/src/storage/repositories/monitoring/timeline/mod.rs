//! Minute history for Monitoring → Timeline (`monitoring_minutes`).
//!
//! Decision: one row per organization per minute plus a platform row
//! (`PLATFORM_ORG`), because peaks do not add up across organizations. The
//! sampler folds 10-second population samples into the current minute; a
//! finished minute's counters are recomputed from committed events, lease
//! history, and model generations, so a late write is picked up when the
//! minute is closed again. Organization rows exist only for minutes with
//! activity; averages divide by the platform row's sample count so an idle
//! sample counts as zero. Rows are never purged; reads aggregate minutes into
//! the requested bucket.

use super::{Database, POPULATION};
use anyhow::Result;
use chrono::{DateTime, Duration, DurationRound, Utc};
use sqlx::{FromRow, QueryBuilder};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Key of the platform-wide row in `monitoring_minutes`.
pub const PLATFORM_ORG: i64 = 0;

/// Population of one organization at one sample.
#[derive(Debug, Clone, Default, FromRow)]
pub struct TimelineSampleRow {
    pub org_id: i64,
    pub active: i64,
    pub model: i64,
    pub tools: i64,
    pub waiting: i64,
    pub runnable: i64,
    pub setup: i64,
    pub leased: i64,
    pub expired: i64,
    pub oldest_runnable_ms: Option<i64>,
}

/// Registered worker load at one sample (platform row only).
#[derive(Debug, Clone)]
pub struct TimelineWorkerSample {
    pub id: String,
    pub load: i64,
    pub capacity: i64,
}

/// One aggregated bucket of minute rows.
#[derive(Debug, Clone, FromRow)]
pub struct TimelineBucketRow {
    pub bucket: DateTime<Utc>,
    pub samples: i64,
    pub active_sum: i64,
    pub active_peak: i32,
    pub model_sum: i64,
    pub tools_sum: i64,
    pub waiting_sum: i64,
    pub runnable_sum: i64,
    pub setup_sum: i64,
    pub runnable_peak: i32,
    pub leases_peak: i32,
    pub expired_peak: i32,
    pub oldest_runnable_ms: Option<i64>,
    pub organizations_peak: i32,
    pub turns_started: i64,
    pub turns_completed: i64,
    pub turns_failed: i64,
    pub turns_cancelled: i64,
    pub model_calls: i64,
    pub tool_calls: i64,
    pub tool_failures: i64,
    pub lease_takeovers: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
    pub model_latency_p95_ms: Option<i32>,
    pub workers: Option<i32>,
    pub worker_capacity: Option<i32>,
    pub worker_load_sum: Option<i64>,
    pub worker_load_peak: Option<i32>,
    pub backfilled: bool,
    /// Minute of the highest active peak in the bucket.
    pub peak_at: Option<DateTime<Utc>>,
}

/// Per-worker load for one bucket: worker id → (load sum, samples, capacity).
pub type TimelineWorkerBucket = BTreeMap<String, (i64, i64, i64)>;

#[derive(Debug, Clone, FromRow)]
pub struct TimelineRankRow {
    pub key: Option<String>,
    pub label: Option<String>,
    pub sessions: i64,
    pub turns: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct TimelineFailureRow {
    pub code: String,
    pub turns: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct TimelineMarkerRow {
    pub at: DateTime<Utc>,
    pub kind: String,
    pub label: String,
}

/// Start of the minute containing `at`.
pub fn timeline_minute(at: DateTime<Utc>) -> DateTime<Utc> {
    at.duration_trunc(Duration::minutes(1)).unwrap_or(at)
}

// Counters for every minute in [$1, $2), per organization and for the
// platform (grouping set `(minute)`). Minutes without events produce no row.
const CLOSE_MINUTES: &str = r#"
WITH ev AS (
    SELECT date_trunc('minute', e.ts) AS minute,
        CASE WHEN GROUPING(s.org_id) = 1 THEN 0 ELSE s.org_id END AS org_id,
        COUNT(*) FILTER (WHERE e.event_type = 'turn.started') AS turns_started,
        COUNT(*) FILTER (WHERE e.event_type = 'turn.completed') AS turns_completed,
        COUNT(*) FILTER (WHERE e.event_type = 'turn.failed') AS turns_failed,
        COUNT(*) FILTER (WHERE e.event_type = 'turn.cancelled') AS turns_cancelled,
        COUNT(*) FILTER (WHERE e.event_type = 'reason.started') AS model_calls,
        COUNT(*) FILTER (WHERE e.event_type = 'tool.started') AS tool_calls,
        COUNT(*) FILTER (WHERE e.event_type = 'tool.completed' AND (
            e.data->>'status' IN ('error', 'timeout') OR e.data->>'success' = 'false')) AS tool_failures
    FROM events e JOIN sessions s ON s.id = e.session_id
    WHERE e.ts >= $1 AND e.ts < $2
      AND e.event_type IN ('turn.started', 'reason.started', 'tool.started', 'tool.completed',
        'turn.completed', 'turn.failed', 'turn.cancelled')
    GROUP BY GROUPING SETS ((date_trunc('minute', e.ts), s.org_id), (date_trunc('minute', e.ts)))
), ls AS (
    SELECT date_trunc('minute', h.occurred_at) AS minute,
        CASE WHEN GROUPING(h.org_id) = 1 THEN 0 ELSE h.org_id END AS org_id,
        COUNT(*) AS lease_takeovers
    FROM session_actor_lease_history h
    WHERE h.kind = 'reacquired' AND h.occurred_at >= $1 AND h.occurred_at < $2
    GROUP BY GROUPING SETS ((date_trunc('minute', h.occurred_at), h.org_id), (date_trunc('minute', h.occurred_at)))
), gen AS (
    SELECT date_trunc('minute', g.created_at) AS minute,
        CASE WHEN GROUPING(g.org_id) = 1 THEN 0 ELSE g.org_id END AS org_id,
        SUM(g.input_tokens)::bigint AS input_tokens,
        SUM(g.output_tokens)::bigint AS output_tokens,
        SUM(COALESCE(g.actual_cost_usd, g.estimated_cost_usd, 0)) AS cost_usd,
        (percentile_cont(0.95) WITHIN GROUP (ORDER BY g.duration_ms))::integer AS model_latency_p95_ms
    FROM llm_generations g
    WHERE g.created_at >= $1 AND g.created_at < $2
    GROUP BY GROUPING SETS ((date_trunc('minute', g.created_at), g.org_id), (date_trunc('minute', g.created_at)))
), keys AS (
    SELECT minute, org_id FROM ev UNION SELECT minute, org_id FROM ls UNION SELECT minute, org_id FROM gen
)
INSERT INTO monitoring_minutes AS m (org_id, minute, turns_started, turns_completed, turns_failed,
    turns_cancelled, model_calls, tool_calls, tool_failures, lease_takeovers, input_tokens,
    output_tokens, cost_usd, model_latency_p95_ms, backfilled)
SELECT k.org_id, k.minute,
    COALESCE(ev.turns_started, 0), COALESCE(ev.turns_completed, 0), COALESCE(ev.turns_failed, 0),
    COALESCE(ev.turns_cancelled, 0), COALESCE(ev.model_calls, 0), COALESCE(ev.tool_calls, 0),
    COALESCE(ev.tool_failures, 0), COALESCE(ls.lease_takeovers, 0), COALESCE(gen.input_tokens, 0),
    COALESCE(gen.output_tokens, 0), COALESCE(gen.cost_usd, 0), gen.model_latency_p95_ms, $3
FROM keys k
LEFT JOIN ev ON ev.minute = k.minute AND ev.org_id = k.org_id
LEFT JOIN ls ON ls.minute = k.minute AND ls.org_id = k.org_id
LEFT JOIN gen ON gen.minute = k.minute AND gen.org_id = k.org_id
ON CONFLICT (org_id, minute) DO UPDATE SET
    turns_started = EXCLUDED.turns_started, turns_completed = EXCLUDED.turns_completed,
    turns_failed = EXCLUDED.turns_failed, turns_cancelled = EXCLUDED.turns_cancelled,
    model_calls = EXCLUDED.model_calls, tool_calls = EXCLUDED.tool_calls,
    tool_failures = EXCLUDED.tool_failures, lease_takeovers = EXCLUDED.lease_takeovers,
    input_tokens = EXCLUDED.input_tokens, output_tokens = EXCLUDED.output_tokens,
    cost_usd = EXCLUDED.cost_usd, model_latency_p95_ms = EXCLUDED.model_latency_p95_ms
"#;

impl Database {
    /// Current population per organization, from the same classification
    /// the live snapshot uses.
    pub async fn timeline_sample(&self, now: DateTime<Utc>) -> Result<Vec<TimelineSampleRow>> {
        let mut sql = QueryBuilder::new(POPULATION);
        sql.push(
            " SELECT org_key AS org_id, COUNT(*) AS active, \
            COUNT(*) FILTER (WHERE phase = 'model') AS model, \
            COUNT(*) FILTER (WHERE phase = 'tools') AS tools, \
            COUNT(*) FILTER (WHERE phase = 'waiting') AS waiting, \
            COUNT(*) FILTER (WHERE phase = 'runnable') AS runnable, \
            COUNT(*) FILTER (WHERE phase = 'setup') AS setup, \
            COUNT(*) FILTER (WHERE leased) AS leased, COUNT(*) FILTER (WHERE expired) AS expired, \
            MAX(runnable_age_ms) AS oldest_runnable_ms FROM population GROUP BY org_key",
        );
        Ok(sql
            .build_query_as()
            .bind(None::<i64>)
            .bind(now)
            .bind(None::<Uuid>)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Fold one sample into the minute containing `at`. The platform row is
    /// always written, so it carries the minute's sample count.
    pub async fn timeline_record_sample(
        &self,
        at: DateTime<Utc>,
        orgs: &[TimelineSampleRow],
        workers: Option<&[TimelineWorkerSample]>,
    ) -> Result<()> {
        let minute = timeline_minute(at);
        let mut platform = TimelineSampleRow {
            org_id: PLATFORM_ORG,
            ..Default::default()
        };
        for o in orgs {
            platform.active += o.active;
            platform.model += o.model;
            platform.tools += o.tools;
            platform.waiting += o.waiting;
            platform.runnable += o.runnable;
            platform.setup += o.setup;
            platform.leased += o.leased;
            platform.expired += o.expired;
            platform.oldest_runnable_ms = platform.oldest_runnable_ms.max(o.oldest_runnable_ms);
        }
        let organizations = orgs.iter().filter(|o| o.active > 0).count() as i64;
        let (count, capacity, load, detail) = match workers {
            Some(workers) => {
                let detail: serde_json::Map<String, serde_json::Value> = workers
                    .iter()
                    .map(|w| {
                        (
                            w.id.clone(),
                            serde_json::json!({"load_sum": w.load, "samples": 1, "capacity": w.capacity}),
                        )
                    })
                    .collect();
                (
                    Some(workers.len() as i64),
                    Some(workers.iter().map(|w| w.capacity).sum::<i64>()),
                    Some(workers.iter().map(|w| w.load).sum::<i64>()),
                    Some(serde_json::Value::Object(detail)),
                )
            }
            None => (None, None, None, None),
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"INSERT INTO monitoring_minutes AS m (org_id, minute, samples, active_sum, active_peak,
                model_sum, tools_sum, waiting_sum, runnable_sum, setup_sum, runnable_peak, leases_peak,
                expired_peak, oldest_runnable_ms, organizations_peak, workers, worker_capacity,
                worker_load_sum, worker_load_peak, worker_detail)
            VALUES ($1, $2, 1, $3, $3, $4, $5, $6, $7, $8, $7, $9, $10, $11, $12, $13, $14, $15, $15, $16)
            ON CONFLICT (org_id, minute) DO UPDATE SET
                samples = m.samples + 1, active_sum = m.active_sum + EXCLUDED.active_sum,
                active_peak = GREATEST(m.active_peak, EXCLUDED.active_peak),
                model_sum = m.model_sum + EXCLUDED.model_sum, tools_sum = m.tools_sum + EXCLUDED.tools_sum,
                waiting_sum = m.waiting_sum + EXCLUDED.waiting_sum,
                runnable_sum = m.runnable_sum + EXCLUDED.runnable_sum,
                setup_sum = m.setup_sum + EXCLUDED.setup_sum,
                runnable_peak = GREATEST(m.runnable_peak, EXCLUDED.runnable_peak),
                leases_peak = GREATEST(m.leases_peak, EXCLUDED.leases_peak),
                expired_peak = GREATEST(m.expired_peak, EXCLUDED.expired_peak),
                oldest_runnable_ms = GREATEST(m.oldest_runnable_ms, EXCLUDED.oldest_runnable_ms),
                organizations_peak = GREATEST(m.organizations_peak, EXCLUDED.organizations_peak),
                workers = GREATEST(m.workers, EXCLUDED.workers),
                worker_capacity = GREATEST(m.worker_capacity, EXCLUDED.worker_capacity),
                worker_load_sum = COALESCE(m.worker_load_sum, 0) + EXCLUDED.worker_load_sum,
                worker_load_peak = GREATEST(m.worker_load_peak, EXCLUDED.worker_load_peak),
                worker_detail = CASE WHEN EXCLUDED.worker_detail IS NULL THEN m.worker_detail
                    ELSE (SELECT jsonb_object_agg(k, jsonb_build_object(
                        'load_sum', COALESCE((m.worker_detail->k->>'load_sum')::bigint, 0)
                            + COALESCE((EXCLUDED.worker_detail->k->>'load_sum')::bigint, 0),
                        'samples', COALESCE((m.worker_detail->k->>'samples')::bigint, 0)
                            + COALESCE((EXCLUDED.worker_detail->k->>'samples')::bigint, 0),
                        'capacity', GREATEST(COALESCE((m.worker_detail->k->>'capacity')::bigint, 0),
                            COALESCE((EXCLUDED.worker_detail->k->>'capacity')::bigint, 0))))
                        FROM (SELECT jsonb_object_keys(COALESCE(m.worker_detail, '{}'::jsonb)) AS k
                              UNION SELECT jsonb_object_keys(EXCLUDED.worker_detail)) keys)
                    END,
                backfilled = FALSE"#,
        )
        .bind(PLATFORM_ORG)
        .bind(minute)
        .bind(platform.active)
        .bind(platform.model)
        .bind(platform.tools)
        .bind(platform.waiting)
        .bind(platform.runnable)
        .bind(platform.setup)
        .bind(platform.leased)
        .bind(platform.expired)
        .bind(platform.oldest_runnable_ms)
        .bind(organizations)
        .bind(count)
        .bind(capacity)
        .bind(load)
        .bind(detail)
        .execute(&mut *tx)
        .await?;
        let active: Vec<&TimelineSampleRow> = orgs.iter().filter(|o| o.active > 0).collect();
        if !active.is_empty() {
            let col =
                |f: fn(&TimelineSampleRow) -> i64| active.iter().map(|o| f(o)).collect::<Vec<_>>();
            sqlx::query(
                r#"INSERT INTO monitoring_minutes AS m (org_id, minute, samples, active_sum, active_peak,
                    model_sum, tools_sum, waiting_sum, runnable_sum, setup_sum, runnable_peak,
                    leases_peak, expired_peak, oldest_runnable_ms)
                SELECT u.org_id, $1, 1, u.active, u.active, u.model, u.tools, u.waiting, u.runnable,
                    u.setup, u.runnable, u.leased, u.expired, u.oldest
                FROM UNNEST($2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[], $6::bigint[],
                    $7::bigint[], $8::bigint[], $9::bigint[], $10::bigint[], $11::bigint[])
                    AS u(org_id, active, model, tools, waiting, runnable, setup, leased, expired, oldest)
                ON CONFLICT (org_id, minute) DO UPDATE SET
                    samples = m.samples + 1, active_sum = m.active_sum + EXCLUDED.active_sum,
                    active_peak = GREATEST(m.active_peak, EXCLUDED.active_peak),
                    model_sum = m.model_sum + EXCLUDED.model_sum,
                    tools_sum = m.tools_sum + EXCLUDED.tools_sum,
                    waiting_sum = m.waiting_sum + EXCLUDED.waiting_sum,
                    runnable_sum = m.runnable_sum + EXCLUDED.runnable_sum,
                    setup_sum = m.setup_sum + EXCLUDED.setup_sum,
                    runnable_peak = GREATEST(m.runnable_peak, EXCLUDED.runnable_peak),
                    leases_peak = GREATEST(m.leases_peak, EXCLUDED.leases_peak),
                    expired_peak = GREATEST(m.expired_peak, EXCLUDED.expired_peak),
                    oldest_runnable_ms = GREATEST(m.oldest_runnable_ms, EXCLUDED.oldest_runnable_ms),
                    backfilled = FALSE"#,
            )
            .bind(minute)
            .bind(col(|o| o.org_id))
            .bind(col(|o| o.active))
            .bind(col(|o| o.model))
            .bind(col(|o| o.tools))
            .bind(col(|o| o.waiting))
            .bind(col(|o| o.runnable))
            .bind(col(|o| o.setup))
            .bind(col(|o| o.leased))
            .bind(col(|o| o.expired))
            .bind(
                active
                    .iter()
                    .map(|o| o.oldest_runnable_ms)
                    .collect::<Vec<_>>(),
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Recompute counters for every minute in `[from, to)`. Idempotent.
    pub async fn timeline_close_minutes(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(CLOSE_MINUTES)
            .bind(timeline_minute(from))
            .bind(timeline_minute(to))
            .bind(false)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Earliest minute with history, if any.
    pub async fn timeline_earliest_minute(&self) -> Result<Option<DateTime<Utc>>> {
        Ok(
            sqlx::query_scalar("SELECT MIN(minute) FROM monitoring_minutes WHERE org_id = $1")
                .bind(PLATFORM_ORG)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Earliest committed turn, the backfill's stopping point.
    pub async fn timeline_earliest_turn(&self) -> Result<Option<DateTime<Utc>>> {
        Ok(
            sqlx::query_scalar("SELECT MIN(ts) FROM events WHERE event_type = 'turn.started'")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Rebuild `[from, to)` from turn history: running turns per minute from
    /// projected turn intervals, and counters from committed events. Never
    /// overwrites sampled population. A marker row at `from` records progress
    /// even for a day without activity.
    pub async fn timeline_backfill(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<()> {
        let (from, to) = (timeline_minute(from), timeline_minute(to));
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"WITH t AS (
                SELECT s.org_id, tt.started_at, COALESCE(tt.ended_at, tt.started_at) AS ended_at
                FROM session_trace_turns tt JOIN sessions s ON s.id = tt.session_id
                WHERE tt.started_at >= $1 - interval '1 day' AND tt.started_at < $2
                  AND COALESCE(tt.ended_at, tt.started_at) > $1
            ), m AS (
                SELECT generate_series($1, $2 - interval '1 minute', interval '1 minute') AS minute
            ), c AS (
                SELECT m.minute, CASE WHEN GROUPING(t.org_id) = 1 THEN 0 ELSE t.org_id END AS org_id,
                    ROUND(SUM(EXTRACT(EPOCH FROM (LEAST(t.ended_at, m.minute + interval '1 minute')
                        - GREATEST(t.started_at, m.minute)))) / 60.0)::bigint AS active_avg,
                    COUNT(*)::integer AS active_peak
                FROM m JOIN t ON t.started_at < m.minute + interval '1 minute' AND t.ended_at > m.minute
                GROUP BY GROUPING SETS ((m.minute, t.org_id), (m.minute))
            )
            INSERT INTO monitoring_minutes (org_id, minute, samples, active_sum, active_peak, backfilled)
            SELECT org_id, minute, 1, active_avg, active_peak, TRUE FROM c
            UNION ALL SELECT 0, $1, 1, 0, 0, TRUE
            ON CONFLICT (org_id, minute) DO NOTHING"#,
        )
        .bind(from)
        .bind(to)
        .execute(&mut *tx)
        .await?;
        sqlx::query(CLOSE_MINUTES)
            .bind(from)
            .bind(to)
            .bind(true)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Minute rows aggregated into `bucket_secs` buckets for one org or the
    /// platform. Buckets without history are absent.
    pub async fn timeline_buckets(
        &self,
        org: i64,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        bucket_secs: i64,
    ) -> Result<Vec<TimelineBucketRow>> {
        Ok(sqlx::query_as(
            r#"WITH p AS (
                SELECT minute, samples FROM monitoring_minutes
                WHERE org_id = 0 AND minute >= $2 AND minute < $3
            ), o AS (
                SELECT * FROM monitoring_minutes WHERE org_id = $1 AND minute >= $2 AND minute < $3
            ), j AS (
                SELECT to_timestamp(floor(extract(epoch FROM COALESCE(o.minute, p.minute)) / $4) * $4) AS bucket,
                    COALESCE(o.minute, p.minute) AS minute,
                    GREATEST(COALESCE(p.samples, 0), COALESCE(o.samples, 0)) AS samples,
                    o.active_sum, o.active_peak, o.model_sum, o.tools_sum, o.waiting_sum,
                    o.runnable_sum, o.setup_sum, o.runnable_peak, o.leases_peak, o.expired_peak,
                    o.oldest_runnable_ms, o.organizations_peak, o.turns_started, o.turns_completed,
                    o.turns_failed, o.turns_cancelled, o.model_calls, o.tool_calls, o.tool_failures,
                    o.lease_takeovers, o.input_tokens, o.output_tokens, o.cost_usd,
                    o.model_latency_p95_ms, o.workers, o.worker_capacity, o.worker_load_sum,
                    o.worker_load_peak, o.backfilled
                FROM p FULL JOIN o ON o.minute = p.minute
            )
            SELECT bucket, SUM(samples)::bigint AS samples,
                COALESCE(SUM(active_sum), 0)::bigint AS active_sum,
                COALESCE(MAX(active_peak), 0) AS active_peak,
                COALESCE(SUM(model_sum), 0)::bigint AS model_sum,
                COALESCE(SUM(tools_sum), 0)::bigint AS tools_sum,
                COALESCE(SUM(waiting_sum), 0)::bigint AS waiting_sum,
                COALESCE(SUM(runnable_sum), 0)::bigint AS runnable_sum,
                COALESCE(SUM(setup_sum), 0)::bigint AS setup_sum,
                COALESCE(MAX(runnable_peak), 0) AS runnable_peak,
                COALESCE(MAX(leases_peak), 0) AS leases_peak,
                COALESCE(MAX(expired_peak), 0) AS expired_peak,
                MAX(oldest_runnable_ms) AS oldest_runnable_ms,
                COALESCE(MAX(organizations_peak), 0) AS organizations_peak,
                COALESCE(SUM(turns_started), 0)::bigint AS turns_started,
                COALESCE(SUM(turns_completed), 0)::bigint AS turns_completed,
                COALESCE(SUM(turns_failed), 0)::bigint AS turns_failed,
                COALESCE(SUM(turns_cancelled), 0)::bigint AS turns_cancelled,
                COALESCE(SUM(model_calls), 0)::bigint AS model_calls,
                COALESCE(SUM(tool_calls), 0)::bigint AS tool_calls,
                COALESCE(SUM(tool_failures), 0)::bigint AS tool_failures,
                COALESCE(SUM(lease_takeovers), 0)::bigint AS lease_takeovers,
                COALESCE(SUM(input_tokens), 0)::bigint AS input_tokens,
                COALESCE(SUM(output_tokens), 0)::bigint AS output_tokens,
                COALESCE(SUM(cost_usd), 0)::float8 AS cost_usd,
                MAX(model_latency_p95_ms) AS model_latency_p95_ms,
                MAX(workers) AS workers, MAX(worker_capacity) AS worker_capacity,
                SUM(worker_load_sum)::bigint AS worker_load_sum, MAX(worker_load_peak) AS worker_load_peak,
                COALESCE(bool_or(backfilled), FALSE) AS backfilled,
                (array_agg(minute ORDER BY active_peak DESC NULLS LAST, minute))[1] AS peak_at
            FROM j GROUP BY bucket ORDER BY bucket"#,
        )
        .bind(org)
        .bind(from)
        .bind(to)
        .bind(bucket_secs)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Per-worker load aggregated into buckets (platform only).
    pub async fn timeline_worker_buckets(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        bucket_secs: i64,
    ) -> Result<BTreeMap<DateTime<Utc>, TimelineWorkerBucket>> {
        let rows: Vec<(DateTime<Utc>, String, i64, i64, i64)> = sqlx::query_as(
            r#"SELECT to_timestamp(floor(extract(epoch FROM m.minute) / $3) * $3) AS bucket, w.key,
                SUM((w.value->>'load_sum')::bigint)::bigint, SUM((w.value->>'samples')::bigint)::bigint,
                MAX((w.value->>'capacity')::bigint)::bigint
            FROM monitoring_minutes m, jsonb_each(m.worker_detail) w
            WHERE m.org_id = 0 AND m.minute >= $1 AND m.minute < $2 AND m.worker_detail IS NOT NULL
            GROUP BY 1, 2 ORDER BY 1, 2"#,
        )
        .bind(from)
        .bind(to)
        .bind(bucket_secs)
        .fetch_all(&self.pool)
        .await?;
        let mut out: BTreeMap<DateTime<Utc>, TimelineWorkerBucket> = BTreeMap::new();
        for (bucket, id, load, samples, capacity) in rows {
            out.entry(bucket)
                .or_default()
                .insert(id, (load, samples, capacity));
        }
        Ok(out)
    }

    /// Busiest agents (organization) or organizations (platform) by sessions
    /// that started a turn in the window.
    pub async fn timeline_busiest(
        &self,
        org: Option<i64>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<TimelineRankRow>> {
        let sql = if org.is_some() {
            r#"SELECT a.public_id AS key, a.name::text AS label,
                COUNT(DISTINCT e.session_id) AS sessions, COUNT(*) AS turns
            FROM events e JOIN sessions s ON s.id = e.session_id LEFT JOIN agents a ON a.id = s.agent_id
            WHERE e.event_type = 'turn.started' AND e.ts >= $2 AND e.ts < $3 AND s.org_id = $1
            GROUP BY a.public_id, a.name ORDER BY sessions DESC, turns DESC LIMIT 5"#
        } else {
            r#"SELECT o.public_id::text AS key, NULL::text AS label,
                COUNT(DISTINCT e.session_id) AS sessions, COUNT(*) AS turns
            FROM events e JOIN sessions s ON s.id = e.session_id
            JOIN organizations o ON o.org_id = s.org_id
            WHERE e.event_type = 'turn.started' AND e.ts >= $2 AND e.ts < $3 AND ($1::bigint IS NULL)
            GROUP BY o.public_id ORDER BY turns DESC LIMIT 5"#
        };
        Ok(sqlx::query_as(sql)
            .bind(org)
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Failed turns in the window grouped by classified error code.
    pub async fn timeline_failures(
        &self,
        org: Option<i64>,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<TimelineFailureRow>> {
        Ok(sqlx::query_as(
            r#"SELECT COALESCE(NULLIF(e.data->>'error_code', ''), 'other') AS code, COUNT(*) AS turns
            FROM events e JOIN sessions s ON s.id = e.session_id
            WHERE e.event_type = 'turn.failed' AND e.ts >= $2 AND e.ts < $3
              AND ($1::bigint IS NULL OR s.org_id = $1)
            GROUP BY 1 ORDER BY turns DESC LIMIT 6"#,
        )
        .bind(org)
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Deploys (first start of each server version) and, for the platform,
    /// worker joins and departures inside the range.
    pub async fn timeline_markers(
        &self,
        platform: bool,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<TimelineMarkerRow>> {
        Ok(sqlx::query_as(
            r#"SELECT at, kind, label FROM (
                SELECT MIN(started_at) AS at, 'deploy' AS kind, version AS label
                FROM system_instances WHERE kind = 'server' GROUP BY version
                UNION ALL
                SELECT started_at, 'worker_joined', id FROM durable_workers WHERE $3
                UNION ALL
                SELECT last_heartbeat_at, 'worker_left', id FROM durable_workers
                WHERE $3 AND last_heartbeat_at < now() - interval '2 minutes'
            ) x WHERE at >= $1 AND at < $2 ORDER BY at LIMIT 200"#,
        )
        .bind(from)
        .bind(to)
        .bind(platform)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests;
