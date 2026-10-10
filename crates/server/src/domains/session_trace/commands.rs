// Session trace commands: the read API behind the session Trace view.
//
// Decision: each read first catches the session's trace index up with its
// events, so a reader sees every committed event; the index is otherwise kept
// current by the write path and the backfill. See
// knowledge/ui/session-trace.md.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{EventId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::plan::{PlannedRow, plan_turn};
use super::types::*;
use crate::domains::common::*;
use crate::domains::sessions::SESSION_VIEW;
use crate::storage::{TraceStepRow, TraceTurnRow};

/// Turns per page by default and at most.
const DEFAULT_TURNS: i32 = 20;
const MAX_TURNS: i32 = 50;
/// Minimap buckets by default and at most.
const DEFAULT_BUCKETS: i32 = 120;
const MAX_BUCKETS: i32 = 500;
/// Error turns listed on the overview.
const ERROR_TURNS: i64 = 500;
/// Failed calls returned inline with a batch.
const BATCH_FAILURES: i64 = 20;
/// Steps per page by default and at most.
const DEFAULT_STEPS: i32 = 100;
const MAX_STEPS: i32 = 500;
/// Payloads over this size are cut unless `full` is asked for.
const INLINE_PAYLOAD_BYTES: usize = 50 * 1024;
/// Event payloads inlined in event lists.
const INLINE_EVENT_BYTES: i32 = 8 * 1024;
/// Events listed for one step or one page of a turn.
const MAX_EVENTS: i64 = 500;
/// New request messages shown in a step's detail.
const NEW_MESSAGES: usize = 50;
/// Characters of a request message preview.
const MESSAGE_PREVIEW_CHARS: usize = 300;

async fn session_uuid(ctx: &Ctx, raw: &str) -> Result<Uuid, CommandError> {
    let session_id: SessionId = raw
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid session ID: {e}")))?;
    if ctx
        .db
        .get_session(ctx.org_id(), session_id)
        .await?
        .is_none()
    {
        return Err(CommandError::not_found("Session"));
    }
    let id = session_id.uuid();
    ctx.db.database().catch_up_session_trace_fully(id).await?;
    Ok(id)
}

async fn turn_row(ctx: &Ctx, session: Uuid, turn: i32) -> Result<TraceTurnRow, CommandError> {
    ctx.db
        .database()
        .list_trace_turns(session, turn, turn)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::not_found("Turn"))
}

fn step_out(row: TraceStepRow, turn_started: DateTime<Utc>, child: Option<Uuid>) -> TraceStep {
    TraceStep {
        turn: row.turn_no,
        step: row.step_no,
        kind: row.kind,
        status: row.status,
        offset_ms: (row.started_at - turn_started).num_milliseconds().max(0),
        started_at: row.started_at,
        duration_ms: row.duration_ms,
        name: row.name,
        target: row.target,
        result: row.result,
        narration: row.narration,
        tool_call_id: row.tool_call_id,
        model: row.model,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        requested_tool_call_ids: row.requested_tool_call_ids,
        child_session_id: child.map(|id| SessionId::from_uuid(id).to_string()),
        start_sequence: row.start_sequence,
        end_sequence: row.end_sequence,
    }
}

fn turn_out(row: &TraceTurnRow, items: Vec<TraceItem>) -> TraceTurn {
    TraceTurn {
        turn: row.turn_no,
        turn_id: TurnId::from_uuid(row.turn_id).to_string(),
        status: row.status.clone(),
        prompt: row.prompt_preview.clone(),
        error: row.error.clone(),
        started_at: row.started_at,
        ended_at: row.ended_at,
        duration_ms: row
            .ended_at
            .map(|end| (end - row.started_at).num_milliseconds().max(0)),
        step_count: row.step_count,
        model_calls: row.model_calls,
        tool_calls: row.tool_calls,
        subagent_calls: row.subagent_calls,
        error_count: row.error_count,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        start_sequence: row.start_sequence,
        end_sequence: row.end_sequence,
        items,
    }
}

/// Child sessions for the `spawn_agent` steps among `steps`.
async fn children(
    ctx: &Ctx,
    session: Uuid,
    steps: &[TraceStepRow],
) -> Result<HashMap<String, Uuid>, CommandError> {
    let ids: Vec<String> = steps
        .iter()
        .filter(|s| s.kind == "agent")
        .filter_map(|s| s.tool_call_id.clone())
        .collect();
    Ok(ctx
        .db
        .database()
        .trace_child_sessions(session, &ids)
        .await?
        .into_iter()
        .collect())
}

fn child_of(children: &HashMap<String, Uuid>, step: &TraceStepRow) -> Option<Uuid> {
    step.tool_call_id
        .as_ref()
        .and_then(|id| children.get(id))
        .copied()
}

fn payload(value: Value, full: bool) -> TracePayload {
    let text = value.to_string();
    let size_bytes = text.len();
    if full || size_bytes <= INLINE_PAYLOAD_BYTES {
        return TracePayload {
            value: Some(value),
            preview: None,
            size_bytes,
            truncated: false,
        };
    }
    let mut cut = INLINE_PAYLOAD_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    TracePayload {
        value: None,
        preview: Some(text[..cut].to_string()),
        size_bytes,
        truncated: true,
    }
}

fn event_ref(row: crate::storage::TraceEventRefRow) -> TraceEventRef {
    TraceEventRef {
        id: EventId::from_uuid(row.id).to_string(),
        sequence: row.sequence,
        event_type: row.event_type,
        ts: row.ts,
        data: row.data,
        size_bytes: row.size_bytes,
    }
}

// ============================================================================
// GetSessionTrace
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct GetSessionTrace {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
    /// Minimap buckets wanted, 1 to 500. Defaults to 120.
    #[serde(default)]
    pub buckets: Option<i32>,
}

#[command(
    name = "get_session_trace",
    category = "sessions",
    description = "Summarize a session's trace: turn, step and error totals, a minimap of turn buckets, and the turns that failed.",
    method = "GET",
    path = "/v1/sessions/{session_id}/trace",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "trace"], "get").with_examples(&[CliExample::new("Count a session's turns, steps and errors", "everruns sessions trace get --session-id session_01h9")]),
    positional = "session_id",
    http = plain,
    params(GetSessionTrace),
)]
impl Command for GetSessionTrace {
    type Output = TraceOverview;

    async fn execute(self, ctx: &Ctx) -> Result<TraceOverview, CommandError> {
        let session = session_uuid(ctx, &self.session_id).await?;
        let db = ctx.db.database();
        let totals = db.trace_totals(session).await?;
        let wanted = i64::from(
            self.buckets
                .unwrap_or(DEFAULT_BUCKETS)
                .clamp(1, MAX_BUCKETS),
        );
        let bucket_size =
            i32::try_from(((totals.turn_count + wanted - 1) / wanted).max(1)).unwrap_or(i32::MAX);
        let buckets = db
            .trace_buckets(session, bucket_size)
            .await?
            .into_iter()
            .map(|b| TraceBucket {
                from_turn: b.from_turn,
                to_turn: b.to_turn,
                steps: b.steps,
                duration_ms: b.duration_ms,
                errors: b.errors,
            })
            .collect();
        Ok(TraceOverview {
            turn_count: totals.turn_count,
            step_count: totals.step_count,
            error_count: totals.error_count,
            input_tokens: totals.input_tokens,
            output_tokens: totals.output_tokens,
            first_started_at: totals.first_started_at,
            last_activity_at: totals.last_activity_at,
            bucket_size,
            buckets,
            error_turns: db.trace_error_turns(session, ERROR_TURNS).await?,
        })
    }
}

// ============================================================================
// ListSessionTraceTurns
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListSessionTraceTurns {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
    /// Turns before this turn number. Without a cursor, the last turns.
    #[serde(default)]
    pub before: Option<i32>,
    /// Turns after this turn number.
    #[serde(default)]
    pub after: Option<i32>,
    /// Turns centered on this turn number.
    #[serde(default)]
    pub around: Option<i32>,
    /// Turns centered on the turn holding this event sequence.
    #[serde(default)]
    pub sequence: Option<i32>,
    /// Turns per page, 1 to 50. Defaults to 20.
    #[serde(default)]
    pub limit: Option<i32>,
}

#[command(
    name = "list_session_trace_turns",
    category = "sessions",
    description = "List a page of a session's turns with their steps: model calls, tool calls, approvals, sub-agents and messages, with repeated calls folded into batches.",
    method = "GET",
    path = "/v1/sessions/{session_id}/trace/turns",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "trace", "turns"], "list").with_examples(&[CliExample::new("Open the latest turns of a long session", "everruns sessions trace turns list --session-id session_01h9 --limit 20")]),
    positional = "session_id",
    http = plain,
    params(ListSessionTraceTurns),
)]
impl Command for ListSessionTraceTurns {
    type Output = TraceTurnsPage;

    async fn execute(self, ctx: &Ctx) -> Result<TraceTurnsPage, CommandError> {
        let cursors = [
            self.before.is_some(),
            self.after.is_some(),
            self.around.is_some(),
            self.sequence.is_some(),
        ];
        if cursors.iter().filter(|c| **c).count() > 1 {
            return Err(CommandError::bad_request(
                "Use one of before, after, around or sequence",
            ));
        }
        let session = session_uuid(ctx, &self.session_id).await?;
        let db = ctx.db.database();
        let limit = self.limit.unwrap_or(DEFAULT_TURNS).clamp(1, MAX_TURNS);
        let turn_count = i32::try_from(db.trace_totals(session).await?.turn_count).unwrap_or(0);

        let around = match self.sequence {
            Some(sequence) => db.trace_turn_at_sequence(session, sequence).await?,
            None => self.around,
        };
        let (from, to) = if let Some(before) = self.before {
            ((before - limit).max(1), before - 1)
        } else if let Some(after) = self.after {
            (after + 1, after + limit)
        } else if let Some(center) = around {
            let from = (center - limit / 2).max(1);
            (from, from + limit - 1)
        } else {
            ((turn_count - limit + 1).max(1), turn_count)
        };
        let to = to.min(turn_count);
        if turn_count == 0 || from > to {
            return Ok(TraceTurnsPage {
                turns: Vec::new(),
                has_earlier: from > 1 && turn_count > 0,
                has_later: false,
                turn_count: i64::from(turn_count),
            });
        }

        let turns = db.list_trace_turns(session, from, to).await?;
        let runs = db.trace_runs(session, from, to).await?;
        let mut runs_by_turn: HashMap<i32, Vec<crate::storage::TraceRunRow>> = HashMap::new();
        for run in runs {
            runs_by_turn.entry(run.turn_no).or_default().push(run);
        }
        let plans: Vec<(i32, Vec<PlannedRow>)> = turns
            .iter()
            .map(|t| {
                let runs = runs_by_turn.remove(&t.turn_no).unwrap_or_default();
                (t.turn_no, plan_turn(&runs))
            })
            .collect();

        let mut keys = Vec::new();
        let mut ranges = Vec::new();
        for (turn_no, plan) in &plans {
            for row in plan {
                match row {
                    PlannedRow::Step { step } => keys.push((*turn_no, *step)),
                    PlannedRow::Batch(run) if run.failed > 0 => {
                        ranges.push((*turn_no, run.first_step, run.last_step));
                    }
                    _ => {}
                }
            }
        }
        let steps = db.trace_steps_by_key(session, &keys).await?;
        let failures = db
            .trace_failures_in_ranges(session, &ranges, BATCH_FAILURES)
            .await?;
        let children = children(ctx, session, &steps).await?;
        let starts: HashMap<i32, DateTime<Utc>> =
            turns.iter().map(|t| (t.turn_no, t.started_at)).collect();
        let mut steps_by_key: HashMap<(i32, i32), TraceStepRow> = steps
            .into_iter()
            .map(|s| ((s.turn_no, s.step_no), s))
            .collect();
        let mut failures_by_turn: HashMap<i32, Vec<TraceStepRow>> = HashMap::new();
        for failure in failures {
            failures_by_turn
                .entry(failure.turn_no)
                .or_default()
                .push(failure);
        }

        let mut out = Vec::with_capacity(turns.len());
        for (turn, (turn_no, plan)) in turns.iter().zip(plans) {
            let started = starts.get(&turn_no).copied().unwrap_or(turn.started_at);
            let mut items = Vec::with_capacity(plan.len());
            for row in plan {
                match row {
                    PlannedRow::Step { step } => {
                        if let Some(row) = steps_by_key.remove(&(turn_no, step)) {
                            let child = child_of(&children, &row);
                            items.push(TraceItem::Step(step_out(row, started, child)));
                        }
                    }
                    PlannedRow::Batch(run) => {
                        let batch_failures = failures_by_turn
                            .get(&turn_no)
                            .map(|all| {
                                all.iter()
                                    .filter(|f| {
                                        f.step_no >= run.first_step && f.step_no <= run.last_step
                                    })
                                    .cloned()
                                    .map(|f| step_out(f, started, None))
                                    .collect()
                            })
                            .unwrap_or_default();
                        items.push(TraceItem::Batch(TraceBatch {
                            turn: turn_no,
                            name: run.name.clone().unwrap_or_default(),
                            first_step: run.first_step,
                            last_step: run.last_step,
                            count: run.count,
                            succeeded: run.count - run.failed - run.running,
                            failed: run.failed,
                            running: run.running,
                            p50_ms: run.p50_ms.map(|v| v.round() as i64),
                            p95_ms: run.p95_ms.map(|v| v.round() as i64),
                            offset_ms: (run.started_at - started).num_milliseconds().max(0),
                            started_at: run.started_at,
                            wall_ms: run
                                .ended_at
                                .map(|end| (end - run.started_at).num_milliseconds().max(0)),
                            failures: batch_failures,
                        }));
                    }
                    PlannedRow::Gap {
                        first_step,
                        last_step,
                        count,
                        errors,
                    } => items.push(TraceItem::Gap(TraceGap {
                        turn: turn_no,
                        first_step,
                        last_step,
                        count,
                        errors,
                    })),
                }
            }
            out.push(turn_out(turn, items));
        }

        Ok(TraceTurnsPage {
            has_earlier: from > 1,
            has_later: to < turn_count,
            turns: out,
            turn_count: i64::from(turn_count),
        })
    }
}

// ============================================================================
// ListSessionTraceSteps
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListSessionTraceSteps {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
    /// Turn number (a path parameter).
    #[param(ignore)]
    pub turn: i32,
    /// First step of the range, inclusive. Defaults to 1.
    #[serde(default)]
    pub from_step: Option<i32>,
    /// Last step of the range, inclusive. Defaults to the turn's last step.
    #[serde(default)]
    pub to_step: Option<i32>,
    /// Only failed steps.
    #[serde(default)]
    pub errors_only: Option<bool>,
    /// Steps per page, 1 to 500. Defaults to 100.
    #[serde(default)]
    pub limit: Option<i32>,
}

#[command(
    name = "list_session_trace_steps",
    category = "sessions",
    description = "Page through the steps of one turn of a session's trace, optionally only the failed ones.",
    method = "GET",
    path = "/v1/sessions/{session_id}/trace/turns/{turn}/steps",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "trace", "turns", "steps"], "list").with_examples(&[CliExample::new("List only the failed steps of turn 12", "everruns sessions trace turns steps list --session-id session_01h9 --turn 12 --errors-only true")]),
    positional = "session_id",
    http = plain,
    params(ListSessionTraceSteps),
)]
impl Command for ListSessionTraceSteps {
    type Output = TraceStepsPage;

    async fn execute(self, ctx: &Ctx) -> Result<TraceStepsPage, CommandError> {
        let session = session_uuid(ctx, &self.session_id).await?;
        let turn = turn_row(ctx, session, self.turn).await?;
        let limit = self.limit.unwrap_or(DEFAULT_STEPS).clamp(1, MAX_STEPS);
        let from = self.from_step.unwrap_or(1).max(1);
        let to = self.to_step.unwrap_or(turn.step_count).min(turn.step_count);
        let mut rows = ctx
            .db
            .database()
            .trace_steps_page(
                session,
                turn.turn_no,
                from,
                to,
                self.errors_only.unwrap_or(false),
                i64::from(limit) + 1,
            )
            .await?;
        let next_step = if rows.len() > limit as usize {
            rows.truncate(limit as usize);
            rows.last().map(|s| s.step_no + 1)
        } else {
            None
        };
        let children = children(ctx, session, &rows).await?;
        Ok(TraceStepsPage {
            steps: rows
                .into_iter()
                .map(|row| {
                    let child = child_of(&children, &row);
                    step_out(row, turn.started_at, child)
                })
                .collect(),
            next_step,
        })
    }
}

// ============================================================================
// GetSessionTraceStep
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct GetSessionTraceStep {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
    /// Turn number (a path parameter).
    #[param(ignore)]
    pub turn: i32,
    /// Step number within the turn (a path parameter).
    #[param(ignore)]
    pub step: i32,
    /// Return payloads whole, however large.
    #[serde(default)]
    pub full: Option<bool>,
}

#[command(
    name = "get_session_trace_step",
    category = "sessions",
    description = "Read one step of a session's trace in full: tool input and output, the model request and response, and its raw events.",
    method = "GET",
    path = "/v1/sessions/{session_id}/trace/turns/{turn}/steps/{step}",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "trace", "turns", "steps"], "get").with_examples(&[CliExample::new("Read a tool call's full input and output", "everruns sessions trace turns steps get --session-id session_01h9 --turn 12 --step 4 --full true")]),
    positional = "session_id",
    http = plain,
    params(GetSessionTraceStep),
)]
impl Command for GetSessionTraceStep {
    type Output = TraceStepDetail;

    async fn execute(self, ctx: &Ctx) -> Result<TraceStepDetail, CommandError> {
        let session = session_uuid(ctx, &self.session_id).await?;
        let turn = turn_row(ctx, session, self.turn).await?;
        let db = ctx.db.database();
        let row = db
            .trace_steps_by_key(session, &[(self.turn, self.step)])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| CommandError::not_found("Step"))?;
        let full = self.full.unwrap_or(false);
        let end = row.end_sequence.unwrap_or(row.start_sequence);
        let mut input = None;
        let mut output = None;
        let mut request = None;
        let events;

        if let Some(call_id) = row.tool_call_id.as_deref() {
            let calls = db
                .trace_tool_call_events(session, row.start_sequence, end, call_id)
                .await?;
            let sequences: Vec<i32> = calls.iter().map(|(seq, _, _)| *seq).collect();
            for (_, event_type, data) in calls {
                match event_type.as_str() {
                    "tool.started" => {
                        input = data
                            .get("tool_call")
                            .and_then(|c| c.get("arguments"))
                            .cloned()
                            .map(|v| payload(v, full));
                    }
                    _ => {
                        let value = match data.get("error") {
                            Some(error) if !error.is_null() => {
                                serde_json::json!({ "error": error, "status": data.get("status") })
                            }
                            _ => data.get("result").cloned().unwrap_or(Value::Null),
                        };
                        output = Some(payload(value, full));
                    }
                }
            }
            events = db
                .trace_events_in_range(
                    session,
                    row.start_sequence,
                    end,
                    INLINE_EVENT_BYTES,
                    MAX_EVENTS,
                )
                .await?
                .into_iter()
                .filter(|e| {
                    sequences.contains(&e.sequence)
                        || !matches!(e.event_type.as_str(), "tool.started" | "tool.completed")
                })
                .map(event_ref)
                .collect();
        } else {
            if let Some((_, data)) = db.trace_event_data(session, row.start_sequence).await? {
                let new_from = db
                    .trace_previous_message_count(session, row.turn_no, row.step_no)
                    .await?
                    .unwrap_or(0);
                request = Some(request_summary(&data, new_from));
                output = data.get("output").cloned().map(|v| payload(v, full));
            }
            events = db
                .trace_events_in_range(
                    session,
                    row.start_sequence,
                    end,
                    INLINE_EVENT_BYTES,
                    MAX_EVENTS,
                )
                .await?
                .into_iter()
                .map(event_ref)
                .collect();
        }

        let children = children(ctx, session, std::slice::from_ref(&row)).await?;
        let child = child_of(&children, &row);
        Ok(TraceStepDetail {
            step: step_out(row, turn.started_at, child),
            input,
            output,
            request,
            events,
        })
    }
}

fn messages(data: &Value) -> &[Value] {
    data.get("messages")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn message_out(index: usize, message: &Value, with_content: bool) -> TraceRequestMessage {
    let text = message.to_string();
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    TraceRequestMessage {
        index: i32::try_from(index).unwrap_or(i32::MAX),
        role,
        preview: message_preview(message),
        size_bytes: text.len(),
        content: (with_content && text.len() <= INLINE_PAYLOAD_BYTES).then(|| message.clone()),
    }
}

/// The text a person would read first: text parts, else the tool calls asked for.
fn message_preview(message: &Value) -> String {
    let mut out = String::new();
    let content = message.get("content");
    match content {
        Some(Value::String(text)) => out.push_str(text),
        Some(Value::Array(parts)) => {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(text);
                } else if let Some(name) = part
                    .get("name")
                    .or_else(|| part.get("tool_call").and_then(|c| c.get("name")))
                    .and_then(Value::as_str)
                {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(&format!("[{name}]"));
                }
                if out.len() > MESSAGE_PREVIEW_CHARS * 4 {
                    break;
                }
            }
        }
        _ => {}
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            if let Some(name) = call.get("name").and_then(Value::as_str) {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&format!("[{name}]"));
            }
        }
    }
    match out.char_indices().nth(MESSAGE_PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}…", &out[..cut]),
        None => out,
    }
}

fn request_summary(data: &Value, new_from: i32) -> TraceRequestSummary {
    let all = messages(data);
    let count = all.len();
    // Compaction can shrink the history; then everything counts as new.
    let new_from = usize::try_from(new_from).unwrap_or(0);
    let new_from = if new_from > count { 0 } else { new_from };
    let system_preview = all
        .iter()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("system"))
        .map(message_preview);
    TraceRequestSummary {
        message_count: i32::try_from(count).unwrap_or(i32::MAX),
        new_from: i32::try_from(new_from).unwrap_or(i32::MAX),
        tool_count: data
            .get("tools")
            .and_then(Value::as_array)
            .map_or(0, |t| i32::try_from(t.len()).unwrap_or(i32::MAX)),
        system_preview,
        new_messages: all
            .iter()
            .enumerate()
            .skip(new_from)
            .take(NEW_MESSAGES)
            .map(|(i, m)| message_out(i, m, false))
            .collect(),
    }
}

// ============================================================================
// ListSessionTraceRequest
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListSessionTraceRequest {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
    /// Turn number (a path parameter).
    #[param(ignore)]
    pub turn: i32,
    /// Step number of a model call (a path parameter).
    #[param(ignore)]
    pub step: i32,
    /// Only messages of this role: `system`, `user`, `assistant` or `tool`.
    #[serde(default)]
    pub role: Option<String>,
    /// First message index. Defaults to 0.
    #[serde(default)]
    pub offset: Option<i32>,
    /// Messages per page, 1 to 200. Defaults to 100.
    #[serde(default)]
    pub limit: Option<i32>,
}

#[command(
    name = "list_session_trace_request",
    category = "sessions",
    description = "Page through the messages a model call in a session's trace was sent, with each message's full content.",
    method = "GET",
    path = "/v1/sessions/{session_id}/trace/turns/{turn}/steps/{step}/request",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "trace", "turns", "steps", "request"], "list").with_examples(&[CliExample::new("See exactly what a model call was sent", "everruns sessions trace turns steps request list --session-id session_01h9 --turn 12 --step 3")]),
    positional = "session_id",
    http = plain,
    params(ListSessionTraceRequest),
)]
impl Command for ListSessionTraceRequest {
    type Output = TraceRequestPage;

    async fn execute(self, ctx: &Ctx) -> Result<TraceRequestPage, CommandError> {
        let session = session_uuid(ctx, &self.session_id).await?;
        let db = ctx.db.database();
        let row = db
            .trace_steps_by_key(session, &[(self.turn, self.step)])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| CommandError::not_found("Step"))?;
        if !matches!(row.kind.as_str(), "model" | "answer") {
            return Err(CommandError::bad_request("Step is not a model call"));
        }
        let (_, data) = db
            .trace_event_data(session, row.start_sequence)
            .await?
            .ok_or_else(|| CommandError::not_found("Model call event"))?;
        let all = messages(&data);
        let new_from = db
            .trace_previous_message_count(session, row.turn_no, row.step_no)
            .await?
            .unwrap_or(0);
        let offset = usize::try_from(self.offset.unwrap_or(0).max(0)).unwrap_or(0);
        let limit = usize::try_from(self.limit.unwrap_or(100).clamp(1, 200)).unwrap_or(100);
        let role = self.role.filter(|r| !r.is_empty());
        Ok(TraceRequestPage {
            message_count: i32::try_from(all.len()).unwrap_or(i32::MAX),
            new_from: if usize::try_from(new_from).unwrap_or(0) > all.len() {
                0
            } else {
                new_from
            },
            model: row.model,
            messages: all
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    role.as_deref()
                        .is_none_or(|r| m.get("role").and_then(Value::as_str) == Some(r))
                })
                .skip(offset)
                .take(limit)
                .map(|(i, m)| message_out(i, m, true))
                .collect(),
        })
    }
}

// ============================================================================
// ListSessionTraceTurnEvents
// ============================================================================

#[derive(Debug, Deserialize, ToSchema, IntoParams, Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListSessionTraceTurnEvents {
    /// Session's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub session_id: String,
    /// Turn number (a path parameter).
    #[param(ignore)]
    pub turn: i32,
    /// Only events after this sequence.
    #[serde(default)]
    pub after_sequence: Option<i32>,
    /// Events per page, 1 to 500. Defaults to 500.
    #[serde(default)]
    pub limit: Option<i32>,
}

#[command(
    name = "list_session_trace_turn_events",
    category = "sessions",
    description = "List the raw events of one turn of a session's trace, without deltas; small payloads are inlined.",
    method = "GET",
    path = "/v1/sessions/{session_id}/trace/turns/{turn}/events",
    policy = SESSION_VIEW,
    cli = CliRoute::new(&["sessions", "trace", "turns", "events"], "list").with_examples(&[CliExample::new("Read the raw events of one turn", "everruns sessions trace turns events list --session-id session_01h9 --turn 12")]),
    positional = "session_id",
    http = plain,
    params(ListSessionTraceTurnEvents),
)]
impl Command for ListSessionTraceTurnEvents {
    type Output = TraceEventsPage;

    async fn execute(self, ctx: &Ctx) -> Result<TraceEventsPage, CommandError> {
        let session = session_uuid(ctx, &self.session_id).await?;
        let turn = turn_row(ctx, session, self.turn).await?;
        let limit = i64::from(self.limit.unwrap_or(500).clamp(1, 500));
        let from = self
            .after_sequence
            .map_or(turn.start_sequence, |s| (s + 1).max(turn.start_sequence));
        let to = turn.end_sequence.unwrap_or(i32::MAX);
        let mut rows = ctx
            .db
            .database()
            .trace_events_in_range(
                session,
                from,
                to,
                INLINE_EVENT_BYTES,
                limit.min(MAX_EVENTS) + 1,
            )
            .await?;
        let next_after_sequence = if rows.len() as i64 > limit {
            rows.truncate(limit as usize);
            rows.last().map(|e| e.sequence)
        } else {
            None
        };
        Ok(TraceEventsPage {
            events: rows.into_iter().map(event_ref).collect(),
            next_after_sequence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn small_payloads_inline_and_large_ones_are_cut() {
        let small = payload(json!({"a": 1}), false);
        assert!(small.value.is_some() && !small.truncated);
        let big = payload(json!({"a": "é".repeat(INLINE_PAYLOAD_BYTES)}), false);
        assert!(big.value.is_none() && big.truncated);
        assert!(big.preview.unwrap().len() <= INLINE_PAYLOAD_BYTES);
        assert!(
            payload(json!({"a": "x".repeat(INLINE_PAYLOAD_BYTES)}), true)
                .value
                .is_some()
        );
    }

    #[test]
    fn request_summary_lists_only_new_messages() {
        let data = json!({
            "messages": [
                {"role": "system", "content": [{"type": "text", "text": "You are helpful."}]},
                {"role": "user", "content": [{"type": "text", "text": "hi"}]},
                {"role": "assistant", "content": [], "tool_calls": [{"id": "c", "name": "web_fetch"}]},
                {"role": "tool", "content": [{"type": "text", "text": "200 OK"}]}
            ],
            "tools": [{"name": "web_fetch"}]
        });
        let summary = request_summary(&data, 2);
        assert_eq!(summary.message_count, 4);
        assert_eq!(summary.tool_count, 1);
        assert_eq!(summary.system_preview.as_deref(), Some("You are helpful."));
        let roles: Vec<_> = summary
            .new_messages
            .iter()
            .map(|m| m.role.as_str())
            .collect();
        assert_eq!(roles, ["assistant", "tool"]);
        assert_eq!(summary.new_messages[0].preview, "[web_fetch]");
        // A shrunk (compacted) history makes everything new.
        assert_eq!(request_summary(&data, 9).new_from, 0);
    }
}
