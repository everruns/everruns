// Trace API over real events in a real database: batching, elision, step
// detail, request pages, event lists and org isolation.

use std::sync::Arc;

use chrono::Utc;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::events::{
    Event, EventContext, LlmGenerationData, ToolCompletedData, ToolStartedData, TurnCompletedData,
    TurnStartedData,
};
use everruns_core::message::RuntimeMessage;
use everruns_core::{Caller, ContentPart, DEFAULT_ORG_ID, TokenUsage};
use serde_json::json;

use super::*;
use crate::domains::common::{Command, CommandErrorKind, Ctx};
use crate::storage::{CreateEventRow, StorageBackend};

fn row(event: Event) -> CreateEventRow {
    let value = serde_json::to_value(&event).expect("serialize event");
    CreateEventRow {
        session_id: event.session_id,
        event_type: event.event_type.clone(),
        ts: Utc::now(),
        context: value["context"].clone(),
        data: value["data"].clone(),
        metadata: None,
        tags: None,
    }
}

fn ctx(db: &Arc<StorageBackend>, org: i64) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(org), db.clone(), None)
}

/// One turn: a model call asking for `calls` (tool name, succeeds), the
/// calls, then an answer. The history grows by two messages per turn.
fn turn(
    session: SessionId,
    prompt: &str,
    history: usize,
    calls: &[(&str, bool)],
) -> Vec<CreateEventRow> {
    let turn = TurnId::new();
    let message = MessageId::new();
    let ctx = EventContext::turn(turn, message);
    let messages: Vec<RuntimeMessage> = (0..history)
        .map(|i| RuntimeMessage::user(format!("message {i}")))
        .collect();
    let tool_calls: Vec<ToolCall> = calls
        .iter()
        .enumerate()
        .map(|(i, (name, _))| ToolCall {
            id: format!("call_{}_{i}", turn.uuid().simple()),
            name: (*name).to_string(),
            arguments: json!({"i": i}),
        })
        .collect();
    let mut out = vec![
        row(Event::new(
            session,
            ctx.clone(),
            TurnStartedData {
                turn_id: turn,
                input_message_id: message,
                input_content: Some(prompt.to_string()),
                agent_id: None,
                agent_name: None,
                agent_description: None,
            },
        )),
        row(Event::new(
            session,
            ctx.clone(),
            LlmGenerationData::success(
                messages.clone(),
                vec![],
                Some("Working.".to_string()),
                tool_calls.clone(),
                "gpt-6.1-sol".to_string(),
                None,
                Some(TokenUsage::new(100, 10)),
                Some(500),
                None,
            ),
        )),
    ];
    for (call, (_, ok)) in tool_calls.iter().zip(calls) {
        out.push(row(Event::new(
            session,
            ctx.clone(),
            ToolStartedData {
                tool_call: call.clone(),
                tool_call_fingerprint: None,
                display_name: None,
                narration: None,
            },
        )));
        let mut done = ToolCompletedData::success(
            call.id.clone(),
            call.name.clone(),
            vec![ContentPart::text("ok")],
            Some(10),
        );
        if !ok {
            done.success = false;
            done.status = "error".to_string();
            done.error = Some("boom".to_string());
        }
        out.push(row(Event::new(session, ctx.clone(), done)));
    }
    let mut longer = messages;
    longer.push(RuntimeMessage::user("tool results"));
    out.push(row(Event::new(
        session,
        ctx.clone(),
        LlmGenerationData::success(
            longer,
            vec![],
            Some("Done.".to_string()),
            vec![],
            "gpt-6.1-sol".to_string(),
            None,
            Some(TokenUsage::new(100, 10)),
            Some(500),
            None,
        ),
    )));
    out.push(row(Event::new(
        session,
        ctx,
        serde_json::from_value::<TurnCompletedData>(json!({
            "turn_id": turn.to_string(),
            "iterations": 2,
            "usage": {"input_tokens": 200, "output_tokens": 20}
        }))
        .expect("turn.completed payload"),
    )));
    out
}

async fn seeded() -> (Arc<StorageBackend>, SessionId) {
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    // Turn 1: one call. Turn 2: a batch of 12 web_fetch calls (5 failed),
    // then 3 get_agent calls. Turn 3: 250 alternating calls, never batched.
    db.create_events(turn(session, "first", 2, &[("web_fetch", true)]))
        .await
        .unwrap();
    let mut calls: Vec<(&str, bool)> = (0..12)
        .map(|i| ("web_fetch", i % 3 != 0 && i != 1))
        .collect();
    calls.extend([("get_agent", true); 3]);
    db.create_events(turn(session, "second", 4, &calls))
        .await
        .unwrap();
    let alternating: Vec<(&str, bool)> = (0..250)
        .map(|i| (if i % 2 == 0 { "a" } else { "b" }, i % 50 != 7))
        .collect();
    db.create_events(turn(session, "third", 6, &alternating))
        .await
        .unwrap();
    (db, session)
}

#[tokio::test]
async fn overview_counts_and_buckets() {
    let (db, session) = seeded().await;
    let overview = GetSessionTrace {
        session_id: session.to_string(),
        buckets: Some(2),
    }
    .execute(&ctx(&db, DEFAULT_ORG_ID))
    .await
    .unwrap();
    assert_eq!(overview.turn_count, 3);
    // Model call + answer per turn, plus the tool calls.
    assert_eq!(overview.step_count, 3 * 2 + 1 + 15 + 250);
    assert_eq!(overview.bucket_size, 2);
    assert_eq!(overview.buckets.len(), 2);
    assert_eq!(overview.buckets[0].from_turn, 1);
    assert_eq!(overview.buckets[1].to_turn, 3);
    assert_eq!(overview.error_turns, vec![3, 2]);
}

#[tokio::test]
async fn turns_fold_repeated_calls_and_elide_long_turns() {
    let (db, session) = seeded().await;
    let page = ListSessionTraceTurns {
        session_id: session.to_string(),
        before: None,
        after: None,
        around: None,
        sequence: None,
        limit: None,
    }
    .execute(&ctx(&db, DEFAULT_ORG_ID))
    .await
    .unwrap();
    assert_eq!(page.turn_count, 3);
    assert!(!page.has_earlier && !page.has_later);
    assert_eq!(page.turns.len(), 3);

    let first = &page.turns[0];
    assert_eq!(first.prompt.as_deref(), Some("first"));
    assert_eq!(first.items.len(), 3);

    let second = &page.turns[1];
    let shape: Vec<&str> = second
        .items
        .iter()
        .map(|i| match i {
            TraceItem::Step(s) => s.kind.as_str(),
            TraceItem::Batch(_) => "batch",
            TraceItem::Gap(_) => "gap",
        })
        .collect();
    assert_eq!(shape, ["model", "batch", "tool", "tool", "tool", "answer"]);
    let TraceItem::Batch(batch) = &second.items[1] else {
        unreachable!()
    };
    assert_eq!((batch.name.as_str(), batch.count), ("web_fetch", 12));
    assert_eq!((batch.failed, batch.succeeded), (5, 7));
    assert_eq!(batch.failures.len(), 5);
    assert!(batch.failures.iter().all(|f| f.status == "error"));

    let third = &page.turns[2];
    assert_eq!(third.items.len(), 25);
    let TraceItem::Gap(gap) = &third.items[12] else {
        unreachable!()
    };
    // 252 rows; 12 kept at each end.
    assert_eq!((gap.first_step, gap.last_step, gap.count), (13, 240, 228));
    assert_eq!(gap.errors, 4);

    // A cursor page around the third turn's event sequence.
    let around = ListSessionTraceTurns {
        session_id: session.to_string(),
        before: None,
        after: None,
        around: None,
        sequence: Some(third.start_sequence + 3),
        limit: Some(1),
    }
    .execute(&ctx(&db, DEFAULT_ORG_ID))
    .await
    .unwrap();
    assert_eq!(around.turns.len(), 1);
    assert_eq!(around.turns[0].turn, 3);
    assert!(around.has_earlier && !around.has_later);

    let gap_steps = ListSessionTraceSteps {
        session_id: session.to_string(),
        turn: 3,
        from_step: Some(gap.first_step),
        to_step: Some(gap.last_step),
        errors_only: Some(true),
        limit: Some(2),
    }
    .execute(&ctx(&db, DEFAULT_ORG_ID))
    .await
    .unwrap();
    assert_eq!(gap_steps.steps.len(), 2);
    assert!(gap_steps.next_step.is_some());
}

#[tokio::test]
async fn step_detail_and_request_show_what_changed() {
    let (db, session) = seeded().await;
    let c = ctx(&db, DEFAULT_ORG_ID);
    let tool = GetSessionTraceStep {
        session_id: session.to_string(),
        turn: 1,
        step: 2,
        full: None,
    }
    .execute(&c)
    .await
    .unwrap();
    assert_eq!(tool.step.kind, "tool");
    assert_eq!(tool.input.unwrap().value, Some(json!({"i": 0})));
    assert!(tool.output.is_some());
    let types: Vec<_> = tool.events.iter().map(|e| e.event_type.as_str()).collect();
    assert_eq!(types, ["tool.started", "tool.completed"]);

    let answer = GetSessionTraceStep {
        session_id: session.to_string(),
        turn: 1,
        step: 3,
        full: None,
    }
    .execute(&c)
    .await
    .unwrap();
    let request = answer.request.unwrap();
    // The answer's request adds one message to the previous call's two.
    assert_eq!((request.message_count, request.new_from), (3, 2));
    assert_eq!(request.new_messages.len(), 1);

    let page = ListSessionTraceRequest {
        session_id: session.to_string(),
        turn: 1,
        step: 3,
        role: Some("user".to_string()),
        offset: Some(1),
        limit: Some(1),
    }
    .execute(&c)
    .await
    .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].index, 1);
    assert!(page.messages[0].content.is_some());

    let not_model = ListSessionTraceRequest {
        session_id: session.to_string(),
        turn: 1,
        step: 2,
        role: None,
        offset: None,
        limit: None,
    }
    .execute(&c)
    .await;
    assert!(not_model.is_err());

    let events = ListSessionTraceTurnEvents {
        session_id: session.to_string(),
        turn: 1,
        after_sequence: None,
        limit: Some(3),
    }
    .execute(&c)
    .await
    .unwrap();
    assert_eq!(events.events.len(), 3);
    assert_eq!(events.events[0].event_type, "turn.started");
    let rest = ListSessionTraceTurnEvents {
        session_id: session.to_string(),
        turn: 1,
        after_sequence: events.next_after_sequence,
        limit: None,
    }
    .execute(&c)
    .await
    .unwrap();
    assert_eq!(rest.events.len(), 3);
    assert_eq!(rest.events.last().unwrap().event_type, "turn.completed");
    assert!(rest.next_after_sequence.is_none());
}

#[tokio::test]
async fn other_orgs_cannot_read_a_trace() {
    let (db, session) = seeded().await;
    let err = GetSessionTrace {
        session_id: session.to_string(),
        buckets: None,
    }
    .execute(&ctx(&db, 424_242))
    .await
    .unwrap_err();
    assert!(matches!(err.kind, CommandErrorKind::NotFound(_)), "{err:?}");
}

/// Seeded large-session check against the success bars in
/// knowledge/ui/session-trace.md: 10K turns and ~1M steps, one turn with 10K
/// tool calls. Seeding takes a while, so it runs only with
/// `EVERRUNS_TRACE_BENCH=1`; it prints p95s and asserts the server bars.
#[tokio::test]
async fn large_session_meets_the_success_bars() {
    if std::env::var("EVERRUNS_TRACE_BENCH").as_deref() != Ok("1") {
        return;
    }
    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    let id = session.uuid();
    let pool = db.database().pool().clone();
    // Index rows straight from SQL: 10K turns of ~100 steps, alternating
    // tools so nothing batches, and turn 5000 with 10K calls of one tool.
    for sql in [
        "INSERT INTO session_trace_turns (session_id, turn_no, turn_id, start_sequence, end_sequence, \
            started_at, ended_at, status, prompt_preview, step_count, model_calls, tool_calls, \
            subagent_calls, error_count, input_tokens, output_tokens) \
         SELECT $1, t, gen_random_uuid(), t * 1000, t * 1000 + 999, \
            NOW() - make_interval(secs => 100000 - t * 10), NOW() - make_interval(secs => 100000 - t * 10 - 5), \
            CASE WHEN t % 97 = 0 THEN 'failed' ELSE 'completed' END, 'prompt ' || t, \
            CASE WHEN t = 5000 THEN 10002 ELSE 100 END, 2, 98, 0, (t % 97 = 0)::INT, 1000, 100 \
         FROM generate_series(1, 10000) t",
        "INSERT INTO session_trace_steps (session_id, turn_no, step_no, kind, status, start_sequence, \
            end_sequence, started_at, duration_ms, name, target, result, tool_call_id) \
         SELECT $1, t, s, 'tool', CASE WHEN s % 31 = 0 THEN 'error' ELSE 'success' END, \
            t * 1000 + s, t * 1000 + s, NOW() - make_interval(secs => 100000 - t * 10), 20, \
            CASE WHEN t = 5000 THEN 'web_fetch' WHEN s % 2 = 0 THEN 'a' ELSE 'b' END, \
            'https://example.com/' || s, 'ok', 'call_' || t || '_' || s \
         FROM generate_series(1, 10000) t, \
              generate_series(1, CASE WHEN t = 5000 THEN 10002 ELSE 100 END) s",
        "INSERT INTO session_trace_state (session_id, projected_sequence, turn_count) \
         VALUES ($1, 0, 10000)",
        "ANALYZE session_trace_turns",
        "ANALYZE session_trace_steps",
    ] {
        sqlx::query(sql).bind(id).execute(&pool).await.unwrap();
    }
    let c = ctx(&db, DEFAULT_ORG_ID);

    async fn p95<F, Fut>(runs: usize, mut f: F) -> std::time::Duration
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = usize>,
    {
        let mut times = Vec::with_capacity(runs);
        for _ in 0..runs {
            let started = std::time::Instant::now();
            f().await;
            times.push(started.elapsed());
        }
        times.sort();
        times[(runs * 95).div_ceil(100) - 1]
    }

    let mut size = 0;
    let first_screen = p95(40, || async {
        let overview = GetSessionTrace {
            session_id: session.to_string(),
            buckets: None,
        }
        .execute(&c)
        .await
        .unwrap();
        let turns = ListSessionTraceTurns {
            session_id: session.to_string(),
            before: None,
            after: None,
            around: None,
            sequence: None,
            limit: None,
        }
        .execute(&c)
        .await
        .unwrap();
        serde_json::to_vec(&overview).unwrap().len() + serde_json::to_vec(&turns).unwrap().len()
    })
    .await;
    for _ in 0..1 {
        size = serde_json::to_vec(
            &ListSessionTraceTurns {
                session_id: session.to_string(),
                before: None,
                after: None,
                around: None,
                sequence: None,
                limit: None,
            }
            .execute(&c)
            .await
            .unwrap(),
        )
        .unwrap()
        .len();
    }
    let big_turn = p95(40, || async {
        let page = ListSessionTraceTurns {
            session_id: session.to_string(),
            before: None,
            after: None,
            around: Some(5000),
            sequence: None,
            limit: Some(1),
        }
        .execute(&c)
        .await
        .unwrap();
        assert_eq!(page.turns[0].items.len(), 1);
        page.turns.len()
    })
    .await;
    let step = p95(40, || async {
        ListSessionTraceSteps {
            session_id: session.to_string(),
            turn: 5000,
            from_step: Some(4000),
            to_step: None,
            errors_only: None,
            limit: None,
        }
        .execute(&c)
        .await
        .unwrap()
        .steps
        .len()
    })
    .await;
    println!(
        "trace bench: first screen p95 {first_screen:?} ({size} bytes of turns), \
         10K-step turn p95 {big_turn:?}, steps page p95 {step:?}"
    );
    assert!(
        first_screen.as_millis() < 150,
        "first screen {first_screen:?}"
    );
    assert!(size < 250 * 1024, "first screen is {size} bytes");
    assert!(big_turn.as_millis() < 150, "10K-step turn {big_turn:?}");
    assert!(step.as_millis() < 100, "steps page {step:?}");
}
