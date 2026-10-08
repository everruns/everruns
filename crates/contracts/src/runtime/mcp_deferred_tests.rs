use super::*;
use crate::runtime::mcp_proxy::{McpToolInvoker, build_mcp_proxy_tools};
use crate::runtime::mcp_server::ScopedMcpServer;
use crate::runtime::session_services::{KeyInfo, SecretInfo};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MemoryStorage(Mutex<HashMap<String, String>>);

#[async_trait]
impl SessionStorageStore for MemoryStorage {
    async fn set_value(&self, _session: SessionId, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    async fn get_value(&self, _session: SessionId, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    async fn delete_value(&self, _session: SessionId, key: &str) -> Result<bool> {
        Ok(self.0.lock().unwrap().remove(key).is_some())
    }
    async fn list_keys(&self, _session: SessionId) -> Result<Vec<KeyInfo>> {
        let now = chrono::Utc::now();
        Ok(self
            .0
            .lock()
            .unwrap()
            .keys()
            .map(|key| KeyInfo {
                key: key.clone(),
                created_at: now,
                updated_at: now,
            })
            .collect())
    }
    async fn set_secret(&self, _session: SessionId, _name: &str, _value: &str) -> Result<()> {
        Ok(())
    }
    async fn get_secret(&self, _session: SessionId, _name: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn delete_secret(&self, _session: SessionId, _name: &str) -> Result<bool> {
        Ok(false)
    }
    async fn list_secrets(&self, _session: SessionId) -> Result<Vec<SecretInfo>> {
        Ok(Vec::new())
    }
}

struct PanickingInvoker;

#[async_trait]
impl McpToolInvoker for PanickingInvoker {
    async fn invoke(&self, call: &ToolCall) -> Result<crate::runtime::tool_types::ToolResult> {
        panic!(
            "a placeholder must never reach the MCP server: {}",
            call.name
        )
    }
}

fn server(deferred: bool) -> ScopedMcpServer {
    ScopedMcpServer {
        url: "https://mcp.example/mcp".into(),
        deferred,
        ..Default::default()
    }
}

#[test]
fn placeholder_names_never_collide_with_real_mcp_tools() {
    assert_eq!(deferred_mcp_server_tool_name("My Linear"), "mcp_my_linear");
    assert_eq!(
        deferred_mcp_server_prefix("mcp_my_linear"),
        Some("my_linear")
    );
    assert_eq!(deferred_mcp_server_prefix("mcp_linear__create_issue"), None);
    assert_eq!(deferred_mcp_server_prefix("mcp_"), None);
    assert_eq!(deferred_mcp_server_prefix("read_file"), None);
}

#[test]
fn placeholder_is_one_never_deferred_line_with_name_and_description() {
    let definition = deferred_mcp_server_definition("linear", Some("Issue tracking"));
    assert_eq!(definition.name(), "mcp_linear");
    assert!(
        definition
            .description()
            .contains("`linear`: Issue tracking")
    );
    assert!(definition.description().contains("tool_search"));
    assert!(matches!(definition.deferrable(), DeferrablePolicy::Never));

    // The worker protocol drops the policy; normalizing restores it.
    let ToolDefinition::Builtin(mut stripped) = definition else {
        unreachable!()
    };
    stripped.deferrable = DeferrablePolicy::Automatic;
    let restored = normalize_deferred_mcp_server_definition(ToolDefinition::Builtin(stripped));
    assert!(matches!(restored.deferrable(), DeferrablePolicy::Never));
}

#[test]
fn only_unrevealed_deferred_servers_are_held_back() {
    let servers = ScopedMcpServers::from([
        ("eager".to_string(), server(false)),
        ("lazy".to_string(), server(true)),
        ("Lazy Shown".to_string(), server(true)),
        (
            "silent".to_string(),
            ScopedMcpServer {
                tool_discovery: false,
                ..server(true)
            },
        ),
    ]);
    let revealed = HashSet::from(["lazy_shown".to_string()]);
    let (listed, deferred) = partition_deferred_mcp_servers(&servers, &revealed);
    assert_eq!(deferred, vec!["lazy".to_string()]);
    assert!(listed.contains_key("eager"));
    assert!(
        listed.contains_key("Lazy Shown"),
        "revealed servers are listed"
    );
    assert!(
        listed.contains_key("silent"),
        "a server without tool discovery keeps its existing behavior"
    );
}

#[tokio::test]
async fn calling_the_placeholder_reveals_its_server_without_contacting_it() {
    let storage = Arc::new(MemoryStorage::default());
    let session = SessionId::new();
    let tools = build_mcp_proxy_tools(
        &[deferred_mcp_server_definition("linear", None)],
        Arc::new(PanickingInvoker),
    );
    assert_eq!(tools.len(), 1);
    let context = ToolContext::with_storage_store(session, storage.clone());
    let result = tools[0].execute_with_context(json!({}), &context).await;
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        revealed_mcp_servers(storage.as_ref(), session).await,
        HashSet::from(["linear".to_string()])
    );
}

#[tokio::test]
async fn placeholder_without_storage_fails_as_a_tool_error() {
    let tool = DeferredMcpServerTool::new(match deferred_mcp_server_definition("linear", None) {
        ToolDefinition::Builtin(builtin) => builtin,
        ToolDefinition::ClientSide(_) => unreachable!(),
    });
    let result = tool
        .execute_with_context(json!({}), &ToolContext::new(SessionId::new()))
        .await;
    assert!(result.is_error());
}
