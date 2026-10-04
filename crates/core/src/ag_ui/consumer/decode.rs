//! Step one of the consumer pipeline: one JSON value off the wire becomes
//! one typed [`Event`], or nothing.
//!
//! The 1.0 processing model, as applied here:
//!
//! - an event `type` this crate does not know is **dropped** with a warning;
//! - an unrecognised member of a union in an optional or list position (a
//!   `RUN_FINISHED` outcome, a content part, a message in a snapshot, a JSON
//!   Patch operation) is **stripped** with a warning;
//! - an undescribed property is ignored (serde does that);
//! - a known field holding a wrong-typed value is **fatal**.

use serde_json::{Map, Value};

use super::ProtocolError;
use crate::ag_ui::Event;

/// Every event `type` [`Event`] describes.
const KNOWN_EVENT_TYPES: &[&str] = &[
    "TEXT_MESSAGE_START",
    "TEXT_MESSAGE_CONTENT",
    "TEXT_MESSAGE_END",
    "TEXT_MESSAGE_CHUNK",
    "TOOL_CALL_START",
    "TOOL_CALL_ARGS",
    "TOOL_CALL_END",
    "TOOL_CALL_CHUNK",
    "TOOL_CALL_RESULT",
    "STATE_SNAPSHOT",
    "STATE_DELTA",
    "MESSAGES_SNAPSHOT",
    "ACTIVITY_SNAPSHOT",
    "ACTIVITY_DELTA",
    "RAW",
    "CUSTOM",
    "RUN_STARTED",
    "RUN_FINISHED",
    "RUN_ERROR",
    "STEP_STARTED",
    "STEP_FINISHED",
    "REASONING_START",
    "REASONING_MESSAGE_START",
    "REASONING_MESSAGE_CONTENT",
    "REASONING_MESSAGE_END",
    "REASONING_MESSAGE_CHUNK",
    "REASONING_END",
    "REASONING_ENCRYPTED_VALUE",
    "SUBAGENT_STARTED",
    "SUBAGENT_FINISHED",
    "SUBAGENT_ERROR",
];

const KNOWN_OUTCOMES: &[&str] = &["success", "interrupt", "cancelled"];
const KNOWN_PART_TYPES: &[&str] = &["text", "image", "audio", "video", "document"];
const KNOWN_ROLES: &[&str] = &[
    "developer",
    "system",
    "assistant",
    "user",
    "tool",
    "activity",
    "reasoning",
];
const KNOWN_PATCH_OPS: &[&str] = &["add", "remove", "replace", "move", "copy", "test"];

/// The result of decoding one wire value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decoded {
    /// The event, or `None` when the whole event was dropped as unrecognised.
    pub event: Option<Event>,
    /// What was dropped or stripped on the way, one line each.
    pub warnings: Vec<String>,
}

/// Decodes one event as it arrived on the wire, applying the 1.0
/// processing model (see the [consumer module docs](super)).
///
/// ```
/// use everruns_core::ag_ui::consumer::decode_event;
/// use serde_json::json;
///
/// // Unknown event types are dropped, not fatal.
/// let dropped = decode_event(json!({ "type": "FUTURE_EVENT" })).unwrap();
/// assert!(dropped.event.is_none());
/// assert_eq!(dropped.warnings.len(), 1);
///
/// // A known field with a wrong-typed value is fatal.
/// let malformed = json!({ "type": "TEXT_MESSAGE_CONTENT", "messageId": "m", "delta": 42 });
/// let err = decode_event(malformed).unwrap_err();
/// assert!(err.to_string().contains("delta"));
/// ```
pub fn decode_event(value: Value) -> Result<Decoded, ProtocolError> {
    let Value::Object(mut object) = value else {
        return Err(ProtocolError::new("an event must be a JSON object"));
    };
    let event_type = match object.get("type") {
        Some(Value::String(event_type)) => event_type.clone(),
        Some(_) => return Err(ProtocolError::new("an event's 'type' must be a string")),
        None => return Err(ProtocolError::new("an event must carry a 'type'")),
    };
    let mut warnings = Vec::new();
    if !KNOWN_EVENT_TYPES.contains(&event_type.as_str()) {
        warnings.push(format!("dropped unrecognised event type '{event_type}'"));
        return Ok(Decoded {
            event: None,
            warnings,
        });
    }

    strip_unrecognised(&event_type, &mut object, &mut warnings)?;

    let original = object.clone();
    let event = serde_json::from_value::<Event>(Value::Object(object)).map_err(|err| {
        let detail = err.to_string();
        match locate_field(&original, &detail) {
            Some(field) => ProtocolError::new(format!(
                "malformed {event_type} event: field '{field}': {detail}"
            )),
            None => ProtocolError::new(format!("malformed {event_type} event: {detail}")),
        }
    })?;
    Ok(Decoded {
        event: Some(event),
        warnings,
    })
}

fn strip_unrecognised(
    event_type: &str,
    object: &mut Map<String, Value>,
    warnings: &mut Vec<String>,
) -> Result<(), ProtocolError> {
    match event_type {
        "RUN_FINISHED" => {
            if let Some(outcome) = object.get("outcome") {
                let Value::Object(outcome) = outcome else {
                    // A malformed value in a described slot, not an
                    // unrecognised member: stripping it would report success
                    // for a run that paused and lose its interrupts.
                    return Err(ProtocolError::new(format!(
                        "malformed RUN_FINISHED event: field 'outcome': expected object, received {}",
                        json_kind(outcome)
                    )));
                };
                if let Some(Value::String(kind)) = outcome.get("type")
                    && !KNOWN_OUTCOMES.contains(&kind.as_str())
                {
                    warnings.push(format!(
                        "stripped unrecognised RUN_FINISHED outcome '{kind}'"
                    ));
                    object.remove("outcome");
                }
            }
        }
        "TOOL_CALL_RESULT" => {
            if let Some(content) = object.get_mut("content") {
                strip_parts(content, "TOOL_CALL_RESULT.content", warnings);
            }
        }
        "MESSAGES_SNAPSHOT" => {
            if let Some(messages) = object.get_mut("messages") {
                strip_messages(messages, "MESSAGES_SNAPSHOT.messages", warnings);
            }
        }
        "RUN_STARTED" => {
            if let Some(Value::Object(input)) = object.get_mut("input")
                && let Some(messages) = input.get_mut("messages")
            {
                strip_messages(messages, "RUN_STARTED.input.messages", warnings);
            }
        }
        "STATE_DELTA" => {
            if let Some(patch) = object.get_mut("delta") {
                strip_patch(patch, "STATE_DELTA.delta", warnings);
            }
        }
        "ACTIVITY_DELTA" => {
            if let Some(patch) = object.get_mut("patch") {
                strip_patch(patch, "ACTIVITY_DELTA.patch", warnings);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Removes array elements that are objects whose `key` names a member the
/// union does not describe. Anything else is left for serde to judge.
fn retain_known(
    list: &mut Value,
    key: &str,
    known: &[&str],
    path: &str,
    warnings: &mut Vec<String>,
) {
    let Value::Array(items) = list else {
        return;
    };
    items.retain(|item| match item.get(key) {
        Some(Value::String(kind)) if !known.contains(&kind.as_str()) => {
            warnings.push(format!("stripped unrecognised {key} '{kind}' from {path}"));
            false
        }
        _ => true,
    });
}

fn strip_parts(content: &mut Value, path: &str, warnings: &mut Vec<String>) {
    retain_known(content, "type", KNOWN_PART_TYPES, path, warnings);
}

fn strip_messages(messages: &mut Value, path: &str, warnings: &mut Vec<String>) {
    retain_known(messages, "role", KNOWN_ROLES, path, warnings);
    if let Value::Array(items) = messages {
        for message in items {
            if let Some(content) = message.get_mut("content") {
                strip_parts(content, &format!("{path}[].content"), warnings);
            }
        }
    }
}

fn strip_patch(patch: &mut Value, path: &str, warnings: &mut Vec<String>) {
    retain_known(patch, "op", KNOWN_PATCH_OPS, path, warnings);
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Best effort: names the top-level field a serde error is about. Events
/// flatten their shared base, which costs serde its field path, so this reads
/// the offending value back out of the message and finds the one field
/// holding it.
fn locate_field(object: &Map<String, Value>, detail: &str) -> Option<String> {
    if let Some(name) = between(detail, "missing field `", "`") {
        return Some(name.to_owned());
    }
    let wanted: Value = if let Some(variant) = between(detail, "unknown variant `", "`") {
        Value::String(variant.to_owned())
    } else if let Some(number) = between(detail, "invalid type: integer `", "`") {
        serde_json::from_str(number).ok()?
    } else if let Some(number) = between(detail, "invalid type: floating point `", "`") {
        serde_json::from_str(number).ok()?
    } else if let Some(boolean) = between(detail, "invalid type: boolean `", "`") {
        serde_json::from_str(boolean).ok()?
    } else {
        let string = between(detail, "invalid type: string \"", "\"")?;
        Value::String(string.to_owned())
    };
    let mut matches = object
        .iter()
        .filter(|(key, value)| key.as_str() != "type" && **value == wanted);
    let (field, _) = matches.next()?;
    // Ambiguous when two fields hold the same value; say nothing rather
    // than name the wrong one.
    matches.next().is_none().then(|| field.clone())
}

fn between<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = haystack.find(start)? + start.len();
    let len = haystack[from..].find(end)?;
    Some(&haystack[from..from + len])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;

    use super::*;

    #[test]
    fn unknown_outcome_is_stripped_but_an_array_is_fatal() {
        let stripped = decode_event(json!({
            "type": "RUN_FINISHED", "threadId": "t", "runId": "r",
            "outcome": { "type": "paused" },
        }))
        .unwrap();
        let Some(Event::RunFinished(finished)) = stripped.event else {
            panic!("expected RUN_FINISHED");
        };
        assert!(finished.outcome.is_none());
        assert_eq!(stripped.warnings.len(), 1);

        let err = decode_event(json!({
            "type": "RUN_FINISHED", "threadId": "t", "runId": "r", "outcome": [],
        }))
        .unwrap_err();
        assert!(err.to_string().contains("expected object, received array"));
    }

    #[test]
    fn unknown_list_members_are_stripped() {
        let decoded = decode_event(json!({
            "type": "STATE_DELTA",
            "delta": [
                { "op": "increment", "path": "/n", "value": 1 },
                { "op": "replace", "path": "/n", "value": 5 },
            ],
        }))
        .unwrap();
        let Some(Event::StateDelta(delta)) = decoded.event else {
            panic!("expected STATE_DELTA");
        };
        assert_eq!(delta.delta.len(), 1);

        let decoded = decode_event(json!({
            "type": "MESSAGES_SNAPSHOT",
            "messages": [
                { "id": "u", "role": "user", "content": [
                    { "type": "text", "text": "a" },
                    { "type": "hologram" },
                ] },
                { "id": "n", "role": "narrator", "content": "?" },
            ],
        }))
        .unwrap();
        let Some(Event::MessagesSnapshot(snapshot)) = decoded.event else {
            panic!("expected MESSAGES_SNAPSHOT");
        };
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(decoded.warnings.len(), 2);
    }

    #[test]
    fn errors_name_the_field_when_they_can() {
        let err = decode_event(json!({
            "type": "TEXT_MESSAGE_START", "messageId": "m", "role": "narrator",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("field 'role'"), "{err}");

        let err =
            decode_event(json!({ "type": "SUBAGENT_STARTED", "subagentRunId": "s" })).unwrap_err();
        assert!(err.to_string().contains("name"), "{err}");

        assert!(decode_event(json!([])).is_err());
        assert!(decode_event(json!({ "type": 1 })).is_err());
    }
}
