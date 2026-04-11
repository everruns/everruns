//! PI Integration
//!
//! Sandbox coding agent capability. PI provisions an isolated sandbox,
//! checks out a repository, configures an LLM-powered coding agent with
//! a chosen model, and executes tasks autonomously.
//!
//! Decision: external integration crate, auto-registered via inventory.
//! Decision: session-scoped agent state + leased-resource cleanup.
//! Decision: model is user-configurable per start_pi call (default: claude-sonnet-4-20250514).
//! Decision: subagent-like semantics — start_pi / check_pi / message_pi.

pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::tools::Tool;

use tools::{CheckPiTool, MessagePiTool, StartPiTool};

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(PiCapability),
    }
}

pub const PI_AGENT_SECRET_PREFIX: &str = "pi_agent:";
pub const PI_DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";
pub const PI_DEFAULT_TIMEOUT_SECS: u64 = 60 * 60;

pub struct PiCapability;

impl Capability for PiCapability {
    fn id(&self) -> &str {
        "pi"
    }

    fn name(&self) -> &str {
        "PI"
    }

    fn description(&self) -> &str {
        "Run autonomous coding tasks in isolated sandboxes with a configurable LLM model. PI handles sandbox provisioning, repo checkout, and task execution."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("cpu")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## PI

Autonomous coding agent running in isolated sandboxes.

Use `start_pi` to launch a PI agent with a task. Each PI agent:
- Provisions its own sandbox (or reuses an existing one)
- Checks out the specified repository and branch
- Uses the configured LLM model to work on the task autonomously

Use `check_pi` to monitor progress (by agent ID, or list all).
Use `message_pi` to steer a running agent or cancel it.

The `model` parameter controls which LLM powers the PI agent (default: claude-sonnet-4-20250514).
You can run multiple PI agents in parallel on different tasks."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(StartPiTool),
            Box::new(CheckPiTool),
            Box::new(MessagePiTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_metadata() {
        let cap = PiCapability;
        assert_eq!(cap.id(), "pi");
        assert_eq!(cap.name(), "PI");
        assert_eq!(cap.icon(), Some("cpu"));
        assert_eq!(cap.category(), Some("Execution"));
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
        assert_eq!(cap.risk_level(), RiskLevel::High);
    }

    #[test]
    fn capability_tools() {
        let cap = PiCapability;
        let names: Vec<_> = cap
            .tools()
            .into_iter()
            .map(|tool| tool.name().to_string())
            .collect();
        assert!(names.contains(&"start_pi".to_string()));
        assert!(names.contains(&"check_pi".to_string()));
        assert!(names.contains(&"message_pi".to_string()));
    }

    #[test]
    fn system_prompt_mentions_model() {
        let cap = PiCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("model"));
        assert!(prompt.contains("start_pi"));
        assert!(prompt.contains("check_pi"));
        assert!(prompt.contains("message_pi"));
    }
}
