// Trace index against a real database: real event payloads in, the SQL slice
// and the projection together.

use chrono::Utc;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
use everruns_core::ContentPart;
use everruns_core::TokenUsage;
use everruns_core::events::{
    Event, EventContext, LlmGenerationData, ToolCompletedData, ToolStartedData, TurnCompletedData,
    TurnStartedData,
};
use everruns_core::message::RuntimeMessage;
use serde_json::json;

use super::projection::{kind, status};
use super::rows::TraceCatchUp;
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

/// One turn: a model call that asks for a tool, the tool, then the answer.
fn turn_events(session: SessionId, prompt: &str, tool_ok: bool) -> Vec<CreateEventRow> {
    let turn = TurnId::new();
    let message = MessageId::new();
    let ctx = EventContext::turn(turn, message);
    let call = ToolCall {
        id: format!("call_{}", turn.uuid().simple()),
        name: "web_fetch".to_string(),
        arguments: json!({"url": "https://example.com", "body": "x".repeat(10_000)}),
    };
    // A prompt far larger than anything the index keeps.
    let history: Vec<RuntimeMessage> = (0..50)
        .map(|i| RuntimeMessage::user(format!("message {i} {}", "y".repeat(2_000))))
        .collect();
    let completed = if tool_ok {
        ToolCompletedData::success(
            call.id.clone(),
            call.name.clone(),
            vec![ContentPart::text(format!("200 OK {}", "z".repeat(5_000)))],
            Some(120),
        )
    } else {
        let mut failed =
            ToolCompletedData::success(call.id.clone(), call.name.clone(), vec![], Some(80));
        failed.success = false;
        failed.status = "error".to_string();
        failed.error = Some("404 Not Found".to_string());
        failed
    };
    vec![
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
                origin: None,
            },
        )),
        row(Event::new(
            session,
            ctx.clone(),
            LlmGenerationData::success(
                history.clone(),
                vec![],
                Some("Fetching the page.".to_string()),
                vec![call.clone()],
                "gpt-6.1-sol".to_string(),
                None,
                Some(TokenUsage::new(900, 30)),
                Some(2_000),
                None,
            ),
        )),
        row(Event::new(
            session,
            ctx.clone(),
            ToolStartedData {
                tool_call: call.clone(),
                tool_call_fingerprint: None,
                display_name: None,
                narration: Some("Fetched example.com".to_string()),
                side_effect_class: None,
            },
        )),
        row(Event::new(session, ctx.clone(), completed)),
        row(Event::new(
            session,
            ctx.clone(),
            LlmGenerationData::success(
                history,
                vec![],
                Some("Done.".to_string()),
                vec![],
                "gpt-6.1-sol".to_string(),
                None,
                Some(TokenUsage::new(1_000, 10)),
                Some(1_000),
                None,
            ),
        )),
        row(Event::new(
            session,
            ctx,
            serde_json::from_value::<TurnCompletedData>(json!({
                "turn_id": turn.to_string(),
                "iterations": 2,
                "usage": {"input_tokens": 1900, "output_tokens": 40}
            }))
            .expect("turn.completed payload"),
        )),
    ]
}

#[tokio::test]
async fn real_events_project_into_turns_and_steps() {
    let db = StorageBackend::test_database();
    let session = db.create_test_session().await;
    db.create_events(turn_events(session, "Fetch the page", true))
        .await
        .unwrap();
    db.create_events(turn_events(session, "Fetch it again", false))
        .await
        .unwrap();

    let database = db.database();
    database
        .catch_up_session_trace_fully(session.uuid())
        .await
        .unwrap();
    let turns = database
        .list_trace_turns(session.uuid(), 1, 100)
        .await
        .unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].prompt_preview.as_deref(), Some("Fetch the page"));
    assert_eq!(turns[0].status, status::COMPLETED);
    assert_eq!(
        (
            turns[0].step_count,
            turns[0].model_calls,
            turns[0].tool_calls
        ),
        (3, 2, 1)
    );
    assert_eq!((turns[0].input_tokens, turns[0].output_tokens), (1900, 40));
    assert_eq!(turns[1].error_count, 1);

    let steps = database
        .list_trace_steps(session.uuid(), 1, 1, 100)
        .await
        .unwrap();
    let kinds: Vec<_> = steps.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, [kind::MODEL, kind::TOOL, kind::ANSWER]);
    assert_eq!(steps[0].narration.as_deref(), Some("Fetching the page."));
    assert_eq!(steps[0].message_count, Some(50));
    assert_eq!(steps[0].model.as_deref(), Some("gpt-6.1-sol"));
    let tool = &steps[1];
    assert_eq!(tool.status, status::SUCCESS);
    assert_eq!(tool.target.as_deref(), Some("Fetched example.com"));
    let result = tool.result.as_deref().unwrap();
    assert!(result.starts_with("200 OK"));
    assert!(result.chars().count() <= 301, "result summary is bounded");
    assert_eq!(
        steps[0].requested_tool_call_ids,
        [tool.tool_call_id.clone().unwrap()]
    );

    let failed = database
        .list_trace_steps(session.uuid(), 2, 2, 2)
        .await
        .unwrap();
    assert_eq!(failed[0].status, status::ERROR);
    assert_eq!(failed[0].result.as_deref(), Some("404 Not Found"));
}

#[tokio::test]
async fn passes_resume_from_the_watermark_and_match_a_single_pass() {
    let db = StorageBackend::test_database();
    let session = db.create_test_session().await;
    for i in 0..4 {
        db.create_events(turn_events(session, &format!("turn {i}"), i % 2 == 0))
            .await
            .unwrap();
    }
    let database = db.database();

    // Tiny passes split turns and tools across commits.
    loop {
        match database
            .catch_up_session_trace(session.uuid(), 2, true)
            .await
            .unwrap()
        {
            TraceCatchUp::UpToDate { .. } => break,
            TraceCatchUp::Partial { .. } | TraceCatchUp::Busy => {}
        }
    }
    let turns = database
        .list_trace_turns(session.uuid(), 1, 100)
        .await
        .unwrap();
    let mut steps = Vec::new();
    for t in &turns {
        steps.extend(
            database
                .list_trace_steps(session.uuid(), t.turn_no, 1, 100)
                .await
                .unwrap(),
        );
    }

    // Rebuild from scratch in one pass on a second session with the same
    // events and compare everything but identities and timestamps.
    let other = db.create_test_session().await;
    let rows: Vec<CreateEventRow> =
        sqlx::query_as::<_, (String, serde_json::Value, serde_json::Value)>(
            "SELECT event_type, context, data FROM events WHERE session_id = $1 ORDER BY sequence",
        )
        .bind(session.uuid())
        .fetch_all(database.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|(event_type, context, data)| CreateEventRow {
            session_id: other,
            event_type,
            ts: Utc::now(),
            context,
            data,
            metadata: None,
            tags: None,
        })
        .collect();
    db.create_events(rows).await.unwrap();
    database
        .catch_up_session_trace_fully(other.uuid())
        .await
        .unwrap();
    let other_turns = database
        .list_trace_turns(other.uuid(), 1, 100)
        .await
        .unwrap();
    let mut other_steps = Vec::new();
    for t in &other_turns {
        other_steps.extend(
            database
                .list_trace_steps(other.uuid(), t.turn_no, 1, 100)
                .await
                .unwrap(),
        );
    }

    let shape = |t: &crate::storage::TraceTurnRow| {
        (
            t.turn_no,
            t.turn_id,
            t.status.clone(),
            t.step_count,
            t.error_count,
            t.input_tokens,
        )
    };
    assert_eq!(turns.len(), 4);
    assert_eq!(
        turns.iter().map(shape).collect::<Vec<_>>(),
        other_turns.iter().map(shape).collect::<Vec<_>>()
    );
    let step_shape = |s: &crate::storage::TraceStepRow| {
        (
            s.turn_no,
            s.step_no,
            s.kind.clone(),
            s.status.clone(),
            s.result.clone(),
            s.narration.clone(),
        )
    };
    assert_eq!(
        steps.iter().map(step_shape).collect::<Vec<_>>(),
        other_steps.iter().map(step_shape).collect::<Vec<_>>()
    );
}

// The test database is shared, so other tests' sessions can fill any small limit.
#[tokio::test]
async fn sessions_behind_are_listed_until_caught_up() {
    let db = StorageBackend::test_database();
    let session = db.create_test_session().await;
    db.create_events(turn_events(session, "hello", true))
        .await
        .unwrap();
    let database = db.database();
    let watermark = database
        .catch_up_session_trace_fully(session.uuid())
        .await
        .unwrap();
    assert!(
        !database
            .sessions_behind_trace(i64::MAX)
            .await
            .unwrap()
            .contains(&session.uuid())
    );
    // An event the trace ignores still moves the session ahead of its index.
    db.create_event(CreateEventRow {
        session_id: session,
        event_type: "session.title.updated".to_string(),
        ts: Utc::now(),
        context: json!({}),
        data: json!({"title": "x"}),
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();
    // The latest sequence passes the index's watermark. Whether the session is
    // listed is racy: another test's app may run the backfill job, which
    // projects listed sessions, so a session already caught up counts too.
    let (latest, projected): (i32, Option<i32>) = sqlx::query_as(
        "SELECT es.next_sequence - 1, st.projected_sequence FROM event_sequences es \
         LEFT JOIN session_trace_state st USING (session_id) WHERE es.session_id = $1",
    )
    .bind(session.uuid())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert!(latest > watermark);
    let listed = database
        .sessions_behind_trace(i64::MAX)
        .await
        .unwrap()
        .contains(&session.uuid());
    assert!(listed || projected.is_some_and(|p| p >= latest));
    database
        .catch_up_session_trace_fully(session.uuid())
        .await
        .unwrap();
    assert!(
        !database
            .sessions_behind_trace(i64::MAX)
            .await
            .unwrap()
            .contains(&session.uuid())
    );
}

type TurnShape = (i32, uuid::Uuid, String, i32, i32, i64, Option<String>);
type StepShape = (
    i32,
    i32,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i32>,
);

async fn trace_shape(db: &StorageBackend, session: SessionId) -> (Vec<TurnShape>, Vec<StepShape>) {
    let database = db.database();
    let turns = database
        .list_trace_turns(session.uuid(), 1, 100)
        .await
        .unwrap();
    let mut steps = Vec::new();
    for t in &turns {
        for s in database
            .list_trace_steps(session.uuid(), t.turn_no, 1, 100)
            .await
            .unwrap()
        {
            steps.push((
                s.turn_no,
                s.step_no,
                s.kind,
                s.status,
                s.result,
                s.narration,
                s.target,
                s.message_count,
            ));
        }
    }
    let turns = turns
        .into_iter()
        .map(|t| {
            (
                t.turn_no,
                t.turn_id,
                t.status,
                t.step_count,
                t.error_count,
                t.input_tokens,
                t.prompt_preview,
            )
        })
        .collect();
    (turns, steps)
}

/// Turns projected from the events the listener feed delivered come out the
/// same as turns projected from storage, including a turn this replica never
/// saw, which the projection reads from storage.
#[tokio::test]
async fn feed_delivered_turns_project_like_stored_ones() {
    use crate::listeners::SessionTraceListener;
    use everruns_core::EventListener;
    use std::sync::Arc;

    let db = Arc::new(StorageBackend::test_database());
    let session = db.create_test_session().await;
    for i in 0..3 {
        db.create_events(turn_events(session, &format!("turn {i}"), i != 1))
            .await
            .unwrap();
    }
    let rows = db
        .list_events(session, None, None, &[], &[], None, None)
        .await
        .unwrap();
    let mut events: Vec<Event> = rows
        .into_iter()
        .map(crate::services::EventService::row_to_event)
        .collect();
    events.sort_by_key(|e| e.sequence);
    let first_end = events
        .iter()
        .find(|e| e.event_type == "turn.completed")
        .and_then(|e| e.sequence)
        .unwrap();

    // The first turn was delivered to another replica that never projected it.
    let listener = SessionTraceListener::new(db.clone());
    for event in events.iter().filter(|e| e.sequence > Some(first_end)) {
        listener.on_event(event).await;
    }
    let (projected, latest): (i32, i32) = sqlx::query_as(
        "SELECT st.projected_sequence, es.next_sequence - 1 FROM session_trace_state st \
         JOIN event_sequences es USING (session_id) WHERE st.session_id = $1",
    )
    .bind(session.uuid())
    .fetch_one(db.database().pool())
    .await
    .unwrap();
    assert_eq!(
        projected, latest,
        "the feed projected through the last turn"
    );
    let fed = trace_shape(&db, session).await;

    let other = db.create_test_session().await;
    let copies: Vec<CreateEventRow> = events
        .iter()
        .map(|e| CreateEventRow {
            session_id: other,
            ..row(e.clone())
        })
        .collect();
    db.create_events(copies).await.unwrap();
    db.database()
        .catch_up_session_trace_fully(other.uuid())
        .await
        .unwrap();
    let stored = trace_shape(&db, other).await;

    assert_eq!(fed.0.len(), 3);
    assert_eq!(fed.1.len(), 9);
    // Turn ids are the events' own, so both sessions share them.
    assert_eq!(fed, stored);
}
