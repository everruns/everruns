// Session trace reads: bounded queries over the trace index for the Trace view.
//
// Decision: every read is bounded by what one screen needs, never by session
// size. The overview aggregates turn rows into a fixed number of buckets; a
// page of turns returns runs ("islands") of steps rather than steps, so a turn
// with thousands of consecutive calls to one tool costs one row; individual
// step rows are fetched only for what survives batching and elision.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::Database;
use super::rows::TraceStepRow;

/// Session-wide totals of the trace index.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct TraceTotalsRow {
    pub turn_count: i64,
    pub step_count: i64,
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub first_started_at: Option<DateTime<Utc>>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

/// One minimap bucket: a fixed-size range of turns.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TraceBucketRow {
    pub from_turn: i32,
    pub to_turn: i32,
    pub steps: i64,
    pub duration_ms: i64,
    pub errors: i64,
}

/// A run of consecutive steps that share a run key: one tool name, or a single
/// step of any other kind.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TraceRunRow {
    pub turn_no: i32,
    pub first_step: i32,
    pub last_step: i32,
    pub kind: String,
    pub name: Option<String>,
    pub count: i64,
    pub failed: i64,
    pub running: i64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// An event in a step's or turn's sequence range, without its payload.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TraceEventRefRow {
    pub id: Uuid,
    pub sequence: i32,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    /// Small payloads inline; `None` when the payload is over the inline limit.
    pub data: Option<serde_json::Value>,
    pub size_bytes: i32,
}

macro_rules! step_select {
    () => {
        "s.turn_no, s.step_no, s.kind, s.status, s.start_sequence, s.end_sequence, \
         s.started_at, s.duration_ms, s.name, s.target, s.result, s.narration, s.tool_call_id, \
         s.model, s.input_tokens, s.output_tokens, s.message_count, s.requested_tool_call_ids"
    };
}

impl Database {
    pub async fn trace_totals(&self, session_id: Uuid) -> Result<TraceTotalsRow> {
        let row = sqlx::query_as::<_, TraceTotalsRow>(
            "SELECT COUNT(*)::BIGINT AS turn_count, \
                COALESCE(SUM(step_count), 0)::BIGINT AS step_count, \
                COALESCE(SUM(error_count), 0)::BIGINT AS error_count, \
                COALESCE(SUM(input_tokens), 0)::BIGINT AS input_tokens, \
                COALESCE(SUM(output_tokens), 0)::BIGINT AS output_tokens, \
                MIN(started_at) AS first_started_at, \
                MAX(COALESCE(ended_at, started_at)) AS last_activity_at \
             FROM session_trace_turns WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Turn ranges of `bucket_size` turns, in turn order.
    pub async fn trace_buckets(
        &self,
        session_id: Uuid,
        bucket_size: i32,
    ) -> Result<Vec<TraceBucketRow>> {
        let rows = sqlx::query_as::<_, TraceBucketRow>(
            "SELECT MIN(turn_no) AS from_turn, MAX(turn_no) AS to_turn, \
                SUM(step_count)::BIGINT AS steps, \
                COALESCE(SUM(EXTRACT(EPOCH FROM (ended_at - started_at)) * 1000), 0)::BIGINT \
                    AS duration_ms, \
                SUM(error_count)::BIGINT AS errors \
             FROM session_trace_turns WHERE session_id = $1 \
             GROUP BY (turn_no - 1) / $2 ORDER BY from_turn",
        )
        .bind(session_id)
        .bind(bucket_size.max(1))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Numbers of the turns with errors, newest first.
    pub async fn trace_error_turns(&self, session_id: Uuid, limit: i64) -> Result<Vec<i32>> {
        let rows = sqlx::query_scalar::<_, i32>(
            "SELECT turn_no FROM session_trace_turns \
             WHERE session_id = $1 AND error_count > 0 ORDER BY turn_no DESC LIMIT $2",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// The turn that contains `sequence`.
    pub async fn trace_turn_at_sequence(
        &self,
        session_id: Uuid,
        sequence: i32,
    ) -> Result<Option<i32>> {
        let row = sqlx::query_scalar::<_, i32>(
            "SELECT turn_no FROM session_trace_turns \
             WHERE session_id = $1 AND start_sequence <= $2 \
             ORDER BY start_sequence DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(sequence)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Runs of steps for turns `from_turn..=to_turn`, in step order.
    pub async fn trace_runs(
        &self,
        session_id: Uuid,
        from_turn: i32,
        to_turn: i32,
    ) -> Result<Vec<TraceRunRow>> {
        let rows = sqlx::query_as::<_, TraceRunRow>(
            r#"
            WITH s AS (
                SELECT turn_no, step_no, kind, status, name, duration_ms, started_at,
                    CASE WHEN kind = 'tool' THEN 'tool:' || COALESCE(name, '')
                         ELSE 'step:' || step_no END AS run_key
                FROM session_trace_steps
                WHERE session_id = $1 AND turn_no BETWEEN $2 AND $3
            ), g AS (
                SELECT *, step_no - ROW_NUMBER() OVER (
                    PARTITION BY turn_no, run_key ORDER BY step_no) AS grp
                FROM s
            )
            SELECT turn_no, MIN(step_no) AS first_step, MAX(step_no) AS last_step,
                MIN(kind) AS kind, MIN(name) AS name, COUNT(*)::BIGINT AS count,
                (COUNT(*) FILTER (WHERE status = 'error'))::BIGINT AS failed,
                (COUNT(*) FILTER (WHERE status = 'running'))::BIGINT AS running,
                percentile_cont(0.5) WITHIN GROUP (ORDER BY duration_ms) AS p50_ms,
                percentile_cont(0.95) WITHIN GROUP (ORDER BY duration_ms) AS p95_ms,
                MIN(started_at) AS started_at,
                MAX(started_at + COALESCE(duration_ms, 0) * INTERVAL '1 millisecond') AS ended_at
            FROM g
            GROUP BY turn_no, run_key, grp
            ORDER BY turn_no, first_step
            "#,
        )
        .bind(session_id)
        .bind(from_turn)
        .bind(to_turn)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Step rows by `(turn_no, step_no)` key, in key order.
    pub async fn trace_steps_by_key(
        &self,
        session_id: Uuid,
        keys: &[(i32, i32)],
    ) -> Result<Vec<TraceStepRow>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let turns: Vec<i32> = keys.iter().map(|k| k.0).collect();
        let steps: Vec<i32> = keys.iter().map(|k| k.1).collect();
        let rows = sqlx::query_as::<_, TraceStepRow>(concat!(
            "SELECT ",
            step_select!(),
            " FROM UNNEST($2::INT[], $3::INT[]) AS k(turn_no, step_no) \
             JOIN session_trace_steps s ON s.session_id = $1 \
                AND s.turn_no = k.turn_no AND s.step_no = k.step_no \
             ORDER BY s.turn_no, s.step_no"
        ))
        .bind(session_id)
        .bind(&turns)
        .bind(&steps)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Failed steps inside each `(turn_no, first_step, last_step)` range, at
    /// most `per_range` of each, in step order.
    pub async fn trace_failures_in_ranges(
        &self,
        session_id: Uuid,
        ranges: &[(i32, i32, i32)],
        per_range: i64,
    ) -> Result<Vec<TraceStepRow>> {
        if ranges.is_empty() {
            return Ok(Vec::new());
        }
        let turns: Vec<i32> = ranges.iter().map(|r| r.0).collect();
        let firsts: Vec<i32> = ranges.iter().map(|r| r.1).collect();
        let lasts: Vec<i32> = ranges.iter().map(|r| r.2).collect();
        let rows = sqlx::query_as::<_, TraceStepRow>(concat!(
            "SELECT f.* FROM UNNEST($2::INT[], $3::INT[], $4::INT[]) AS r(turn_no, first_step, last_step) \
             CROSS JOIN LATERAL (SELECT ",
            step_select!(),
            " FROM session_trace_steps s WHERE s.session_id = $1 AND s.turn_no = r.turn_no \
                AND s.step_no BETWEEN r.first_step AND r.last_step AND s.status = 'error' \
                ORDER BY s.step_no LIMIT $5) f \
             ORDER BY f.turn_no, f.step_no"
        ))
        .bind(session_id)
        .bind(&turns)
        .bind(&firsts)
        .bind(&lasts)
        .bind(per_range)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Steps of one turn from `from_step`, optionally only failed ones.
    pub async fn trace_steps_page(
        &self,
        session_id: Uuid,
        turn_no: i32,
        from_step: i32,
        to_step: i32,
        errors_only: bool,
        limit: i64,
    ) -> Result<Vec<TraceStepRow>> {
        let rows = sqlx::query_as::<_, TraceStepRow>(concat!(
            "SELECT ",
            step_select!(),
            " FROM session_trace_steps s WHERE s.session_id = $1 AND s.turn_no = $2 \
             AND s.step_no BETWEEN $3 AND $4 AND (NOT $5 OR s.status = 'error') \
             ORDER BY s.step_no LIMIT $6"
        ))
        .bind(session_id)
        .bind(turn_no)
        .bind(from_step)
        .bind(to_step)
        .bind(errors_only)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Child sessions started by `spawn_agent` calls, by tool call id.
    pub async fn trace_child_sessions(
        &self,
        session_id: Uuid,
        tool_call_ids: &[String],
    ) -> Result<Vec<(String, Uuid)>> {
        if tool_call_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, (String, Uuid)>(
            "SELECT tool_call_id, child_session_id FROM subagent_spawn_handles \
             WHERE parent_session_id = $1 AND tool_call_id = ANY($2) \
                AND child_session_id IS NOT NULL",
        )
        .bind(session_id)
        .bind(tool_call_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Events in `from_sequence..=to_sequence`, payloads inline up to
    /// `inline_bytes`. Deltas are left out.
    pub async fn trace_events_in_range(
        &self,
        session_id: Uuid,
        from_sequence: i32,
        to_sequence: i32,
        inline_bytes: i32,
        limit: i64,
    ) -> Result<Vec<TraceEventRefRow>> {
        let rows = sqlx::query_as::<_, TraceEventRefRow>(
            "SELECT id, sequence, event_type, ts, \
                CASE WHEN pg_column_size(data) <= $4 THEN data END AS data, \
                pg_column_size(data) AS size_bytes \
             FROM events WHERE session_id = $1 AND sequence BETWEEN $2 AND $3 \
                AND event_type NOT LIKE '%.delta' \
             ORDER BY sequence LIMIT $5",
        )
        .bind(session_id)
        .bind(from_sequence)
        .bind(to_sequence)
        .bind(inline_bytes)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// One event's full payload by sequence.
    pub async fn trace_event_data(
        &self,
        session_id: Uuid,
        sequence: i32,
    ) -> Result<Option<(String, serde_json::Value)>> {
        let row = sqlx::query_as::<_, (String, serde_json::Value)>(
            "SELECT event_type, data FROM events WHERE session_id = $1 AND sequence = $2",
        )
        .bind(session_id)
        .bind(sequence)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// The `tool.started` and `tool.completed` payloads of one call, found in
    /// the step's sequence range.
    pub async fn trace_tool_call_events(
        &self,
        session_id: Uuid,
        from_sequence: i32,
        to_sequence: i32,
        tool_call_id: &str,
    ) -> Result<Vec<(i32, String, serde_json::Value)>> {
        let rows = sqlx::query_as::<_, (i32, String, serde_json::Value)>(
            "SELECT sequence, event_type, data FROM events \
             WHERE session_id = $1 AND sequence BETWEEN $2 AND $3 \
                AND event_type IN ('tool.started', 'tool.completed') \
                AND COALESCE(data->'tool_call'->>'id', data->>'tool_call_id') = $4 \
             ORDER BY sequence",
        )
        .bind(session_id)
        .bind(from_sequence)
        .bind(to_sequence)
        .bind(tool_call_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// The message count of the model call before this one in the session, to
    /// tell which request messages are new.
    pub async fn trace_previous_message_count(
        &self,
        session_id: Uuid,
        turn_no: i32,
        step_no: i32,
    ) -> Result<Option<i32>> {
        let row = sqlx::query_scalar::<_, Option<i32>>(
            "SELECT message_count FROM session_trace_steps \
             WHERE session_id = $1 AND (turn_no, step_no) < ($2, $3) \
                AND kind IN ('model', 'answer') \
             ORDER BY turn_no DESC, step_no DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(turn_no)
        .bind(step_no)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.flatten())
    }
}
