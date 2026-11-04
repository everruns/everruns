// What a scorer reads for one trace score, by scope.
//
// Decisions:
// - Turn scope reads the events of that turn, as before.
// - Session scope reads the whole session as a "User: ... / Agent: ..."
//   transcript (the same shape multistep eval cases give graders), the last
//   exchange as input and answer, every tool call, the turn count and the
//   session's tokens. It reads at most SESSION_EVENT_LIMIT events, newest
//   kept, so a very long session is judged on its recent part.
// - Tool scope reads one call: its arguments as the input and its result
//   (or error) as the answer, so a judge rubric and the rule scorers work on
//   it unchanged.

use everruns_contracts::typed_id::SessionId;

use crate::domains::evals::multistep::{Line, Speaker, transcript_text};
use crate::domains::evals::runner::{
    extract_final_assistant_content, extract_token_usage, extract_tool_arguments,
    extract_tool_calls,
};
use crate::domains::observers::record::ObserverScope;
use crate::storage::{EventRow, ListEventsParams, StorageBackend, TraceScoreRow};

const SESSION_EVENT_LIMIT: i32 = 2_000;
const SAID: [&str; 2] = ["output.message.completed", "conversation.message"];
const TOOL_STARTED: &str = "tool.started";
const TOOL_COMPLETED: &str = "tool.completed";
const TURN_COMPLETED: &str = "turn.completed";

/// The trace slice one score grades.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Evidence {
    pub input_message: String,
    pub final_answer: String,
    pub tool_calls: Vec<String>,
    pub tool_arguments: Vec<String>,
    /// Empty except for session scope.
    pub transcript: String,
    pub turns: u32,
    pub tokens: u64,
}

/// Read the evidence for `score`. `None` when the slice is not in the trace
/// (a tool call id that never completed).
pub async fn gather(
    db: &StorageBackend,
    score: &TraceScoreRow,
) -> anyhow::Result<Option<Evidence>> {
    let session_id = SessionId::from_uuid(score.session_id);
    match ObserverScope::from(score.scope.as_str()) {
        ObserverScope::Turn => {
            let events = list(db, session_id, Some(&score.turn_id), None).await?;
            Ok(Some(turn_evidence(&events)))
        }
        ObserverScope::Session => {
            let events = list(db, session_id, None, Some(SESSION_EVENT_LIMIT)).await?;
            Ok(Some(session_evidence(&events)))
        }
        ObserverScope::Tool => {
            let params = ListEventsParams {
                session_id,
                filter_types: vec![TOOL_STARTED.into(), TOOL_COMPLETED.into()],
                ..Default::default()
            };
            let events = db.list_events_advanced(&params).await?;
            Ok(tool_evidence(&events, &score.turn_id))
        }
    }
}

async fn list(
    db: &StorageBackend,
    session_id: SessionId,
    turn_id: Option<&str>,
    newest: Option<i32>,
) -> anyhow::Result<Vec<EventRow>> {
    let mut types: Vec<String> = SAID.iter().map(|t| t.to_string()).collect();
    types.extend([TOOL_STARTED, TOOL_COMPLETED, TURN_COMPLETED].map(String::from));
    let params = ListEventsParams {
        session_id,
        turn_id: turn_id.map(str::to_string),
        filter_types: types,
        order_desc: newest.is_some(),
        limit: newest,
        ..Default::default()
    };
    let mut events = db.list_events_advanced(&params).await?;
    events.sort_by_key(|e| e.sequence);
    Ok(events)
}

fn of(events: &[EventRow], types: &[&str]) -> Vec<EventRow> {
    events
        .iter()
        .filter(|e| types.contains(&e.event_type.as_str()))
        .cloned()
        .collect()
}

fn turn_evidence(events: &[EventRow]) -> Evidence {
    let completed = of(events, &[TOOL_COMPLETED]);
    let turn_completed = events.iter().rev().find(|e| e.event_type == TURN_COMPLETED);
    Evidence {
        input_message: input_of(turn_completed),
        final_answer: extract_final_assistant_content(&of(events, &SAID)),
        tool_calls: extract_tool_calls(&completed),
        tool_arguments: extract_tool_arguments(&completed, &of(events, &[TOOL_STARTED])),
        transcript: String::new(),
        turns: turn_completed
            .and_then(|e| e.data.get("iterations"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32,
        tokens: {
            let (input, output) = extract_token_usage(events);
            input + output
        },
    }
}

fn session_evidence(events: &[EventRow]) -> Evidence {
    let mut lines = Vec::new();
    let mut said_in_turn: Option<String> = None;
    let mut last_input = String::new();
    let mut last_answer = String::new();
    let mut turns = 0u32;
    for event in events {
        if SAID.contains(&event.event_type.as_str()) {
            if let Some(text) =
                everruns_core::conversation::said_in_event(&event.event_type, &event.data)
            {
                said_in_turn = Some(text);
            }
        } else if event.event_type == TURN_COMPLETED {
            turns += 1;
            let input = input_of(Some(event));
            if !input.is_empty() {
                lines.push(Line {
                    speaker: Speaker::User,
                    text: input.clone(),
                });
                last_input = input;
            }
            if let Some(answer) = said_in_turn.take() {
                lines.push(Line {
                    speaker: Speaker::Agent,
                    text: answer.clone(),
                });
                last_answer = answer;
            }
        }
    }
    let completed = of(events, &[TOOL_COMPLETED]);
    Evidence {
        input_message: last_input,
        final_answer: last_answer,
        tool_calls: extract_tool_calls(&completed),
        tool_arguments: extract_tool_arguments(&completed, &of(events, &[TOOL_STARTED])),
        transcript: transcript_text(&lines),
        turns,
        tokens: {
            let (input, output) = extract_token_usage(events);
            input + output
        },
    }
}

fn tool_evidence(events: &[EventRow], call_id: &str) -> Option<Evidence> {
    let is_call = |e: &&EventRow| {
        e.data.get("tool_call_id").and_then(|v| v.as_str()) == Some(call_id)
            || e.data
                .get("tool_call")
                .and_then(|c| c.get("id"))
                .and_then(|v| v.as_str())
                == Some(call_id)
    };
    let completed = events
        .iter()
        .filter(is_call)
        .find(|e| e.event_type == TOOL_COMPLETED)?
        .clone();
    let started: Vec<EventRow> = events
        .iter()
        .filter(is_call)
        .filter(|e| e.event_type == TOOL_STARTED)
        .cloned()
        .collect();
    let completed = vec![completed];
    let tool_calls = extract_tool_calls(&completed);
    let tool_arguments = extract_tool_arguments(&completed, &started);
    let name = tool_calls.first().cloned().unwrap_or_default();
    let arguments = tool_arguments.first().cloned().unwrap_or_default();
    Some(Evidence {
        input_message: format!("Call to {name} with arguments: {arguments}"),
        final_answer: tool_result_text(&completed[0].data),
        tool_calls,
        tool_arguments,
        transcript: String::new(),
        turns: 1,
        tokens: 0,
    })
}

/// A completed call's result text, or its error.
fn tool_result_text(data: &serde_json::Value) -> String {
    let parts: Vec<&str> = data
        .get("result")
        .and_then(|r| r.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if !parts.is_empty() {
        return parts.join("\n");
    }
    data.get("error")
        .and_then(|e| e.as_str())
        .map(|e| format!("Error: {e}"))
        .unwrap_or_default()
}

fn input_of(turn_completed: Option<&EventRow>) -> String {
    turn_completed
        .and_then(|e| e.data.get("input_content"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::EventId;
    use serde_json::json;

    fn event(sequence: i32, event_type: &str, data: serde_json::Value) -> EventRow {
        EventRow {
            id: EventId::from_uuid(uuid::Uuid::now_v7()),
            session_id: SessionId::from_uuid(uuid::Uuid::now_v7()),
            sequence,
            event_type: event_type.into(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn said(sequence: i32, text: &str) -> EventRow {
        event(
            sequence,
            "output.message.completed",
            json!({"message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}),
        )
    }

    fn tool(sequence: i32, id: &str, name: &str, args: serde_json::Value) -> [EventRow; 2] {
        [
            event(
                sequence,
                TOOL_STARTED,
                json!({"tool_call": {"id": id, "name": name, "arguments": args}}),
            ),
            event(
                sequence + 1,
                TOOL_COMPLETED,
                json!({"tool_call_id": id, "tool_name": name, "success": true, "status": "success",
                       "result": [{"type": "text", "text": "{\"status\":\"open\"}"}]}),
            ),
        ]
    }

    #[test]
    fn session_reads_the_whole_conversation() {
        let mut events = vec![
            said(1, "Which order?"),
            event(
                2,
                TURN_COMPLETED,
                json!({"input_content": "I was charged twice", "usage": {"input_tokens": 10, "output_tokens": 5}}),
            ),
        ];
        events.extend(tool(3, "c1", "create_ticket", json!({"order": "ORD-1"})));
        events.push(said(5, "Ticket T-1 is open."));
        events.push(event(
            6,
            TURN_COMPLETED,
            json!({"input_content": "ORD-1", "usage": {"input_tokens": 20, "output_tokens": 5}}),
        ));
        let evidence = session_evidence(&events);
        assert_eq!(evidence.turns, 2);
        assert_eq!(evidence.tokens, 40);
        assert_eq!(evidence.input_message, "ORD-1");
        assert_eq!(evidence.final_answer, "Ticket T-1 is open.");
        assert_eq!(evidence.tool_calls, vec!["create_ticket"]);
        assert!(evidence.tool_arguments[0].contains("ORD-1"));
        assert_eq!(
            evidence.transcript,
            "User: I was charged twice\n\nAgent: Which order?\n\nUser: ORD-1\n\nAgent: Ticket T-1 is open."
        );
    }

    #[test]
    fn tool_reads_one_call() {
        let mut events = Vec::new();
        events.extend(tool(1, "c1", "lookup_order", json!({"order": "ORD-1"})));
        events.extend(tool(3, "c2", "create_ticket", json!({"order": "ORD-2"})));
        let evidence = tool_evidence(&events, "c2").unwrap();
        assert_eq!(evidence.tool_calls, vec!["create_ticket"]);
        assert!(evidence.input_message.starts_with("Call to create_ticket"));
        assert!(evidence.input_message.contains("ORD-2"));
        assert_eq!(evidence.final_answer, "{\"status\":\"open\"}");
        assert!(tool_evidence(&events, "missing").is_none());
    }

    #[test]
    fn failed_tool_reads_its_error() {
        assert_eq!(
            tool_result_text(&json!({"success": false, "error": "timeout"})),
            "Error: timeout"
        );
    }
}
