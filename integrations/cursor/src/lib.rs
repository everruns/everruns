//! Cursor Cloud Agents integration for Everruns.
//!
//! This integration contributes tools for launching and managing Cursor Cloud
//! Agents to the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_integrations_cursor::CursorCapability;
//!
//! let capability = CursorCapability;
//! # let _ = capability;
//! ```

pub mod client;
pub mod connection;
mod tools;

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_core::tool_narration::ToolNarrationPhase;
use everruns_core::tools::Tool;
use everruns_platform::connector::ConnectorPlugin;
use everruns_provider::tool_types::{ToolCall, ToolDefinition};

use connection::CursorConnector;
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
    ConnectorPlugin {
        experimental_only: false,
        factory: || Box::new(CursorConnector),
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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Cursor",
            "Створюйте агентів Cursor Cloud Agents, що асинхронно працюють із репозиторіями \
             GitHub, і керуйте ними.",
        )]
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
            "Delegate async coding work to Cursor Cloud Agents when a remote repository task should run outside this session. Launch with a precise repo, base ref, task, and optional branch; track status, send follow-ups while running, inspect the transcript before summarizing, and delete agent records only when asked. Omit model unless the user specifies one.",
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

    fn narrate(
        &self,
        _tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
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

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = CursorCapability;
        let ctx = everruns_core::capabilities::SystemPromptContext::without_file_store(
            everruns_provider::typed_id::SessionId::new(),
        );
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(prompt.len() <= 475, "prompt is {} bytes", prompt.len());
    }
}
