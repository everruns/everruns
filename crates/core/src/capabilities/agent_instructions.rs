//! Agent Instructions Capability (AGENTS.md)
//!
//! Reads AGENTS.md from the session workspace and dynamically injects its
//! content into the system prompt on every LLM turn. This provides project-level
//! context and conventions to agents.
//!
//! Design decisions:
//! - Capability encapsulates all AGENTS.md logic: reading, formatting, and injection
//! - system_prompt_contribution() reads /AGENTS.md from session filesystem via context
//! - Re-read every turn so edits are picked up immediately
//! - 32 KiB size limit (truncated with warning), matching Codex convention
//! - Missing file is silently ignored
//! - Content wrapped in `<agent-instructions>` XML tags to separate user-provided
//!   instructions from system capability prompts (reduces prompt injection surface)

use super::{Capability, CapabilityStatus, SystemPromptContext};
use async_trait::async_trait;

/// Maximum size of AGENTS.md content in bytes (32 KiB).
pub const MAX_AGENTS_MD_SIZE: usize = 32_768;

/// Path to AGENTS.md in the session filesystem.
pub const AGENTS_MD_PATH: &str = "/AGENTS.md";

/// Capability ID constant.
pub const AGENT_INSTRUCTIONS_CAPABILITY_ID: &str = "agent_instructions";

/// Agent Instructions capability — reads AGENTS.md from session workspace.
pub struct AgentInstructionsCapability;

#[async_trait]
impl Capability for AgentInstructionsCapability {
    fn id(&self) -> &str {
        AGENT_INSTRUCTIONS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "AGENTS.md"
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

    // No static system_prompt_addition — content is dynamic via system_prompt_contribution

    /// Reads AGENTS.md from the session filesystem and returns formatted content.
    ///
    /// This replaces the previous approach where ReasonAtom had hardcoded AGENTS.md
    /// reading logic. Now the capability fully encapsulates its own prompt generation.
    async fn system_prompt_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        let file_store = ctx.file_store.as_ref()?;

        match file_store.read_file(ctx.session_id, AGENTS_MD_PATH).await {
            Ok(Some(file)) => file.content.as_deref().and_then(format_agents_md_content),
            Ok(None) => {
                // File doesn't exist — silently skip
                None
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    session_id = %ctx.session_id,
                    "Failed to read AGENTS.md, skipping"
                );
                None
            }
        }
    }

    fn system_prompt_preview(&self) -> Option<String> {
        Some(
            "<agent-instructions source=\"AGENTS.md\">\n\
             (contents of /workspace/AGENTS.md, re-read every turn)\n\
             </agent-instructions>"
                .to_string(),
        )
    }

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

    let mut result = format!("<agent-instructions source=\"AGENTS.md\">\n{}", body);
    if was_truncated {
        result.push_str("\n\n[AGENTS.md was truncated — content exceeds 32 KiB limit]");
    }
    result.push_str("\n</agent-instructions>");
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;
    use crate::error::Result;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::traits::SessionFileStore;
    use crate::typed_id::SessionId;
    use std::sync::Arc;
    use uuid::Uuid;

    /// Mock file store for testing dynamic system prompt contribution
    struct MockFileStore {
        content: Option<String>,
    }

    #[async_trait::async_trait]
    impl SessionFileStore for MockFileStore {
        async fn read_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> Result<Option<SessionFile>> {
            Ok(self.content.as_ref().map(|c| SessionFile {
                id: Uuid::nil(),
                session_id: Uuid::nil(),
                path: AGENTS_MD_PATH.to_string(),
                name: "AGENTS.md".to_string(),
                content: Some(c.clone()),
                encoding: "text".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: c.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }))
        }

        async fn write_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _content: &str,
            _encoding: &str,
        ) -> Result<SessionFile> {
            unimplemented!("not needed for test")
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            unimplemented!("not needed for test")
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(&self, _session_id: SessionId, _path: &str) -> Result<Option<FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(&self, _session_id: SessionId, _path: &str) -> Result<FileInfo> {
            unimplemented!("not needed for test")
        }
    }

    fn test_session_id() -> SessionId {
        SessionId::from_uuid(Uuid::nil())
    }

    #[test]
    fn test_capability_metadata() {
        let cap = AgentInstructionsCapability;

        assert_eq!(cap.id(), "agent_instructions");
        assert_eq!(cap.name(), "AGENTS.md");
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
    fn test_system_prompt_preview() {
        let cap = AgentInstructionsCapability;
        let preview = cap.system_prompt_preview().unwrap();
        assert!(preview.contains("AGENTS.md"));
        assert!(preview.contains("re-read every turn"));
        assert!(preview.starts_with("<agent-instructions"));
        assert!(preview.ends_with("</agent-instructions>"));
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

        assert!(result.starts_with("<agent-instructions source=\"AGENTS.md\">"));
        assert!(result.ends_with("</agent-instructions>"));
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

        let header = "<agent-instructions source=\"AGENTS.md\">\n";
        let truncation_notice = "\n\n[AGENTS.md was truncated — content exceeds 32 KiB limit]";
        let closing = "\n</agent-instructions>";
        let expected_len =
            header.len() + MAX_AGENTS_MD_SIZE + truncation_notice.len() + closing.len();
        assert_eq!(result.len(), expected_len);
        assert!(result.starts_with("<agent-instructions"));
        assert!(result.ends_with("</agent-instructions>"));
        assert!(result.contains("truncated"));
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
        assert_eq!(cap.name(), "AGENTS.md");
    }

    #[test]
    fn test_constants() {
        assert_eq!(MAX_AGENTS_MD_SIZE, 32_768);
        assert_eq!(AGENTS_MD_PATH, "/AGENTS.md");
        assert_eq!(AGENT_INSTRUCTIONS_CAPABILITY_ID, "agent_instructions");
    }

    // ========================================================================
    // Dynamic system_prompt_contribution tests
    // ========================================================================

    #[tokio::test]
    async fn test_contribution_reads_agents_md() {
        let cap = AgentInstructionsCapability;
        let store = Arc::new(MockFileStore {
            content: Some("## Style\nUse snake_case.".to_string()),
        });
        let ctx = SystemPromptContext {
            session_id: test_session_id(),
            locale: None,
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("Use snake_case"));
        assert!(result.starts_with("<agent-instructions"));
        assert!(result.ends_with("</agent-instructions>"));
    }

    #[tokio::test]
    async fn test_contribution_none_when_file_missing() {
        let cap = AgentInstructionsCapability;
        let store = Arc::new(MockFileStore { content: None });
        let ctx = SystemPromptContext {
            session_id: test_session_id(),
            locale: None,
            file_store: Some(store),
        };

        assert!(cap.system_prompt_contribution(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_contribution_none_when_no_file_store() {
        let cap = AgentInstructionsCapability;
        let ctx = SystemPromptContext::without_file_store(test_session_id());

        assert!(cap.system_prompt_contribution(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_contribution_none_when_empty_content() {
        let cap = AgentInstructionsCapability;
        let store = Arc::new(MockFileStore {
            content: Some("   \n  ".to_string()),
        });
        let ctx = SystemPromptContext {
            session_id: test_session_id(),
            locale: None,
            file_store: Some(store),
        };

        assert!(cap.system_prompt_contribution(&ctx).await.is_none());
    }
}
