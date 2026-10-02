#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The consumer pipeline on its own: what a [`RunResult`] holds, and the
//! resume coverage rule. The sequencing rules themselves are held by the
//! upstream corpus in `tests/conformance.rs`.

use everruns_ag_ui::consumer::{RunConsumer, RunOutcome, RunResult};
use everruns_ag_ui::{Interrupt, Message, ResumeBuilder, ResumeError, ResumeStatus, RunAgentInput};
use serde_json::{Value, json};

fn run(events: &[Value]) -> RunResult {
    let mut consumer = RunConsumer::new();
    for event in events {
        consumer.push_value(event.clone()).unwrap();
    }
    consumer.finish().unwrap()
}

fn started() -> Value {
    json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "r" })
}

fn finished() -> Value {
    json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "r" })
}

#[test]
fn result_assembles_text_tool_calls_and_tool_results() {
    let result = run(&[
        started(),
        json!({ "type": "TEXT_MESSAGE_START", "messageId": "m1", "role": "assistant" }),
        json!({ "type": "TEXT_MESSAGE_CONTENT", "messageId": "m1", "delta": "Looking " }),
        json!({ "type": "TEXT_MESSAGE_CONTENT", "messageId": "m1", "delta": "it up." }),
        json!({ "type": "TEXT_MESSAGE_END", "messageId": "m1" }),
        json!({ "type": "TOOL_CALL_CHUNK", "toolCallId": "c1", "toolCallName": "search",
                "parentMessageId": "m1", "delta": "{\"q\":" }),
        json!({ "type": "TOOL_CALL_CHUNK", "delta": "\"x\"}" }),
        json!({ "type": "TOOL_CALL_RESULT", "messageId": "tm1", "toolCallId": "c1", "content": "found" }),
        json!({ "type": "SUBAGENT_STARTED", "subagentRunId": "s1", "name": "helper" }),
        json!({ "type": "TEXT_MESSAGE_CHUNK", "messageId": "m2", "delta": "inner", "subagentRunId": "s1" }),
        json!({ "type": "SUBAGENT_FINISHED", "subagentRunId": "s1" }),
        json!({ "type": "TEXT_MESSAGE_CHUNK", "messageId": "m3", "delta": "Done." }),
        json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "r", "result": { "ok": true },
                "outcome": { "type": "success", "pendingToolCallIds": ["c9"] } }),
    ]);
    assert_eq!(result.messages.len(), 3);
    // The subagent's message is kept, but is not the agent's own answer.
    assert_eq!(result.text(), "Looking it up.\n\nDone.");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].arguments, r#"{"q":"x"}"#);
    assert_eq!(
        result.tool_calls[0].parent_message_id.as_deref(),
        Some("m1")
    );
    assert_eq!(
        result.tool_calls[0].result.as_ref().map(|c| c.to_text()),
        Some("found".to_owned())
    );
    assert_eq!(
        result.outcome,
        RunOutcome::Success {
            pending_tool_call_ids: vec!["c9".into()]
        }
    );
    assert_eq!(result.result, Some(json!({ "ok": true })));
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
}

#[test]
fn usage_is_summed_across_runs_and_the_last_run_decides_the_outcome() {
    let usage =
        |n: u64| json!([{ "provider": "p", "model": "m", "inputTokens": n, "outputTokens": 1 }]);
    let result = run(&[
        started(),
        json!({ "type": "RUN_ERROR", "message": "first failed", "code": "E1", "usage": usage(5) }),
        json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "r2" }),
        json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "r2", "usage": usage(7),
                "outcome": { "type": "cancelled" } }),
    ]);
    assert_eq!(result.outcome, RunOutcome::Cancelled);
    assert_eq!(result.run_id.as_deref(), Some("r2"));
    assert_eq!(result.usage.len(), 1);
    assert_eq!(result.usage[0].input_tokens, Some(12));
    assert_eq!(result.usage[0].output_tokens, Some(2));

    let failed = run(&[
        started(),
        json!({ "type": "RUN_ERROR", "message": "boom", "code": "E2" }),
    ]);
    assert_eq!(failed.error(), Some(("boom", Some("E2"))));
}

#[test]
fn snapshot_replaces_messages_but_not_with_request_history() {
    let input = RunAgentInput {
        thread_id: "t".into(),
        run_id: "r".into(),
        messages: vec![Message::user("u1", "question")],
        ..RunAgentInput::default()
    };
    let mut consumer = RunConsumer::for_input(&input);
    for event in [
        started(),
        json!({ "type": "TEXT_MESSAGE_CHUNK", "messageId": "a1", "delta": "draft" }),
        json!({ "type": "TEXT_MESSAGE_CHUNK", "messageId": "a2", "delta": "retracted" }),
        json!({ "type": "MESSAGES_SNAPSHOT", "messages": [
            { "id": "u1", "role": "user", "content": "question" },
            { "id": "a1", "role": "assistant", "content": "final" },
        ] }),
        finished(),
    ] {
        consumer.push_value(event).unwrap();
    }
    let result = consumer.finish().unwrap();
    assert_eq!(result.messages.len(), 1);
    assert_eq!(result.text(), "final");
}

#[test]
fn version_declarations_other_than_ours_warn() {
    let result = run(&[
        json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "r", "protocolVersion": "1.1" }),
        finished(),
    ]);
    assert_eq!(result.protocol_version.as_deref(), Some("1.1"));
    assert_eq!(result.warnings.len(), 1);
}

#[test]
fn an_empty_or_unfinished_stream_is_an_error() {
    assert!(RunConsumer::new().finish().is_err());
    let mut consumer = RunConsumer::new();
    consumer.push_value(started()).unwrap();
    assert!(consumer.finish().is_err());
}

#[test]
fn resume_builder_refuses_unknown_duplicate_and_uncovered_answers() {
    let open = [
        Interrupt::new("i1", "tool_approval"),
        Interrupt::new("i2", "input_required"),
    ];
    let mut resume = ResumeBuilder::new(&open);
    assert_eq!(
        resume.resolve("nope", json!(true)).err(),
        Some(ResumeError::UnknownInterrupt("nope".into()))
    );
    resume.resolve("i2", json!("blue")).unwrap();
    assert_eq!(
        resume.cancel("i2").err(),
        Some(ResumeError::DuplicateAnswer("i2".into()))
    );
    assert_eq!(
        resume.clone().build(),
        Err(ResumeError::Uncovered(vec!["i1".into()]))
    );
    resume.resolve("i1", json!({ "approved": true })).unwrap();
    let entries = resume.build().unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e.interrupt_id.as_str()).collect();
    assert_eq!(ids, ["i1", "i2"]);
    assert!(entries.iter().all(|e| e.status == ResumeStatus::Resolved));

    // With nothing open, an empty resume is complete.
    assert_eq!(ResumeBuilder::new(&[]).build(), Ok(vec![]));
}
