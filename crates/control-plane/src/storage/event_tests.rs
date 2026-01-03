//! Event serialization tests
//!
//! These tests verify that events are correctly serialized to JSON format
//! for storage in the events table.

use everruns_core::events::{EventContext, InputReceivedData, ToolCallCompletedData};
use everruns_core::message::Message;
use everruns_core::{ContentPart, Event};
use uuid::Uuid;

#[test]
fn test_event_serialization() {
    let session_id = Uuid::now_v7();
    let event_context = EventContext::empty();
    let event = Event::new(
        session_id,
        event_context,
        InputReceivedData::new(Message::user("test")),
    );

    let json = serde_json::to_value(&event).unwrap();

    assert!(json.is_object());
    assert_eq!(json["type"], "input.received");
    assert_eq!(json["session_id"], session_id.to_string());
    assert!(json["context"].is_object());
}

#[test]
fn test_event_type() {
    let session_id = Uuid::now_v7();
    let event_context = EventContext::empty();
    let event = Event::new(
        session_id,
        event_context,
        InputReceivedData::new(Message::user("test")),
    );

    assert_eq!(event.event_type, "input.received");
}

#[test]
fn test_event_session_id() {
    let session_id = Uuid::now_v7();
    let event_context = EventContext::empty();
    let event = Event::new(
        session_id,
        event_context,
        InputReceivedData::new(Message::user("test")),
    );

    assert_eq!(event.session_id(), session_id);
}

#[test]
fn test_tool_call_completed_event_serialization() {
    // This test verifies the exact JSON structure that the UI expects
    let session_id = Uuid::now_v7();
    let completed = ToolCallCompletedData::success(
        "call_abc123".to_string(),
        "get_weather".to_string(),
        vec![ContentPart::text("Sunny, 72°F")],
    );
    let event = Event::new(session_id, EventContext::empty(), completed);

    let json = serde_json::to_value(&event).unwrap();
    println!(
        "tool.call_completed event JSON:\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    // Verify top-level structure
    assert_eq!(json["type"], "tool.call_completed");
    assert_eq!(json["session_id"], session_id.to_string());

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
    let session_id = Uuid::now_v7();
    let completed = ToolCallCompletedData::failure(
        "call_xyz789".to_string(),
        "read_file".to_string(),
        "error".to_string(),
        "File not found".to_string(),
    );
    let event = Event::new(session_id, EventContext::empty(), completed);

    let json = serde_json::to_value(&event).unwrap();
    println!(
        "tool.call_completed error event JSON:\n{}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    let data = &json["data"];
    assert_eq!(data["tool_call_id"], "call_xyz789");
    assert_eq!(data["success"], false);
    assert_eq!(data["error"], "File not found");
}
