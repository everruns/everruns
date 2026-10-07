use super::*;
use async_trait::async_trait;
use everruns_contracts::tool_types::{ConnectionRequiredSubject, ToolPolicy};
use everruns_contracts::typed_id::SessionId;
use everruns_core::mcp::{
    McpLogin, McpLoginPrompter, UserMcpLoginStatus, UserMcpServerEntry, UserMcpServerSummary,
    UserMcpStore, UserMcpStoreError, UserMcpStoreResult,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;
use std::sync::Mutex;

/// An in-memory list for one person.
#[derive(Default)]
struct MemoryStore {
    entries: Mutex<Vec<(String, UserMcpServerEntry)>>,
    /// Servers added for this chat only.
    chat: Mutex<Vec<(String, everruns_core::ScopedMcpServer)>>,
    /// Names the agent already has; reported as skipped.
    agent_servers: Vec<String>,
    unavailable: bool,
}

impl MemoryStore {
    fn summary(&self, name: &str, entry: &UserMcpServerEntry) -> UserMcpServerSummary {
        UserMcpServerSummary {
            name: name.to_string(),
            enabled: entry.enabled,
            catalog: entry
                .server
                .preset
                .as_ref()
                .map(|preset| preset.catalog_name().to_string()),
            url: entry.server.url.clone(),
            login: UserMcpLoginStatus::NotConnected,
            skipped: self
                .agent_servers
                .iter()
                .any(|agent| agent == name)
                .then(|| format!("name clash with agent server '{name}'")),
            chat_only: false,
        }
    }

    fn check(&self) -> UserMcpStoreResult<()> {
        if self.unavailable {
            Err(UserMcpStoreError::Unavailable(
                "No person is chatting in this turn".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl UserMcpStore for MemoryStore {
    async fn list(&self) -> UserMcpStoreResult<Vec<UserMcpServerSummary>> {
        self.check()?;
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .iter()
            .map(|(name, entry)| self.summary(name, entry))
            .collect())
    }

    async fn upsert(
        &self,
        name: &str,
        entry: UserMcpServerEntry,
    ) -> UserMcpStoreResult<UserMcpServerSummary> {
        self.check()?;
        let summary = self.summary(name, &entry);
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|(existing, _)| existing != name);
        entries.push((name.to_string(), entry));
        Ok(summary)
    }

    async fn remove(&self, name: &str) -> UserMcpStoreResult<bool> {
        self.check()?;
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|(existing, _)| existing != name);
        Ok(entries.len() != before)
    }

    async fn add_to_chat(
        &self,
        name: &str,
        server: everruns_core::ScopedMcpServer,
    ) -> UserMcpStoreResult<UserMcpServerSummary> {
        self.check()?;
        let mut summary = self.summary(name, &UserMcpServerEntry::enabled(server.clone()));
        summary.chat_only = true;
        self.chat.lock().unwrap().push((name.to_string(), server));
        Ok(summary)
    }

    async fn set_enabled(
        &self,
        name: &str,
        enabled: bool,
    ) -> UserMcpStoreResult<UserMcpServerSummary> {
        self.check()?;
        let mut entries = self.entries.lock().unwrap();
        let (_, entry) = entries
            .iter_mut()
            .find(|(existing, _)| existing == name)
            .ok_or_else(|| UserMcpStoreError::NotFound(name.to_string()))?;
        entry.enabled = enabled;
        Ok(self.summary(name, entry))
    }
}

struct PendingPrompter;

#[async_trait]
impl McpLoginPrompter for PendingPrompter {
    async fn start_login(&self, name: &str) -> UserMcpStoreResult<McpLogin> {
        if name == "public" {
            return Ok(McpLogin::NotNeeded);
        }
        if name == "agent-github" {
            return Ok(McpLogin::Pending {
                provider: "mcp_oauth_456".into(),
                setup_url: "/agents/agent_1?tab=mcp".into(),
                for_agent: true,
                connect_in_chat: Default::default(),
            });
        }
        // Attachments that say `connectInChat: never`.
        if name == "quiet-github" {
            return Ok(McpLogin::Pending {
                provider: "mcp_oauth_789".into(),
                setup_url: "/settings/connections".into(),
                for_agent: false,
                connect_in_chat: everruns_core::McpConnectInChat::Never,
            });
        }
        if name == "quiet-agent-github" {
            return Ok(McpLogin::Pending {
                provider: "mcp_oauth_790".into(),
                setup_url: "/agents/agent_1?tab=mcp".into(),
                for_agent: true,
                connect_in_chat: everruns_core::McpConnectInChat::Never,
            });
        }
        Ok(McpLogin::Pending {
            provider: "mcp_oauth_123".into(),
            setup_url: "/settings/connections".into(),
            for_agent: false,
            connect_in_chat: Default::default(),
        })
    }
}

fn context(store: Arc<MemoryStore>) -> ToolContext {
    ToolContext::new(SessionId::new())
        .with_extension(Arc::new(UserMcpStoreExt(store)))
        .with_extension(Arc::new(McpLoginPrompterExt(Arc::new(PendingPrompter))))
}

fn tool(config: Value, name: &str) -> Box<dyn Tool> {
    UserMcpCapability
        .tools_with_config(&config)
        .into_iter()
        .find(|tool| tool.name() == name)
        .unwrap_or_else(|| panic!("{name} not offered for {config}"))
}

fn error_text(result: &ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::ToolError(message) => message.clone(),
        other => panic!("expected a tool error, got {other:?}"),
    }
}

#[test]
fn settings_default_and_validate() {
    assert!(user_mcp_use_enabled(&json!({})));
    assert!(user_mcp_use_enabled(&Value::Null));
    assert!(!user_mcp_use_enabled(&json!({"use": false})));
    assert!(!user_mcp_manage_enabled(&json!({})));
    assert!(user_mcp_manage_enabled(&json!({"manage": true})));
    // Custom URLs mean nothing without manage.
    assert!(!user_mcp_custom_urls_allowed(
        &json!({"allow_custom_urls": true})
    ));
    assert!(user_mcp_custom_urls_allowed(
        &json!({"manage": true, "allow_custom_urls": true})
    ));
    for ok in [
        json!({"use": true}),
        json!({"manage": true, "allow_custom_urls": false}),
        json!({"use": false, "connect": true}),
    ] {
        assert!(UserMcpCapability.validate_config(&ok).is_ok(), "{ok}");
    }
    for bad in [
        json!({"use": "yes"}),
        json!({"manage": 1}),
        json!({"in_shared_sessions": true}),
        json!({"connect": "yes"}),
    ] {
        assert!(UserMcpCapability.validate_config(&bad).is_err(), "{bad}");
    }
}

#[test]
fn manage_tools_only_with_manage_and_add_enable_need_approval() {
    assert!(UserMcpCapability.tools_with_config(&json!({})).is_empty());
    let tools = UserMcpCapability.tools_with_config(&json!({"manage": true}));
    let mut names: Vec<_> = tools.iter().map(|tool| tool.name().to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "add_user_mcp_server",
            "connect_mcp_server",
            "disable_user_mcp_server",
            "enable_user_mcp_server",
            "list_user_mcp_servers",
            "remove_user_mcp_server",
        ]
    );
    for tool in &tools {
        let gated = matches!(tool.policy(), ToolPolicy::RequiresApproval);
        assert_eq!(
            gated,
            USER_MCP_APPROVAL_TOOLS.contains(&tool.name()),
            "{}",
            tool.name()
        );
        // Nothing the agent may run unasked is flagged as destructive or
        // outward, so the generic `tool_approval` gate agrees.
        if !gated {
            assert_ne!(tool.hints().destructive, Some(true), "{}", tool.name());
            assert_ne!(tool.hints().open_world, Some(true), "{}", tool.name());
        }
    }
}

#[tokio::test]
async fn custom_url_is_refused_unless_allowed() {
    let add = tool(json!({"manage": true}), "add_user_mcp_server");
    assert!(add.parameters_schema()["properties"].get("url").is_none());
    let store = Arc::new(MemoryStore::default());
    let result = add
        .execute_with_context(
            json!({"name": "notes", "url": "https://mcp.example.com/mcp"}),
            &context(store.clone()),
        )
        .await;
    assert!(error_text(&result).contains("catalog"));
    assert!(store.entries.lock().unwrap().is_empty());

    let add = tool(
        json!({"manage": true, "allow_custom_urls": true}),
        "add_user_mcp_server",
    );
    assert!(add.parameters_schema()["properties"].get("url").is_some());
    let result = add
        .execute_with_context(
            json!({"name": "notes", "url": "https://mcp.example.com/mcp"}),
            &context(store.clone()),
        )
        .await;
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        store.entries.lock().unwrap()[0].1.server.url,
        "https://mcp.example.com/mcp"
    );
}

#[tokio::test]
async fn chat_scope_adds_to_this_chat_only() {
    let store = Arc::new(MemoryStore::default());
    let add = tool(json!({"manage": true}), "add_user_mcp_server");
    assert_eq!(
        add.parameters_schema()["properties"]["scope"]["enum"],
        json!(["list", "chat"])
    );
    let ToolExecutionResult::Success(added) = add
        .execute_with_context(
            json!({"catalog": "linear", "scope": "chat"}),
            &context(store.clone()),
        )
        .await
    else {
        panic!("chat add failed");
    };
    assert_eq!(added["added"]["chat_only"], true);
    assert!(
        store.entries.lock().unwrap().is_empty(),
        "the list is untouched"
    );
    let chat = store.chat.lock().unwrap().clone();
    assert_eq!(chat.len(), 1);
    assert_eq!(chat[0].0, "linear");
    assert_eq!(
        chat[0].1.preset.as_ref().map(|p| p.catalog_name()),
        Some("linear")
    );

    // The custom-URL rule holds for this chat too, and an unknown scope fails.
    let result = add
        .execute_with_context(
            json!({"name": "notes", "url": "https://mcp.example.com/mcp", "scope": "chat"}),
            &context(store.clone()),
        )
        .await;
    assert!(error_text(&result).contains("catalog"));
    let result = add
        .execute_with_context(
            json!({"catalog": "linear", "scope": "forever"}),
            &context(store.clone()),
        )
        .await;
    assert!(error_text(&result).contains("Unknown scope"));
    assert_eq!(store.chat.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn chat_scope_is_refused_by_a_store_without_conversations() {
    // A store that keeps only the list (a terminal host) refuses it.
    struct ListOnly;
    #[async_trait]
    impl UserMcpStore for ListOnly {
        async fn list(&self) -> UserMcpStoreResult<Vec<UserMcpServerSummary>> {
            Ok(Vec::new())
        }
        async fn upsert(
            &self,
            _: &str,
            _: UserMcpServerEntry,
        ) -> UserMcpStoreResult<UserMcpServerSummary> {
            unreachable!("chat scope never touches the list")
        }
        async fn remove(&self, _: &str) -> UserMcpStoreResult<bool> {
            Ok(false)
        }
        async fn set_enabled(&self, _: &str, _: bool) -> UserMcpStoreResult<UserMcpServerSummary> {
            unreachable!()
        }
    }
    let context = ToolContext::new(SessionId::new())
        .with_extension(Arc::new(UserMcpStoreExt(Arc::new(ListOnly))));
    let result = tool(json!({"manage": true}), "add_user_mcp_server")
        .execute_with_context(json!({"catalog": "linear", "scope": "chat"}), &context)
        .await;
    assert!(error_text(&result).contains("this conversation only"));
}

#[tokio::test]
async fn add_from_catalog_then_list_reports_clashes() {
    let store = Arc::new(MemoryStore {
        agent_servers: vec!["github".into()],
        ..Default::default()
    });
    let config = json!({"manage": true});
    for catalog in ["linear", "github"] {
        let result = tool(config.clone(), "add_user_mcp_server")
            .execute_with_context(json!({"catalog": catalog}), &context(store.clone()))
            .await;
        assert!(result.is_success(), "{result:?}");
    }
    let ToolExecutionResult::Success(listed) = tool(config, "list_user_mcp_servers")
        .execute_with_context(json!({}), &context(store))
        .await
    else {
        panic!("list failed");
    };
    let servers = listed["servers"].as_array().unwrap();
    assert_eq!(servers[0]["catalog"], "linear");
    assert!(servers[0].get("skipped").is_none());
    assert_eq!(servers[1]["name"], "github");
    assert!(
        servers[1]["skipped"]
            .as_str()
            .unwrap()
            .contains("name clash")
    );
}

#[tokio::test]
async fn tools_fail_clearly_without_a_person_or_a_store() {
    let config = json!({"manage": true});
    let store = Arc::new(MemoryStore {
        unavailable: true,
        ..Default::default()
    });
    let result = tool(config.clone(), "list_user_mcp_servers")
        .execute_with_context(json!({}), &context(store))
        .await;
    assert!(error_text(&result).contains("No person"));

    let bare = ToolContext::new(SessionId::new());
    let result = tool(config, "remove_user_mcp_server")
        .execute_with_context(json!({"name": "linear"}), &bare)
        .await;
    assert!(error_text(&result).contains("not available"));
}

#[tokio::test]
async fn remove_disable_and_missing_names() {
    let store = Arc::new(MemoryStore::default());
    let config = json!({"manage": true});
    tool(config.clone(), "add_user_mcp_server")
        .execute_with_context(json!({"catalog": "linear"}), &context(store.clone()))
        .await;
    let result = tool(config.clone(), "disable_user_mcp_server")
        .execute_with_context(json!({"name": "linear"}), &context(store.clone()))
        .await;
    assert!(result.is_success());
    assert!(!store.entries.lock().unwrap()[0].1.enabled);
    let result = tool(config.clone(), "remove_user_mcp_server")
        .execute_with_context(json!({"name": "linear"}), &context(store.clone()))
        .await;
    assert!(result.is_success());
    let result = tool(config, "remove_user_mcp_server")
        .execute_with_context(json!({"name": "linear"}), &context(store))
        .await;
    assert!(error_text(&result).contains("No MCP server named 'linear'"));
}

#[test]
fn connect_setting_offers_only_the_connect_tool() {
    let config = json!({"use": false, "connect": true});
    assert!(user_mcp_connect_enabled(&config));
    assert!(!user_mcp_connect_enabled(&json!({})));
    let names: Vec<_> = UserMcpCapability
        .tools_with_config(&config)
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, ["connect_mcp_server"]);
    assert!(
        UserMcpCapability
            .pre_tool_use_hooks_with_config(&config)
            .is_empty()
    );
}

#[tokio::test]
async fn connect_for_an_agent_server_asks_for_the_agents_login() {
    let store = Arc::new(MemoryStore::default());
    let connect = tool(json!({"connect": true}), "connect_mcp_server");
    let result = connect
        .execute_with_context(json!({"name": "agent-github"}), &context(store))
        .await;
    let ToolExecutionResult::ConnectionRequired {
        provider,
        subject,
        setup_url,
    } = result
    else {
        panic!("expected a Connect card, got {result:?}");
    };
    assert_eq!(provider, "mcp_oauth_456");
    assert_eq!(subject, Some(ConnectionRequiredSubject::Agent));
    assert_eq!(setup_url.as_deref(), Some("/agents/agent_1?tab=mcp"));
}

#[tokio::test]
async fn connect_shows_the_connect_card_and_never_a_credential() {
    let store = Arc::new(MemoryStore::default());
    let connect = tool(json!({"manage": true}), "connect_mcp_server");
    let result = connect
        .execute_with_context(json!({"name": "linear"}), &context(store.clone()))
        .await;
    let ToolExecutionResult::ConnectionRequired {
        provider,
        subject,
        setup_url,
    } = result
    else {
        panic!("expected a Connect card, got {result:?}");
    };
    assert_eq!(provider, "mcp_oauth_123");
    assert_eq!(subject, Some(ConnectionRequiredSubject::User));
    assert_eq!(setup_url.as_deref(), Some("/settings/connections"));

    let result = connect
        .execute_with_context(json!({"name": "public"}), &context(store))
        .await;
    assert!(result.is_success());
}

#[tokio::test]
async fn connect_in_chat_never_returns_the_settings_link_instead_of_a_card() {
    let store = Arc::new(MemoryStore::default());
    let connect = tool(json!({"connect": true}), "connect_mcp_server");
    for (name, setup_url) in [
        ("quiet-github", "/settings/connections"),
        ("quiet-agent-github", "/agents/agent_1?tab=mcp"),
    ] {
        let result = connect
            .execute_with_context(json!({ "name": name }), &context(store.clone()))
            .await;
        assert!(
            !matches!(result, ToolExecutionResult::ConnectionRequired { .. }),
            "{name}: `never` must not show a card, got {result:?}"
        );
        let message = error_text(&result);
        assert!(message.contains(name), "{message}");
        assert!(message.contains(setup_url), "{message}");
    }
}

#[cfg(feature = "portable-builtins")]
#[tokio::test]
async fn manage_holds_add_and_enable_for_approval() {
    use everruns_contracts::tool_types::ToolCall;
    use everruns_core::tool_hooks::PreToolUseDecision;

    assert!(
        UserMcpCapability
            .pre_tool_use_hooks_with_config(&json!({}))
            .is_empty()
    );
    let hooks = UserMcpCapability.pre_tool_use_hooks_with_config(&json!({"manage": true}));
    assert_eq!(hooks.len(), 1);
    let context = ToolContext::new(SessionId::new());
    for tool in UserMcpCapability.tools_with_config(&json!({"manage": true})) {
        let call = ToolCall {
            id: format!("call_{}", tool.name()),
            name: tool.name().to_string(),
            arguments: json!({"name": "linear"}),
        };
        let decision = hooks[0]
            .before_exec(call, &tool.to_definition(), &context)
            .await;
        let ran = matches!(decision, PreToolUseDecision::Continue(_));
        assert_eq!(
            ran,
            !USER_MCP_APPROVAL_TOOLS.contains(&tool.name()),
            "{}",
            tool.name()
        );
    }
}
