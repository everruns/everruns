use super::*;
use crate::validation::validate_interaction_steps;
use everruns_core::capabilities::Capability;

#[test]
fn test_validate_url_accepts_public_https_url() {
    assert!(validate_url("https://example.com").is_ok());
}

#[test]
fn test_validate_url_rejects_localhost() {
    let err = validate_url("http://localhost:3000").unwrap_err();
    match err {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("blocked"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[test]
fn test_validate_url_rejects_private_ip() {
    let err = validate_url("http://10.0.0.5/admin").unwrap_err();
    match err {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("blocked"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[test]
fn test_validate_url_rejects_cloud_metadata_ip() {
    let err = validate_url("http://169.254.169.254/latest/meta-data/").unwrap_err();
    match err {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("blocked"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[test]
fn test_validate_interaction_steps_rejects_blocked_navigate_url() {
    let steps = vec![json!({
        "action": "navigate",
        "value": "http://127.0.0.1:8080"
    })];
    let err = validate_interaction_steps(None, &steps).unwrap_err();
    match err {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("blocked"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

fn acl_context(patterns: &[&str], block: bool) -> ToolContext {
    use everruns_contracts::typed_id::SessionId;
    use everruns_core::network_access::NetworkAccessList;
    let mut context = ToolContext::new(SessionId::new());
    context.network_access = Some(if block {
        NetworkAccessList::block(patterns.iter().copied())
    } else {
        NetworkAccessList::allow_only(patterns.iter().copied())
    });
    context
}

async fn tool_error(tool: &dyn Tool, context: &ToolContext, args: Value) -> String {
    match tool.execute_with_context(args, context).await {
        ToolExecutionResult::ToolError(message) => message,
        other => panic!("expected a tool error, got {other:?}"),
    }
}

#[tokio::test]
async fn session_acl_blocks_a_public_host_before_any_request_and_keeps_an_allowed_host() {
    use crate::session_tools::BrowserlessOpenBrowserTool;

    let allow = acl_context(&["example.com"], false);
    let block = acl_context(&["evil.test"], true);
    let open = ToolContext::new(everruns_contracts::typed_id::SessionId::new());

    let cases: Vec<(Box<dyn Tool>, Value, Value)> = vec![
        (
            Box::new(BrowserlessScreenshotTool),
            json!({"url": "https://evil.test/private"}),
            json!({"url": "https://example.com/"}),
        ),
        (
            Box::new(BrowserlessContentTool),
            json!({"url": "https://evil.test/private"}),
            json!({"url": "https://example.com/"}),
        ),
        (
            Box::new(BrowserlessScrapeTool),
            json!({"url": "https://evil.test/private", "elements": [{"selector": "h1"}]}),
            json!({"url": "https://example.com/", "elements": [{"selector": "h1"}]}),
        ),
        (
            Box::new(BrowserlessInteractTool),
            json!({"url": "https://evil.test/private", "steps": [{"action": "wait", "wait_ms": 1}]}),
            json!({"url": "https://example.com/", "steps": [{"action": "wait", "wait_ms": 1}]}),
        ),
        (
            Box::new(BrowserlessNavigateTool),
            json!({"url": "https://evil.test/private"}),
            json!({"url": "https://example.com/"}),
        ),
        (
            Box::new(BrowserlessOpenBrowserTool),
            json!({"url": "https://evil.test/private"}),
            json!({"url": "https://example.com/"}),
        ),
    ];

    for (tool, denied, allowed) in cases {
        let name = tool.name().to_string();
        let denied_message = tool_error(tool.as_ref(), &allow, denied).await;
        assert!(
            denied_message.contains("network access"),
            "{name} should reject the denied host before calling Browserless: {denied_message}"
        );
        assert!(
            !denied_message.contains("API token"),
            "{name} consulted the connection before the ACL: {denied_message}"
        );

        let allowed_message = tool_error(tool.as_ref(), &allow, allowed).await;
        assert!(
            allowed_message.contains("API token"),
            "{name} should keep the allowed host usable past the ACL: {allowed_message}"
        );
        assert!(
            !allowed_message.contains("network access"),
            "{name} blocked an allowed host: {allowed_message}"
        );
    }

    // A blocklist denies the named public host and still allows others.
    let blocked = tool_error(
        &BrowserlessScreenshotTool,
        &block,
        json!({"url": "https://evil.test/"}),
    )
    .await;
    assert!(blocked.contains("network access"), "{blocked}");
    let not_blocked = tool_error(
        &BrowserlessScreenshotTool,
        &block,
        json!({"url": "https://example.com/"}),
    )
    .await;
    assert!(not_blocked.contains("API token"), "{not_blocked}");

    // No session ACL: a public host is not denied, a private host still is.
    let public_host = tool_error(
        &BrowserlessContentTool,
        &open,
        json!({"url": "https://evil.test/"}),
    )
    .await;
    assert!(public_host.contains("API token"), "{public_host}");
    let private_host = tool_error(
        &BrowserlessContentTool,
        &open,
        json!({"url": "http://10.1.2.3/admin"}),
    )
    .await;
    assert!(private_host.contains("blocked"), "{private_host}");
    assert!(!private_host.contains("API token"), "{private_host}");
}

#[tokio::test]
async fn interact_nested_navigate_honors_the_session_acl() {
    let allow = acl_context(&["example.com"], false);
    let message = tool_error(
        &BrowserlessInteractTool,
        &allow,
        json!({
            "url": "https://example.com/",
            "steps": [{ "action": "navigate", "value": "https://evil.test/next" }]
        }),
    )
    .await;
    assert!(message.contains("network access"), "{message}");
    assert!(!message.contains("API token"), "{message}");
}

#[test]
fn test_screenshot_tool_metadata() {
    let tool = BrowserlessScreenshotTool;
    assert_eq!(tool.name(), "browserless_screenshot");
    assert!(tool.requires_context());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("url")));
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn test_content_tool_metadata() {
    let tool = BrowserlessContentTool;
    assert_eq!(tool.name(), "browserless_content");
    assert!(tool.requires_context());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("url")));
}

#[test]
fn test_scrape_tool_metadata() {
    let tool = BrowserlessScrapeTool;
    assert_eq!(tool.name(), "browserless_scrape");
    assert!(tool.requires_context());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("url")));
    assert!(required.contains(&json!("elements")));
}

#[test]
fn test_interact_tool_metadata() {
    let tool = BrowserlessInteractTool;
    assert_eq!(tool.name(), "browserless_interact");
    assert!(tool.requires_context());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("url")));
    assert!(required.contains(&json!("steps")));
}

#[test]
fn test_navigate_tool_metadata() {
    let tool = BrowserlessNavigateTool;
    assert_eq!(tool.name(), "browserless_navigate");
    assert!(tool.requires_context());
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("url")));
}

#[test]
fn test_all_schemas_have_no_additional_properties() {
    let cap = crate::BrowserlessCapability;
    for tool in cap.tools() {
        let schema = tool.parameters_schema();
        assert_eq!(
            schema["additionalProperties"],
            false,
            "Tool {} should disallow additional properties",
            tool.name()
        );
    }
}

#[test]
fn test_should_suppress_interact_content_when_secrets_present() {
    assert!(should_suppress_interact_content(true, 1));
    assert!(!should_suppress_interact_content(false, 1));
    assert!(!should_suppress_interact_content(true, 0));
}

#[tokio::test]
async fn test_screenshot_tool_no_context_error() {
    let tool = BrowserlessScreenshotTool;
    let result = tool.execute(json!({"url": "https://example.com"})).await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("requires context"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_content_tool_no_context_error() {
    let tool = BrowserlessContentTool;
    let result = tool.execute(json!({"url": "https://example.com"})).await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("requires context"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

// ============================================================================
// EVE-339 — Reading-tool truncation envelope conformance
// ============================================================================

#[test]
fn test_truncate_html_under_cap() {
    let html = "<html><body>short</body></html>".to_string();
    let (content, was_truncated) = truncate_html(html);
    assert!(!was_truncated);
    let mut response = json!({
        "url": "https://example.com",
        "content": content.clone(),
        "size_bytes": 0,
        "truncated": was_truncated
    });
    attach_content_truncation(&mut response, &content, 0, false);
    everruns_core::truncation_info::assert_conforms("browserless_content", &response);
    assert_eq!(response["truncation"]["truncated"], false);
}

#[test]
fn test_truncate_html_over_cap_emits_without_resume() {
    // Build a >100 KB HTML source.
    let huge = "a".repeat(MAX_HTML_BYTES + 5_000);
    let total = huge.len();
    let (content, was_truncated) = truncate_html(huge);
    assert!(was_truncated);
    let mut response = json!({
        "url": "https://example.com",
        "content": content.clone(),
        "size_bytes": total,
        "truncated": was_truncated
    });
    attach_content_truncation(&mut response, &content, total, true);
    everruns_core::truncation_info::assert_conforms("browserless_content", &response);
    assert_eq!(response["truncation"]["truncated"], true);
    assert_eq!(response["truncation"]["reason"], "size_cap");
    assert_eq!(response["truncation"]["bytes_total"], total);
    assert!(
        response["truncation"].get("next_offset").is_none(),
        "browserless_content does not support in-place resume"
    );
}

#[test]
fn test_truncate_html_utf8_boundary_safe() {
    // Build a source whose byte-boundary for truncation would cut a
    // 4-byte emoji: pad with single-byte chars up to MAX_HTML_BYTES - 2,
    // then add the emoji. A naive byte-count cut at MAX_HTML_BYTES would
    // land in the middle of the emoji's bytes; boundary-safe truncation
    // must exclude the emoji entirely.
    let mut src = String::new();
    src.push_str(&"a".repeat(MAX_HTML_BYTES - 2));
    src.push('🚀');
    src.push_str(&"z".repeat(100));
    let total = src.len();
    let (content, was_truncated) = truncate_html(src);
    assert!(was_truncated);
    // Boundary-safe cut: no partial emoji bytes, only the padding 'a's
    // survive, and the truncation suffix is appended (not any trailing
    // 'z' from beyond the cap).
    assert!(content.is_char_boundary(content.len()));
    assert!(
        !content.contains('🚀'),
        "truncated content must not include the straddling emoji"
    );
    assert!(!content.contains('z'), "content after cap must not appear");
    assert!(total > MAX_HTML_BYTES);
}

#[tokio::test]
async fn test_scrape_tool_no_context_error() {
    let tool = BrowserlessScrapeTool;
    let result = tool
        .execute(json!({"url": "https://example.com", "elements": []}))
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("requires context"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_interact_tool_no_context_error() {
    let tool = BrowserlessInteractTool;
    let result = tool
        .execute(json!({"url": "https://example.com", "steps": []}))
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("requires context"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_navigate_tool_no_context_error() {
    let tool = BrowserlessNavigateTool;
    let result = tool.execute(json!({"url": "https://example.com"})).await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("requires context"));
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}
