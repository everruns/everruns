//! Deferred MCP servers ("load on demand") inside a shell call.

use super::*;
use everruns_contracts::runtime::mcp_deferred::{
    DEFERRED_MCP_REVEAL_KV_PREFIX, DeferredMcpServerTool, deferred_mcp_server_definition,
};
use everruns_contracts::runtime::mcp_proxy::{
    McpServerTools, McpToolInvoker, ScopedMcpToolInvoker,
};
use everruns_contracts::runtime::session_services::SessionStorageStore;
use everruns_contracts::runtime::tools::ToolRegistry;
use everruns_core::host::InMemorySessionStorageStore;

#[derive(Clone, Copy)]
enum Listing {
    Tools,
    NeedsConnection,
    CannotList,
}

/// One MCP server, `docs`, with one tool, `search_docs`, that echoes its input.
struct DocsServer {
    listing: Listing,
    listed: Mutex<usize>,
}

impl DocsServer {
    fn new(listing: Listing) -> Arc<Self> {
        Arc::new(Self {
            listing,
            listed: Mutex::new(0),
        })
    }
}

fn result(result: Option<Value>, error: Option<String>) -> ToolResult {
    ToolResult {
        details: None,
        tool_call_id: String::new(),
        result,
        images: None,
        error,
        connection_required: None,
        raw_output: None,
    }
}

#[async_trait]
impl McpToolInvoker for DocsServer {
    async fn invoke(&self, call: &ToolCall) -> everruns_contracts::error::Result<ToolResult> {
        Ok(result(
            Some(json!({"tool": call.name, "input": call.arguments})),
            None,
        ))
    }

    async fn list_server_tools(
        &self,
        server_prefix: &str,
        _session_id: uuid::Uuid,
    ) -> everruns_contracts::error::Result<Option<McpServerTools>> {
        assert_eq!(server_prefix, "docs");
        *self.listed.lock().unwrap() += 1;
        Ok(match self.listing {
            Listing::Tools => {
                let tool = BuiltinTool {
                    name: "mcp_docs__search_docs".into(),
                    display_name: None,
                    description: "Search the product docs.".into(),
                    parameters: json!({
                        "type": "object",
                        "properties": {"q": {"type": "string"}},
                        "required": ["q"]
                    }),
                    policy: ToolPolicy::Auto,
                    category: Some("MCP Servers".into()),
                    deferrable: DeferrablePolicy::Automatic,
                    hints: ToolHints {
                        readonly: Some(true),
                        metadata: Some(json!({"publisher": "docs-test"})),
                        ..ToolHints::default()
                    },
                    full_parameters: None,
                };
                Some(McpServerTools::Listed(vec![ToolDefinition::Builtin(tool)]))
            }
            Listing::NeedsConnection => Some(McpServerTools::ConnectionRequired(result(
                None,
                Some("MCP server 'docs' requires an OAuth connection.".into()),
            ))),
            Listing::CannotList => None,
        })
    }
}

struct Fixture {
    context: ToolContext,
    storage: Arc<InMemorySessionStorageStore>,
    server: Arc<DocsServer>,
}

fn fixture(listing: Listing) -> Fixture {
    fixture_with_metadata(
        listing,
        Some(json!({"mcp_server": {
            "name": "docs",
            "description": "Product documentation"
        }})),
    )
}

fn fixture_with_metadata(listing: Listing, metadata: Option<Value>) -> Fixture {
    let (mut context, _) = crate::bashkit::tests::create_context_with_mock_store();
    let mut placeholder = deferred_mcp_server_definition("docs", Some("Product documentation"));
    if let Some(metadata) = metadata
        && let ToolDefinition::Builtin(builtin) = &mut placeholder
    {
        builtin.hints.metadata = Some(metadata);
    }
    let ToolDefinition::Builtin(builtin) = placeholder.clone() else {
        unreachable!()
    };
    let mut registry = ToolRegistry::new();
    registry.register(DeferredMcpServerTool::new(builtin));
    registry.register(EchoTool::named("web_fetch"));
    registry.register(ToolsMarkerTool);
    install_cli_surface(&mut context, &registry);
    context.tool_registry = Some(Arc::new(registry));
    context.tool_call_id = Some("call_outer".to_string());
    let server = DocsServer::new(listing);
    // The turn's real scope rule: only the placeholder is in the definitions.
    let invoker: Arc<dyn McpToolInvoker> =
        Arc::new(ScopedMcpToolInvoker::new(&[placeholder], server.clone()));
    context.mcp_invoker = Some(invoker);
    let storage = Arc::new(InMemorySessionStorageStore::new());
    context.storage_store = Some(storage.clone());
    context = context.with_nested_tool_policy(Arc::new(Policy::default()));
    Fixture {
        context,
        storage,
        server,
    }
}

async fn revealed(fixture: &Fixture) -> bool {
    fixture
        .storage
        .get_value(
            fixture.context.session_id,
            &format!("{DEFERRED_MCP_REVEAL_KV_PREFIX}docs"),
        )
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn help_lists_a_deferred_server_as_not_loaded_without_loading_it() {
    let fixture = fixture(Listing::Tools);
    let output = run("tools --help", &fixture.context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.contains("docs  (not loaded"), "{stdout}");
    assert!(!stdout.contains("mcp-docs"), "{stdout}");
    assert_eq!(*fixture.server.listed.lock().unwrap(), 0);
    assert!(!revealed(&fixture).await);
}

#[tokio::test]
async fn first_call_loads_the_server_in_the_same_shell_call() {
    let fixture = fixture(Listing::Tools);
    let output = run(
        "tools docs search-docs q=billing | jq -c .input; tools docs search-docs q=refunds | jq -r .tool",
        &fixture.context,
    )
    .await;
    assert_eq!(output["exit_code"], 0, "{output}");
    assert_eq!(
        output["stdout"],
        "{\"q\":\"billing\"}\nmcp_docs__search_docs\n"
    );
    assert_eq!(
        *fixture.server.listed.lock().unwrap(),
        1,
        "listed once per shell call"
    );
    assert!(revealed(&fixture).await, "the next step lists it normally");
}

#[tokio::test]
async fn server_help_loads_it_and_lists_its_tools() {
    let fixture = fixture(Listing::Tools);
    let output = run("tools docs --help", &fixture.context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert!(
        stdout.contains("search-docs  Search the product docs."),
        "{stdout}"
    );
}

async fn assert_repeated_source_search(fixture: &Fixture, query: &str) {
    let command = format!("tools search {query}; tools search {query}");
    let output = run(&command, &fixture.context).await;
    let stdout = output["stdout"].as_str().unwrap();
    assert_eq!(output["exit_code"], 0, "{output}");
    assert_eq!(
        stdout.matches("Server description matches").count(),
        2,
        "{stdout}"
    );
    assert_eq!(stdout.matches("tools docs --help").count(), 2, "{stdout}");
    assert!(
        !stdout.contains("tools docs search-docs"),
        "server description terms must not match the child operation: {stdout}"
    );
    assert_eq!(
        *fixture.server.listed.lock().unwrap(),
        1,
        "both searches run in one shell invocation, so loading happens once"
    );
}

#[tokio::test]
async fn server_description_matches_are_reported_as_browse_suggestions() {
    let fixture = fixture(Listing::Tools);
    assert_repeated_source_search(&fixture, "documentation").await;
    assert!(revealed(&fixture).await);

    let fixture = self::fixture(Listing::Tools);
    run("tools search fetch", &fixture.context).await;
    assert_eq!(
        *fixture.server.listed.lock().unwrap(),
        0,
        "a search that does not name the server leaves it alone"
    );
}

#[tokio::test]
async fn placeholder_description_is_fallback_source_context() {
    let fixture = fixture_with_metadata(Listing::Tools, None);
    assert_repeated_source_search(&fixture, "not loaded yet").await;
}

#[tokio::test]
async fn name_only_source_metadata_uses_placeholder_description() {
    let fixture = fixture_with_metadata(
        Listing::Tools,
        Some(json!({"mcp_server": {"name": "docs"}})),
    );
    assert_repeated_source_search(&fixture, "documentation").await;
}

#[tokio::test]
async fn blank_source_description_uses_placeholder_description() {
    let fixture = fixture_with_metadata(
        Listing::Tools,
        Some(json!({"mcp_server": {
            "name": "docs",
            "description": "  "
        }})),
    );
    assert_repeated_source_search(&fixture, "documentation").await;
}

#[tokio::test]
async fn a_server_needing_a_connection_is_an_error_and_not_revealed() {
    let fixture = fixture(Listing::NeedsConnection);
    let output = run("tools docs search-docs q=x", &fixture.context).await;
    assert_eq!(output["exit_code"], 1, "{output}");
    assert_eq!(error_code(&output), "connection_required");
    assert!(!revealed(&fixture).await);
}

#[tokio::test]
async fn a_host_that_cannot_list_mid_call_reveals_for_the_next_step() {
    let fixture = fixture(Listing::CannotList);
    let output = run("tools docs search-docs q=x", &fixture.context).await;
    assert_eq!(output["exit_code"], 1, "{output}");
    assert_eq!(error_code(&output), "unavailable");
    assert!(revealed(&fixture).await);
}

#[test]
fn hook_hides_the_placeholder_without_rewriting_bash_description() {
    let hook = HideBehindToolsHook::default();
    let kept = hook.transform(vec![
        definition("bash", ToolPolicy::Auto, ToolHints::default()),
        deferred_mcp_server_definition("docs", None),
        definition("web_fetch", ToolPolicy::Auto, ToolHints::default()),
    ]);
    let names: Vec<&str> = kept.iter().map(ToolDefinition::name).collect();
    assert_eq!(names, ["bash"]);
    assert_eq!(kept[0].description(), "bash tool");
}
