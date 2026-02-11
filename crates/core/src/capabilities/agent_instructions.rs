//! Agent Instructions Capability (AGENTS.md)
//!
//! Reads AGENTS.md from the session workspace and dynamically injects its
//! content into the system prompt on every LLM turn. This provides project-level
//! context and conventions to agents.
//!
//! Design decisions:
//! - Capability is a marker: no static system_prompt_addition (content is dynamic)
//! - ReasonAtom reads /AGENTS.md from session file store when this capability is enabled
//! - Re-read every turn so edits are picked up immediately
//! - 32 KiB size limit (truncated with warning), matching Codex convention
//! - Missing file is silently ignored

use super::{Capability, CapabilityStatus};

/// Maximum size of AGENTS.md content in bytes (32 KiB).
pub const MAX_AGENTS_MD_SIZE: usize = 32_768;

/// Path to AGENTS.md in the session filesystem.
pub const AGENTS_MD_PATH: &str = "/AGENTS.md";

/// Capability ID constant.
pub const AGENT_INSTRUCTIONS_CAPABILITY_ID: &str = "agent_instructions";

/// Agent Instructions capability — reads AGENTS.md from session workspace.
pub struct AgentInstructionsCapability;

impl Capability for AgentInstructionsCapability {
    fn id(&self) -> &str {
        AGENT_INSTRUCTIONS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Agent Instructions"
    }

    fn description(&self) -> &str {
        "Reads AGENTS.md from the session workspace and includes it as context in the system prompt. Content is re-read on every turn, so changes are picked up automatically.\n\n> [!TIP]\n> Write an `AGENTS.md` file to your session workspace with project conventions, coding style, or any instructions you want the agent to follow."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("file-text")
    }

    fn category(&self) -> Option<&str> {
        Some("Configuration")
    }

    // No system_prompt_addition — content is dynamic (read at runtime by ReasonAtom)
    // No tools
    // No dependencies
    // No mounts
}

/// Format AGENTS.md content for injection into the system prompt.
///
/// Truncates to `MAX_AGENTS_MD_SIZE` if content exceeds the limit.
/// Returns `None` if content is empty.
pub fn format_agents_md_content(content: &str) -> Option<String> {
    let content = content.trim();
    if content.is_empty() {
        return None;
    }

    let (body, was_truncated) = if content.len() > MAX_AGENTS_MD_SIZE {
        tracing::warn!(
            content_size = content.len(),
            max_size = MAX_AGENTS_MD_SIZE,
            "AGENTS.md exceeds size limit, truncating"
        );
        (&content[..MAX_AGENTS_MD_SIZE], true)
    } else {
        (content, false)
    };

    let mut result = format!("# Instructions (AGENTS.md)\n\n{}", body);
    if was_truncated {
        result.push_str("\n\n[AGENTS.md was truncated — content exceeds 32 KiB limit]");
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;

    #[test]
    fn test_capability_metadata() {
        let cap = AgentInstructionsCapability;

        assert_eq!(cap.id(), "agent_instructions");
        assert_eq!(cap.name(), "Agent Instructions");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("file-text"));
        assert_eq!(cap.category(), Some("Configuration"));
    }

    #[test]
    fn test_no_static_system_prompt() {
        let cap = AgentInstructionsCapability;
        assert!(cap.system_prompt_addition().is_none());
    }

    #[test]
    fn test_no_tools() {
        let cap = AgentInstructionsCapability;
        assert!(cap.tools().is_empty());
    }

    #[test]
    fn test_no_dependencies() {
        let cap = AgentInstructionsCapability;
        assert!(cap.dependencies().is_empty());
    }

    #[test]
    fn test_no_mounts() {
        let cap = AgentInstructionsCapability;
        assert!(cap.mounts().is_empty());
    }

    #[test]
    fn test_format_agents_md_content_normal() {
        let content = "## Style\nUse snake_case for variables.";
        let result = format_agents_md_content(content).unwrap();

        assert!(result.starts_with("# Instructions (AGENTS.md)"));
        assert!(result.contains("Use snake_case"));
    }

    #[test]
    fn test_format_agents_md_content_empty() {
        assert!(format_agents_md_content("").is_none());
        assert!(format_agents_md_content("   ").is_none());
        assert!(format_agents_md_content("\n\n").is_none());
    }

    #[test]
    fn test_format_agents_md_content_truncation() {
        let content = "x".repeat(MAX_AGENTS_MD_SIZE + 1000);
        let result = format_agents_md_content(&content).unwrap();

        let header = "# Instructions (AGENTS.md)\n\n";
        let suffix = "\n\n[AGENTS.md was truncated — content exceeds 32 KiB limit]";
        let expected_len = header.len() + MAX_AGENTS_MD_SIZE + suffix.len();
        assert_eq!(result.len(), expected_len);
        assert!(result.ends_with(suffix));
    }

    #[test]
    fn test_format_agents_md_content_trims_whitespace() {
        let content = "  \n  Hello  \n  ";
        let result = format_agents_md_content(content).unwrap();
        assert!(result.contains("Hello"));
        // Should not contain leading/trailing whitespace from original
        assert!(!result.ends_with("  "));
    }

    #[test]
    fn test_capability_in_registry() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get("agent_instructions").unwrap();

        assert_eq!(cap.id(), "agent_instructions");
        assert_eq!(cap.name(), "Agent Instructions");
    }

    #[test]
    fn test_constants() {
        assert_eq!(MAX_AGENTS_MD_SIZE, 32_768);
        assert_eq!(AGENTS_MD_PATH, "/AGENTS.md");
        assert_eq!(AGENT_INSTRUCTIONS_CAPABILITY_ID, "agent_instructions");
    }
}
