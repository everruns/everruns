//! Integration tests: tool execute_with_context against wiremock Cursor API.

use async_trait::async_trait;
use everruns_core::capabilities::Capability;
use everruns_core::error::Result;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::typed_id::SessionId;
use everruns_core::{connection_services::UserConnectionResolver, tool_context::ToolContext};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use everruns_integrations_cursor as _;

static ENV_LOCK: Mutex<()> = Mutex::const_new(());

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

struct MockConnectionResolver {
    token: Option<String>,
}

#[async_trait]
impl UserConnectionResolver for MockConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(self.token.clone())
    }
}

fn resolver(token: Option<&str>) -> Arc<dyn UserConnectionResolver> {
    Arc::new(MockConnectionResolver {
        token: token.map(str::to_string),
    })
}

fn get_tool(name: &str) -> Box<dyn Tool> {
    everruns_integrations_cursor::CursorCapability
        .tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

fn context_with_token(token: Option<&str>) -> ToolContext {
    ToolContext::new(SessionId::new()).with_connection_resolver(resolver(token))
}

#[tokio::test]
async fn launch_agent_tool_posts_expected_flow() {
    let _lock = ENV_LOCK.lock().await;
    let mock_server = MockServer::start().await;
    let _base = EnvGuard::set(
        everruns_integrations_cursor::CURSOR_API_BASE_ENV,
        mock_server.uri(),
    );

    Mock::given(method("POST"))
        .and(path("/v0/agents"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "bc_abc123",
            "name": "Fix checkout bug",
            "status": "CREATING",
            "source": { "repository": "https://github.com/acme/app", "ref": "main" },
            "target": {
                "url": "https://cursor.com/agents?id=bc_abc123",
                "branchName": "cursor/fix-checkout",
                "autoCreatePr": true
            },
            "createdAt": "2026-01-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let tool = get_tool("cursor_launch_agent");
    let result = tool
        .execute_with_context(
            json!({
                "name": "Fix checkout bug",
                "prompt": "Fix checkout bug and add regression tests.",
                "repository": "https://github.com/acme/app",
                "ref": "main",
                "branch_name": "cursor/fix-checkout",
                "auto_create_pr": true
            }),
            &context_with_token(Some("test_key")),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["id"], "bc_abc123");
            assert_eq!(value["status"], "CREATING");
            assert_eq!(value["target"]["branchName"], "cursor/fix-checkout");
        }
        other => panic!("Expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn get_agent_tool_reads_status() {
    let _lock = ENV_LOCK.lock().await;
    let mock_server = MockServer::start().await;
    let _base = EnvGuard::set(
        everruns_integrations_cursor::CURSOR_API_BASE_ENV,
        mock_server.uri(),
    );

    Mock::given(method("GET"))
        .and(path("/v0/agents/bc_abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "bc_abc123",
            "name": "Fix checkout bug",
            "status": "RUNNING",
            "source": { "repository": "https://github.com/acme/app", "ref": "main" },
            "target": {
                "url": "https://cursor.com/agents?id=bc_abc123",
                "branchName": "cursor/fix-checkout",
                "autoCreatePr": false
            },
            "createdAt": "2026-01-01T00:00:00Z",
            "summary": "Working on tests"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let tool = get_tool("cursor_get_agent");
    let result = tool
        .execute_with_context(
            json!({"agent_id": "bc_abc123"}),
            &context_with_token(Some("test_key")),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["id"], "bc_abc123");
            assert_eq!(value["status"], "RUNNING");
            assert_eq!(value["summary"], "Working on tests");
        }
        other => panic!("Expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn followup_tool_posts_instruction() {
    let _lock = ENV_LOCK.lock().await;
    let mock_server = MockServer::start().await;
    let _base = EnvGuard::set(
        everruns_integrations_cursor::CURSOR_API_BASE_ENV,
        mock_server.uri(),
    );

    Mock::given(method("POST"))
        .and(path("/v0/agents/bc_abc123/followup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "bc_abc123"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let tool = get_tool("cursor_add_followup");
    let result = tool
        .execute_with_context(
            json!({"agent_id": "bc_abc123", "prompt": "Also update docs."}),
            &context_with_token(Some("test_key")),
        )
        .await;

    match result {
        ToolExecutionResult::Success(value) => assert_eq!(value["id"], "bc_abc123"),
        other => panic!("Expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn list_models_tool_reads_models() {
    let _lock = ENV_LOCK.lock().await;
    let mock_server = MockServer::start().await;
    let _base = EnvGuard::set(
        everruns_integrations_cursor::CURSOR_API_BASE_ENV,
        mock_server.uri(),
    );

    Mock::given(method("GET"))
        .and(path("/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": ["composer-1.5", "default"]
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let tool = get_tool("cursor_list_models");
    let result = tool
        .execute_with_context(json!({}), &context_with_token(Some("test_key")))
        .await;

    match result {
        ToolExecutionResult::Success(value) => {
            assert_eq!(value["models"][0], "composer-1.5");
        }
        other => panic!("Expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_token_prompts_connection() {
    let tool = get_tool("cursor_get_agent");
    let result = tool
        .execute_with_context(json!({"agent_id": "bc_abc123"}), &context_with_token(None))
        .await;

    match result {
        ToolExecutionResult::ConnectionRequired { provider } => assert_eq!(provider, "cursor"),
        other => panic!("Expected ConnectionRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn launch_agent_rejects_empty_prompt() {
    let tool = get_tool("cursor_launch_agent");
    let result = tool
        .execute_with_context(
            json!({"prompt": "", "repository": "https://github.com/acme/app"}),
            &context_with_token(Some("test_key")),
        )
        .await;

    match result {
        ToolExecutionResult::ToolError(message) => {
            assert!(message.contains("Missing required parameter: prompt"));
        }
        other => panic!("Expected ToolError, got {other:?}"),
    }
}
