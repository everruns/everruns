use super::*;
use crate::runtime::mcp_proxy::{ScopedMcpToolInvoker, build_mcp_proxy_tools};
use crate::runtime::mcp_server::{McpResource, McpResourcesPage};
use crate::runtime::tool_types::ToolResult;
use std::sync::Mutex;

fn mcp_tool(name: &str) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: String::new(),
        parameters: json!({"type": "object"}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
        full_parameters: None,
    })
}

/// Invoker standing in for a server `docs` that publishes resources.
#[derive(Default)]
struct DocsServer {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl McpToolInvoker for DocsServer {
    async fn invoke(&self, _tool_call: &ToolCall) -> crate::runtime::error::Result<ToolResult> {
        unreachable!("resource tools never call tools")
    }

    async fn list_resources(
        &self,
        server_prefix: &str,
        cursor: Option<&str>,
    ) -> crate::runtime::error::Result<Option<McpResourceOutcome<McpResourceListing>>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("list {server_prefix} {cursor:?}"));
        Ok(Some(McpResourceOutcome::Done(McpResourceListing {
            page: McpResourcesPage {
                resources: vec![McpResource {
                    uri: "file:///readme.md".into(),
                    name: "readme.md".into(),
                    mime_type: Some("text/markdown".into()),
                    ..Default::default()
                }],
                next_cursor: Some("page-2".into()),
            },
            templates: vec![McpResourceTemplate {
                uri_template: "file:///{path}".into(),
                name: "files".into(),
                ..Default::default()
            }],
        })))
    }

    async fn read_resource(
        &self,
        server_prefix: &str,
        uri: &str,
    ) -> crate::runtime::error::Result<Option<McpResourceOutcome<McpReadResourceResult>>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("read {server_prefix} {uri}"));
        let contents = match uri {
            "file:///readme.md" => vec![McpResourceContents {
                uri: uri.into(),
                mime_type: Some("text/markdown".into()),
                text: Some("# Hello".into()),
                blob: None,
            }],
            "file:///chart.png" => vec![McpResourceContents {
                uri: uri.into(),
                mime_type: Some("image/png".into()),
                text: None,
                blob: Some("AAAA".into()),
            }],
            _ => vec![McpResourceContents {
                uri: uri.into(),
                mime_type: Some("application/pdf".into()),
                text: None,
                blob: Some("AAAAAAAA".into()),
            }],
        };
        Ok(Some(McpResourceOutcome::Done(McpReadResourceResult {
            contents,
        })))
    }
}

fn tool(name: &str, invoker: Arc<dyn McpToolInvoker>) -> Box<dyn Tool> {
    let mut definitions = vec![mcp_tool("mcp_docs__search")];
    add_mcp_resource_tool_definitions(&mut definitions);
    build_mcp_proxy_tools(&definitions, invoker)
        .into_iter()
        .find(|t| t.name() == name)
        .expect("resource tool registered")
}

#[test]
fn definitions_join_only_turns_with_an_mcp_server() {
    let mut none = vec![];
    add_mcp_resource_tool_definitions(&mut none);
    assert!(none.is_empty());

    let mut with_server = vec![mcp_tool("mcp_docs__search")];
    add_mcp_resource_tool_definitions(&mut with_server);
    add_mcp_resource_tool_definitions(&mut with_server);
    let names: Vec<&str> = with_server.iter().map(|d| d.name()).collect();
    assert_eq!(
        names,
        [
            "mcp_docs__search",
            "list_mcp_resources",
            "read_mcp_resource"
        ]
    );

    // A deferred server's placeholder counts as an attached server.
    let mut deferred = vec![mcp_tool("mcp_linear")];
    add_mcp_resource_tool_definitions(&mut deferred);
    assert_eq!(deferred.len(), 3);
}

#[test]
fn resource_tools_are_read_only_and_never_deferred_placeholders() {
    let list = tool(LIST_MCP_RESOURCES_TOOL, Arc::new(DocsServer::default()));
    assert_eq!(list.hints().readonly, Some(true));
    assert!(matches!(list.deferrable_policy(), DeferrablePolicy::Never));
    assert!(crate::runtime::mcp_deferred::deferred_mcp_server_prefix(list.name()).is_none());
}

#[tokio::test]
async fn lists_resources_templates_and_cursor_for_a_sanitized_server() {
    let server = Arc::new(DocsServer::default());
    let list = tool(LIST_MCP_RESOURCES_TOOL, server.clone());
    let result = list.execute(json!({ "server": "Docs" })).await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("expected success, got {result:?}");
    };
    assert_eq!(value["server"], "docs");
    assert_eq!(value["resources"][0]["uri"], "file:///readme.md");
    assert_eq!(value["resources"][0]["mime_type"], "text/markdown");
    assert_eq!(
        value["resource_templates"][0]["uri_template"],
        "file:///{path}"
    );
    assert_eq!(value["next_cursor"], "page-2");
    assert_eq!(server.calls.lock().unwrap()[0], "list docs None");
}

#[tokio::test]
async fn reads_text_inline_images_as_images_and_describes_other_binaries() {
    let read = tool(READ_MCP_RESOURCE_TOOL, Arc::new(DocsServer::default()));

    let text = read
        .execute(json!({ "server": "docs", "uri": "file:///readme.md" }))
        .await;
    let ToolExecutionResult::Success(value) = text else {
        panic!("expected success, got {text:?}");
    };
    assert_eq!(value["contents"][0]["text"], "# Hello");

    let image = read
        .execute(json!({ "server": "docs", "uri": "file:///chart.png" }))
        .await;
    let ToolExecutionResult::SuccessWithImages { result, images } = image else {
        panic!("expected an image, got {image:?}");
    };
    assert_eq!(images[0].media_type, "image/png");
    assert_eq!(result["contents"][0]["image"], true);

    // No session files outside a context: described, never inlined.
    let binary = read
        .execute(json!({ "server": "docs", "uri": "file:///report.pdf" }))
        .await;
    let ToolExecutionResult::Success(value) = binary else {
        panic!("expected success, got {binary:?}");
    };
    assert_eq!(value["contents"][0]["size_bytes"], 6);
    assert!(value["contents"][0].get("blob").is_none());
}

#[tokio::test]
async fn missing_arguments_are_tool_errors() {
    let read = tool(READ_MCP_RESOURCE_TOOL, Arc::new(DocsServer::default()));
    let result = read.execute(json!({ "server": "docs" })).await;
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "{result:?}"
    );
    let result = read.execute(json!({ "uri": "file:///x" })).await;
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "{result:?}"
    );
}

#[tokio::test]
async fn scoped_invoker_only_reaches_servers_attached_to_the_turn() {
    let definitions = vec![mcp_tool("mcp_docs__search"), mcp_tool("mcp_linear")];
    let server = Arc::new(DocsServer::default());
    let scoped = ScopedMcpToolInvoker::new(&definitions, server.clone());

    assert!(scoped.list_resources("docs", None).await.is_ok());
    // A deferred server is attached too.
    assert!(scoped.read_resource("linear", "x://y").await.is_ok());

    let err = scoped
        .read_resource("billing", "file:///secret")
        .await
        .expect_err("unattached server");
    let message = err.to_string();
    assert!(message.contains("not attached"), "{message}");
    assert!(message.contains("docs, linear"), "{message}");
    assert_eq!(server.calls.lock().unwrap().len(), 2);
}
