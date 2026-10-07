// User MCP servers capability (knowledge/integrations/user-mcp-servers.md, D2, D3).
//
// With `use` on (the default), the MCP servers the chatting person added for
// themselves join the agent's MCP servers for that person's turns. Which
// servers apply depends on who sent the message, so the hosted control plane
// resolves them per turn from the person's own list; `use` contributes no
// tools and no static servers.
//
// With `manage` on, the agent also gets tools to list, add, remove, enable,
// disable and connect that person's servers. The tools act through two host
// seams from `everruns-core` (`UserMcpStore`, `McpLoginPrompter`), so the same
// tools run in the hosted control plane and in a terminal host.
//
// Decision: only the turn's verified initiating person counts, never the
// session owner or the agent's service account. Unattended runs (triggers,
// schedules) have no such person and get nothing, and sessions with several
// people get nothing, so one person's tools never appear in another's turn.
// An agent or capability server with the same name wins over a user server.
//
// Decision: adding a server is how a prompt injection would exfiltrate data,
// so `add` and `enable` are held for the person's approval: the tools declare
// `requires_approval`, and with `manage` on the capability contributes its own
// durable approval gate for exactly those two tools, whether or not the agent
// enables `tool_approval`. `remove`, `disable` and `list` only reduce what the
// agent can reach, and run without asking.
//
// Decision: custom URLs are refused unless `allow_custom_urls` is on. The tool
// checks the setting it was built with, and the hosted store re-derives it from
// the agent's resolved configuration, so a forged call cannot skip it.

mod forwarding;
mod tools;

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use async_trait::async_trait;
use everruns_core::capabilities::SystemPromptContext;
use everruns_core::tools::Tool;
use serde_json::{Value, json};
use std::sync::Arc;

pub use forwarding::{ForwardingUserMcpStore, UserMcpCallInvoker, install_user_mcp_store};
// Re-exported so hosts that forward calls (the worker) need no direct core
// dependency for the wire types.
pub use everruns_core::mcp::{
    UserMcpStoreCall, UserMcpStoreError, UserMcpStoreReply, UserMcpStoreResult,
};
pub use tools::{
    AddUserMcpServerTool, ConnectMcpServerTool, ListUserMcpServersTool, McpLoginPrompterExt,
    RemoveUserMcpServerTool, SetUserMcpServerEnabledTool, UserMcpStoreExt,
};

pub const USER_MCP_CAPABILITY_ID: &str = "user_mcp";

/// Tools that change what the agent can reach and so wait for approval.
pub const USER_MCP_APPROVAL_TOOLS: &[&str] = &[tools::ADD_TOOL, tools::ENABLE_TOOL];

/// Whether the person's own MCP servers join their turns (`use`, default on).
pub fn user_mcp_use_enabled(config: &Value) -> bool {
    config.get("use").and_then(Value::as_bool).unwrap_or(true)
}

/// Whether the agent gets the manage tools (`manage`, default off).
pub fn user_mcp_manage_enabled(config: &Value) -> bool {
    config
        .get("manage")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Whether `add` may take a URL outside the catalog (`allow_custom_urls`,
/// default off). Only meaningful with `manage`.
pub fn user_mcp_custom_urls_allowed(config: &Value) -> bool {
    user_mcp_manage_enabled(config)
        && config
            .get("allow_custom_urls")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

const MANAGE_PROMPT: &str = "You can manage the MCP servers of the person you are talking to: \
list_user_mcp_servers, add_user_mcp_server, remove_user_mcp_server, enable_user_mcp_server, \
disable_user_mcp_server and connect_mcp_server. These change only that person's own list, never \
the organization's. Adding or enabling a server asks them to approve it first. A server added or \
enabled now is usable from their next message. After adding a server that needs a sign-in, offer \
connect_mcp_server: it shows them a Connect card and you never see or handle their credentials.";

/// Lets an agent use, and optionally manage, the MCP servers the chatting
/// person added for themselves.
pub struct UserMcpCapability;

#[async_trait]
impl Capability for UserMcpCapability {
    fn id(&self) -> &str {
        USER_MCP_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "User MCP Servers"
    }

    fn description(&self) -> &str {
        "Use the MCP servers the person chatting added for themselves in Settings, signed in as that person. Optionally let the agent add, remove and connect them in chat."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "MCP-сервери користувача",
            "Використовуйте MCP-сервери, які людина в розмові додала для себе в Налаштуваннях, від імені цієї людини. За бажанням агент може додавати, вилучати й підключати їх у чаті.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("plug")
    }

    fn category(&self) -> Option<&str> {
        Some("Integrations")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        // The catalog lists what `manage` adds; `tools_with_config` decides
        // what a given agent actually gets.
        tools::manage_tools(true)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        if !user_mcp_manage_enabled(config) {
            return Vec::new();
        }
        tools::manage_tools(user_mcp_custom_urls_allowed(config))
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        user_mcp_manage_enabled(config).then(|| MANAGE_PROMPT.to_string())
    }

    fn pre_tool_use_hooks_with_config(
        &self,
        config: &Value,
    ) -> Vec<Arc<dyn everruns_core::tool_hooks::PreToolUseHook>> {
        if !user_mcp_manage_enabled(config) {
            return Vec::new();
        }
        approval_gate()
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "use": {
                    "type": "boolean",
                    "title": "Use the person's MCP servers",
                    "description": "Add the enabled MCP servers of the person chatting to their turns.",
                    "default": true
                },
                "manage": {
                    "type": "boolean",
                    "title": "Let the agent manage them",
                    "description": "Give the agent tools to list, add, remove, enable, disable and connect the person's MCP servers. Adding and enabling ask the person to approve first.",
                    "default": false
                },
                "allow_custom_urls": {
                    "type": "boolean",
                    "title": "Allow servers outside the catalog",
                    "description": "With manage on, let the agent add a server by URL instead of only from the organization's MCP catalog.",
                    "default": false
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let Some(object) = config.as_object() else {
            return Err("user_mcp config must be an object".to_string());
        };
        for (key, value) in object {
            match key.as_str() {
                "use" | "manage" | "allow_custom_urls" if value.is_boolean() => {}
                "use" | "manage" | "allow_custom_urls" => {
                    return Err(format!("user_mcp.{key} must be a boolean"));
                }
                other => return Err(format!("Unknown user_mcp setting: {other}")),
            }
        }
        Ok(())
    }
}

/// The durable approval gate for `add` and `enable`.
#[cfg(feature = "portable-builtins")]
fn approval_gate() -> Vec<Arc<dyn everruns_core::tool_hooks::PreToolUseHook>> {
    use everruns_core::builtins::{DurableToolApprover, ToolApprovalCapability};
    ToolApprovalCapability::new(Arc::new(DurableToolApprover))
        .with_policy(Arc::new(|call, _definition| {
            USER_MCP_APPROVAL_TOOLS.contains(&call.name.as_str())
        }))
        .pre_tool_use_hooks()
}

/// Without the portable builtins there is no durable approver to park on; the
/// tools still declare `requires_approval` for a host gate to honor.
#[cfg(not(feature = "portable-builtins"))]
fn approval_gate() -> Vec<Arc<dyn everruns_core::tool_hooks::PreToolUseHook>> {
    Vec::new()
}

#[cfg(test)]
mod tests;
