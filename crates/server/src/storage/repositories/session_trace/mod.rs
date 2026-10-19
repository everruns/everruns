// Session trace index repository: keeps `session_trace_*` caught up with events.
//
// Decision: projection runs right after events are written (spawned after the
// writing transaction commits, never on the write's own latency) and again
// before every trace read, both through `catch_up_session_trace`. A per-session
// watermark makes the passes idempotent: each event is applied exactly once,
// whichever pass gets there first, and a pass that fails or is skipped is made
// up by the next one. The background backfill uses the same entry point for
// sessions written before the index existed.
//
// Decision: the session's state row is the lock. A write-path pass skips a
// session another pass holds (`SKIP LOCKED`); a read waits for it, so a reader
// always sees every committed event.
//
// Decision: the upper bound of a pass is `event_sequences.next_sequence - 1`,
// read before the events. Allocating a sequence updates that row, so a writer
// holding sequence N blocks writers of later sequences until it commits; every
// sequence at or below the committed head is therefore readable.

pub mod projection;
pub mod reads;
pub mod rows;

use anyhow::Result;
use sqlx::{Postgres, QueryBuilder, Transaction};
use uuid::Uuid;

use super::Database;
use projection::{TRACE_EVENT_TYPES, TraceEvent, TraceWorkingSet, parse_uuid};
use rows::{TraceCatchUp, TraceStepRow, TraceTurnRow};

/// Events one write-path pass projects; a session behind by more is left to
/// the backfill and to the next read.
pub const TRACE_WRITE_PATH_BUDGET: i64 = 2_000;
/// Events one read-path or backfill pass projects before committing.
pub const TRACE_PASS_BUDGET: i64 = 5_000;

macro_rules! turn_columns {
    () => {
        "turn_no, turn_id, start_sequence, end_sequence, started_at, ended_at, \
     status, prompt_preview, error, step_count, model_calls, tool_calls, subagent_calls, \
     error_count, input_tokens, output_tokens"
    };
}

macro_rules! step_columns {
    () => {
        "turn_no, step_no, kind, status, start_sequence, end_sequence, \
     started_at, duration_ms, name, target, result, narration, tool_call_id, model, \
     input_tokens, output_tokens, message_count, requested_tool_call_ids"
    };
}

/// Rows per upsert statement; 18 bind parameters per row stays far under the
/// 65535 PostgreSQL allows.
const UPSERT_CHUNK: usize = 1_000;

#[derive(sqlx::FromRow)]
struct EventSlice {
    sequence: i32,
    event_type: String,
    ts: chrono::DateTime<chrono::Utc>,
    turn_id: Option<String>,
    data: serde_json::Value,
}

impl Database {
    /// Is any of these event types one the trace index reads?
    pub fn is_trace_event_type(event_type: &str) -> bool {
        TRACE_EVENT_TYPES.contains(&event_type)
    }

    /// Project a session's trace in the background once the current
    /// transaction commits. Never fails the caller: a skipped or failed pass is
    /// made up by the next one.
    pub fn schedule_session_trace_projection(&self, session_id: Uuid) {
        let db = self.clone();
        crate::storage::transaction::spawn_after_commit(async move {
            if let Err(error) = db
                .catch_up_session_trace(session_id, TRACE_WRITE_PATH_BUDGET, false)
                .await
            {
                tracing::debug!(%session_id, %error, "session trace projection deferred");
            }
        });
    }

    /// Bring a session's trace index up to date with its events, projecting at
    /// most `budget` events in this pass.
    ///
    /// With `wait`, block until another pass on the same session finishes;
    /// without, return [`TraceCatchUp::Busy`] instead.
    pub async fn catch_up_session_trace(
        &self,
        session_id: Uuid,
        budget: i64,
        wait: bool,
    ) -> Result<TraceCatchUp> {
        let mut tx = self.pool.begin_detached().await?;
        sqlx::query(
            "INSERT INTO session_trace_state (session_id) VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
        let lock = if wait {
            "SELECT projected_sequence, turn_count FROM session_trace_state \
             WHERE session_id = $1 FOR UPDATE"
        } else {
            "SELECT projected_sequence, turn_count FROM session_trace_state \
             WHERE session_id = $1 FOR UPDATE SKIP LOCKED"
        };
        let Some((projected, turn_count)) = sqlx::query_as::<_, (i32, i32)>(lock)
            .bind(session_id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(TraceCatchUp::Busy);
        };

        let head: i32 = sqlx::query_scalar(
            "SELECT next_sequence - 1 FROM event_sequences WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);
        if head <= projected {
            tx.commit().await?;
            return Ok(TraceCatchUp::UpToDate {
                projected_sequence: projected,
            });
        }

        let events = load_event_slices(&mut tx, session_id, projected, head, budget).await?;
        let partial = i64::try_from(events.len()).unwrap_or(i64::MAX) >= budget;
        let watermark = if partial {
            events.last().map_or(head, |e| e.sequence)
        } else {
            head
        };

        let events: Vec<TraceEvent> = events
            .into_iter()
            .map(|e| TraceEvent {
                sequence: e.sequence,
                event_type: e.event_type,
                ts: e.ts,
                turn_id: e.turn_id.as_deref().and_then(parse_uuid),
                data: e.data,
            })
            .collect();
        let mut turn_ids: Vec<Uuid> = events
            .iter()
            .filter_map(|e| {
                e.turn_id.or_else(|| {
                    e.data
                        .get("turn_id")
                        .and_then(serde_json::Value::as_str)
                        .and_then(parse_uuid)
                })
            })
            .collect();
        turn_ids.sort_unstable();
        turn_ids.dedup();

        let turns = sqlx::query_as::<_, TraceTurnRow>(concat!(
            "SELECT ",
            turn_columns!(),
            " FROM session_trace_turns WHERE session_id = $1 AND turn_id = ANY($2)"
        ))
        .bind(session_id)
        .bind(&turn_ids)
        .fetch_all(&mut *tx)
        .await?;
        let turn_nos: Vec<i32> = turns.iter().map(|t| t.turn_no).collect();
        let steps = sqlx::query_as::<_, TraceStepRow>(concat!(
            "SELECT ",
            step_columns!(),
            " FROM session_trace_steps \
             WHERE session_id = $1 AND turn_no = ANY($2) AND status = 'running'"
        ))
        .bind(session_id)
        .bind(&turn_nos)
        .fetch_all(&mut *tx)
        .await?;

        let mut working_set = TraceWorkingSet::new(turn_count, turns, steps);
        for event in &events {
            working_set.apply(event);
        }
        let (turn_count, turns, steps) = working_set.into_changes();
        upsert_turns(&mut tx, session_id, &turns).await?;
        upsert_steps(&mut tx, session_id, &steps).await?;
        sqlx::query(
            "UPDATE session_trace_state \
             SET projected_sequence = $2, turn_count = $3, updated_at = NOW() \
             WHERE session_id = $1",
        )
        .bind(session_id)
        .bind(watermark)
        .bind(turn_count)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(if partial {
            TraceCatchUp::Partial {
                projected_sequence: watermark,
            }
        } else {
            TraceCatchUp::UpToDate {
                projected_sequence: watermark,
            }
        })
    }

    /// Catch a session's trace up completely, in bounded passes. For reads,
    /// which must see every committed event.
    pub async fn catch_up_session_trace_fully(&self, session_id: Uuid) -> Result<i32> {
        // Fast path without a lock or transaction: most reads find the index
        // already current, because the write path projected right after commit.
        let current: Option<i32> = sqlx::query_scalar(
            "SELECT s.projected_sequence FROM session_trace_state s \
             LEFT JOIN event_sequences q ON q.session_id = s.session_id \
             WHERE s.session_id = $1 \
               AND s.projected_sequence >= COALESCE(q.next_sequence - 1, 0)",
        )
        .bind(session_id)
        .fetch_optional(self.pool.raw())
        .await?;
        if let Some(projected_sequence) = current {
            return Ok(projected_sequence);
        }
        loop {
            match self
                .catch_up_session_trace(session_id, TRACE_PASS_BUDGET, true)
                .await?
            {
                TraceCatchUp::UpToDate { projected_sequence } => return Ok(projected_sequence),
                TraceCatchUp::Partial { .. } | TraceCatchUp::Busy => continue,
            }
        }
    }

    /// Sessions whose trace index is behind their events, most recently
    /// written first. For the backfill.
    pub async fn sessions_behind_trace(&self, limit: i64) -> Result<Vec<Uuid>> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT es.session_id FROM event_sequences es \
             LEFT JOIN session_trace_state st ON st.session_id = es.session_id \
             WHERE es.next_sequence - 1 > COALESCE(st.projected_sequence, 0) \
             ORDER BY es.updated_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.background_pool())
        .await?;
        Ok(ids)
    }

    /// Turns of a session's trace index, in turn order. For tests and the
    /// trace read API.
    pub async fn list_trace_turns(
        &self,
        session_id: Uuid,
        from_turn: i32,
        to_turn: i32,
    ) -> Result<Vec<TraceTurnRow>> {
        let rows = sqlx::query_as::<_, TraceTurnRow>(concat!(
            "SELECT ",
            turn_columns!(),
            " FROM session_trace_turns \
             WHERE session_id = $1 AND turn_no BETWEEN $2 AND $3 ORDER BY turn_no"
        ))
        .bind(session_id)
        .bind(from_turn)
        .bind(to_turn)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Steps of one turn in step order, `from_step` to `to_step` inclusive.
    pub async fn list_trace_steps(
        &self,
        session_id: Uuid,
        turn_no: i32,
        from_step: i32,
        to_step: i32,
    ) -> Result<Vec<TraceStepRow>> {
        let rows = sqlx::query_as::<_, TraceStepRow>(concat!(
            "SELECT ",
            step_columns!(),
            " FROM session_trace_steps \
             WHERE session_id = $1 AND turn_no = $2 AND step_no BETWEEN $3 AND $4 \
             ORDER BY step_no"
        ))
        .bind(session_id)
        .bind(turn_no)
        .bind(from_step)
        .bind(to_step)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

/// The events a pass projects, with payloads cut down in SQL to what the
/// projection reads, so a stored prompt or a full tool result never leaves the
/// database.
async fn load_event_slices(
    tx: &mut Transaction<'static, Postgres>,
    session_id: Uuid,
    after: i32,
    up_to: i32,
    limit: i64,
) -> Result<Vec<EventSlice>> {
    let types: Vec<String> = TRACE_EVENT_TYPES.iter().map(|t| t.to_string()).collect();
    let rows = sqlx::query_as::<_, EventSlice>(
        r#"
        SELECT sequence, event_type, ts, context->>'turn_id' AS turn_id,
            CASE event_type
                WHEN 'llm.generation' THEN jsonb_build_object(
                    'output', jsonb_build_object(
                        'text', LEFT(data->'output'->>'text', 2001),
                        'tool_calls', COALESCE((
                            SELECT jsonb_agg(jsonb_build_object('id', tc->>'id', 'name', tc->>'name'))
                            FROM jsonb_array_elements(
                                CASE jsonb_typeof(data->'output'->'tool_calls')
                                    WHEN 'array' THEN data->'output'->'tool_calls'
                                    ELSE '[]'::jsonb END) tc
                        ), '[]'::jsonb)),
                    'metadata', jsonb_build_object(
                        'model', data->'metadata'->'model',
                        'success', data->'metadata'->'success',
                        'error', data->'metadata'->'error',
                        'duration_ms', data->'metadata'->'duration_ms',
                        'usage', data->'metadata'->'usage'),
                    'message_count', CASE jsonb_typeof(data->'messages')
                        WHEN 'array' THEN jsonb_array_length(data->'messages') END)
                WHEN 'tool.started' THEN jsonb_build_object(
                    'tool_call', jsonb_build_object(
                        'id', data->'tool_call'->'id',
                        'name', data->'tool_call'->'name',
                        'arguments_preview', LEFT((data->'tool_call'->'arguments')::text, 301)),
                    'display_name', data->'display_name',
                    'narration', data->'narration')
                WHEN 'tool.completed' THEN jsonb_build_object(
                    'tool_call_id', data->'tool_call_id',
                    'tool_name', data->'tool_name',
                    'success', data->'success',
                    'status', data->'status',
                    'error', LEFT(data->>'error', 301),
                    'duration_ms', data->'duration_ms',
                    'narration', data->'narration',
                    'result_preview', LEFT(data->'result'->0->>'text', 301))
                WHEN 'turn.started' THEN jsonb_build_object(
                    'turn_id', data->'turn_id',
                    'input_content', LEFT(data->>'input_content', 501))
                ELSE jsonb_build_object(
                    'turn_id', data->'turn_id',
                    'error', LEFT(data->>'error', 301),
                    'usage', data->'usage')
            END AS data
        FROM events
        WHERE session_id = $1 AND sequence > $2 AND sequence <= $3
            AND event_type = ANY($4)
        ORDER BY sequence
        LIMIT $5
        "#,
    )
    .bind(session_id)
    .bind(after)
    .bind(up_to)
    .bind(&types)
    .bind(limit)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows)
}

async fn upsert_turns(
    tx: &mut Transaction<'static, Postgres>,
    session_id: Uuid,
    turns: &[TraceTurnRow],
) -> Result<()> {
    for chunk in turns.chunks(UPSERT_CHUNK) {
        let mut qb = QueryBuilder::<Postgres>::new(concat!(
            "INSERT INTO session_trace_turns (session_id, ",
            turn_columns!(),
            ") "
        ));
        qb.push_values(chunk, |mut row, t| {
            row.push_bind(session_id)
                .push_bind(t.turn_no)
                .push_bind(t.turn_id)
                .push_bind(t.start_sequence)
                .push_bind(t.end_sequence)
                .push_bind(t.started_at)
                .push_bind(t.ended_at)
                .push_bind(&t.status)
                .push_bind(&t.prompt_preview)
                .push_bind(&t.error)
                .push_bind(t.step_count)
                .push_bind(t.model_calls)
                .push_bind(t.tool_calls)
                .push_bind(t.subagent_calls)
                .push_bind(t.error_count)
                .push_bind(t.input_tokens)
                .push_bind(t.output_tokens);
        });
        qb.push(
            " ON CONFLICT (session_id, turn_no) DO UPDATE SET \
             end_sequence = EXCLUDED.end_sequence, ended_at = EXCLUDED.ended_at, \
             status = EXCLUDED.status, error = EXCLUDED.error, \
             step_count = EXCLUDED.step_count, model_calls = EXCLUDED.model_calls, \
             tool_calls = EXCLUDED.tool_calls, subagent_calls = EXCLUDED.subagent_calls, \
             error_count = EXCLUDED.error_count, input_tokens = EXCLUDED.input_tokens, \
             output_tokens = EXCLUDED.output_tokens",
        );
        qb.build().execute(&mut **tx).await?;
    }
    Ok(())
}

async fn upsert_steps(
    tx: &mut Transaction<'static, Postgres>,
    session_id: Uuid,
    steps: &[TraceStepRow],
) -> Result<()> {
    for chunk in steps.chunks(UPSERT_CHUNK) {
        let mut qb = QueryBuilder::<Postgres>::new(concat!(
            "INSERT INTO session_trace_steps (session_id, ",
            step_columns!(),
            ") "
        ));
        qb.push_values(chunk, |mut row, s| {
            row.push_bind(session_id)
                .push_bind(s.turn_no)
                .push_bind(s.step_no)
                .push_bind(&s.kind)
                .push_bind(&s.status)
                .push_bind(s.start_sequence)
                .push_bind(s.end_sequence)
                .push_bind(s.started_at)
                .push_bind(s.duration_ms)
                .push_bind(&s.name)
                .push_bind(&s.target)
                .push_bind(&s.result)
                .push_bind(&s.narration)
                .push_bind(&s.tool_call_id)
                .push_bind(&s.model)
                .push_bind(s.input_tokens)
                .push_bind(s.output_tokens)
                .push_bind(s.message_count)
                .push_bind(&s.requested_tool_call_ids);
        });
        qb.push(
            " ON CONFLICT (session_id, turn_no, step_no) DO UPDATE SET \
             status = EXCLUDED.status, end_sequence = EXCLUDED.end_sequence, \
             duration_ms = EXCLUDED.duration_ms, target = EXCLUDED.target, \
             result = EXCLUDED.result",
        );
        qb.build().execute(&mut **tx).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
