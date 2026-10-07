// The `user_mcp` manage tools. Spec: knowledge/integrations/user-mcp-servers.md (D3, D5).
//
// Each tool resolves the host's store or prompter from the tool context. The
// hosted worker installs one bound to the turn's input message, so the control
// plane resolves the person; a tool holding it has no argument naming a person.

use async_trait::async_trait;
use everruns_contracts::tool_types::{ConnectionRequiredSubject, ToolHints, ToolPolicy};
use everruns_core::mcp::{
    McpLogin, McpLoginPrompter, UserMcpServerEntry, UserMcpServerSummary, UserMcpStore,
    UserMcpStoreError,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{McpServerAuthMode, ScopedMcpServer};
use serde_json::{Value, json};
use std::sync::Arc;

/// Plain-language narration for the person's own MCP server tools.
fn narrate_user_mcp(
    arguments: &Value,
    phase: ToolNarrationPhase,
    started: &str,
    done: &str,
) -> String {
    let name = arguments
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty());
    let target = match name {
        Some(name) => format!("MCP server {name}"),
        None => "your MCP servers".to_string(),
    };
    match phase {
        ToolNarrationPhase::Started => format!("{started} {target}"),
        ToolNarrationPhase::Waiting => format!("{started} {target} (waiting for approval)"),
        ToolNarrationPhase::Completed => format!("{done} {target}"),
        ToolNarrationPhase::Failed => format!("{started} {target} failed"),
    }
}

pub(super) const LIST_TOOL: &str = "list_user_mcp_servers";
pub(super) const ADD_TOOL: &str = "add_user_mcp_server";
pub(super) const REMOVE_TOOL: &str = "remove_user_mcp_server";
pub(super) const ENABLE_TOOL: &str = "enable_user_mcp_server";
pub(super) const DISABLE_TOOL: &str = "disable_user_mcp_server";
pub(super) const CONNECT_TOOL: &str = "connect_mcp_server";

/// Tool-context extension carrying the person's [`UserMcpStore`].
#[derive(Clone)]
pub struct UserMcpStoreExt(pub Arc<dyn UserMcpStore>);

/// Tool-context extension carrying the host's [`McpLoginPrompter`].
#[derive(Clone)]
pub struct McpLoginPrompterExt(pub Arc<dyn McpLoginPrompter>);

const NO_STORE: &str =
    "Managing the person's MCP servers is not available here: this host has no store for them.";

/// The tool an agent gets when only `connect` is on.
pub(super) fn connect_tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(ConnectMcpServerTool)]
}

pub(super) fn manage_tools(allow_custom_urls: bool) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ListUserMcpServersTool),
        Box::new(AddUserMcpServerTool { allow_custom_urls }),
        Box::new(RemoveUserMcpServerTool),
        Box::new(SetUserMcpServerEnabledTool { enabled: true }),
        Box::new(SetUserMcpServerEnabledTool { enabled: false }),
        Box::new(ConnectMcpServerTool),
    ]
}

fn store(context: &ToolContext) -> Result<Arc<dyn UserMcpStore>, ToolExecutionResult> {
    context
        .extension::<UserMcpStoreExt>()
        .map(|ext| ext.0.clone())
        .ok_or_else(|| ToolExecutionResult::tool_error(NO_STORE))
}

fn store_error(error: UserMcpStoreError) -> ToolExecutionResult {
    match error {
        UserMcpStoreError::Internal(message) => ToolExecutionResult::internal_error_msg(message),
        other => ToolExecutionResult::tool_error(other.to_string()),
    }
}

fn required_name(arguments: &Value) -> Result<String, ToolExecutionResult> {
    arguments
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolExecutionResult::tool_error("Missing required parameter: name"))
}

fn optional_str<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn server_json(server: &UserMcpServerSummary) -> Value {
    serde_json::to_value(server).unwrap_or(Value::Null)
}

fn name_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": description }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

// ============================================================================
// list
// ============================================================================

pub struct ListUserMcpServersTool;

#[async_trait]
impl Tool for ListUserMcpServersTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_user_mcp(
            &tool_call.arguments,
            phase,
            "Listing",
            "Listed",
        ))
    }

    fn name(&self) -> &str {
        LIST_TOOL
    }

    fn display_name(&self) -> Option<&str> {
        Some("List My MCP Servers")
    }

    fn description(&self) -> &str {
        "List the MCP servers the person you are talking to added for themselves: name, whether it is enabled, whether they are signed in, and whether this agent skips it because one of its own servers has the same name."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            readonly: Some(true),
            ..Default::default()
        }
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(NO_STORE)
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = match store(context) {
            Ok(store) => store,
            Err(error) => return error,
        };
        match store.list().await {
            Ok(servers) => ToolExecutionResult::success(json!({
                "servers": servers.iter().map(server_json).collect::<Vec<_>>(),
            })),
            Err(error) => store_error(error),
        }
    }
}

// ============================================================================
// add
// ============================================================================

pub struct AddUserMcpServerTool {
    /// Whether `url` is offered and accepted (`allow_custom_urls`).
    pub allow_custom_urls: bool,
}

#[async_trait]
impl Tool for AddUserMcpServerTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_user_mcp(
            &tool_call.arguments,
            phase,
            "Adding",
            "Added",
        ))
    }

    fn name(&self) -> &str {
        ADD_TOOL
    }

    fn display_name(&self) -> Option<&str> {
        Some("Add My MCP Server")
    }

    fn description(&self) -> &str {
        if self.allow_custom_urls {
            "Add an MCP server to the person's own list: give `catalog` with the name of a server in the organization's MCP catalog, or `name` and an HTTPS `url`. The person approves it first. It is usable from their next message; offer connect_mcp_server if it needs a sign-in."
        } else {
            "Add a server from the organization's MCP catalog to the person's own list: give `catalog` with its catalog name. The person approves it first. It is usable from their next message; offer connect_mcp_server if it needs a sign-in."
        }
    }

    fn parameters_schema(&self) -> Value {
        let mut properties = json!({
            "catalog": {
                "type": "string",
                "description": "Name of the server in the organization's MCP catalog, e.g. 'linear'."
            },
            "name": {
                "type": "string",
                "description": "Name to save it under; also the prefix of its tools. Defaults to the catalog name."
            }
        });
        if self.allow_custom_urls {
            properties["url"] = json!({
                "type": "string",
                "description": "HTTPS endpoint of a server that is not in the catalog. Needs `name`."
            });
            properties["oauth"] = json!({
                "type": "boolean",
                "description": "With `url`: the server asks the person to sign in with OAuth."
            });
        }
        json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": false
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn policy(&self) -> ToolPolicy {
        ToolPolicy::RequiresApproval
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            readonly: Some(false),
            open_world: Some(true),
            ..Default::default()
        }
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(NO_STORE)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let catalog = optional_str(&arguments, "catalog");
        let url = optional_str(&arguments, "url");
        let name = optional_str(&arguments, "name");
        let (name, server) = match (catalog, url) {
            (Some(_), Some(_)) => {
                return ToolExecutionResult::tool_error("Give either `catalog` or `url`, not both");
            }
            (Some(catalog), None) => {
                let preset = match format!("catalog:{catalog}").parse() {
                    Ok(preset) => preset,
                    Err(error) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid catalog name: {error}"
                        ));
                    }
                };
                (
                    name.unwrap_or(catalog).to_string(),
                    ScopedMcpServer {
                        preset: Some(preset),
                        ..Default::default()
                    },
                )
            }
            (None, Some(_)) if !self.allow_custom_urls => {
                return ToolExecutionResult::tool_error(
                    "This agent can only add servers from the organization's MCP catalog. The person can add a custom URL themselves in Settings > My MCP servers.",
                );
            }
            (None, Some(url)) => {
                let Some(name) = name else {
                    return ToolExecutionResult::tool_error("A server added by URL needs a `name`");
                };
                (
                    name.to_string(),
                    ScopedMcpServer {
                        url: url.to_string(),
                        auth_mode: if arguments.get("oauth").and_then(Value::as_bool) == Some(true)
                        {
                            McpServerAuthMode::OAuth
                        } else {
                            McpServerAuthMode::None
                        },
                        ..Default::default()
                    },
                )
            }
            (None, None) => {
                return ToolExecutionResult::tool_error(if self.allow_custom_urls {
                    "Give `catalog`, or `name` and `url`"
                } else {
                    "Give `catalog`: the name of a server in the organization's MCP catalog"
                });
            }
        };
        let store = match store(context) {
            Ok(store) => store,
            Err(error) => return error,
        };
        match store
            .upsert(&name, UserMcpServerEntry::enabled(server))
            .await
        {
            Ok(server) => ToolExecutionResult::success(json!({
                "added": server_json(&server),
                "usable_from": "the person's next message",
            })),
            Err(error) => store_error(error),
        }
    }
}

// ============================================================================
// remove
// ============================================================================

pub struct RemoveUserMcpServerTool;

#[async_trait]
impl Tool for RemoveUserMcpServerTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_user_mcp(
            &tool_call.arguments,
            phase,
            "Removing",
            "Removed",
        ))
    }

    fn name(&self) -> &str {
        REMOVE_TOOL
    }

    fn display_name(&self) -> Option<&str> {
        Some("Remove My MCP Server")
    }

    fn description(&self) -> &str {
        "Remove an MCP server from the person's own list. It stops joining their conversations from their next message."
    }

    fn parameters_schema(&self) -> Value {
        name_schema("Name of the server: one in the person's list, or one of your own MCP servers.")
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            readonly: Some(false),
            idempotent: Some(true),
            ..Default::default()
        }
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(NO_STORE)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match required_name(&arguments) {
            Ok(name) => name,
            Err(error) => return error,
        };
        let store = match store(context) {
            Ok(store) => store,
            Err(error) => return error,
        };
        match store.remove(&name).await {
            Ok(true) => ToolExecutionResult::success(json!({ "removed": name })),
            Ok(false) => store_error(UserMcpStoreError::NotFound(name)),
            Err(error) => store_error(error),
        }
    }
}

// ============================================================================
// enable / disable
// ============================================================================

/// `enable_user_mcp_server` (approval) or `disable_user_mcp_server`.
pub struct SetUserMcpServerEnabledTool {
    pub enabled: bool,
}

#[async_trait]
impl Tool for SetUserMcpServerEnabledTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_user_mcp(
            &tool_call.arguments,
            phase,
            if self.enabled {
                "Turning on"
            } else {
                "Turning off"
            },
            if self.enabled {
                "Turned on"
            } else {
                "Turned off"
            },
        ))
    }

    fn name(&self) -> &str {
        if self.enabled {
            ENABLE_TOOL
        } else {
            DISABLE_TOOL
        }
    }

    fn display_name(&self) -> Option<&str> {
        Some(if self.enabled {
            "Enable My MCP Server"
        } else {
            "Disable My MCP Server"
        })
    }

    fn description(&self) -> &str {
        if self.enabled {
            "Turn on an MCP server in the person's own list so it joins their conversations from their next message. The person approves it first."
        } else {
            "Turn off an MCP server in the person's own list without removing it. It stops joining their conversations from their next message."
        }
    }

    fn parameters_schema(&self) -> Value {
        name_schema("Name of the server in the person's list.")
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn policy(&self) -> ToolPolicy {
        if self.enabled {
            ToolPolicy::RequiresApproval
        } else {
            ToolPolicy::Auto
        }
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            readonly: Some(false),
            idempotent: Some(true),
            open_world: self.enabled.then_some(true),
            ..Default::default()
        }
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(NO_STORE)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match required_name(&arguments) {
            Ok(name) => name,
            Err(error) => return error,
        };
        let store = match store(context) {
            Ok(store) => store,
            Err(error) => return error,
        };
        match store.set_enabled(&name, self.enabled).await {
            Ok(server) => ToolExecutionResult::success(json!({ "server": server_json(&server) })),
            Err(error) => store_error(error),
        }
    }
}

// ============================================================================
// connect
// ============================================================================

pub struct ConnectMcpServerTool;

#[async_trait]
impl Tool for ConnectMcpServerTool {
    fn narrate(
        &self,
        tool_call: &everruns_contracts::tool_types::ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_user_mcp(
            &tool_call.arguments,
            phase,
            "Connecting",
            "Asked you to connect",
        ))
    }

    fn name(&self) -> &str {
        CONNECT_TOOL
    }

    fn display_name(&self) -> Option<&str> {
        Some("Connect MCP Server")
    }

    fn description(&self) -> &str {
        "Ask the person to sign in to an MCP server: one of their own, or one of yours that acts as the person chatting. Shows them a Connect card (or, for a server set not to connect from chat, returns the settings link to pass on); they sign in in their own browser and you never see a credential. Use it before a tool of that server fails, e.g. right after adding it."
    }

    fn parameters_schema(&self) -> Value {
        name_schema("Name of the server in the person's list.")
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn hints(&self) -> ToolHints {
        ToolHints {
            readonly: Some(true),
            ..Default::default()
        }
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(NO_STORE)
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match required_name(&arguments) {
            Ok(name) => name,
            Err(error) => return error,
        };
        let Some(prompter) = context.extension::<McpLoginPrompterExt>() else {
            return ToolExecutionResult::tool_error(
                "Signing in from chat is not available here. The person can connect the server in Settings > My MCP servers.",
            );
        };
        match prompter.0.start_login(&name).await {
            // The agent's attachment opted out of in-chat cards: hand the
            // model the same link the card would have used.
            Ok(McpLogin::Pending {
                setup_url,
                for_agent,
                connect_in_chat,
                ..
            }) if !connect_in_chat.allows_card() => ToolExecutionResult::tool_error(if for_agent {
                format!(
                    "MCP server '{name}' cannot be connected from this chat. An admin must authorize the agent's sign-in at {setup_url}."
                )
            } else {
                format!(
                    "MCP server '{name}' cannot be connected from this chat. The person can connect it at {setup_url}."
                )
            }),
            Ok(McpLogin::Pending {
                provider,
                setup_url,
                for_agent,
                ..
            }) => ToolExecutionResult::connection_required_with_setup(
                provider,
                // An agent sign-in routes to the agent's MCP servers sheet,
                // where only someone with MCP management permission can
                // authorize it; everyone else is told to ask an admin.
                if for_agent {
                    ConnectionRequiredSubject::Agent
                } else {
                    ConnectionRequiredSubject::User
                },
                setup_url,
            ),
            Ok(McpLogin::NotNeeded) => ToolExecutionResult::success(json!({
                "server": name, "status": "not_needed",
                "message": "This server needs no sign-in.",
            })),
            Ok(McpLogin::AlreadyConnected) => ToolExecutionResult::success(json!({
                "server": name, "status": "connected",
                "message": "The person is already signed in to this server.",
            })),
            Ok(McpLogin::Completed) => ToolExecutionResult::success(json!({
                "server": name, "status": "connected",
                "message": "The person signed in. The server's tools are usable from their next message.",
            })),
            Err(error) => store_error(error),
        }
    }
}
