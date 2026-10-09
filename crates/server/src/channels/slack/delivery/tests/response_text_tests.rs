//! What counts as reply text in an `output.message.completed` event.

use super::*;

#[test]
fn test_extract_response_text_valid() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "text", "text": "Hello from the agent!"},
                {"type": "text", "text": "Second part."}
            ]
        }
    });
    let result = extract_response_text(&data);
    assert_eq!(
        result,
        Some("Hello from the agent!\nSecond part.".to_string())
    );
}

#[test]
fn test_extract_response_text_single_part() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "text", "text": "Only one part."}
            ]
        }
    });
    assert_eq!(
        extract_response_text(&data),
        Some("Only one part.".to_string())
    );
}

#[test]
fn test_extract_response_text_no_text() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "tool_use", "name": "search"}
            ]
        }
    });
    assert_eq!(extract_response_text(&data), None);
}

#[test]
fn test_extract_response_text_empty_content() {
    let data = serde_json::json!({
        "message": {
            "content": []
        }
    });
    assert_eq!(extract_response_text(&data), None);
}

#[test]
fn test_extract_response_text_missing_message() {
    let data = serde_json::json!({});
    assert_eq!(extract_response_text(&data), None);
}

#[test]
fn test_extract_response_text_missing_content() {
    let data = serde_json::json!({
        "message": {}
    });
    assert_eq!(extract_response_text(&data), None);
}

#[test]
fn test_extract_response_text_mixed_content() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "text", "text": "Part 1"},
                {"type": "tool_use", "name": "search"},
                {"type": "text", "text": "Part 2"}
            ]
        }
    });
    // tool_use part causes early return via `?` operator on type check
    // Only "Part 1" is extracted before the tool_use part returns None from the iterator
    let result = extract_response_text(&data);
    // The `?` in part.get("type")?.as_str()? causes the for loop to
    // short-circuit the entire function when a non-text part doesn't have
    // the expected structure. But tool_use does have "type", so it just
    // doesn't match "text" and the `&&` short-circuits. Both text parts
    // should be captured.
    assert_eq!(result, Some("Part 1\nPart 2".to_string()));
}

/// EVERRUNS-28: an assistant message that only calls tools can still carry an
/// empty text part. Treating that as a reply posted an empty message, which
/// Slack refuses with `no_text`, so a turn's tool-call step paged as an error.
#[test]
fn blank_text_parts_are_not_a_reply() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "text", "text": ""},
                {"type": "tool_use", "name": "search"},
                {"type": "text", "text": "  \n\t"}
            ]
        }
    });
    assert_eq!(extract_response_text(&data), None);
    assert_eq!(
        extract_delivery_text(
            "output.message.completed",
            SlackReplyMode::AllMessages,
            &data
        ),
        None
    );
}

#[test]
fn blank_parts_do_not_pad_a_real_reply() {
    let data = serde_json::json!({
        "message": {
            "content": [
                {"type": "text", "text": ""},
                {"type": "text", "text": "Answer."}
            ]
        }
    });
    assert_eq!(extract_response_text(&data), Some("Answer.".to_string()));
}
