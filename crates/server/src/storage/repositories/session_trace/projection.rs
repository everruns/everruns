// Session trace projection: session events in, trace turn and step rows out.
//
// Decision: a pure fold with no I/O, so incremental projection (a few events
// at a time) and a full rebuild run the same code and can be compared in tests.
// The caller loads the working set (the turns the events refer to and their
// steps that are still running), applies events in sequence order, and writes
// back the rows marked dirty.
//
// Decision: steps come from small events only. `llm.generation` is read through
// a slim projection the repository builds in SQL (output text, tool call ids,
// metadata, message count), never its stored prompt; `tool.completed` arrives
// with a result preview instead of the full result.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::rows::{TraceStepRow, TraceTurnRow};

/// Event types the projection reads. Everything else leaves the trace as it is.
pub const TRACE_EVENT_TYPES: &[&str] = &[
    "turn.started",
    "turn.completed",
    "turn.failed",
    "turn.cancelled",
    "turn.sealed",
    "llm.generation",
    "tool.started",
    "tool.completed",
];

/// Longest prompt preview kept on a turn row.
pub const PROMPT_PREVIEW_CHARS: usize = 500;
/// Longest narration kept on a model step row.
pub const NARRATION_CHARS: usize = 2000;
/// Longest target or result summary kept on a step row.
pub const SUMMARY_CHARS: usize = 300;

pub mod kind {
    pub const MODEL: &str = "model";
    pub const ANSWER: &str = "answer";
    pub const TOOL: &str = "tool";
    pub const APPROVAL: &str = "approval";
    pub const AGENT: &str = "agent";
    pub const SEND: &str = "send";
}

pub mod status {
    pub const RUNNING: &str = "running";
    pub const SUCCESS: &str = "success";
    pub const ERROR: &str = "error";
    pub const CANCELLED: &str = "cancelled";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const SEALED: &str = "sealed";
}

/// One event as the projection sees it: envelope fields plus a slim payload.
#[derive(Debug, Clone)]
pub struct TraceEvent {
    pub sequence: i32,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    /// `context.turn_id`, when the event is turn-scoped.
    pub turn_id: Option<Uuid>,
    pub data: Value,
}

/// The rows a batch of events can touch, plus what changed.
#[derive(Debug, Default)]
pub struct TraceWorkingSet {
    pub turn_count: i32,
    turns: HashMap<Uuid, TraceTurnRow>,
    /// Steps by `(turn_no, step_no)`.
    steps: HashMap<(i32, i32), TraceStepRow>,
    dirty_turns: Vec<Uuid>,
    dirty_steps: Vec<(i32, i32)>,
}

impl TraceWorkingSet {
    /// A working set loaded from storage: the turns the next events refer to
    /// and their running steps.
    pub fn new(turn_count: i32, turns: Vec<TraceTurnRow>, steps: Vec<TraceStepRow>) -> Self {
        Self {
            turn_count,
            turns: turns.into_iter().map(|t| (t.turn_id, t)).collect(),
            steps: steps
                .into_iter()
                .map(|s| ((s.turn_no, s.step_no), s))
                .collect(),
            dirty_turns: Vec::new(),
            dirty_steps: Vec::new(),
        }
    }

    /// Rows changed since the working set was built, in first-change order.
    pub fn into_changes(self) -> (i32, Vec<TraceTurnRow>, Vec<TraceStepRow>) {
        let Self {
            turn_count,
            mut turns,
            mut steps,
            dirty_turns,
            dirty_steps,
        } = self;
        let turns = dirty_turns
            .into_iter()
            .filter_map(|id| turns.remove(&id))
            .collect();
        let steps = dirty_steps
            .into_iter()
            .filter_map(|key| steps.remove(&key))
            .collect();
        (turn_count, turns, steps)
    }

    fn touch_turn(&mut self, turn_id: Uuid) {
        if !self.dirty_turns.contains(&turn_id) {
            self.dirty_turns.push(turn_id);
        }
    }

    fn touch_step(&mut self, key: (i32, i32)) {
        if !self.dirty_steps.contains(&key) {
            self.dirty_steps.push(key);
        }
    }

    /// Apply one event. Events must arrive in sequence order, each exactly once.
    pub fn apply(&mut self, event: &TraceEvent) {
        match event.event_type.as_str() {
            "turn.started" => self.turn_started(event),
            "turn.completed" => self.turn_ended(event, status::COMPLETED, None),
            "turn.sealed" => self.turn_ended(event, status::SEALED, None),
            "turn.cancelled" => self.turn_ended(event, status::CANCELLED, None),
            "turn.failed" => {
                let error = str_field(&event.data, "error").map(|e| truncate(e, SUMMARY_CHARS));
                self.turn_ended(event, status::FAILED, error);
            }
            "llm.generation" => self.model_call(event),
            "tool.started" => self.tool_started(event),
            "tool.completed" => self.tool_completed(event),
            _ => {}
        }
    }

    fn event_turn_id(event: &TraceEvent) -> Option<Uuid> {
        event
            .turn_id
            .or_else(|| str_field(&event.data, "turn_id").and_then(parse_uuid))
    }

    fn turn_started(&mut self, event: &TraceEvent) {
        let Some(turn_id) = Self::event_turn_id(event) else {
            return;
        };
        if self.turns.contains_key(&turn_id) {
            return;
        }
        self.turn_count += 1;
        let turn = TraceTurnRow {
            turn_no: self.turn_count,
            turn_id,
            start_sequence: event.sequence,
            end_sequence: None,
            started_at: event.ts,
            ended_at: None,
            status: status::RUNNING.to_string(),
            prompt_preview: str_field(&event.data, "input_content")
                .map(|p| truncate(p, PROMPT_PREVIEW_CHARS)),
            error: None,
            step_count: 0,
            model_calls: 0,
            tool_calls: 0,
            subagent_calls: 0,
            error_count: 0,
            input_tokens: 0,
            output_tokens: 0,
        };
        self.turns.insert(turn_id, turn);
        self.touch_turn(turn_id);
    }

    fn turn_ended(&mut self, event: &TraceEvent, end_status: &str, error: Option<String>) {
        let Some(turn_id) = Self::event_turn_id(event) else {
            return;
        };
        let Some(turn) = self.turns.get_mut(&turn_id) else {
            return;
        };
        turn.status = end_status.to_string();
        turn.end_sequence = Some(event.sequence);
        turn.ended_at = Some(event.ts);
        if let Some(error) = error {
            turn.error = Some(error);
            turn.error_count += 1;
        }
        // The turn's own usage is authoritative once it reports one.
        if let Some(usage) = event.data.get("usage") {
            if let Some(input) = usage.get("input_tokens").and_then(Value::as_i64) {
                turn.input_tokens = input;
            }
            if let Some(output) = usage.get("output_tokens").and_then(Value::as_i64) {
                turn.output_tokens = output;
            }
        }
        let turn_no = turn.turn_no;
        self.touch_turn(turn_id);
        // A step still running when its turn ends never finished (a crash or a
        // cancellation); it must not look live forever.
        let open: Vec<(i32, i32)> = self
            .steps
            .iter()
            .filter(|(key, step)| key.0 == turn_no && step.status == status::RUNNING)
            .map(|(key, _)| *key)
            .collect();
        for key in open {
            if let Some(step) = self.steps.get_mut(&key) {
                step.status = status::CANCELLED.to_string();
                step.end_sequence = Some(event.sequence);
                step.duration_ms = Some(millis_between(step.started_at, event.ts));
            }
            self.touch_step(key);
        }
    }

    /// Open a new step at the end of `turn_id`'s step list.
    fn push_step(&mut self, turn_id: Uuid, build: impl FnOnce(i32, i32) -> TraceStepRow) {
        let Some(turn) = self.turns.get_mut(&turn_id) else {
            return;
        };
        turn.step_count += 1;
        let key = (turn.turn_no, turn.step_count);
        let step = build(key.0, key.1);
        match step.kind.as_str() {
            kind::MODEL | kind::ANSWER => turn.model_calls += 1,
            kind::AGENT => turn.subagent_calls += 1,
            _ => turn.tool_calls += 1,
        }
        if step.status == status::ERROR {
            turn.error_count += 1;
        }
        turn.input_tokens += i64::from(step.input_tokens.unwrap_or(0));
        turn.output_tokens += i64::from(step.output_tokens.unwrap_or(0));
        self.steps.insert(key, step);
        self.touch_turn(turn_id);
        self.touch_step(key);
    }

    fn model_call(&mut self, event: &TraceEvent) {
        let Some(turn_id) = Self::event_turn_id(event) else {
            return;
        };
        let data = &event.data;
        let metadata = data.get("metadata").cloned().unwrap_or(Value::Null);
        let output = data.get("output").cloned().unwrap_or(Value::Null);
        let requested: Vec<String> = output
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|c| str_field(c, "id").map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let narration = str_field(&output, "text")
            .filter(|t| !t.trim().is_empty())
            .map(|t| truncate(t, NARRATION_CHARS));
        let success = metadata
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let duration_ms = metadata.get("duration_ms").and_then(Value::as_i64);
        let usage = metadata.get("usage");
        let tokens = |field: &str| {
            usage
                .and_then(|u| u.get(field))
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
        };
        // A call that asked for no tools and said something is the turn's answer.
        let step_kind = if requested.is_empty() && narration.is_some() {
            kind::ANSWER
        } else {
            kind::MODEL
        };
        let step = TraceStepRow {
            turn_no: 0,
            step_no: 0,
            kind: step_kind.to_string(),
            status: if success {
                status::SUCCESS
            } else {
                status::ERROR
            }
            .to_string(),
            start_sequence: event.sequence,
            end_sequence: Some(event.sequence),
            // `llm.generation` is emitted when the call ends.
            started_at: event.ts - Duration::milliseconds(duration_ms.unwrap_or(0)),
            duration_ms,
            name: str_field(&metadata, "model").map(str::to_string),
            target: None,
            result: str_field(&metadata, "error").map(|e| truncate(e, SUMMARY_CHARS)),
            narration,
            tool_call_id: None,
            model: str_field(&metadata, "model").map(str::to_string),
            input_tokens: tokens("input_tokens"),
            output_tokens: tokens("output_tokens"),
            message_count: data
                .get("message_count")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok()),
            requested_tool_call_ids: requested,
        };
        self.push_step(turn_id, |turn_no, step_no| TraceStepRow {
            turn_no,
            step_no,
            ..step
        });
    }

    fn tool_started(&mut self, event: &TraceEvent) {
        let Some(turn_id) = Self::event_turn_id(event) else {
            return;
        };
        let call = event.data.get("tool_call").cloned().unwrap_or(Value::Null);
        let Some(call_id) = str_field(&call, "id").map(str::to_string) else {
            return;
        };
        let name = str_field(&call, "name").unwrap_or_default().to_string();
        let target = str_field(&event.data, "narration")
            .or_else(|| str_field(&event.data, "display_name"))
            .or_else(|| str_field(&call, "arguments_preview"))
            .map(|t| truncate(t, SUMMARY_CHARS));
        let step = TraceStepRow {
            turn_no: 0,
            step_no: 0,
            kind: tool_kind(&name).to_string(),
            status: status::RUNNING.to_string(),
            start_sequence: event.sequence,
            end_sequence: None,
            started_at: event.ts,
            duration_ms: None,
            name: Some(name),
            target,
            result: None,
            narration: None,
            tool_call_id: Some(call_id),
            model: None,
            input_tokens: None,
            output_tokens: None,
            message_count: None,
            requested_tool_call_ids: Vec::new(),
        };
        self.push_step(turn_id, |turn_no, step_no| TraceStepRow {
            turn_no,
            step_no,
            ..step
        });
    }

    fn tool_completed(&mut self, event: &TraceEvent) {
        let Some(turn_id) = Self::event_turn_id(event) else {
            return;
        };
        let Some(turn_no) = self.turns.get(&turn_id).map(|t| t.turn_no) else {
            return;
        };
        let data = &event.data;
        let Some(call_id) = str_field(data, "tool_call_id").map(str::to_string) else {
            return;
        };
        let success = data
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let duration_ms = data.get("duration_ms").and_then(Value::as_i64);
        let result = if success {
            str_field(data, "result_preview")
        } else {
            str_field(data, "error").or_else(|| str_field(data, "result_preview"))
        }
        .filter(|r| !r.is_empty())
        .map(|r| truncate(r, SUMMARY_CHARS));
        let narration = str_field(data, "narration").map(|t| truncate(t, SUMMARY_CHARS));
        let new_status = if success {
            status::SUCCESS
        } else {
            status::ERROR
        };

        let open = self
            .steps
            .iter()
            .find(|(key, step)| {
                key.0 == turn_no
                    && step.status == status::RUNNING
                    && step.tool_call_id.as_deref() == Some(call_id.as_str())
            })
            .map(|(key, _)| *key);
        match open {
            Some(key) => {
                if let Some(step) = self.steps.get_mut(&key) {
                    step.status = new_status.to_string();
                    step.end_sequence = Some(event.sequence);
                    step.duration_ms =
                        duration_ms.or_else(|| Some(millis_between(step.started_at, event.ts)));
                    step.result = result;
                    if step.target.is_none() {
                        step.target = narration;
                    }
                }
                self.touch_step(key);
                if !success && let Some(turn) = self.turns.get_mut(&turn_id) {
                    turn.error_count += 1;
                    self.touch_turn(turn_id);
                }
            }
            None => {
                // A call with no `tool.started` (client-side tools, older
                // sessions): the completion alone becomes the step.
                let name = str_field(data, "tool_name").unwrap_or_default().to_string();
                let step = TraceStepRow {
                    turn_no: 0,
                    step_no: 0,
                    kind: tool_kind(&name).to_string(),
                    status: new_status.to_string(),
                    start_sequence: event.sequence,
                    end_sequence: Some(event.sequence),
                    started_at: event.ts - Duration::milliseconds(duration_ms.unwrap_or(0)),
                    duration_ms,
                    name: Some(name),
                    target: narration,
                    result,
                    narration: None,
                    tool_call_id: Some(call_id),
                    model: None,
                    input_tokens: None,
                    output_tokens: None,
                    message_count: None,
                    requested_tool_call_ids: Vec::new(),
                };
                self.push_step(turn_id, |turn_no, step_no| TraceStepRow {
                    turn_no,
                    step_no,
                    ..step
                });
            }
        }
    }
}

/// Step kind for a tool by name. Approvals, sub-agents and agent messages get
/// their own rows in the Trace view.
pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "request_approval" => kind::APPROVAL,
        "spawn_agent" => kind::AGENT,
        "send_message" => kind::SEND,
        _ => kind::TOOL,
    }
}

fn str_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

/// Parse a bare or prefixed (`turn_<hex>`) identifier.
pub fn parse_uuid(raw: &str) -> Option<Uuid> {
    let hex = raw.rsplit_once('_').map_or(raw, |(_, hex)| hex);
    Uuid::parse_str(hex).ok()
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

fn millis_between(from: DateTime<Utc>, to: DateTime<Utc>) -> i64 {
    (to - from).num_milliseconds().max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(ms: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(1_790_000_000_000 + ms).unwrap()
    }

    fn ev(sequence: i32, event_type: &str, ms: i64, turn: Uuid, data: Value) -> TraceEvent {
        TraceEvent {
            sequence,
            event_type: event_type.to_string(),
            ts: at(ms),
            turn_id: Some(turn),
            data,
        }
    }

    /// A turn: a model call asking for two tools (one fails), then the answer.
    fn sample_turn(turn: Uuid, base_seq: i32) -> Vec<TraceEvent> {
        vec![
            ev(
                base_seq,
                "turn.started",
                0,
                turn,
                json!({"turn_id": format!("turn_{}", turn.simple()), "input_content": "Check the links"}),
            ),
            ev(
                base_seq + 1,
                "llm.generation",
                4_000,
                turn,
                json!({
                    "output": {"text": "Checking both pages.", "tool_calls": [{"id": "c1", "name": "web_fetch"}, {"id": "c2", "name": "web_fetch"}]},
                    "metadata": {"model": "gpt-6.1-sol", "success": true, "duration_ms": 4000, "usage": {"input_tokens": 600, "output_tokens": 40}},
                    "message_count": 3
                }),
            ),
            ev(
                base_seq + 2,
                "tool.started",
                4_100,
                turn,
                json!({"tool_call": {"id": "c1", "name": "web_fetch", "arguments_preview": "{\"url\":\"a\"}"}, "narration": "Fetch a"}),
            ),
            ev(
                base_seq + 3,
                "tool.started",
                4_100,
                turn,
                json!({"tool_call": {"id": "c2", "name": "web_fetch"}, "narration": "Fetch b"}),
            ),
            ev(
                base_seq + 4,
                "tool.completed",
                4_600,
                turn,
                json!({"tool_call_id": "c1", "tool_name": "web_fetch", "success": true, "status": "success", "duration_ms": 500, "result_preview": "200 OK"}),
            ),
            ev(
                base_seq + 5,
                "tool.completed",
                4_900,
                turn,
                json!({"tool_call_id": "c2", "tool_name": "web_fetch", "success": false, "status": "error", "duration_ms": 800, "error": "404 Not Found"}),
            ),
            ev(
                base_seq + 6,
                "llm.generation",
                8_000,
                turn,
                json!({
                    "output": {"text": "One link is broken."},
                    "metadata": {"model": "gpt-6.1-sol", "success": true, "duration_ms": 3000, "usage": {"input_tokens": 700, "output_tokens": 20}},
                    "message_count": 6
                }),
            ),
            ev(
                base_seq + 7,
                "turn.completed",
                8_100,
                turn,
                json!({"usage": {"input_tokens": 1300, "output_tokens": 60}}),
            ),
        ]
    }

    fn project(events: &[TraceEvent]) -> (i32, Vec<TraceTurnRow>, Vec<TraceStepRow>) {
        let mut ws = TraceWorkingSet::default();
        for event in events {
            ws.apply(event);
        }
        ws.into_changes()
    }

    #[test]
    fn a_turn_becomes_one_turn_row_and_a_step_per_call() {
        let turn = Uuid::now_v7();
        let (count, turns, steps) = project(&sample_turn(turn, 1));
        assert_eq!(count, 1);
        let t = &turns[0];
        assert_eq!((t.turn_no, t.status.as_str()), (1, status::COMPLETED));
        assert_eq!(t.prompt_preview.as_deref(), Some("Check the links"));
        assert_eq!(
            (t.step_count, t.model_calls, t.tool_calls, t.error_count),
            (4, 2, 2, 1)
        );
        assert_eq!((t.input_tokens, t.output_tokens), (1300, 60));
        assert_eq!((t.start_sequence, t.end_sequence), (1, Some(8)));

        let kinds: Vec<_> = steps.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [kind::MODEL, kind::TOOL, kind::TOOL, kind::ANSWER],
            "steps in start order"
        );
        let model = &steps[0];
        assert_eq!(model.narration.as_deref(), Some("Checking both pages."));
        assert_eq!(model.requested_tool_call_ids, ["c1", "c2"]);
        assert_eq!(model.started_at, at(0), "start is end minus duration");
        assert_eq!(model.message_count, Some(3));
        let ok = &steps[1];
        assert_eq!(
            (
                ok.status.as_str(),
                ok.target.as_deref(),
                ok.result.as_deref()
            ),
            (status::SUCCESS, Some("Fetch a"), Some("200 OK"))
        );
        assert_eq!(ok.duration_ms, Some(500));
        let failed = &steps[2];
        assert_eq!(
            (failed.status.as_str(), failed.result.as_deref()),
            (status::ERROR, Some("404 Not Found"))
        );
    }

    #[test]
    fn incremental_projection_matches_a_full_rebuild() {
        let first = Uuid::now_v7();
        let second = Uuid::now_v7();
        let mut events = sample_turn(first, 1);
        events.extend(sample_turn(second, 9));
        let (full_count, full_turns, full_steps) = project(&events);

        // Replay in small batches, persisting changes between batches and
        // reloading only what the next batch can touch, as the repository does.
        let mut stored_turns: HashMap<i32, TraceTurnRow> = HashMap::new();
        let mut stored_steps: HashMap<(i32, i32), TraceStepRow> = HashMap::new();
        let mut count = 0;
        for batch in events.chunks(3) {
            let ids: Vec<Uuid> = batch.iter().filter_map(|e| e.turn_id).collect();
            let turns: Vec<_> = stored_turns
                .values()
                .filter(|t| ids.contains(&t.turn_id))
                .cloned()
                .collect();
            let nos: Vec<i32> = turns.iter().map(|t| t.turn_no).collect();
            let steps: Vec<_> = stored_steps
                .values()
                .filter(|s| nos.contains(&s.turn_no) && s.status == status::RUNNING)
                .cloned()
                .collect();
            let mut ws = TraceWorkingSet::new(count, turns, steps);
            for event in batch {
                ws.apply(event);
            }
            let (c, turns, steps) = ws.into_changes();
            count = c;
            for t in turns {
                stored_turns.insert(t.turn_no, t);
            }
            for s in steps {
                stored_steps.insert((s.turn_no, s.step_no), s);
            }
        }

        assert_eq!(count, full_count);
        let mut turns: Vec<_> = stored_turns.into_values().collect();
        turns.sort_by_key(|t| t.turn_no);
        assert_eq!(turns, full_turns);
        let mut steps: Vec<_> = stored_steps.into_values().collect();
        steps.sort_by_key(|s| (s.turn_no, s.step_no));
        let mut full_steps = full_steps;
        full_steps.sort_by_key(|s| (s.turn_no, s.step_no));
        assert_eq!(steps, full_steps);
    }

    #[test]
    fn a_failed_turn_cancels_its_running_steps_and_counts_the_error() {
        let turn = Uuid::now_v7();
        let mut events = sample_turn(turn, 1);
        events.truncate(4); // started, model call, two tools started
        events.push(ev(
            5,
            "turn.failed",
            6_000,
            turn,
            json!({"error": "worker lost"}),
        ));
        let (_, turns, steps) = project(&events);
        assert_eq!(turns[0].status, status::FAILED);
        assert_eq!(turns[0].error.as_deref(), Some("worker lost"));
        assert_eq!(turns[0].error_count, 1);
        let tools: Vec<_> = steps.iter().filter(|s| s.kind == kind::TOOL).collect();
        assert!(tools.iter().all(|s| s.status == status::CANCELLED));
        assert!(tools.iter().all(|s| s.duration_ms == Some(1_900)));
    }

    #[test]
    fn a_completion_without_a_start_still_becomes_a_step() {
        let turn = Uuid::now_v7();
        let events = vec![
            ev(1, "turn.started", 0, turn, json!({})),
            ev(
                2,
                "tool.completed",
                900,
                turn,
                json!({"tool_call_id": "x", "tool_name": "request_approval", "success": true, "duration_ms": 300}),
            ),
        ];
        let (_, turns, steps) = project(&events);
        assert_eq!(turns[0].tool_calls, 1);
        assert_eq!(steps[0].kind, kind::APPROVAL);
        assert_eq!(steps[0].started_at, at(600));
    }

    #[test]
    fn events_of_unknown_turns_and_duplicate_starts_are_ignored() {
        let turn = Uuid::now_v7();
        let stranger = Uuid::now_v7();
        let events = vec![
            ev(1, "turn.started", 0, turn, json!({})),
            ev(2, "turn.started", 1, turn, json!({})),
            ev(
                3,
                "tool.started",
                2,
                stranger,
                json!({"tool_call": {"id": "a", "name": "x"}}),
            ),
        ];
        let (count, turns, steps) = project(&events);
        assert_eq!(count, 1);
        assert_eq!(turns.len(), 1);
        assert!(steps.is_empty());
    }

    #[test]
    fn previews_are_bounded() {
        let turn = Uuid::now_v7();
        let long = "é".repeat(PROMPT_PREVIEW_CHARS + 50);
        let (_, turns, _) = project(&[ev(
            1,
            "turn.started",
            0,
            turn,
            json!({"input_content": long}),
        )]);
        let preview = turns[0].prompt_preview.as_deref().unwrap();
        assert_eq!(preview.chars().count(), PROMPT_PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn prefixed_and_bare_ids_parse() {
        let id = Uuid::now_v7();
        assert_eq!(parse_uuid(&format!("turn_{}", id.simple())), Some(id));
        assert_eq!(parse_uuid(&id.to_string()), Some(id));
        assert_eq!(parse_uuid("nope"), None);
    }
}
