// Deferred MCP servers revealed through `tool_search` (user MCP servers D6).

use super::*;
use crate::builtins::tools::ToolRegistry;
use crate::host::InMemorySessionStorageStore;
use crate::session_services::SessionStorageStore;

struct NeverCalled;

#[async_trait]
impl crate::McpToolInvoker for NeverCalled {
    async fn invoke(
        &self,
        call: &crate::builtins::tool_types::ToolCall,
    ) -> everruns_contracts::error::Result<crate::builtins::tool_types::ToolResult> {
        panic!("revealing a server must not call it: {}", call.name)
    }
}

fn mcp_tool(name: &str) -> ToolDefinition {
    ToolDefinition::Builtin(crate::builtins::tool_types::BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: "Create an issue".to_string(),
        parameters: json!({"type": "object", "properties": {"title": {"type": "string"}}}),
        policy: crate::builtins::tool_types::ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::Automatic,
        hints: ToolHints::default(),
        full_parameters: None,
    })
}

#[tokio::test]
async fn search_matching_a_deferred_server_reveals_it_and_its_tools() {
    let cap = ToolSearchCapability::with_threshold(2);
    let session = SessionId::new();
    let storage = Arc::new(InMemorySessionStorageStore::new());

    // The turn carries the placeholder only: no tools were listed.
    let placeholder = crate::deferred_mcp_server_definition("linear", Some("Issue tracking"));
    let mut registry = ToolRegistry::new();
    for tool in crate::build_mcp_proxy_tools(&[placeholder], Arc::new(NeverCalled)) {
        registry.register_boxed(tool);
    }
    let mut ctx = ToolContext::with_storage_store(session, storage.clone());
    ctx.tool_registry = Some(Arc::new(registry));
    ctx.visible_tool_names = Some(Arc::new(HashSet::from(["mcp_linear".to_string()])));

    let result = cap.tools()[0]
        .execute_with_context(json!({ "query": "linear issues" }), &ctx)
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };
    assert_eq!(value["loading_mcp_servers"], json!(["linear"]));
    assert_eq!(
        crate::revealed_mcp_servers(storage.as_ref() as &dyn SessionStorageStore, session).await,
        HashSet::from(["linear".to_string()])
    );

    // Next step: the server's listed tools keep their full schemas without a
    // second search, while other deferred tools stay stubbed.
    let hooks = cap.tool_definition_hooks_with_context(
        &SystemPromptContext::without_file_store(session),
        &json!({}),
    );
    let next = hooks[0].transform(vec![
        mcp_tool("mcp_linear__create_issue"),
        mcp_tool("mcp_github__create_issue"),
        mcp_tool("other"),
    ]);
    let schema = |name: &str| {
        next.iter()
            .find(|tool| tool.name() == name)
            .unwrap()
            .parameters()
            .clone()
    };
    assert!(
        schema("mcp_linear__create_issue")
            .get("properties")
            .is_some()
    );
    assert!(
        schema("mcp_github__create_issue")
            .get("properties")
            .is_none()
    );
}

#[tokio::test]
async fn ordinary_matches_reveal_no_server() {
    let cap = ToolSearchCapability::with_threshold(2);
    let session = SessionId::new();
    let storage = Arc::new(InMemorySessionStorageStore::new());
    let mut registry = ToolRegistry::new();
    for tool in crate::build_mcp_proxy_tools(
        &[mcp_tool("mcp_linear__create_issue")],
        Arc::new(NeverCalled),
    ) {
        registry.register_boxed(tool);
    }
    let mut ctx = ToolContext::with_storage_store(session, storage.clone());
    ctx.tool_registry = Some(Arc::new(registry));
    ctx.visible_tool_names = Some(Arc::new(HashSet::from([
        "mcp_linear__create_issue".to_string()
    ])));

    let result = cap.tools()[0]
        .execute_with_context(json!({ "query": "create issue" }), &ctx)
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success");
    };
    assert!(value.get("loading_mcp_servers").is_none());
    assert!(
        crate::revealed_mcp_servers(storage.as_ref() as &dyn SessionStorageStore, session)
            .await
            .is_empty()
    );
}
