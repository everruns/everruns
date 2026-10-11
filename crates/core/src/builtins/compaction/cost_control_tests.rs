use super::*;
use crate::builtins::tool_types::ToolCall;
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
fn test_cost_control_masks_old_read_file_results() {
    let mut messages = vec![RuntimeMessage::user("inspect files")];
    for index in 0..5 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "read_file",
            json!({
                "path": "/workspace/src/lib.rs",
                "content": format!("{}{}", "line\n".repeat(400), index),
                "total_lines": 900,
                "lines_shown": {"start": 1, "end": 400},
                "truncated": true,
                "content_hash": format!("sha256:{index}"),
                "truncation": {"truncated": true, "next_offset": 400, "reason": "line_cap"}
            }),
        ));
    }

    let config = RuntimeCompactionConfig::from_json(&json!({
        "cost_control": {
            "keep_recent_tool_results": 2,
            "mask_after_tool_results": 4,
            "min_masked_tool_result_bytes": 0
        }
    }));
    let result = apply_cost_control_masking(&messages, &config, None);

    assert_eq!(result.masked_count, 3);
    assert!(result.tool_result_bytes_after < result.tool_result_bytes_before);

    let first_tool = result.messages[2].tool_result_content().unwrap();
    let masked = first_tool.result.as_ref().unwrap();
    assert_eq!(masked["masked"], true);
    let summary = masked["summary"].as_str().unwrap();
    assert!(summary.contains("read_file"));
    assert!(summary.contains("/workspace/src/lib.rs"));
    assert!(summary.contains("lines 1-400"));
    assert!(summary.contains("next_offset=400"));
    assert!(!summary.contains("line\nline"));

    let last_tool = result
        .messages
        .last()
        .unwrap()
        .tool_result_content()
        .unwrap();
    assert!(last_tool.result.as_ref().unwrap().get("content").is_some());
}

#[test]
fn test_cost_control_keeps_recent_paginated_read_group() {
    let mut messages = vec![RuntimeMessage::user("inspect saved output")];
    messages.extend(make_message_tool_turn(
        "call_bash",
        "bash",
        json!({
            "stdout": "old command output",
            "stderr": "",
            "exit_code": 0,
            "success": true
        }),
    ));
    messages.extend(make_message_tool_turn(
        "call_read_first",
        "read_file",
        json!({
            "path": "/workspace/outputs/call_123.stdout",
            "content": "first page\n".repeat(200),
            "total_lines": 400,
            "lines_shown": {"start": 1, "end": 200},
            "truncated": true,
            "content_hash": "sha256:same-output",
            "truncation": {"truncated": true, "next_offset": 200, "reason": "line_cap"}
        }),
    ));
    messages.extend(make_message_tool_turn(
        "call_read_second",
        "read_file",
        json!({
            "path": "/workspace/outputs/call_123.stdout",
            "content": "second page\n".repeat(200),
            "total_lines": 400,
            "lines_shown": {"start": 201, "end": 400},
            "truncated": false,
            "content_hash": "sha256:same-output"
        }),
    ));

    let config = RuntimeCompactionConfig::from_json(&json!({
        "cost_control": {
            "keep_recent_tool_results": 1,
            "mask_after_tool_results": 2,
            "min_masked_tool_result_bytes": 0
        }
    }));
    let result = build_model_view_messages(&messages, &config, None);

    assert_eq!(result.masked_count, 1);
    let bash_result = result.messages[2].tool_result_content().unwrap();
    assert_eq!(bash_result.result.as_ref().unwrap()["masked"], true);
    let first_page = result.messages[4].tool_result_content().unwrap();
    assert!(first_page.result.as_ref().unwrap().get("content").is_some());
    let second_page = result.messages[6].tool_result_content().unwrap();
    assert!(
        second_page
            .result
            .as_ref()
            .unwrap()
            .get("content")
            .is_some()
    );
}

#[test]
fn test_compaction_capability_contributes_model_view_provider() {
    let mut messages = vec![RuntimeMessage::user("inspect files repeatedly")];
    for index in 0..9 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "read_file",
            json!({
                "path": "/workspace/src/lib.rs",
                "content": format!("{}{}", "large file line\n".repeat(1000), index),
                "total_lines": 1000,
                "lines_shown": {"start": 1, "end": 1000},
                "truncated": false
            }),
        ));
    }

    let capability = CompactionCapability;
    let provider = capability.model_view_provider().unwrap();
    let context = ModelViewContext {
        session_id: crate::builtins::typed_id::SessionId::new(),
        prior_usage: None,
        provider_managed_reduction: false,
    };
    let result = provider.apply_model_view(messages, &json!({}), &context);

    let first_tool = result[2].tool_result_content().unwrap();
    assert_eq!(first_tool.result.as_ref().unwrap()["masked"], true);
    let last_tool = result.last().unwrap().tool_result_content().unwrap();
    assert!(last_tool.result.as_ref().unwrap().get("content").is_some());
}

#[test]
fn test_model_view_respects_disabled_cost_control_config() {
    let mut messages = vec![RuntimeMessage::user("inspect files repeatedly")];
    for index in 0..5 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "read_file",
            json!({
                "path": "/workspace/src/lib.rs",
                "content": "line\n".repeat(400),
                "total_lines": 400,
                "lines_shown": {"start": 1, "end": 400},
                "truncated": false
            }),
        ));
    }

    let config = RuntimeCompactionConfig::from_json(&json!({
        "cost_control": {
            "enabled": false,
            "keep_recent_tool_results": 1,
            "mask_after_tool_results": 2
        }
    }));
    let result = build_model_view_messages(&messages, &config, None);

    assert_eq!(result.masked_count, 0);
    assert_eq!(
        result.tool_result_bytes_after,
        result.tool_result_bytes_before
    );
}

#[test]
fn test_cost_control_uses_prior_usage_signal() {
    let mut messages = vec![RuntimeMessage::user("run commands")];
    for index in 0..3 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "bash",
            json!({
                "stdout": "small output",
                "stderr": "",
                "exit_code": 0,
                "success": true
            }),
        ));
    }

    let config = RuntimeCompactionConfig::from_json(&json!({
        "cost_control": {
            "keep_recent_tool_results": 1,
            "mask_after_tool_results": 99,
            "max_live_tool_result_bytes": 999999,
            "min_masked_tool_result_bytes": 0,
            "max_uncached_input_tokens": 1000
        }
    }));
    let usage = TokenUsage::with_cache(10_000, 100, Some(0), None);
    let result = apply_cost_control_masking(&messages, &config, Some(&usage));

    assert_eq!(result.masked_count, 2);
    let first_tool = result.messages[2].tool_result_content().unwrap();
    let summary = first_tool.result.as_ref().unwrap()["summary"]
        .as_str()
        .unwrap();
    assert!(summary.contains("bash exit=0"));
}

/// A working set of small results (a pull request, its diff, a short
/// command) stays visible under the default policy even past the count
/// trigger: masking them made the model re-fetch them in a loop.
#[test]
fn test_cost_control_keeps_small_results_past_the_count_trigger() {
    let mut messages = vec![RuntimeMessage::user("review the pull request")];
    for index in 0..6 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "get_github_pull_request_diff",
            json!({ "diff": format!("{}{index}", "+ line\n".repeat(400)) }),
        ));
    }

    let config = RuntimeCompactionConfig::default();
    let result = apply_cost_control_masking(&messages, &config, None);
    assert_eq!(result.masked_count, 0);

    // Once a result is big enough to be worth it, it is masked as before.
    messages.extend(make_message_tool_turn(
        "call_big",
        "bash",
        json!({ "stdout": "x".repeat(8 * 1024), "exit_code": 0, "success": true }),
    ));
    let mut big_first = vec![RuntimeMessage::user("review the pull request")];
    big_first.extend(messages.split_off(messages.len() - 2));
    big_first.extend(messages.into_iter().skip(1));
    let result = apply_cost_control_masking(&big_first, &config, None);
    assert_eq!(result.masked_count, 1);
    let first_tool = result.messages[2].tool_result_content().unwrap();
    assert_eq!(first_tool.result.as_ref().unwrap()["masked"], true);
}

#[test]
fn test_cost_control_masking_uses_disjoint_cache_buckets() {
    // Disjoint convention: `input_tokens` is the non-cached prompt and the
    // cache-read ratio is measured against the full prompt (all buckets).
    let config = CostControlConfig {
        mask_after_tool_results: usize::MAX,
        max_live_tool_result_bytes: usize::MAX,
        max_uncached_input_tokens: 50_000,
        min_cache_read_ratio: 0.35,
        ..CostControlConfig::default()
    };

    // 20K non-cached input + 180K cache reads: a well-cached run (90% hit,
    // non-cached under threshold) — usage signals must not trigger masking.
    let well_cached = TokenUsage::with_cache(20_000, 100, Some(180_000), None);
    assert!(!should_apply_cost_control_masking(
        0,
        0,
        &config,
        Some(&well_cached)
    ));

    // Same total prompt but cache-poor (10% hit): low ratio triggers masking.
    let cache_poor = TokenUsage::with_cache(20_000, 100, Some(2_000), None);
    assert!(should_apply_cost_control_masking(
        0,
        0,
        &config,
        Some(&cache_poor)
    ));

    // High non-cached input alone triggers masking regardless of cache ratio.
    let heavy_uncached = TokenUsage::with_cache(60_000, 100, Some(200_000), None);
    assert!(should_apply_cost_control_masking(
        0,
        0,
        &config,
        Some(&heavy_uncached)
    ));
}

#[test]
fn test_model_view_uses_provider_cache_signal_from_compaction_config() {
    let mut messages = vec![RuntimeMessage::user("run commands")];
    for index in 0..3 {
        messages.extend(make_message_tool_turn(
            &format!("call_{index}"),
            "bash",
            json!({
                "stdout": "large output\n".repeat(400),
                "stderr": "",
                "exit_code": 0,
                "success": true
            }),
        ));
    }
    let usage = TokenUsage::with_cache(150_000, 100, Some(0), None);

    let config = RuntimeCompactionConfig::default();
    let result = build_model_view_messages(&messages, &config, Some(&usage));

    assert_eq!(result.masked_count, 1);
    let first_tool = result.messages[2].tool_result_content().unwrap();
    assert_eq!(first_tool.result.as_ref().unwrap()["masked"], true);
}
