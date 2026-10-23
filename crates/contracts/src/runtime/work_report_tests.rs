use super::*;
use crate::runtime::typed_id::{EventId, MessageId, SessionId, TurnId};
use chrono::{Duration, TimeZone};
use serde_json::{Value, json};

struct Log {
    session: String,
    turn: String,
    start: DateTime<Utc>,
    events: Vec<Event>,
}

impl Log {
    fn new() -> Self {
        let mut log = Self {
            session: SessionId::new().to_string(),
            turn: TurnId::new().to_string(),
            start: Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap(),
            events: Vec::new(),
        };
        let turn = log.turn.clone();
        log.push(
            0,
            "turn.started",
            json!({
                "turn_id": turn,
                "input_message_id": MessageId::new().to_string(),
                "agent_name": "Coding Agent",
            }),
        );
        log
    }

    fn push(&mut self, at_ms: i64, event_type: &str, data: Value) -> &mut Self {
        self.push_in_turn(&self.turn.clone(), at_ms, event_type, data)
    }

    fn push_in_turn(&mut self, turn: &str, at_ms: i64, event_type: &str, data: Value) -> &mut Self {
        let event = json!({
            "id": EventId::new().to_string(),
            "type": event_type,
            "ts": (self.start + Duration::milliseconds(at_ms)).to_rfc3339(),
            "session_id": self.session,
            "context": { "turn_id": turn },
            "data": data,
        });
        self.events
            .push(serde_json::from_value(event).expect("valid event"));
        self
    }

    fn started(&mut self, at_ms: i64, id: &str, name: &str, narration: &str) -> &mut Self {
        self.started_with(at_ms, id, name, narration, json!({}))
    }

    fn started_with(
        &mut self,
        at_ms: i64,
        id: &str,
        name: &str,
        narration: &str,
        arguments: Value,
    ) -> &mut Self {
        self.push(
            at_ms,
            "tool.started",
            json!({
                "tool_call": { "id": id, "name": name, "arguments": arguments },
                "narration": narration,
            }),
        )
    }

    fn completed(&mut self, at_ms: i64, id: &str, name: &str, narration: &str) -> &mut Self {
        self.push(
            at_ms,
            "tool.completed",
            json!({
                "tool_call_id": id, "tool_name": name, "success": true, "status": "success",
                "result": [], "duration_ms": 1500, "narration": narration,
            }),
        )
    }

    fn failed(&mut self, at_ms: i64, id: &str, name: &str, narration: &str) -> &mut Self {
        self.push(
            at_ms,
            "tool.completed",
            json!({
                "tool_call_id": id, "tool_name": name, "success": false, "status": "error",
                "severity": "issue", "error": "boom", "duration_ms": 400, "narration": narration,
            }),
        )
    }

    fn report_at(&self, at_ms: i64) -> WorkReport {
        WorkReport::from_events(&self.events, self.start + Duration::milliseconds(at_ms))
            .expect("report")
    }
}

#[test]
fn no_turn_started_means_no_report() {
    assert!(WorkReport::from_events(&[], Utc::now()).is_none());
}

#[test]
fn working_without_a_plan_names_the_running_step_and_counts_no_progress() {
    let mut log = Log::new();
    log.push(
        100,
        "reason.started",
        json!({ "harness_id": crate::runtime::typed_id::HarnessId::new().to_string(),
                "metadata": { "model": "gpt-6.1-sol" } }),
    )
    .started(200, "c1", "read_file", "Read 12 files")
    .completed(2100, "c1", "read_file", "Read 12 files")
    .started(2200, "c2", "grep", "Search for prorateInvoice")
    .push(
        2600,
        "tool.progress",
        json!({ "tool_call_id": "c2", "tool_name": "grep", "message": "Scanning src/" }),
    );

    let report = log.report_at(4_000);
    assert_eq!(report.state, WorkState::Working);
    assert_eq!(report.headline, "Search for prorateInvoice");
    assert_eq!(report.agent_name.as_deref(), Some("Coding Agent"));
    assert_eq!(report.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(report.duration_ms, Some(4_000));
    assert!(report.plan.is_none(), "no plan, no step N of M");
    assert!(report.result.is_none());
    assert_eq!(report.steps.len(), 2);
    assert_eq!(report.steps[0].state, WorkStepState::Done);
    assert_eq!(report.steps[0].duration_ms, Some(1500));
    assert_eq!(report.steps[1].state, WorkStepState::Running);
    assert_eq!(
        report.steps[1]
            .progress
            .as_ref()
            .and_then(|p| p.label.as_deref()),
        Some("Scanning src/")
    );
}

#[test]
fn a_counted_progress_update_draws_a_bar_and_keeps_it_through_plain_updates() {
    let mut log = Log::new();
    log.started(100, "c1", "generate_image", "Generating image")
        .push(
            300,
            "tool.progress",
            json!({ "tool_call_id": "c1", "tool_name": "generate_image",
                    "message": "Preview 2 of 3", "current": 2, "total": 3 }),
        );
    let report = log.report_at(500);
    assert_eq!(report.headline, "Generating image 2 of 3");
    let progress = report.steps[0].progress.clone().unwrap_or_default();
    assert_eq!((progress.current, progress.total), (Some(2), Some(3)));

    log.push(
        400,
        "tool.progress",
        json!({ "tool_call_id": "c1", "tool_name": "generate_image", "message": "Finishing" }),
    );
    let progress = log.report_at(500).steps[0]
        .progress
        .clone()
        .unwrap_or_default();
    assert_eq!(progress.label.as_deref(), Some("Finishing"));
    assert_eq!((progress.current, progress.total), (Some(2), Some(3)));

    log.push(
        450,
        "tool.progress",
        json!({ "tool_call_id": "c1", "tool_name": "generate_image",
                "message": "Odd", "current": 9, "total": 3 }),
    );
    let progress = log.report_at(500).steps[0]
        .progress
        .clone()
        .unwrap_or_default();
    assert_eq!(progress.current, Some(3), "current never passes total");
}

#[test]
fn a_plan_tool_call_becomes_the_plan_and_not_a_step() {
    let mut log = Log::new();
    let todos = json!({ "todos": [
        { "content": "Find the bug", "activeForm": "Finding the bug", "status": "completed" },
        { "content": "Fix the rounding", "activeForm": "Fixing the rounding", "status": "in_progress" },
        { "content": "Run the tests", "activeForm": "Running the tests", "status": "pending" },
    ]});
    log.started_with(100, "p1", "write_todos", "Update todos", todos)
        .completed(150, "p1", "write_todos", "Updated todos");

    let report = log.report_at(1_000);
    assert!(report.steps.is_empty());
    let plan = report.plan.expect("plan");
    assert_eq!((plan.done, plan.total), (1, 3));
    assert_eq!(plan.current.as_deref(), Some("Fixing the rounding"));
    assert_eq!(plan.items[1].status, ProgressStepStatus::InProgress);
    // With nothing running, the in-progress plan item leads.
    assert_eq!(report.headline, "Fixing the rounding");
}

#[test]
fn a_failed_plan_update_keeps_the_previous_plan() {
    let mut log = Log::new();
    let first = json!({ "steps": [
        { "title": "Plan", "status": "done" }, { "title": "Ship", "status": "in_progress" } ]});
    let second = json!({ "steps": [ { "title": "Something else", "status": "pending" } ]});
    log.started_with(100, "p1", "update_checklist", "Update checklist", first)
        .completed(110, "p1", "update_checklist", "Updated checklist")
        .started_with(200, "p2", "update_checklist", "Update checklist", second)
        .failed(210, "p2", "update_checklist", "Could not update checklist");

    let plan = log.report_at(300).plan.expect("plan");
    assert_eq!(plan.items[0].title, "Plan");
    assert_eq!((plan.done, plan.total), (1, 2));
}

#[test]
fn an_approval_request_parks_the_work_and_hides_the_gated_call() {
    let mut log = Log::new();
    let gated_result = json!({ "code": "tool_approval_required", "tool": "bash" }).to_string();
    log.started(100, "c1", "bash", "Run git push")
        .push(
            200,
            "tool.completed",
            json!({
                "tool_call_id": "c1", "tool_name": "bash", "success": false, "status": "error",
                "severity": "issue", "error": "Waiting for approval",
                "result": [{ "type": "text", "text": gated_result }],
            }),
        )
        .push(
            250,
            "tool.call_requested",
            json!({ "tool_calls": [{ "id": "tool_approval_c1", "name": "approve_tool_call",
                                     "arguments": { "tool": "bash" } }],
                    "tool_summaries": [] }),
        );

    let waiting = log.report_at(1_000);
    assert_eq!(waiting.state, WorkState::NeedsYou);
    assert_eq!(waiting.headline, "Needs your approval · Run git push");
    let attention = waiting.attention.expect("attention");
    assert_eq!(attention.kind, WorkAttentionKind::Approval);
    assert_eq!(
        attention.tool_call_ids,
        vec!["tool_approval_c1".to_string()]
    );
    assert!(waiting.steps.is_empty(), "the gated call did not run");

    // Approved: the request closes and the model runs the call again.
    log.completed(5_000, "tool_approval_c1", "approve_tool_call", "Approved")
        .started(5_100, "c2", "bash", "Run git push")
        .completed(6_500, "c2", "bash", "Ran git push");
    let resumed = log.report_at(7_000);
    assert_eq!(resumed.state, WorkState::Working);
    assert!(resumed.attention.is_none());
    assert_eq!(resumed.steps.len(), 1);
    assert_eq!(resumed.steps[0].label, "Ran git push");
}

#[test]
fn a_question_waits_for_an_answer_and_becomes_a_step_once_answered() {
    let mut log = Log::new();
    log.push(
        100,
        "tool.call_requested",
        json!({ "tool_calls": [{ "id": "q1", "name": "ask_user", "arguments": {} }],
                "tool_summaries": [{ "id": "q1", "name": "ask_user", "narration": "Ask: Which plan?" }] }),
    );
    let waiting = log.report_at(500);
    assert_eq!(
        waiting.headline,
        "Waiting for your answer · Ask: Which plan?"
    );

    log.completed(900, "q1", "ask_user", "Asked: Which plan?");
    let answered = log.report_at(1_000);
    assert_eq!(answered.state, WorkState::Working);
    assert_eq!(answered.steps[0].label, "Asked: Which plan?");
}

#[test]
fn done_reports_measured_facts_and_the_first_sentence_of_the_reply() {
    let mut log = Log::new();
    let turn = log.turn.clone();
    log.started(100, "c1", "edit_file", "Edit invoice.ts")
        .completed(2_000, "c1", "edit_file", "Edited invoice.ts")
        .push(2_000, "file.written", json!({ "path": "/src/invoice.ts", "operation": "update", "size_bytes": 10, "created": false }))
        .push(2_100, "file.written", json!({ "path": "/src/invoice.ts", "operation": "update", "size_bytes": 12, "created": false }))
        .push(2_200, "file.written", json!({ "path": "/tests/invoice_test.ts", "operation": "create", "size_bytes": 5, "created": true }))
        .started(2_300, "c2", "bash", "Run cargo test")
        .failed(2_500, "c2", "bash", "Could not run cargo test")
        .push(
            13_400,
            "turn.completed",
            json!({ "turn_id": turn, "iterations": 3, "issue_count": 1,
                    "final_answer_preview": "**Fixed** proration rounding. It now rounds per line." }),
        );

    let report = log.report_at(99_000);
    assert_eq!(report.state, WorkState::Done);
    assert_eq!(report.duration_ms, Some(13_400), "ended, so now is ignored");
    assert_eq!(report.headline, "Done in 13s · 2 files changed · 1 issue");
    let result = report.result.as_ref().expect("result");
    assert_eq!(result.summary.as_deref(), Some("Fixed proration rounding."));
    assert_eq!(result.summary_source, Some(WorkSummarySource::Reply));
    assert_eq!(result.artifacts.len(), 2);
    assert_eq!(result.artifacts[0].name, "invoice.ts");
    assert_eq!(report.fact(WorkFactKind::Steps), Some(2));
    assert_eq!(report.steps[1].state, WorkStepState::Issue);
}

#[test]
fn the_agents_own_report_wins_over_the_reply_and_is_not_a_step() {
    let mut log = Log::new();
    let turn = log.turn.clone();
    log.push(100, "file.written", json!({ "path": "/src/invoice.ts", "operation": "update", "size_bytes": 10, "created": false }))
        .started_with(
            200,
            "r1",
            "report_done",
            "Reporting",
            json!({
                "summary": "Fixed proration rounding: partial months round per line.",
                "validation": "Ran the 12 billing tests, all pass.",
                "artifacts": [
                    { "name": "PR #4861", "type": "pr", "url": "https://github.com/o/r/pull/4861" },
                    { "name": "invoice.ts", "type": "file", "path": "/src/invoice.ts" },
                    { "bogus": true }
                ]
            }),
        )
        .completed(300, "r1", "report_done", "Reported")
        .push(
            1_000,
            "turn.completed",
            json!({ "turn_id": turn, "iterations": 2, "final_answer_preview": "Here is what I did." }),
        );

    let report = log.report_at(5_000);
    assert!(report.steps.is_empty(), "reporting is not work");
    let result = report.result.as_ref().expect("result");
    assert_eq!(
        result.summary.as_deref(),
        Some("Fixed proration rounding: partial months round per line.")
    );
    assert_eq!(result.summary_source, Some(WorkSummarySource::Agent));
    assert_eq!(
        result.checks.as_deref(),
        Some("Ran the 12 billing tests, all pass.")
    );
    assert_eq!(result.outcome, Some(WorkOutcome::Done));
    let names: Vec<&str> = result.artifacts.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        ["PR #4861", "invoice.ts"],
        "agent links first, written file once"
    );
    assert_eq!(report.headline, "Done in 1s · 1 file changed");
}

#[test]
fn a_reported_failure_says_so_and_a_rejected_report_is_ignored() {
    let mut log = Log::new();
    let turn = log.turn.clone();
    log.started_with(
        100,
        "r1",
        "complete_assignment",
        "Completing",
        json!({ "summary": "Could not reach the billing API.", "outcome": "failed" }),
    )
    .completed(200, "r1", "complete_assignment", "Completed")
    .started_with(
        300,
        "r2",
        "report_done",
        "Reporting",
        json!({ "summary": "Ignored" }),
    )
    .failed(400, "r2", "report_done", "Could not report")
    .push(
        2_000,
        "turn.completed",
        json!({ "turn_id": turn, "iterations": 1 }),
    );

    let report = log.report_at(5_000);
    let result = report.result.as_ref().expect("result");
    assert_eq!(
        result.summary.as_deref(),
        Some("Could not reach the billing API.")
    );
    assert_eq!(result.outcome, Some(WorkOutcome::Failed));
    assert_eq!(report.headline, "Could not finish · 2s");
    assert!(report.steps.is_empty());
    assert_eq!(
        report.fact(WorkFactKind::Issues),
        None,
        "a rejected report is not an issue"
    );
}

#[test]
fn a_stopped_turn_does_not_leave_steps_running() {
    let mut log = Log::new();
    let turn = log.turn.clone();
    log.started(100, "c1", "bash", "Run tests").push(
        6_200,
        "turn.cancelled",
        json!({ "turn_id": turn }),
    );
    let report = log.report_at(10_000);
    assert_eq!(report.state, WorkState::Stopped);
    assert_eq!(report.headline, "Stopped after 6s");
    assert_eq!(report.steps[0].state, WorkStepState::Issue);
    assert!(report.attention.is_none());
}

#[test]
fn a_failed_turn_says_how_long_it_ran() {
    let mut log = Log::new();
    log.push(
        22_000,
        "turn.failed",
        json!({ "turn_id": log.turn, "error": "provider down" }),
    );
    let report = log.report_at(30_000);
    assert_eq!(report.state, WorkState::Failed);
    assert_eq!(report.headline, "Failed after 22s");
}

#[test]
fn events_of_other_turns_are_ignored() {
    let mut log = Log::new();
    let other = TurnId::new().to_string();
    log.push_in_turn(
        &other,
        100,
        "tool.started",
        json!({
        "tool_call": { "id": "x", "name": "bash", "arguments": {} }, "narration": "Other" }),
    );
    assert!(log.report_at(200).steps.is_empty());
}

#[test]
fn long_turns_keep_the_newest_steps() {
    let mut log = Log::new();
    for index in 0..(MAX_REPORT_STEPS + 5) {
        let id = format!("c{index}");
        log.started(index as i64, &id, "read_file", "Read")
            .completed(index as i64, &id, "read_file", &format!("Read {index}"));
    }
    let report = log.report_at(10_000);
    assert_eq!(report.steps.len(), MAX_REPORT_STEPS);
    assert_eq!(report.omitted_steps, 5);
    assert_eq!(report.steps[0].label, "Read 5");
}

#[test]
fn durations_read_like_a_person_would_say_them() {
    assert_eq!(format_duration(850), "850ms");
    assert_eq!(format_duration(13_400), "13s");
    assert_eq!(format_duration(125_000), "2m 5s");
    assert_eq!(format_duration(120_000), "2m");
    assert_eq!(format_duration(3_780_000), "1h 3m");
}

#[test]
fn first_sentence_is_bounded_and_plain() {
    assert_eq!(
        first_sentence("\n# Done\nMore", 50).as_deref(),
        Some("Done")
    );
    assert_eq!(
        first_sentence("Version 1.2 shipped today. Next", 50).as_deref(),
        Some("Version 1.2 shipped today.")
    );
    let long = "word ".repeat(100);
    let cut = first_sentence(&long, 20).unwrap();
    assert!(cut.chars().count() <= 20);
    assert!(cut.ends_with('…'));
    assert!(first_sentence("   ", 20).is_none());
}
