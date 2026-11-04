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
    build_mcp_proxy_tools(
        &definitions,
        Arc::new(ScopedMcpToolInvoker::new(&definitions, invoker)),
    )
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

#[tokio::test]
async fn registered_resources_preserve_input_binding_and_allowed_results() {
    use crate::runtime::tool_execution::ToolExecutor;
    use crate::runtime::typed_id::{MessageId, SessionId};

    #[derive(Default)]
    struct InertResources(Option<uuid::Uuid>);
    #[async_trait]
    impl McpToolInvoker for InertResources {
        fn for_execution(&self, input: uuid::Uuid) -> Option<Arc<dyn McpToolInvoker>> {
            Some(Arc::new(Self(Some(input))))
        }
        async fn invoke(&self, _: &ToolCall) -> crate::runtime::error::Result<ToolResult> {
            unreachable!("resource fixture never executes MCP tools")
        }
        async fn list_resources(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> crate::runtime::error::Result<Option<McpResourceOutcome<McpResourceListing>>> {
            Ok(Some(McpResourceOutcome::Done(McpResourceListing {
                page: McpResourcesPage {
                    next_cursor: Some(self.0.map_or("unbound".into(), |id| id.to_string())),
                    ..Default::default()
                },
                ..Default::default()
            })))
        }
        async fn read_resource(
            &self,
            _: &str,
            uri: &str,
        ) -> crate::runtime::error::Result<Option<McpResourceOutcome<McpReadResourceResult>>>
        {
            Ok(Some(McpResourceOutcome::Done(McpReadResourceResult {
                contents: vec![McpResourceContents {
                    uri: uri.into(),
                    text: Some(self.0.map_or("unbound".into(), |id| id.to_string())),
                    mime_type: None,
                    blob: None,
                }],
            })))
        }
    }

    let mut definitions = vec![mcp_tool("mcp_docs__search"), mcp_tool("mcp_wiki")];
    add_mcp_resource_tool_definitions(&mut definitions);
    let scoped = Arc::new(ScopedMcpToolInvoker::new(
        &definitions,
        Arc::new(InertResources::default()),
    ));
    let mut registry = crate::runtime::tools::ToolRegistry::new();
    for tool in build_mcp_proxy_tools(&definitions, scoped.clone()) {
        registry.register_boxed(tool);
    }
    let input = MessageId::from_seed(1);
    for bound in [false, true] {
        let mut context = ToolContext::new(SessionId::from_seed(2));
        context.mcp_invoker = Some(scoped.clone());
        if bound {
            context.event_context = Some(crate::runtime::events::EventContext {
                input_message_id: Some(input),
                ..Default::default()
            });
        }
        let expected = if bound {
            input.uuid().to_string()
        } else {
            "unbound".into()
        };
        for (name, server) in [
            (LIST_MCP_RESOURCES_TOOL, "docs"),
            (READ_MCP_RESOURCE_TOOL, "wiki"),
        ] {
            let call = ToolCall {
                id: "fixture".into(),
                name: name.into(),
                arguments: if name == READ_MCP_RESOURCE_TOOL {
                    json!({"server": server, "uri": "fixture://document"})
                } else {
                    json!({"server": server})
                },
            };
            let definition = definitions.iter().find(|d| d.name() == name).unwrap();
            let result = registry
                .execute_with_context(&call, definition, &context)
                .await
                .unwrap();
            assert!(result.error.is_none(), "{result:?}");
            let value = result.result.unwrap();
            if name == READ_MCP_RESOURCE_TOOL {
                assert_eq!(value["contents"][0]["text"], expected);
            } else {
                assert_eq!(value["next_cursor"], expected);
            }
        }
    }
}
