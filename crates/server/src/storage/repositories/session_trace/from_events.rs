// The trace projection's view of an event the listener feed delivered.
//
// Decision: mirrors `load_event_slices` field for field, so a turn projected
// from the feed and one projected by a read or the backfill come out the
// same. `llm.generation` is cut down from its typed form, so the stored
// conversation it carries is never serialized again.

use everruns_core::{Event, EventData};
use serde_json::{Value, json};

use super::projection::{TRACE_EVENT_TYPES, TraceEvent};

/// The projection's slice of `event`, or `None` for an event the trace does
/// not read or one without a stored sequence.
pub fn trace_event(event: &Event) -> Option<TraceEvent> {
    let sequence = event.sequence?;
    if !TRACE_EVENT_TYPES.contains(&event.event_type.as_str()) {
        return None;
    }
    Some(TraceEvent {
        sequence,
        event_type: event.event_type.clone(),
        ts: event.ts,
        turn_id: event.context.turn_id.map(|id| id.uuid()),
        data: slice(&event.event_type, &event.data),
    })
}

fn left(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn left_str(value: Option<&Value>, chars: usize) -> Value {
    match value {
        Some(Value::String(text)) => Value::String(left(text, chars)),
        Some(Value::Null) | None => Value::Null,
        // `data->>'key'` renders a non-string as JSON text.
        Some(other) => Value::String(left(&other.to_string(), chars)),
    }
}

fn pick(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
}

fn slice(event_type: &str, data: &EventData) -> Value {
    if let EventData::LlmGeneration(generation) = data {
        let metadata = serde_json::to_value(&generation.metadata).unwrap_or(Value::Null);
        let tool_calls: Vec<Value> = generation
            .output
            .tool_calls
            .iter()
            .map(|call| json!({ "id": call.id, "name": call.name }))
            .collect();
        return json!({
            "output": {
                "text": generation.output.text.as_deref().map(|text| left(text, 2001)),
                "tool_calls": tool_calls,
            },
            "metadata": {
                "model": pick(&metadata, "model"),
                "success": pick(&metadata, "success"),
                "error": pick(&metadata, "error"),
                "duration_ms": pick(&metadata, "duration_ms"),
                "usage": pick(&metadata, "usage"),
                "retry": pick(&metadata, "retry"),
            },
            "message_count": generation.messages.len(),
        });
    }
    if let EventData::ToolCompleted(completed) = data {
        let first = completed
            .result
            .as_ref()
            .and_then(|parts| parts.first())
            .and_then(|part| serde_json::to_value(part).ok())
            .unwrap_or(Value::Null);
        return json!({
            "tool_call_id": completed.tool_call_id,
            "tool_name": completed.tool_name,
            "success": completed.success,
            "status": completed.status,
            "error": completed.error.as_deref().map(|error| left(error, 301)),
            "duration_ms": completed.duration_ms,
            "narration": completed.narration,
            "result_preview": left_str(first.get("text"), 301),
        });
    }
    let data = serde_json::to_value(data).unwrap_or(Value::Null);
    match event_type {
        "tool.started" => {
            let call = data.get("tool_call").cloned().unwrap_or(Value::Null);
            let arguments = match call.get("arguments") {
                None | Some(Value::Null) => Value::Null,
                Some(arguments) => Value::String(left(&arguments.to_string(), 301)),
            };
            json!({
                "tool_call": {
                    "id": pick(&call, "id"),
                    "name": pick(&call, "name"),
                    "arguments_preview": arguments,
                },
                "display_name": pick(&data, "display_name"),
                "narration": pick(&data, "narration"),
            })
        }
        "turn.started" => json!({
            "turn_id": pick(&data, "turn_id"),
            "input_content": left_str(data.get("input_content"), 501),
        }),
        _ => json!({
            "turn_id": pick(&data, "turn_id"),
            "error": left_str(data.get("error"), 301),
            "error_code": pick(&data, "error_code"),
            "usage": pick(&data, "usage"),
        }),
    }
}
