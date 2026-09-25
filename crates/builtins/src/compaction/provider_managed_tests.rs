use super::*;
use crate::tool_types::ToolCall;
use serde_json::json;

fn make_message_tool_turn(
    call_id: &str,
    tool_name: &str,
    result: serde_json::Value,
) -> Vec<RuntimeMessage> {
    vec![
        RuntimeMessage::assistant_with_tools(
            "",
            vec![ToolCall {
                id: call_id.to_string(),
                name: tool_name.to_string(),
                arguments: json!({"path": "/workspace/src/lib.rs"}),
            }],
        ),
        RuntimeMessage::tool_result(call_id, Some(result), None),
    ]
}

#[test]
fn provider_managed_reduction_bypasses_generic_model_view_masking() {
    let mut messages = vec![RuntimeMessage::user("inspect files repeatedly")];
    for index in 0..9 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "read_file",
            json!({
                "path": "/workspace/src/lib.rs",
                "content": "large file line\n".repeat(1000),
                "total_lines": 1000,
                "lines_shown": {"start": 1, "end": 1000},
                "truncated": false
            }),
        ));
    }
    let original = messages.clone();
    let context = ModelViewContext {
        session_id: crate::typed_id::SessionId::new(),
        prior_usage: None,
        provider_managed_reduction: true,
    };

    let result = CompactionModelViewProvider.apply_model_view(messages, &json!({}), &context);

    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}

#[test]
fn test_model_view_masks_with_compaction_config() {
    let mut messages = vec![RuntimeMessage::user("inspect files repeatedly")];
    for index in 0..9 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "read_file",
            json!({
                "path": "/workspace/session_019e4c9dd1b17021af70ad3227361b16.jsonl",
                "content": format!("{}{}", "large transcript line\n".repeat(1000), index),
                "total_lines": 1000,
                "lines_shown": {"start": 1, "end": 1000},
                "truncated": false,
                "content_hash": format!("sha256:{index}")
            }),
        ));
    }

    let config = RuntimeCompactionConfig::default();
    let result = build_model_view_messages(&messages, &config, None);

    assert_eq!(result.masked_count, 7);
    assert!(result.tool_result_bytes_after < result.tool_result_bytes_before / 4);
    let first_tool = result.messages[2].tool_result_content().unwrap();
    let masked = first_tool.result.as_ref().unwrap();
    assert_eq!(masked["masked"], true);
    assert!(masked["summary"].as_str().unwrap().contains("read_file"));
    let last_tool = result
        .messages
        .last()
        .unwrap()
        .tool_result_content()
        .unwrap();
    assert!(last_tool.result.as_ref().unwrap().get("content").is_some());
}
