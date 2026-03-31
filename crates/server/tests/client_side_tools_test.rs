//! Unit-style tests for client-side tool calls feature.
//!
//! Tests serialization, deserialization, and type behavior for:
//! - Agent with client-side tools
//! - SubmitToolResultsRequest/Response
//! - ToolCallRequestedData event
//! - Session tools propagation
//!
//! No database required.
//!
//! Run with: cargo test -p everruns-server --test client_side_tools_test

use serde_json::json;

// ============================================
// Agent with Client-Side Tools
// ============================================

#[test]
fn test_agent_with_client_side_tools_serialization() {
    let agent_json = json!({
        "id": "agent_550e8400e29b41d4a716446655440000",
        "name": "Browser Agent",
        "system_prompt": "You control a browser.",
        "status": "active",
        "tags": [],
        "tools": [
            {
                "type": "client_side",
                "name": "browser_click",
                "description": "Click an element",
                "parameters": {"type": "object", "properties": {"selector": {"type": "string"}}}
            }
        ],
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    });

    let agent: everruns_core::Agent = serde_json::from_value(agent_json).unwrap();
    assert_eq!(agent.tools.len(), 1);
    assert_eq!(agent.tools[0].name(), "browser_click");
    assert!(matches!(
        &agent.tools[0],
        everruns_core::ToolDefinition::ClientSide(_)
    ));
}

#[test]
fn test_agent_with_mixed_tools_serialization() {
    let agent_json = json!({
        "id": "agent_550e8400e29b41d4a716446655440000",
        "name": "Mixed Agent",
        "system_prompt": "You have both tool types.",
        "status": "active",
        "tags": [],
        "tools": [
            {
                "type": "builtin",
                "name": "fetch_data",
                "description": "Fetch from URL",
                "parameters": {"type": "object"}
            },
            {
                "type": "client_side",
                "name": "run_terminal",
                "description": "Run a terminal command",
                "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}
            }
        ],
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    });

    let agent: everruns_core::Agent = serde_json::from_value(agent_json).unwrap();
    assert_eq!(agent.tools.len(), 2);

    assert!(matches!(
        &agent.tools[0],
        everruns_core::ToolDefinition::Builtin(_)
    ));
    assert!(matches!(
        &agent.tools[1],
        everruns_core::ToolDefinition::ClientSide(_)
    ));

    // Roundtrip
    let serialized = serde_json::to_value(&agent).unwrap();
    let tools = serialized["tools"].as_array().unwrap();
    assert_eq!(tools[0]["type"], "builtin");
    assert_eq!(tools[1]["type"], "client_side");
}

#[test]
fn test_agent_with_no_tools_omits_field() {
    let agent_json = json!({
        "id": "agent_550e8400e29b41d4a716446655440000",
        "name": "No Tools Agent",
        "system_prompt": "No tools.",
        "status": "active",
        "tags": [],
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    });

    let agent: everruns_core::Agent = serde_json::from_value(agent_json).unwrap();
    assert!(agent.tools.is_empty());

    // Serialized output should omit tools field when empty (skip_serializing_if)
    let serialized = serde_json::to_value(&agent).unwrap();
    assert!(serialized.get("tools").is_none());
}

// ============================================
// SubmitToolResultsRequest Serialization
// ============================================

#[test]
fn test_submit_tool_results_request_success() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": [
            {
                "tool_call_id": "call_abc123",
                "result": {"status": "deployed", "url": "https://staging.app"}
            }
        ]
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.tool_results.len(), 1);
    assert_eq!(req.tool_results[0].tool_call_id, "call_abc123");
    assert!(req.tool_results[0].result.is_some());
    assert!(req.tool_results[0].error.is_none());
}

#[test]
fn test_submit_tool_results_request_error() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": [
            {
                "tool_call_id": "call_fail1",
                "error": "Connection timeout after 30s"
            }
        ]
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.tool_results[0].tool_call_id, "call_fail1");
    assert!(req.tool_results[0].result.is_none());
    assert_eq!(
        req.tool_results[0].error.as_ref().unwrap(),
        "Connection timeout after 30s"
    );
}

#[test]
fn test_submit_tool_results_request_mixed() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": [
            {"tool_call_id": "call_1", "result": 42},
            {"tool_call_id": "call_2", "error": "not found"},
            {"tool_call_id": "call_3", "result": null},
            {"tool_call_id": "call_4", "result": {"nested": true}}
        ]
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.tool_results.len(), 4);
    assert_eq!(req.tool_results[0].tool_call_id, "call_1");
    assert!(req.tool_results[0].error.is_none());
    assert_eq!(req.tool_results[1].error.as_deref(), Some("not found"));
    // null result still deserializes as None
    assert!(req.tool_results[2].result.is_none());
    assert!(req.tool_results[3].result.is_some());
}

#[test]
fn test_submit_tool_results_request_empty_results() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": []
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert!(req.tool_results.is_empty());
}

#[test]
fn test_submit_tool_results_response_serialization() {
    use everruns_server::api::tool_results::SubmitToolResultsResponse;

    let resp = SubmitToolResultsResponse {
        accepted: 3,
        status: "active".to_string(),
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["accepted"], 3);
    assert_eq!(json["status"], "active");
}

// ============================================
// ToolCallRequestedData Serialization
// ============================================

#[test]
fn test_tool_call_requested_data_serialization() {
    use everruns_core::ToolCall;
    use everruns_core::events::ToolCallRequestedData;

    let data = ToolCallRequestedData {
        tool_calls: vec![
            ToolCall {
                id: "call_abc".to_string(),
                name: "browser_click".to_string(),
                arguments: json!({"selector": "#submit-btn"}),
            },
            ToolCall {
                id: "call_def".to_string(),
                name: "browser_type".to_string(),
                arguments: json!({"selector": "#input", "text": "hello"}),
            },
        ],
        tool_summaries: vec![],
        headline: None,
    };

    let json = serde_json::to_value(&data).unwrap();
    let tool_calls = json["tool_calls"].as_array().unwrap();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0]["id"], "call_abc");
    assert_eq!(tool_calls[0]["name"], "browser_click");
    assert_eq!(tool_calls[0]["arguments"]["selector"], "#submit-btn");
    assert_eq!(tool_calls[1]["id"], "call_def");
    assert_eq!(tool_calls[1]["name"], "browser_type");
}

#[test]
fn test_tool_call_requested_data_roundtrip() {
    use everruns_core::ToolCall;
    use everruns_core::events::ToolCallRequestedData;

    let original = ToolCallRequestedData {
        tool_calls: vec![ToolCall {
            id: "call_xyz".to_string(),
            name: "deploy_staging".to_string(),
            arguments: json!({"env": "staging", "version": "1.2.3"}),
        }],
        tool_summaries: vec![],
        headline: None,
    };

    let json_str = serde_json::to_string(&original).unwrap();
    let parsed: ToolCallRequestedData = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].id, "call_xyz");
    assert_eq!(parsed.tool_calls[0].name, "deploy_staging");
    assert_eq!(parsed.tool_calls[0].arguments["env"], "staging");
}

#[test]
fn test_tool_call_requested_data_empty_tool_calls() {
    use everruns_core::events::ToolCallRequestedData;

    let data = ToolCallRequestedData {
        tool_calls: vec![],
        tool_summaries: vec![],
        headline: None,
    };

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["tool_calls"].as_array().unwrap().len(), 0);
}

// ============================================
// Client-Side Tool Definition in Session Context
// ============================================

#[test]
fn test_client_side_tool_in_session_tools_json() {
    // Verify client-side tools survive JSON roundtrip in session-like contexts
    let tools_json = json!([
        {
            "type": "client_side",
            "name": "screenshot",
            "description": "Take a screenshot of the current page",
            "parameters": {"type": "object"}
        },
        {
            "type": "client_side",
            "name": "navigate",
            "description": "Navigate to a URL",
            "parameters": {
                "type": "object",
                "properties": {"url": {"type": "string", "format": "uri"}},
                "required": ["url"]
            }
        }
    ]);

    let tools: Vec<everruns_core::ToolDefinition> =
        serde_json::from_value(tools_json.clone()).unwrap();
    assert_eq!(tools.len(), 2);

    for tool in &tools {
        assert!(matches!(tool, everruns_core::ToolDefinition::ClientSide(_)));
        assert_eq!(tool.policy(), &everruns_core::ToolPolicy::ClientSide);
    }

    // Roundtrip
    let serialized = serde_json::to_value(&tools).unwrap();
    assert_eq!(serialized, tools_json);
}

// ============================================
// ToolCall + ToolResult Correlation
// ============================================

#[test]
fn test_tool_call_and_result_correlation() {
    use everruns_core::{ToolCall, ToolResult};

    let tool_call = ToolCall {
        id: "call_corr123".to_string(),
        name: "run_command".to_string(),
        arguments: json!({"cmd": "ls -la"}),
    };

    let tool_result = ToolResult {
        tool_call_id: tool_call.id.clone(),
        result: Some(json!({"output": "total 42\n..."})),
        images: None,
        error: None,
        connection_required: None,
        raw_output: None,
    };

    // IDs correlate
    assert_eq!(tool_call.id, tool_result.tool_call_id);

    // Both serialize/deserialize correctly
    let call_json = serde_json::to_string(&tool_call).unwrap();
    let result_json = serde_json::to_string(&tool_result).unwrap();

    let parsed_call: ToolCall = serde_json::from_str(&call_json).unwrap();
    let parsed_result: ToolResult = serde_json::from_str(&result_json).unwrap();

    assert_eq!(parsed_call.id, parsed_result.tool_call_id);
}
