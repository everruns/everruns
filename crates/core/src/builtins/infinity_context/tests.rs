use super::*;
use crate::builtins::test_fixtures::TestMessageRetriever;
use crate::builtins::typed_id::SessionId;

// Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

#[test]
fn filter_only_view_preserves_filter_without_model_contributions() {
    let capability = InfinityContextFilterOnlyCapability;

    assert!(capability.message_filter_provider().is_some());
    assert!(capability.tools().is_empty());
    assert!(capability.system_prompt_addition().is_none());
}

#[test]
fn test_config_schema_and_validate_config() {
    let capability = InfinityContextCapability;

    let schema = capability.config_schema().expect("config schema");
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["context_budget_tokens"].is_object());
    assert!(schema["properties"]["min_recent_messages"].is_object());
    assert!(schema["properties"]["max_recent_messages"].is_object());
    assert_eq!(
        schema["properties"]["keep_first_messages"]["maximum"],
        MAX_KEEP_FIRST_MESSAGES
    );

    // Null, empty, and valid configs are accepted.
    assert!(capability.validate_config(&Value::Null).is_ok());
    assert!(capability.validate_config(&json!({})).is_ok());
    assert!(
        capability
            .validate_config(&json!({
                "context_budget_tokens": 50_000,
                "min_recent_messages": 5,
                "max_recent_messages": 100
            }))
            .is_ok()
    );

    // Wrong types and out-of-range values are rejected.
    assert!(
        capability
            .validate_config(&json!({"context_budget_tokens": "lots"}))
            .is_err()
    );
    assert!(
        capability
            .validate_config(&json!({"context_budget_tokens": 0}))
            .is_err()
    );
    assert!(
        capability
            .validate_config(&json!({"max_recent_messages": 0}))
            .is_err()
    );
    assert!(
        capability
            .validate_config(&json!({
                "keep_first_messages": MAX_KEEP_FIRST_MESSAGES + 1
            }))
            .is_err()
    );
}

#[test]
fn test_filter_provider_sets_bounded_candidate_load_limit_without_hard_cap() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({"context_budget_tokens": 1_000, "min_recent_messages": 3}),
    );

    assert_eq!(query.limit, Some(16));
    assert!(query.prepend_transform.is_some());
    // The safe default omits the head anchor so a large untrusted first
    // message cannot bypass the configured budget or recent-message cap.
    assert_eq!(query.keep_head, None);
}

#[test]
fn test_filter_provider_sets_keep_head_from_keep_first_messages() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({"context_budget_tokens": 1_000, "keep_first_messages": 3}),
    );
    assert_eq!(query.keep_head, Some(3));
}

#[test]
fn test_filter_provider_omits_keep_head_when_zero() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({"context_budget_tokens": 1_000, "keep_first_messages": 0}),
    );
    assert_eq!(query.keep_head, None);
}

#[test]
fn test_filter_provider_caps_keep_head_for_unvalidated_config() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({
            "context_budget_tokens": 1_000,
            "keep_first_messages": usize::MAX
        }),
    );

    assert_eq!(query.keep_head, Some(MAX_KEEP_FIRST_MESSAGES));
}

#[test]
fn test_filter_provider_caps_explicit_max_to_bounded_candidate_window() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({
            "context_budget_tokens": 500_000,
            "min_recent_messages": 10,
            "max_recent_messages": 1_000_000
        }),
    );

    assert_eq!(query.limit, Some(CANDIDATE_MAX_MESSAGES as i64));
    assert!(query.prepend_transform.is_some());
}

#[test]
fn test_filter_provider_caps_large_min_recent_messages() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({
            "context_budget_tokens": 1_000,
            "min_recent_messages": 1_000_000,
        }),
    );

    assert_eq!(query.limit, Some(CANDIDATE_MAX_MESSAGES as i64));
    assert!(query.prepend_transform.is_some());
}

#[test]
fn test_filter_provider_allows_small_public_chat_window() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({
            "context_budget_tokens": 10_000,
            "min_recent_messages": 10,
            "max_recent_messages": 30
        }),
    );

    assert_eq!(query.limit, Some(30));
    assert!(query.prepend_transform.is_some());
}

#[test]
fn test_filter_provider_falls_back_to_defaults_for_invalid_config() {
    let mut query = MessageQuery::new(SessionId::new());
    let provider = InfinityContextFilterProvider;
    provider.apply_filters(
        &mut query,
        &json!({"context_budget_tokens": "not-a-number"}),
    );

    assert_eq!(query.limit, Some(1_600));
    assert!(query.prepend_transform.is_some());
}

#[test]
fn test_filter_provider_trims_loaded_messages_by_token_budget() {
    let provider = InfinityContextFilterProvider;
    let mut messages = vec![
        RuntimeMessage::user("the original task"),
        RuntimeMessage::assistant("old ".repeat(400)),
        RuntimeMessage::user("recent one"),
        RuntimeMessage::assistant("recent two"),
    ];

    provider.post_load(
        &mut messages,
        &json!({"context_budget_tokens": 1, "min_recent_messages": 2}),
    );

    // Default configuration has no head anchor: a first user message must not
    // bypass the configured token budget or recent-message cap.
    assert_eq!(messages.len(), 3);
    assert!(
        extract_text_content(&messages[0]).contains("2 earlier messages are not in this context")
    );
    assert_eq!(extract_text_content(&messages[1]), "recent one");
    assert_eq!(extract_text_content(&messages[2]), "recent two");
    assert!(
        !messages
            .iter()
            .any(|m| extract_text_content(m) == "the original task")
    );
}

#[test]
fn test_filter_provider_applies_hard_cap_after_loading() {
    let provider = InfinityContextFilterProvider;
    let mut messages = vec![
        RuntimeMessage::user("one"),
        RuntimeMessage::assistant("two"),
        RuntimeMessage::user("three"),
        RuntimeMessage::assistant("four"),
        RuntimeMessage::user("five"),
    ];

    provider.post_load(
        &mut messages,
        &json!({
            "context_budget_tokens": 10_000,
            "min_recent_messages": 10,
            "max_recent_messages": 2
        }),
    );

    // Hard cap keeps only the 2 recent messages by default.
    assert_eq!(messages.len(), 3);
    assert!(
        extract_text_content(&messages[0]).contains("3 earlier messages are not in this context")
    );
    assert_eq!(extract_text_content(&messages[1]), "four");
    assert_eq!(extract_text_content(&messages[2]), "five");
}

#[test]
fn test_filter_provider_anchors_task_through_full_flow() {
    let provider = InfinityContextFilterProvider;
    // Budget large enough that the candidate load fetches every message, but
    // small enough that the big middle turns are dropped by token budget.
    let config = json!({
        "context_budget_tokens": 600,
        "min_recent_messages": 2,
        "keep_first_messages": 1
    });
    let mut query = MessageQuery::new(SessionId::new());
    provider.apply_filters(&mut query, &config);
    let mut messages = vec![
        RuntimeMessage::user("TASK: build the widget"),
        RuntimeMessage::assistant("X".repeat(2000)),
        RuntimeMessage::assistant("Y".repeat(2000)),
        RuntimeMessage::user("recent a"),
        RuntimeMessage::assistant("recent b"),
    ];

    query.apply_windowing(&mut messages);
    provider.post_load(&mut messages, &config);

    // The original task is anchored at the front, the notice follows it, and
    // the recent tail is intact — the model still knows what it is doing.
    assert_eq!(extract_text_content(&messages[0]), "TASK: build the widget");
    assert!(
        extract_text_content(&messages[1]).contains("earlier messages are not in this context")
    );
    assert_eq!(extract_text_content(messages.last().unwrap()), "recent b");
    // The huge first assistant turn was dropped from the middle.
    assert!(
        !messages
            .iter()
            .any(|m| extract_text_content(m).starts_with("XXX"))
    );
}

#[test]
fn message_filter_hook_coordinates_with_compaction_without_changing_user_config() {
    let base = json!({ "context_budget_tokens": 1000 });

    let coordinated = message_filter_config(&base, true, false);
    assert_eq!(coordinated["compaction_active"], json!(true));
    assert_eq!(coordinated["context_budget_tokens"], json!(1000));
    assert!(base.get("compaction_active").is_none());

    assert_eq!(message_filter_config(&base, false, false), base);
    assert_eq!(
        message_filter_config(&Value::Null, true, false),
        json!({
            "compaction_active": true,
            "provider_managed_reduction": false
        })
    );
}

#[test]
fn test_filter_provider_defers_eviction_to_compaction() {
    let provider = InfinityContextFilterProvider;
    let mut messages = vec![
        RuntimeMessage::user("task"),
        RuntimeMessage::assistant("old ".repeat(400)),
        RuntimeMessage::user("recent one"),
        RuntimeMessage::assistant("recent two"),
    ];

    provider.post_load(
        &mut messages,
        &json!({
            "context_budget_tokens": 1,
            "min_recent_messages": 2,
            "compaction_active": true
        }),
    );

    // Compaction owns reduction: infinity context neither trims nor injects a
    // notice when compaction is active.
    assert_eq!(messages.len(), 4);
    assert!(
        messages
            .iter()
            .all(|m| !extract_text_content(m).contains("not in this context"))
    );
}

#[test]
fn test_filter_provider_caps_keep_first_messages_during_post_load() {
    let provider = InfinityContextFilterProvider;
    let mut messages: Vec<RuntimeMessage> = (0..20)
        .map(|idx| RuntimeMessage::user(format!("message {idx}")))
        .collect();

    provider.post_load(
        &mut messages,
        &json!({
            "context_budget_tokens": 1,
            "min_recent_messages": 1,
            "keep_first_messages": usize::MAX
        }),
    );

    assert_eq!(extract_text_content(&messages[0]), "message 0");
    assert_eq!(
        extract_text_content(&messages[MAX_KEEP_FIRST_MESSAGES - 1]),
        format!("message {}", MAX_KEEP_FIRST_MESSAGES - 1)
    );
    assert!(extract_text_content(&messages[MAX_KEEP_FIRST_MESSAGES]).contains("are not in"));
    assert_eq!(extract_text_content(messages.last().unwrap()), "message 19");
}

#[test]
fn test_filter_provider_default_drops_oversized_first_message() {
    let provider = InfinityContextFilterProvider;
    let mut messages = vec![
        RuntimeMessage::user("attacker ".repeat(20_000)),
        RuntimeMessage::assistant("middle"),
        RuntimeMessage::user("recent one"),
        RuntimeMessage::assistant("recent two"),
    ];

    provider.post_load(
        &mut messages,
        &json!({
            "context_budget_tokens": 10,
            "min_recent_messages": 2,
            "max_recent_messages": 2
        }),
    );

    assert_eq!(messages.len(), 3);
    assert!(
        extract_text_content(&messages[0]).contains("2 earlier messages are not in this context")
    );
    assert_eq!(extract_text_content(&messages[1]), "recent one");
    assert_eq!(extract_text_content(&messages[2]), "recent two");
    assert!(
        !messages
            .iter()
            .any(|m| extract_text_content(m).starts_with("attacker"))
    );
}

#[test]
fn test_filter_provider_keep_first_messages_anchors_multiple() {
    let provider = InfinityContextFilterProvider;
    let mut messages = vec![
        RuntimeMessage::user("anchor one"),
        RuntimeMessage::user("anchor two"),
        RuntimeMessage::assistant("mid ".repeat(400)),
        RuntimeMessage::user("recent"),
    ];

    provider.post_load(
        &mut messages,
        &json!({
            "context_budget_tokens": 1,
            "min_recent_messages": 1,
            "keep_first_messages": 2
        }),
    );

    assert_eq!(extract_text_content(&messages[0]), "anchor one");
    assert_eq!(extract_text_content(&messages[1]), "anchor two");
    assert!(extract_text_content(&messages[2]).contains("not in this context"));
    assert_eq!(extract_text_content(messages.last().unwrap()), "recent");
}

#[test]
fn test_estimate_json_value_len_matches_serialized_length() {
    let value = json!({
        "stdout": ["alpha", "beta"],
        "ok": true,
        "count": 2
    });

    assert_eq!(
        estimate_json_value_len(&value),
        serde_json::to_string(&value).unwrap().len()
    );
}

#[tokio::test]
async fn test_query_history_tool_errors_without_retriever() {
    let tool = QueryHistoryTool;
    let result = tool
        .execute_with_context(json!({"query": "api"}), &ToolContext::new(SessionId::new()))
        .await;

    match result {
        ToolExecutionResult::ToolError(message) => {
            assert!(message.contains("No message retriever available"));
        }
        other => panic!("expected tool error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_query_history_tool_rejects_invalid_params() {
    let result = QueryHistoryTool.execute(json!({"limit": "oops"})).await;

    match result {
        ToolExecutionResult::ToolError(message) => {
            assert!(message.contains("requires session context"));
        }
        other => panic!("expected tool error, got {other:?}"),
    }

    let session_id = SessionId::new();
    let retriever = TestMessageRetriever::new();
    let result = QueryHistoryTool
        .execute_with_context(
            json!({"message_range": {"from": "bad", "to": 1}}),
            &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
        )
        .await;

    match result {
        ToolExecutionResult::ToolError(message) => {
            assert!(message.contains("Invalid parameters"));
        }
        other => panic!("expected tool error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_query_history_tool_empty_history() {
    let session_id = SessionId::new();
    let retriever = TestMessageRetriever::new();

    let result = QueryHistoryTool
        .execute_with_context(
            json!({}),
            &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["count"], 0);
            assert_eq!(value["message"], "No history available.");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn test_query_history_tool_searches_history() {
    let session_id = SessionId::new();
    let retriever = TestMessageRetriever::new();
    retriever
        .seed(
            session_id,
            vec![
                RuntimeMessage::user("First topic"),
                RuntimeMessage::assistant("The API key is abc123"),
                RuntimeMessage::user("We should keep discussing logging"),
            ],
        )
        .await;

    let result = QueryHistoryTool
        .execute_with_context(
            json!({"query": "api key"}),
            &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["count"], 1);
            assert_eq!(value["messages"][0]["content"], "The API key is abc123");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn test_query_history_tool_search_no_match() {
    let session_id = SessionId::new();
    let retriever = TestMessageRetriever::new();
    retriever
        .seed(
            session_id,
            vec![
                RuntimeMessage::user("one"),
                RuntimeMessage::assistant("two"),
            ],
        )
        .await;

    let result = QueryHistoryTool
        .execute_with_context(
            json!({"query": "missing"}),
            &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["count"], 0);
            assert_eq!(value["message"], "No matching messages found.");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn test_query_history_tool_reads_range() {
    let session_id = SessionId::new();
    let retriever = TestMessageRetriever::new();
    retriever
        .seed(
            session_id,
            vec![
                RuntimeMessage::user("one"),
                RuntimeMessage::assistant("two"),
                RuntimeMessage::user("three"),
            ],
        )
        .await;

    let result = QueryHistoryTool
        .execute_with_context(
            json!({"message_range": {"from": 1, "to": 3}, "limit": 10}),
            &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["count"], 2);
            assert_eq!(value["messages"][0]["content"], "two");
            assert_eq!(value["messages"][1]["content"], "three");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn test_query_history_tool_clamps_out_of_bounds_range() {
    let session_id = SessionId::new();
    let retriever = TestMessageRetriever::new();
    retriever
        .seed(
            session_id,
            vec![
                RuntimeMessage::user("one"),
                RuntimeMessage::assistant("two"),
                RuntimeMessage::user("three"),
            ],
        )
        .await;

    let result = QueryHistoryTool
        .execute_with_context(
            json!({"message_range": {"from": 99, "to": 100}}),
            &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["count"], 0);
            assert_eq!(value["message"], "No messages in the requested range.");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[test]
fn test_truncate_content_is_utf8_safe() {
    let truncated = truncate_content("hello🙂world", 6);
    assert_eq!(truncated, "hello🙂...");
}

#[test]
fn trim_preserves_locally_unmatched_tool_result_for_stateful_responses() {
    use crate::builtins::tool_types::ToolCall;

    let provider = InfinityContextFilterProvider;
    // min_recent_messages=3 keeps the last 3 messages. With a 1-token budget the
    // two older messages are dropped, including the assistant tool call. The
    // OpenAI Responses path may still have that call in previous_response_id
    // state, so InfinityContext must not drop the tool output before provider
    // serialization decides whether stateful continuation is active.
    let mut messages = vec![
        RuntimeMessage::user("old question"),
        RuntimeMessage::assistant_with_tools(
            "calling tool",
            vec![ToolCall {
                id: "call_old".to_string(),
                name: "edit_file".to_string(),
                arguments: serde_json::json!({}),
            }],
        ),
        // This tool result is in the min-recent window but its call is trimmed away.
        RuntimeMessage::tool_result("call_old", Some(serde_json::json!("done")), None),
        RuntimeMessage::user("new question"),
        RuntimeMessage::assistant("answer"),
    ];

    provider.post_load(
        &mut messages,
        &serde_json::json!({"context_budget_tokens": 1, "min_recent_messages": 3}),
    );

    assert!(
        messages
            .iter()
            .any(|m| m.role == RuntimeMessageRole::ToolResult),
        "locally unmatched tool result must be preserved until provider serialization"
    );

    let llm_messages = messages
        .iter()
        .map(crate::builtins::llm_conversions::llm_message_from_message)
        .collect();
    let stateless_view = crate::retain_complete_llm_tool_exchanges_for_request(llm_messages, false);
    assert!(
        stateless_view
            .iter()
            .all(|message| message.tool_call_id.is_none()),
        "a stateless runtime view must drop a result whose call was trimmed"
    );
}

#[test]
fn trim_removes_a_visible_call_when_its_result_is_evicted() {
    use crate::builtins::tool_types::ToolCall;

    let provider = InfinityContextFilterProvider;
    let mut messages = vec![
        RuntimeMessage::user("original task"),
        RuntimeMessage::assistant_with_tools(
            "calling tool",
            vec![ToolCall {
                id: "call_old".to_string(),
                name: "bash".to_string(),
                arguments: serde_json::json!({}),
            }],
        ),
        RuntimeMessage::tool_result(
            "call_old",
            Some(serde_json::json!("large result ".repeat(500))),
            None,
        ),
        RuntimeMessage::user("recent question"),
        RuntimeMessage::assistant("recent answer"),
    ];

    provider.post_load(
        &mut messages,
        &serde_json::json!({
            "context_budget_tokens": 1,
            "min_recent_messages": 2,
            "keep_first_messages": 2
        }),
    );

    assert!(
        messages
            .iter()
            .flat_map(RuntimeMessage::tool_calls)
            .next()
            .is_none(),
        "the anchored assistant message must not retain a call after its result is hidden"
    );
}

#[test]
fn trim_keeps_tool_result_when_tool_call_is_visible() {
    use crate::builtins::tool_types::ToolCall;

    let provider = InfinityContextFilterProvider;
    // All 3 messages fit in the window: no orphan expected.
    let mut messages = vec![
        RuntimeMessage::assistant_with_tools(
            "calling tool",
            vec![ToolCall {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            }],
        ),
        RuntimeMessage::tool_result("call_1", Some(serde_json::json!("content")), None),
        RuntimeMessage::user("thanks"),
    ];

    provider.post_load(
        &mut messages,
        &serde_json::json!({"context_budget_tokens": 100_000, "min_recent_messages": 10}),
    );

    assert!(
        messages
            .iter()
            .any(|m| m.role == RuntimeMessageRole::ToolResult),
        "tool result must be kept when its tool call is visible"
    );
}
