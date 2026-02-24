//! Event serialization tests
//!
//! These tests verify that events are correctly serialized to JSON format
//! for storage in the events table.

use everruns_core::events::{EventContext, InputMessageData, ToolCompletedData};
use everruns_core::message::Message;
use everruns_core::typed_id::SessionId;
use everruns_core::{ContentPart, Event};
use uuid::Uuid;

#[test]
fn test_event_serialization() {
    let session_id = SessionId::new();
    let event_context = EventContext::empty();
    let event = Event::new(
        session_id,
        event_context,
        InputMessageData::new(Message::user("test")),
    );

    let json = serde_json::to_value(&event).unwrap();

    assert!(json.is_object());
    assert_eq!(json["type"], "input.message");
    // session_id now uses prefixed format (session_{hex})
    let expected_session_id = format!("session_{}", session_id.uuid().simple());
    assert_eq!(json["session_id"], expected_session_id);
    assert!(json["context"].is_object());
}

#[test]
fn test_event_type() {
    let session_id = SessionId::new();
    let event_context = EventContext::empty();
    let event = Event::new(
        session_id,
        event_context,
        InputMessageData::new(Message::user("test")),
    );

    assert_eq!(event.event_type, "input.message");
}

#[test]
fn test_event_session_id() {
    let raw_uuid = Uuid::now_v7();
    let session_id = SessionId::from_uuid(raw_uuid);
    let event_context = EventContext::empty();
    let event = Event::new(
        session_id,
        event_context,
        InputMessageData::new(Message::user("test")),
    );

    assert_eq!(event.session_uuid(), raw_uuid);
}

#[test]
fn test_tool_call_completed_event_serialization() {
    // This test verifies the exact JSON structure that the UI expects
    let session_id = SessionId::new();
    let completed = ToolCompletedData::success(
        "call_abc123".to_string(),
        "get_weather".to_string(),
        vec![ContentPart::text("Sunny, 72°F")],
        Some(150),
    );
    let event = Event::new(session_id, EventContext::empty(), completed);

    let json = serde_json::to_value(&event).unwrap();
    println!(
        "tool.completed event JSON:\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    // Verify top-level structure
    assert_eq!(json["type"], "tool.completed");
    // session_id now uses prefixed format (session_{hex})
    let expected_session_id = format!("session_{}", session_id.uuid().simple());
    assert_eq!(json["session_id"], expected_session_id);

    // Verify data field contains the payload directly (untagged)
    let data = &json["data"];
    assert_eq!(data["tool_call_id"], "call_abc123");
    assert_eq!(data["tool_name"], "get_weather");
    assert_eq!(data["success"], true);
    assert_eq!(data["status"], "success");

    // Verify result is an array of ContentPart
    let result = &data["result"];
    assert!(result.is_array());
    assert_eq!(result[0]["type"], "text");
    assert_eq!(result[0]["text"], "Sunny, 72°F");
}

#[test]
fn test_tool_call_completed_error_serialization() {
    let session_id = SessionId::new();
    let completed = ToolCompletedData::failure(
        "call_xyz789".to_string(),
        "read_file".to_string(),
        "error".to_string(),
        "File not found".to_string(),
        Some(50),
    );
    let event = Event::new(session_id, EventContext::empty(), completed);

    let json = serde_json::to_value(&event).unwrap();
    println!(
        "tool.completed error event JSON:\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    let data = &json["data"];
    assert_eq!(data["tool_call_id"], "call_xyz789");
    assert_eq!(data["success"], false);
    assert_eq!(data["error"], "File not found");
}

// ----------------------------------------------------------------------------
// Events API Contract Tests
// ----------------------------------------------------------------------------

/// Verify that unsupported events cannot be serialized (serde skip)
/// This ensures they can never leak to API responses.
#[test]
fn test_unsupported_event_cannot_serialize() {
    use everruns_core::events::deserialize_event_data;

    // Create an unsupported event by deserializing unknown type
    let data = deserialize_event_data("future.unknown.event", serde_json::json!({"foo": "bar"}));

    // Verify it's unsupported
    assert!(data.is_unsupported(), "Should be unsupported event");

    // Verify event type is "unsupported"
    assert_eq!(data.event_type(), "unsupported");

    // Create event with unsupported data
    let session_id = SessionId::new();
    let event = Event::new(session_id, EventContext::empty(), data);

    // Serialization SHOULD FAIL - this is the contract guarantee
    // Unsupported events must be filtered before serialization
    let result = serde_json::to_value(&event);
    assert!(
        result.is_err(),
        "Unsupported events should fail serialization to prevent API leakage"
    );

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("cannot be serialized"),
        "Error should indicate serialization is not allowed: {}",
        err
    );

    println!(
        "Correctly prevented serialization of unsupported event: {}",
        err
    );
}

/// Verify Event structure matches API contract
#[test]
fn test_event_api_contract_structure() {
    let session_id = SessionId::new();
    let event = Event::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(Message::user("test")),
    );

    let json = serde_json::to_value(&event).unwrap();

    // Verify required fields per specs/events-contract.md
    assert!(json["id"].is_string(), "Event must have 'id' field");
    assert!(json["type"].is_string(), "Event must have 'type' field");
    assert!(json["ts"].is_string(), "Event must have 'ts' field");
    assert!(
        json["session_id"].is_string(),
        "Event must have 'session_id' field"
    );
    assert!(
        json["context"].is_object(),
        "Event must have 'context' field"
    );
    assert!(json["data"].is_object(), "Event must have 'data' field");

    // Verify ID format
    let id = json["id"].as_str().unwrap();
    assert!(
        id.starts_with("event_"),
        "Event ID should have 'event_' prefix"
    );

    // Verify session_id format
    let sid = json["session_id"].as_str().unwrap();
    assert!(
        sid.starts_with("session_"),
        "Session ID should have 'session_' prefix"
    );

    // Verify type is dot-notation
    let event_type = json["type"].as_str().unwrap();
    assert!(
        event_type.contains('.'),
        "Event type should use dot notation"
    );
}

/// Verify EventContext structure matches API contract
#[test]
fn test_event_context_api_contract() {
    use everruns_core::typed_id::{MessageId, TurnId};

    let turn_id = TurnId::new();
    let message_id = MessageId::new();
    let context = EventContext::turn(turn_id, message_id).with_span(
        "trace123".to_string(),
        "span456".to_string(),
        Some("parent789".to_string()),
    );

    let json = serde_json::to_value(&context).unwrap();

    // Verify optional fields are present when set
    assert!(json["turn_id"].is_string());
    assert!(json["input_message_id"].is_string());
    assert!(json["trace_id"].is_string());
    assert!(json["span_id"].is_string());
    assert!(json["parent_span_id"].is_string());

    // Verify ID prefixes
    let tid = json["turn_id"].as_str().unwrap();
    assert!(
        tid.starts_with("turn_"),
        "Turn ID should have 'turn_' prefix"
    );

    let mid = json["input_message_id"].as_str().unwrap();
    assert!(
        mid.starts_with("message_"),
        "Message ID should have 'message_' prefix"
    );
}
