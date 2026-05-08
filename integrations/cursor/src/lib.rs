//! Cursor Cloud Agents integration.
//!
//! Decision: use Cursor's public Background/Cloud Agents REST API directly.
//! Cursor does not publish an official Rust client in the public API docs.
//! Decision: API key resolves from user connections first, then a per-session
//! `CURSOR_API_KEY` session secret. The previous process-wide env-var
//! fallback was removed to enforce per-session credential boundaries.

pub mod client;
pub mod connection;
mod tools;

use std::sync::Arc;

use everruns_core::capabilities::{
    Capability, CapabilityStatus, IntegrationPlugin, RiskLevel, ToolCallHook,
};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::tool_narration::ToolNarrationPhase;
use everruns_core::tool_types::{ToolCall, ToolDefinition};
use everruns_core::tools::Tool;

use connection::CursorConnectionProvider;
use tools::{
    CursorAddFollowupTool, CursorDeleteAgentTool, CursorGetAgentTool, CursorGetConversationTool,
    CursorKeyInfoTool, CursorLaunchAgentTool, CursorListAgentsTool, CursorListModelsTool,
    CursorListRepositoriesTool,
};

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(CursorCapability),
    }
}

inventory::submit! {
    ConnectionProviderPlugin {
        experimental_only: false,
        factory: || Box::new(CursorConnectionProvider),
    }
}

pub const CURSOR_API_BASE: &str = "https://api.cursor.com";
pub const CURSOR_API_KEY_SECRET: &str = "CURSOR_API_KEY";
pub const CURSOR_CONNECTION_PROVIDER: &str = "cursor";
pub const CURSOR_API_BASE_ENV: &str = "EVERRUNS_CURSOR_API_BASE";

pub struct CursorCapability;

impl Capability for CursorCapability {
    fn id(&self) -> &str {
        "cursor"
    }

    fn name(&self) -> &str {
        "Cursor"
    }

    fn description(&self) -> &str {
        "Create and manage Cursor Cloud Agents that work asynchronously on GitHub repositories."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("code")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## Cursor Cloud Agents

Use Cursor tools to delegate coding work to asynchronous Cursor Cloud Agents.

Prerequisites:
- Cursor Cloud Agents API key configured in Settings > Connections (or a per-session `CURSOR_API_KEY` session secret).
- Cursor GitHub app must have access to the target repository.

Recommended workflow:
1. Use `cursor_list_models` only when the user asks for a specific model; otherwise omit model and let Cursor choose automatically.
2. Use `cursor_launch_agent` with a precise repository, base ref, task prompt, and optional branch name.
3. Use `cursor_get_agent` or `cursor_list_agents` to track status.
4. Use `cursor_add_followup` for additional instructions while an agent is running.
5. Use `cursor_get_conversation` to inspect the transcript before summarizing results.
6. Use `cursor_delete_agent` only when the user asks to remove the agent record.

Cursor agents run in Cursor-managed remote environments and can push branches or create PRs depending on the request."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(CursorLaunchAgentTool),
            Box::new(CursorGetAgentTool),
            Box::new(CursorListAgentsTool),
            Box::new(CursorAddFollowupTool),
            Box::new(CursorGetConversationTool),
            Box::new(CursorDeleteAgentTool),
            Box::new(CursorListModelsTool),
            Box::new(CursorListRepositoriesTool),
            Box::new(CursorKeyInfoTool),
        ]
    }

    fn tool_call_hooks(&self) -> Vec<Arc<dyn ToolCallHook>> {
        vec![Arc::new(CursorNarrationHook)]
    }
}

struct CursorNarrationHook;

impl ToolCallHook for CursorNarrationHook {
    fn narration(
        &self,
        _tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
    ) -> Option<String> {
        let target = tool_call
            .arguments
            .get("name")
            .or_else(|| tool_call.arguments.get("agent_id"))
            .or_else(|| tool_call.arguments.get("repository"))
            .and_then(|v| v.as_str())
            .map(truncate_target);

        let (started, completed, failed) = match tool_call.name.as_str() {
            "cursor_launch_agent" => (
                "Starting Cursor agent",
                "Started Cursor agent",
                "Failed to start Cursor agent",
            ),
            "cursor_get_agent" => (
                "Checking Cursor agent",
                "Checked Cursor agent",
                "Failed to check Cursor agent",
            ),
            "cursor_list_agents" => (
                "Listing Cursor agents",
                "Listed Cursor agents",
                "Failed to list Cursor agents",
            ),
            "cursor_add_followup" => (
                "Sending Cursor follow-up",
                "Sent Cursor follow-up",
                "Failed to send Cursor follow-up",
            ),
            "cursor_get_conversation" => (
                "Reading Cursor conversation",
                "Read Cursor conversation",
                "Failed to read Cursor conversation",
            ),
            "cursor_delete_agent" => (
                "Deleting Cursor agent",
                "Deleted Cursor agent",
                "Failed to delete Cursor agent",
            ),
            "cursor_list_models" => (
                "Listing Cursor models",
                "Listed Cursor models",
                "Failed to list Cursor models",
            ),
            "cursor_list_repositories" => (
                "Listing Cursor repositories",
                "Listed Cursor repositories",
                "Failed to list Cursor repositories",
            ),
            "cursor_key_info" => (
                "Checking Cursor connection",
                "Checked Cursor connection",
                "Failed to check Cursor connection",
            ),
            _ => return None,
        };

        let verb = match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => started,
            ToolNarrationPhase::Completed => completed,
            ToolNarrationPhase::Failed => failed,
        };

        Some(match target {
            Some(target) => format!("{verb}: {target}"),
            None => verb.to_string(),
        })
    }
}

fn truncate_target(value: &str) -> String {
    const MAX_CHARS: usize = 48;
    let clean = value.trim();
    if clean.chars().count() <= MAX_CHARS {
        return clean.to_string();
    }
    format!("{}...", clean.chars().take(MAX_CHARS).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_metadata() {
        let cap = CursorCapability;
        assert_eq!(cap.id(), "cursor");
        assert_eq!(cap.name(), "Cursor");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("code"));
        assert_eq!(cap.category(), Some("Execution"));
        assert_eq!(cap.risk_level(), RiskLevel::High);
    }

    #[test]
    fn capability_has_all_tools() {
        let cap = CursorCapability;
        let tools = cap.tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(tools.len(), 9);
        assert!(names.contains(&"cursor_launch_agent"));
        assert!(names.contains(&"cursor_add_followup"));
        assert!(names.contains(&"cursor_get_conversation"));
        assert!(names.contains(&"cursor_list_repositories"));
    }
}
