// Where a session's trace comes from (data plane v2, step 11.6).
//
// Decisions:
// - A v1 session's trace is the control plane's trace index, caught up
//   with its events before each read. A v2 session's events stay in its
//   cell, so its data plane computes the same rows with the same fold
//   (`everruns_core::session_trace`) and the control plane answers from
//   them in memory. The Trace commands read through `TraceSource` with the
//   index's own method names, so both kinds of session share one code path
//   for every Trace response.
// - The rows are fetched once per read, the event payloads only for the
//   ranges a read asks for (a step's detail, a page of a turn's events).
// - Runs (islands of one tool's calls) are grouped here as the index's
//   query groups them: consecutive steps of one tool, or one step of any
//   other kind, with the same percentiles.

use everruns_contracts::session_commands;
use std::collections::HashMap;

use anyhow::Result;
use everruns_contracts::dataplane::{SessionEventRow, SessionTraceRows};
use everruns_contracts::typed_id::{EventId, SessionId};
use serde_json::Value;
use uuid::Uuid;

use crate::domains::data_plane::session_plane::SessionPlane;
use crate::storage::{
    Database, TraceBucketRow, TraceEventRefRow, TraceRunRow, TraceStepRow, TraceTotalsRow,
    TraceTurnRow,
};

/// A session's trace: the control plane's index, or its data plane's rows.
pub enum TraceSource<'a> {
    Indexed(&'a Database),
    DataPlane(Box<DataPlaneTrace>),
}

/// A v2 session's trace rows, with its data plane for event payloads.
pub struct DataPlaneTrace {
    rows: SessionTraceRows,
    plane: SessionPlane,
}

impl DataPlaneTrace {
    /// The rows of `plane`'s session, read from its data plane now.
    pub async fn read(plane: SessionPlane) -> Result<Self> {
        let rows = serde_json::from_value(plane.read(&session_commands::TRACE).await?)?;
        Ok(Self { rows, plane })
    }

    async fn events(&self, from: i32, to: i32) -> Result<Vec<SessionEventRow>> {
        self.plane.events(from, to, &[], None).await
    }

    fn turns(&self) -> impl Iterator<Item = &TraceTurnRow> {
        self.rows.turns.iter()
    }
}

impl TraceSource<'_> {
    pub async fn trace_totals(&self, session: Uuid) -> Result<TraceTotalsRow> {
        let trace = match self {
            Self::Indexed(db) => return db.trace_totals(session).await,
            Self::DataPlane(trace) => trace,
        };
        Ok(totals(&trace.rows.turns))
    }

    pub async fn trace_buckets(
        &self,
        session: Uuid,
        bucket_size: i32,
    ) -> Result<Vec<TraceBucketRow>> {
        match self {
            Self::Indexed(db) => db.trace_buckets(session, bucket_size).await,
            Self::DataPlane(trace) => Ok(buckets(&trace.rows.turns, bucket_size)),
        }
    }

    pub async fn trace_error_turns(&self, session: Uuid, limit: i64) -> Result<Vec<i32>> {
        let trace = match self {
            Self::Indexed(db) => return db.trace_error_turns(session, limit).await,
            Self::DataPlane(trace) => trace,
        };
        let mut turns: Vec<i32> = trace
            .turns()
            .filter(|t| t.error_count > 0)
            .map(|t| t.turn_no)
            .collect();
        turns.sort_unstable_by(|a, b| b.cmp(a));
        turns.truncate(usize::try_from(limit).unwrap_or(0));
        Ok(turns)
    }

    pub async fn trace_turn_at_sequence(
        &self,
        session: Uuid,
        sequence: i32,
    ) -> Result<Option<i32>> {
        let trace = match self {
            Self::Indexed(db) => return db.trace_turn_at_sequence(session, sequence).await,
            Self::DataPlane(trace) => trace,
        };
        Ok(trace
            .turns()
            .filter(|t| t.start_sequence <= sequence)
            .max_by_key(|t| t.start_sequence)
            .map(|t| t.turn_no))
    }

    pub async fn list_trace_turns(
        &self,
        session: Uuid,
        from_turn: i32,
        to_turn: i32,
    ) -> Result<Vec<TraceTurnRow>> {
        let trace = match self {
            Self::Indexed(db) => return db.list_trace_turns(session, from_turn, to_turn).await,
            Self::DataPlane(trace) => trace,
        };
        Ok(trace
            .turns()
            .filter(|t| (from_turn..=to_turn).contains(&t.turn_no))
            .cloned()
            .collect())
    }

    pub async fn trace_runs(
        &self,
        session: Uuid,
        from_turn: i32,
        to_turn: i32,
    ) -> Result<Vec<TraceRunRow>> {
        match self {
            Self::Indexed(db) => db.trace_runs(session, from_turn, to_turn).await,
            Self::DataPlane(trace) => Ok(runs(&trace.rows.steps, from_turn, to_turn)),
        }
    }

    pub async fn trace_steps_by_key(
        &self,
        session: Uuid,
        keys: &[(i32, i32)],
    ) -> Result<Vec<TraceStepRow>> {
        let trace = match self {
            Self::Indexed(db) => return db.trace_steps_by_key(session, keys).await,
            Self::DataPlane(trace) => trace,
        };
        let mut keys = keys.to_vec();
        keys.sort_unstable();
        let by_key: HashMap<(i32, i32), &TraceStepRow> = trace
            .rows
            .steps
            .iter()
            .map(|s| ((s.turn_no, s.step_no), s))
            .collect();
        Ok(keys
            .iter()
            .filter_map(|k| by_key.get(k).map(|s| (*s).clone()))
            .collect())
    }

    pub async fn trace_failures_in_ranges(
        &self,
        session: Uuid,
        ranges: &[(i32, i32, i32)],
        per_range: i64,
    ) -> Result<Vec<TraceStepRow>> {
        let trace = match self {
            Self::Indexed(db) => {
                return db
                    .trace_failures_in_ranges(session, ranges, per_range)
                    .await;
            }
            Self::DataPlane(trace) => trace,
        };
        let per_range = usize::try_from(per_range).unwrap_or(0);
        let mut out: Vec<TraceStepRow> = ranges
            .iter()
            .flat_map(|(turn, first, last)| {
                trace
                    .rows
                    .steps
                    .iter()
                    .filter(move |s| {
                        s.turn_no == *turn
                            && (*first..=*last).contains(&s.step_no)
                            && s.status == "error"
                    })
                    .take(per_range)
                    .cloned()
            })
            .collect();
        out.sort_by_key(|s| (s.turn_no, s.step_no));
        Ok(out)
    }

    pub async fn trace_steps_page(
        &self,
        session: Uuid,
        turn_no: i32,
        from_step: i32,
        to_step: i32,
        errors_only: bool,
        limit: i64,
    ) -> Result<Vec<TraceStepRow>> {
        let trace = match self {
            Self::Indexed(db) => {
                return db
                    .trace_steps_page(session, turn_no, from_step, to_step, errors_only, limit)
                    .await;
            }
            Self::DataPlane(trace) => trace,
        };
        Ok(trace
            .rows
            .steps
            .iter()
            .filter(|s| {
                s.turn_no == turn_no
                    && (from_step..=to_step).contains(&s.step_no)
                    && (!errors_only || s.status == "error")
            })
            .take(usize::try_from(limit).unwrap_or(0))
            .cloned()
            .collect())
    }

    pub async fn trace_child_sessions(
        &self,
        session: Uuid,
        tool_call_ids: &[String],
    ) -> Result<Vec<(String, Uuid)>> {
        let trace = match self {
            Self::Indexed(db) => return db.trace_child_sessions(session, tool_call_ids).await,
            Self::DataPlane(trace) => trace,
        };
        Ok(trace
            .rows
            .children
            .iter()
            .filter(|(call, _)| tool_call_ids.contains(call))
            .filter_map(|(call, child)| {
                let child = child.parse::<SessionId>().ok()?;
                Some((call.clone(), child.uuid()))
            })
            .collect())
    }

    pub async fn trace_events_in_range(
        &self,
        session: Uuid,
        from_sequence: i32,
        to_sequence: i32,
        inline_bytes: i32,
        limit: i64,
    ) -> Result<Vec<TraceEventRefRow>> {
        let trace = match self {
            Self::Indexed(db) => {
                return db
                    .trace_events_in_range(session, from_sequence, to_sequence, inline_bytes, limit)
                    .await;
            }
            Self::DataPlane(trace) => trace,
        };
        let inline = usize::try_from(inline_bytes).unwrap_or(0);
        Ok(trace
            .events(from_sequence, to_sequence)
            .await?
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(0))
            .map(|event| {
                let size = serde_json::to_string(&event.data).map_or(0, |s| s.len());
                TraceEventRefRow {
                    id: event_uuid(&event.id),
                    sequence: event.sequence,
                    event_type: event.event_type,
                    ts: event.ts,
                    data: (size <= inline).then_some(event.data),
                    size_bytes: i32::try_from(size).unwrap_or(i32::MAX),
                }
            })
            .collect())
    }

    pub async fn trace_event_data(
        &self,
        session: Uuid,
        sequence: i32,
    ) -> Result<Option<(String, Value)>> {
        let trace = match self {
            Self::Indexed(db) => return db.trace_event_data(session, sequence).await,
            Self::DataPlane(trace) => trace,
        };
        Ok(trace
            .events(sequence, sequence)
            .await?
            .into_iter()
            .next()
            .map(|e| (e.event_type, e.data)))
    }

    pub async fn trace_tool_call_events(
        &self,
        session: Uuid,
        from_sequence: i32,
        to_sequence: i32,
        tool_call_id: &str,
    ) -> Result<Vec<(i32, String, Value)>> {
        let trace = match self {
            Self::Indexed(db) => {
                return db
                    .trace_tool_call_events(session, from_sequence, to_sequence, tool_call_id)
                    .await;
            }
            Self::DataPlane(trace) => trace,
        };
        Ok(trace
            .events(from_sequence, to_sequence)
            .await?
            .into_iter()
            .filter(|e| matches!(e.event_type.as_str(), "tool.started" | "tool.completed"))
            .filter(|e| {
                e.data["tool_call"]["id"]
                    .as_str()
                    .or_else(|| e.data["tool_call_id"].as_str())
                    == Some(tool_call_id)
            })
            .map(|e| (e.sequence, e.event_type, e.data))
            .collect())
    }

    pub async fn trace_previous_model_step(
        &self,
        session: Uuid,
        turn_no: i32,
        step_no: i32,
    ) -> Result<Option<TraceStepRow>> {
        let trace = match self {
            Self::Indexed(db) => {
                return db
                    .trace_previous_model_step(session, turn_no, step_no)
                    .await;
            }
            Self::DataPlane(trace) => trace,
        };
        Ok(trace
            .rows
            .steps
            .iter()
            .filter(|s| (s.turn_no, s.step_no) < (turn_no, step_no))
            .filter(|s| matches!(s.kind.as_str(), "model" | "answer"))
            .max_by_key(|s| (s.turn_no, s.step_no))
            .cloned())
    }
}

/// An event id as the index stores it: its uuid.
fn event_uuid(id: &str) -> Uuid {
    id.parse::<EventId>()
        .map(|id| id.uuid())
        .or_else(|_| id.parse::<Uuid>())
        .unwrap_or_default()
}

fn totals(turns: &[TraceTurnRow]) -> TraceTotalsRow {
    TraceTotalsRow {
        turn_count: turns.len() as i64,
        step_count: turns.iter().map(|t| i64::from(t.step_count)).sum(),
        error_count: turns.iter().map(|t| i64::from(t.error_count)).sum(),
        input_tokens: turns.iter().map(|t| t.input_tokens).sum(),
        output_tokens: turns.iter().map(|t| t.output_tokens).sum(),
        first_started_at: turns.iter().map(|t| t.started_at).min(),
        last_activity_at: turns
            .iter()
            .map(|t| t.ended_at.unwrap_or(t.started_at))
            .max(),
    }
}

/// Turns grouped `bucket_size` at a time, as `trace_buckets` groups them.
fn buckets(turns: &[TraceTurnRow], bucket_size: i32) -> Vec<TraceBucketRow> {
    let size = bucket_size.max(1);
    let mut out: Vec<(i32, TraceBucketRow)> = Vec::new();
    for turn in turns {
        let bucket = (turn.turn_no - 1) / size;
        let duration = turn
            .ended_at
            .map_or(0, |end| (end - turn.started_at).num_milliseconds());
        match out.last_mut() {
            Some((at, row)) if *at == bucket => {
                row.from_turn = row.from_turn.min(turn.turn_no);
                row.to_turn = row.to_turn.max(turn.turn_no);
                row.steps += i64::from(turn.step_count);
                row.duration_ms += duration;
                row.errors += i64::from(turn.error_count);
            }
            _ => out.push((
                bucket,
                TraceBucketRow {
                    from_turn: turn.turn_no,
                    to_turn: turn.turn_no,
                    steps: i64::from(turn.step_count),
                    duration_ms: duration,
                    errors: i64::from(turn.error_count),
                },
            )),
        }
    }
    out.into_iter().map(|(_, row)| row).collect()
}

/// Runs of steps for turns `from_turn..=to_turn`: consecutive steps of
/// one tool, or one step of any other kind.
fn runs(steps: &[TraceStepRow], from_turn: i32, to_turn: i32) -> Vec<TraceRunRow> {
    let key = |s: &TraceStepRow| match s.kind.as_str() {
        "tool" => format!("tool:{}", s.name.as_deref().unwrap_or_default()),
        _ => format!("step:{}", s.step_no),
    };
    let mut groups: Vec<Vec<&TraceStepRow>> = Vec::new();
    let mut in_range: Vec<&TraceStepRow> = steps
        .iter()
        .filter(|s| (from_turn..=to_turn).contains(&s.turn_no))
        .collect();
    in_range.sort_by_key(|s| (s.turn_no, s.step_no));
    for step in in_range {
        match groups.last_mut() {
            Some(group)
                if group.last().is_some_and(|last| {
                    last.turn_no == step.turn_no
                        && last.step_no + 1 == step.step_no
                        && key(last) == key(step)
                }) =>
            {
                group.push(step);
            }
            _ => groups.push(vec![step]),
        }
    }
    groups
        .into_iter()
        .filter_map(|group| run_of(&group))
        .collect()
}

fn run_of(group: &[&TraceStepRow]) -> Option<TraceRunRow> {
    let first = group.first()?;
    let last = group.last()?;
    let mut durations: Vec<f64> = group
        .iter()
        .filter_map(|s| s.duration_ms.map(|d| d as f64))
        .collect();
    durations.sort_by(f64::total_cmp);
    let ended_at = group
        .iter()
        .map(|s| s.started_at + chrono::Duration::milliseconds(s.duration_ms.unwrap_or(0)))
        .max();
    Some(TraceRunRow {
        turn_no: first.turn_no,
        first_step: first.step_no,
        last_step: last.step_no,
        kind: group.iter().map(|s| s.kind.clone()).min()?,
        name: group.iter().filter_map(|s| s.name.clone()).min(),
        count: group.len() as i64,
        failed: group.iter().filter(|s| s.status == "error").count() as i64,
        running: group.iter().filter(|s| s.status == "running").count() as i64,
        p50_ms: percentile(&durations, 0.5),
        p95_ms: percentile(&durations, 0.95),
        started_at: group.iter().map(|s| s.started_at).min()?,
        ended_at,
    })
}

/// PostgreSQL's `percentile_cont` over sorted `values`.
fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    let last = values.len().checked_sub(1)?;
    let at = fraction * last as f64;
    let (low, high) = (at.floor() as usize, at.ceil() as usize);
    let (a, b) = (values[low.min(last)], values[high.min(last)]);
    Some(a + (b - a) * (at - at.floor()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use chrono::{DateTime, Utc};

    use super::*;

    fn step(turn: i32, n: i32, kind: &str, name: &str, status: &str, ms: i64) -> TraceStepRow {
        TraceStepRow {
            turn_no: turn,
            step_no: n,
            kind: kind.into(),
            status: status.into(),
            start_sequence: n,
            end_sequence: Some(n),
            started_at: DateTime::<Utc>::from_timestamp(1_000 + i64::from(n), 0).unwrap(),
            duration_ms: Some(ms),
            name: Some(name.into()),
            target: None,
            result: None,
            narration: None,
            tool_call_id: None,
            model: None,
            input_tokens: None,
            output_tokens: None,
            message_count: None,
            requested_tool_call_ids: Vec::new(),
            retry_attempts: None,
            retry_wait_ms: None,
        }
    }

    #[test]
    fn runs_group_one_tools_consecutive_calls() {
        let steps = vec![
            step(1, 1, "model", "m", "ok", 10),
            step(1, 2, "tool", "read", "ok", 10),
            step(1, 3, "tool", "read", "error", 20),
            step(1, 4, "tool", "read", "ok", 30),
            step(1, 5, "model", "m", "ok", 10),
            step(1, 6, "tool", "read", "running", 40),
            step(2, 1, "tool", "read", "ok", 5),
        ];
        let runs = runs(&steps, 1, 1);
        let shape: Vec<_> = runs
            .iter()
            .map(|r| (r.first_step, r.last_step, r.count, r.failed, r.running))
            .collect();
        assert_eq!(
            shape,
            vec![
                (1, 1, 1, 0, 0),
                (2, 4, 3, 1, 0),
                (5, 5, 1, 0, 0),
                (6, 6, 1, 0, 1)
            ]
        );
        assert_eq!(runs[1].p50_ms, Some(20.0));
        assert_eq!(runs[1].p95_ms, Some(29.0));
    }

    #[test]
    fn percentiles_interpolate_as_postgres_does() {
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[7.0], 0.95), Some(7.0));
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 0.5), Some(2.5));
    }
}
