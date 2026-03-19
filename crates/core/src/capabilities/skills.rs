// Skills Capability (built-in)
//
// Generic mechanism for filesystem-based skill discovery and activation.
// When enabled on an agent, provides:
// - System prompt explaining the skills system
// - `list_skills` tool: scans /.agents/skills/ in session VFS
// - `activate_skill` tool: loads SKILL.md instructions from VFS
//
// This is the built-in "skills" capability. It does NOT ship with any skills.
// Users upload SKILL.md files to /.agents/skills/{name}/SKILL.md in the
// session filesystem, and the agent discovers them at runtime.
//
// Database-registered skills are attached via AttachSkillCapability, which
// mounts skill files into the VFS so this capability discovers them.

use super::{Capability, CapabilityStatus, SystemPromptContext};
use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolPolicy};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::Value;

/// Skills capability ID (built-in)
pub const SKILLS_CAPABILITY_ID: &str = "skills";

/// Path in session VFS where skills are discovered (reuse shared constant)
use super::attach_skill::SKILLS_DISCOVERY_PATH as SKILLS_PATH;

/// Max skills to include in the system prompt (rest via list_skills tool)
const MAX_SKILLS_IN_PROMPT: usize = 15;

/// Max description length in system prompt (truncated with "…")
const MAX_DESCRIPTION_CHARS: usize = 76;

/// Workspace prefix for agent-facing paths (matches file_system capability convention)
const WORKSPACE_PREFIX: &str = "/workspace";

/// Truncate a string to `max_chars`, appending "…" if truncated.
/// Splits on the nearest char boundary at or before `max_chars`.
fn truncate_description(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated.trim_end())
}

/// Add workspace prefix for display paths shown to the agent
fn workspace_path(path: &str) -> String {
    if path.starts_with('/') {
        format!("{}{}", WORKSPACE_PREFIX, path)
    } else {
        format!("{}/{}", WORKSPACE_PREFIX, path)
    }
}

/// Built-in Skills Discovery Capability
///
/// Provides the generic skills mechanism. No skills are bundled — users upload
/// SKILL.md files to `/.agents/skills/{name}/SKILL.md` in the session VFS.
pub struct SkillsCapability;

/// Static skills system prompt (used by sync callers and as fallback)
const SKILLS_SYSTEM_PROMPT: &str = "Skills location: `/workspace/.agents/skills/{skill-name}/SKILL.md`. \
Only activate skills that are relevant to the current task.";

#[async_trait]
impl Capability for SkillsCapability {
    fn id(&self) -> &str {
        SKILLS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Agent Skills"
    }

    fn description(&self) -> &str {
        r#"Discover and activate skills from the session filesystem.

Skills are instruction packages (SKILL.md files) that teach the agent new abilities. Upload skills to `/workspace/.agents/skills/{name}/SKILL.md` and the agent will discover them automatically.

> [!TIP]
> Use the `list_skills` tool to see available skills, then `activate_skill` to load one."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("wand")
    }

    fn category(&self) -> Option<&str> {
        Some("Skills")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SKILLS_SYSTEM_PROMPT)
    }

    /// Dynamically discovers skills from the session filesystem and includes
    /// them in the system prompt.
    ///
    /// When a file store is available, scans `/.agents/skills/` for SKILL.md
    /// files and lists discovered skills directly in the prompt. Falls back
    /// to the static prompt when no file store is available.
    async fn system_prompt_contribution(&self, ctx: &SystemPromptContext) -> Option<String> {
        let file_store = match ctx.file_store.as_ref() {
            Some(fs) => fs,
            None => {
                // No file store — fall back to static prompt with capability wrapping
                return Some(format!(
                    "<capability id=\"{}\">\n{}\n</capability>",
                    self.id(),
                    SKILLS_SYSTEM_PROMPT
                ));
            }
        };

        // Scan /.agents/skills/ for SKILL.md files
        let entries = match file_store.list_directory(ctx.session_id, SKILLS_PATH).await {
            Ok(entries) => entries,
            Err(_) => {
                // Directory doesn't exist — use static prompt
                return Some(format!(
                    "<capability id=\"{}\">\n{}\n</capability>",
                    self.id(),
                    SKILLS_SYSTEM_PROMPT
                ));
            }
        };

        let mut discovered_skills = Vec::new();
        for entry in &entries {
            if !entry.is_directory {
                continue;
            }

            let skill_md_path = format!("{}/SKILL.md", entry.path);
            if let Ok(Some(file)) = file_store.read_file(ctx.session_id, &skill_md_path).await {
                let content = file.content.as_deref().unwrap_or("");
                if let Ok(parsed) = crate::skill::parse_skill_md(content) {
                    discovered_skills.push((
                        parsed.name,
                        parsed.description,
                        parsed.user_invocable,
                        parsed.disable_model_invocation,
                    ));
                }
            }
        }

        let mut prompt = String::from(SKILLS_SYSTEM_PROMPT);

        if !discovered_skills.is_empty() {
            // Filter out skills where disable_model_invocation is true
            let model_visible_skills: Vec<_> = discovered_skills
                .iter()
                .filter(|(_, _, _, disable_model)| !disable_model)
                .collect();
            let total = model_visible_skills.len();
            if total > 0 {
                prompt.push_str("\n\nAvailable skills:\n");
            }
            for (name, description, user_invocable, _) in
                model_visible_skills.iter().take(MAX_SKILLS_IN_PROMPT)
            {
                let desc = truncate_description(description, MAX_DESCRIPTION_CHARS);
                let invocable_hint = if *user_invocable { " (/{name})" } else { "" };
                prompt.push_str(&format!("- **{name}**: {desc}{invocable_hint}\n"));
            }
            if total > MAX_SKILLS_IN_PROMPT {
                prompt.push_str(&format!(
                    "\n({} more skills available — use `list_skills` to see all)\n",
                    total - MAX_SKILLS_IN_PROMPT
                ));
            }
        }

        Some(format!(
            "<capability id=\"{}\">\n{}\n</capability>",
            self.id(),
            prompt
        ))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ListSkillsTool), Box::new(ActivateSkillFromVfsTool)]
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition::Builtin(BuiltinTool {
                name: "list_skills".to_string(),
                display_name: Some("List Skills".to_string()),
                description: "Discover available skills from the session filesystem. \
                    Scans /workspace/.agents/skills/ for SKILL.md files and returns their names \
                    and descriptions."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
            }),
            ToolDefinition::Builtin(BuiltinTool {
                name: "activate_skill".to_string(),
                display_name: Some("Activate Skill".to_string()),
                description: "Activate a skill by name to load its full instructions. \
                    The skill must exist at /workspace/.agents/skills/{name}/SKILL.md in the \
                    session filesystem."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The skill directory name (e.g., 'pdf-processing')"
                        },
                        "arguments": {
                            "type": "string",
                            "description": "Optional arguments to pass to the skill for $ARGUMENTS substitution"
                        }
                    },
                    "required": ["name"]
                }),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
            }),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }
}

// ============================================================================
// ListSkillsTool - Discovers skills from session VFS
// ============================================================================

/// Tool that scans `/.agents/skills/` in the session VFS for SKILL.md files.
#[derive(Debug)]
struct ListSkillsTool;

#[async_trait]
impl Tool for ListSkillsTool {
    fn name(&self) -> &str {
        "list_skills"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Skills")
    }

    fn description(&self) -> &str {
        "Discover available skills from the session filesystem."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "list_skills requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let file_store = match &context.file_store {
            Some(fs) => fs,
            None => {
                return ToolExecutionResult::tool_error(
                    "File store not available. The session_file_system capability is required.",
                );
            }
        };

        // List directories under /.agents/skills/
        let entries = match file_store
            .list_directory(context.session_id, SKILLS_PATH)
            .await
        {
            Ok(entries) => entries,
            Err(_) => {
                // Directory doesn't exist yet — no skills available
                return ToolExecutionResult::success(serde_json::json!({
                    "skills": [],
                    "message": "No skills found. Upload skills to /workspace/.agents/skills/{name}/SKILL.md"
                }));
            }
        };

        let mut skills = Vec::new();

        for entry in &entries {
            if !entry.is_directory {
                continue;
            }

            let skill_md_path = format!("{}/SKILL.md", entry.path);
            if let Ok(Some(file)) = file_store
                .read_file(context.session_id, &skill_md_path)
                .await
            {
                let content = file.content.as_deref().unwrap_or("");
                match crate::skill::parse_skill_md(content) {
                    Ok(parsed) => {
                        skills.push(serde_json::json!({
                            "name": parsed.name,
                            "description": parsed.description,
                            "path": workspace_path(&skill_md_path),
                            "version": parsed.version,
                            "user_invocable": parsed.user_invocable,
                            "disable_model_invocation": parsed.disable_model_invocation,
                        }));
                    }
                    Err(errors) => {
                        skills.push(serde_json::json!({
                            "name": entry.name,
                            "path": workspace_path(&skill_md_path),
                            "error": format!("Invalid SKILL.md: {}", errors.join(", ")),
                        }));
                    }
                }
            }
        }

        ToolExecutionResult::success(serde_json::json!({
            "skills": skills,
            "count": skills.len(),
            "skills_path": workspace_path(SKILLS_PATH),
        }))
    }
}

// ============================================================================
// ActivateSkillFromVfsTool - Loads skill instructions from session VFS
// ============================================================================

/// Tool that reads a SKILL.md from `/.agents/skills/{name}/SKILL.md` and
/// returns its full instructions.
#[derive(Debug)]
struct ActivateSkillFromVfsTool;

#[async_trait]
impl Tool for ActivateSkillFromVfsTool {
    fn name(&self) -> &str {
        "activate_skill"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Activate Skill")
    }

    fn description(&self) -> &str {
        "Activate a skill by name to load its full instructions from the session filesystem."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill directory name (e.g., 'pdf-processing')"
                },
                "arguments": {
                    "type": "string",
                    "description": "Optional arguments to pass to the skill for $ARGUMENTS substitution"
                }
            },
            "required": ["name"]
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "activate_skill requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: name");
            }
        };

        let skill_args = arguments
            .get("arguments")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Validate name (prevent path traversal)
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return ToolExecutionResult::tool_error(
                "Invalid skill name. Must be a simple directory name without path separators.",
            );
        }

        let file_store = match &context.file_store {
            Some(fs) => fs,
            None => {
                return ToolExecutionResult::tool_error(
                    "File store not available. The session_file_system capability is required.",
                );
            }
        };

        let skill_md_path = format!("{}/{}/SKILL.md", SKILLS_PATH, name);

        let file = match file_store
            .read_file(context.session_id, &skill_md_path)
            .await
        {
            Ok(Some(f)) => f,
            Ok(None) => {
                return ToolExecutionResult::tool_error(format!(
                    "Skill '{name}' not found at {}. \
                     Use list_skills to see available skills.",
                    workspace_path(&skill_md_path)
                ));
            }
            Err(e) => {
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to read skill file: {e}"
                ));
            }
        };

        // Parse the SKILL.md to extract instructions
        let content = file.content.as_deref().unwrap_or("");
        match crate::skill::parse_skill_md(content) {
            Ok(parsed) => {
                // Apply argument substitution, then command injection preprocessing
                let expanded =
                    crate::skill::expand_skill_arguments(&parsed.instructions, skill_args);
                let executor = crate::skill::ProcessCommandExecutor::default();
                let preprocessed =
                    crate::skill::preprocess_command_injections(&expanded, &executor).await;
                let instructions = format!(
                    "<skill name=\"{}\">\n{}\n</skill>",
                    parsed.name, preprocessed
                );

                // List bundled files in the skill directory
                let skill_dir = format!("{}/{}", SKILLS_PATH, name);
                let bundled_files = match file_store
                    .list_directory(context.session_id, &skill_dir)
                    .await
                {
                    Ok(entries) => entries
                        .iter()
                        .filter(|e| !e.is_directory && e.name != "SKILL.md")
                        .map(|e| workspace_path(&e.path))
                        .collect::<Vec<_>>(),
                    Err(_) => vec![],
                };

                let mut result = serde_json::json!({
                    "skill": parsed.name,
                    "instructions": instructions,
                    "description": parsed.description,
                });

                // Include context and agent fields for fork-mode skills
                if parsed.context == crate::skill::SkillContext::Fork {
                    result["context"] = serde_json::json!("fork");
                    result["agent"] =
                        serde_json::json!(parsed.agent.as_deref().unwrap_or("general-purpose"));
                }

                if !bundled_files.is_empty() {
                    result["bundled_files"] = serde_json::json!(bundled_files);
                }

                ToolExecutionResult::success(result)
            }
            Err(errors) => ToolExecutionResult::tool_error(format!(
                "Invalid SKILL.md at {}: {}",
                workspace_path(&skill_md_path),
                errors.join(", ")
            )),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Capability;
    use crate::error::Result;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::traits::SessionFileStore;
    use crate::typed_id::SessionId;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    // ========================================================================
    // MockFileStore for testing skill discovery
    // ========================================================================

    /// In-memory file store supporting both files and directories.
    struct MockFileStore {
        /// Files: (session_id, path) -> content
        files: Mutex<HashMap<(SessionId, String), String>>,
        /// Directories: (session_id, path)
        dirs: Mutex<std::collections::HashSet<(SessionId, String)>>,
    }

    impl MockFileStore {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
                dirs: Mutex::new(std::collections::HashSet::new()),
            }
        }

        /// Add a file and auto-create parent directories
        fn add_file(&self, session_id: SessionId, path: &str, content: &str) {
            self.files
                .lock()
                .unwrap()
                .insert((session_id, path.to_string()), content.to_string());

            // Auto-create parent directories
            let mut dir = path.to_string();
            while let Some(idx) = dir.rfind('/') {
                if idx == 0 {
                    break;
                }
                dir = dir[..idx].to_string();
                self.dirs.lock().unwrap().insert((session_id, dir.clone()));
            }
        }
    }

    #[async_trait]
    impl SessionFileStore for MockFileStore {
        async fn read_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> Result<Option<SessionFile>> {
            let files = self.files.lock().unwrap();
            if let Some(content) = files.get(&(session_id, path.to_string())) {
                Ok(Some(SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.to_string(),
                    name: path.split('/').next_back().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    content: Some(content.clone()),
                    encoding: "text".to_string(),
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            _encoding: &str,
        ) -> Result<SessionFile> {
            self.add_file(session_id, path, content);
            Ok(SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.to_string(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: false,
                is_readonly: false,
                content: Some(content.to_string()),
                encoding: "text".to_string(),
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            Ok(false)
        }

        async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
            let files = self.files.lock().unwrap();
            let dirs = self.dirs.lock().unwrap();
            let mut entries = Vec::new();
            let mut seen_dirs = std::collections::HashSet::new();

            let prefix = if path.ends_with('/') {
                path.to_string()
            } else {
                format!("{}/", path)
            };

            // Find files directly under this path
            for ((sid, file_path), content) in files.iter() {
                if *sid != session_id || !file_path.starts_with(&prefix) {
                    continue;
                }
                let remainder = &file_path[prefix.len()..];
                if !remainder.contains('/') {
                    entries.push(FileInfo {
                        id: uuid::Uuid::new_v4(),
                        session_id: session_id.into(),
                        path: file_path.clone(),
                        name: remainder.to_string(),
                        is_directory: false,
                        is_readonly: false,
                        size_bytes: content.len() as i64,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    });
                }
            }

            // Find subdirectories directly under this path
            for (sid, dir_path) in dirs.iter() {
                if *sid != session_id || !dir_path.starts_with(&prefix) {
                    continue;
                }
                let remainder = &dir_path[prefix.len()..];
                // Only direct children (no nested slashes)
                if !remainder.contains('/')
                    && !remainder.is_empty()
                    && seen_dirs.insert(dir_path.clone())
                {
                    entries.push(FileInfo {
                        id: uuid::Uuid::new_v4(),
                        session_id: session_id.into(),
                        path: dir_path.clone(),
                        name: remainder.to_string(),
                        is_directory: true,
                        is_readonly: false,
                        size_bytes: 0,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    });
                }
            }

            Ok(entries)
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

        async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
            self.dirs
                .lock()
                .unwrap()
                .insert((session_id, path.to_string()));
            Ok(FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.to_string(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    fn valid_skill_md(name: &str, desc: &str) -> String {
        format!("---\nname: {name}\ndescription: {desc}\n---\n\n# Instructions\nDo the thing.")
    }

    fn make_context(file_store: Arc<MockFileStore>) -> ToolContext {
        ToolContext::with_file_store(SessionId::new(), file_store)
    }

    // ========================================================================
    // Capability Trait Tests
    // ========================================================================

    #[test]
    fn test_skills_capability_metadata() {
        let cap = SkillsCapability;

        assert_eq!(cap.id(), "skills");
        assert_eq!(cap.name(), "Agent Skills");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("wand"));
        assert_eq!(cap.category(), Some("Skills"));
    }

    #[test]
    fn test_skills_has_system_prompt() {
        let cap = SkillsCapability;
        let prompt = cap.system_prompt_addition().unwrap();

        assert!(prompt.contains("/workspace/.agents/skills/"));
    }

    #[test]
    fn test_skills_provides_tools() {
        let cap = SkillsCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name(), "list_skills");
        assert_eq!(tools[1].name(), "activate_skill");
    }

    #[test]
    fn test_skills_tool_definitions() {
        let cap = SkillsCapability;
        let defs = cap.tool_definitions();
        assert_eq!(defs.len(), 2);

        let names: Vec<&str> = defs.iter().map(|d| d.name()).collect();
        assert!(names.contains(&"list_skills"));
        assert!(names.contains(&"activate_skill"));
    }

    #[test]
    fn test_skills_dependencies() {
        let cap = SkillsCapability;
        assert_eq!(cap.dependencies(), vec!["session_file_system"]);
    }

    #[test]
    fn test_skills_registered_as_builtin() {
        let registry = crate::capabilities::CapabilityRegistry::with_builtins();
        assert!(
            registry.has("skills"),
            "skills capability should be a built-in"
        );
        let cap = registry.get("skills").unwrap();
        assert_eq!(cap.name(), "Agent Skills");
        assert_eq!(cap.category(), Some("Skills"));
    }

    // ========================================================================
    // Tool Basics (no context)
    // ========================================================================

    #[test]
    fn test_list_skills_requires_context() {
        let tool = ListSkillsTool;
        assert!(tool.requires_context());
    }

    #[test]
    fn test_activate_skill_requires_context() {
        let tool = ActivateSkillFromVfsTool;
        assert!(tool.requires_context());
    }

    #[tokio::test]
    async fn test_list_skills_without_context() {
        let tool = ListSkillsTool;
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_error());
    }

    #[tokio::test]
    async fn test_activate_skill_without_context() {
        let tool = ActivateSkillFromVfsTool;
        let result = tool.execute(serde_json::json!({"name": "test"})).await;
        assert!(result.is_error());
    }

    // ========================================================================
    // Tool Error Cases
    // ========================================================================

    #[tokio::test]
    async fn test_activate_skill_missing_name() {
        let tool = ActivateSkillFromVfsTool;
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Missing required parameter"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_path_traversal_blocked() {
        let tool = ActivateSkillFromVfsTool;
        let context = ToolContext::new(SessionId::new());

        // ".." traversal
        let result = tool
            .execute_with_context(serde_json::json!({"name": "../etc/passwd"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid skill name")),
            other => panic!("Expected ToolError, got: {:?}", other),
        }

        // Forward slash
        let result = tool
            .execute_with_context(serde_json::json!({"name": "foo/bar"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid skill name")),
            other => panic!("Expected ToolError, got: {:?}", other),
        }

        // Backslash
        let result = tool
            .execute_with_context(serde_json::json!({"name": "foo\\bar"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid skill name")),
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_no_file_store() {
        let tool = ListSkillsTool;
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("File store not available"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_no_file_store() {
        let tool = ActivateSkillFromVfsTool;
        let context = ToolContext::new(SessionId::new());
        let result = tool
            .execute_with_context(serde_json::json!({"name": "test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("File store not available"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    // ========================================================================
    // list_skills with MockFileStore
    // ========================================================================

    #[tokio::test]
    async fn test_list_skills_empty_directory() {
        let fs = Arc::new(MockFileStore::new());
        let context = make_context(fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert!(skills.is_empty());
                assert_eq!(val["count"], 0);
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_discovers_valid_skill() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/pdf-tool/SKILL.md",
            &valid_skill_md("pdf-tool", "Extract text from PDFs"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0]["name"], "pdf-tool");
                assert_eq!(skills[0]["description"], "Extract text from PDFs");
                assert_eq!(val["count"], 1);
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_discovers_multiple_skills() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/pdf-tool/SKILL.md",
            &valid_skill_md("pdf-tool", "Extract text from PDFs"),
        );
        fs.add_file(
            session_id,
            "/.agents/skills/data-analysis/SKILL.md",
            &valid_skill_md("data-analysis", "Analyze datasets"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 2);
                assert_eq!(val["count"], 2);
                let names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
                assert!(names.contains(&"pdf-tool"));
                assert!(names.contains(&"data-analysis"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_skills_reports_invalid_skill_md() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/bad-skill/SKILL.md",
            "not valid frontmatter",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ListSkillsTool;

        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                // Should report the error but still include the entry
                assert!(
                    skills[0]["error"]
                        .as_str()
                        .unwrap()
                        .contains("Invalid SKILL.md")
                );
                assert_eq!(skills[0]["name"], "bad-skill");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // ========================================================================
    // activate_skill with MockFileStore
    // ========================================================================

    #[tokio::test]
    async fn test_activate_skill_success() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/pdf-tool/SKILL.md",
            &valid_skill_md("pdf-tool", "Extract text from PDFs"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "pdf-tool"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "pdf-tool");
                assert_eq!(val["description"], "Extract text from PDFs");
                let instructions = val["instructions"].as_str().unwrap();
                assert!(instructions.contains("<skill name=\"pdf-tool\">"));
                assert!(instructions.contains("# Instructions"));
                assert!(instructions.contains("</skill>"));
                // No bundled_files key when no extra files
                assert!(val.get("bundled_files").is_none());
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_not_found() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "nonexistent"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("not found"));
                assert!(msg.contains("nonexistent"));
                assert!(msg.contains("list_skills"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_invalid_skill_md() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/bad-skill/SKILL.md",
            "no frontmatter here",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "bad-skill"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Invalid SKILL.md"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_with_bundled_files() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/data-tool/SKILL.md",
            &valid_skill_md("data-tool", "Analyze data"),
        );
        fs.add_file(
            session_id,
            "/.agents/skills/data-tool/scripts/run.py",
            "print('hello')",
        );
        fs.add_file(
            session_id,
            "/.agents/skills/data-tool/README.md",
            "# Reference",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "data-tool"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "data-tool");
                let bundled = val["bundled_files"].as_array().unwrap();
                // Should have the extra files (not SKILL.md)
                assert!(!bundled.is_empty());
                let paths: Vec<&str> = bundled.iter().map(|f| f.as_str().unwrap()).collect();
                assert!(paths.contains(&"/workspace/.agents/skills/data-tool/README.md"));
                // scripts/run.py is nested so won't be a direct child
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_with_context_fork() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/research/SKILL.md",
            "---\nname: research\ndescription: Deep research.\ncontext: fork\nagent: Explore\n---\n\nResearch the topic.",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "research"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "research");
                assert_eq!(val["context"], "fork");
                assert_eq!(val["agent"], "Explore");
                // Instructions are still included (caller decides how to use them)
                let instructions = val["instructions"].as_str().unwrap();
                assert!(instructions.contains("Research the topic"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_fork_default_agent() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/analyze/SKILL.md",
            "---\nname: analyze\ndescription: Analyze code.\ncontext: fork\n---\n\nAnalyze the code.",
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "analyze"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["context"], "fork");
                assert_eq!(val["agent"], "general-purpose");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_inline_no_context_field() {
        let fs = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        fs.add_file(
            session_id,
            "/.agents/skills/inline-skill/SKILL.md",
            &valid_skill_md("inline-skill", "An inline skill"),
        );

        let context = ToolContext::with_file_store(session_id, fs);
        let tool = ActivateSkillFromVfsTool;

        let result = tool
            .execute_with_context(serde_json::json!({"name": "inline-skill"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "inline-skill");
                // No context or agent fields for inline skills
                assert!(val.get("context").is_none());
                assert!(val.get("agent").is_none());
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    // ========================================================================
    // Integration: CapabilityInfo DTO
    // ========================================================================

    #[test]
    fn test_capability_info_from_core_marks_is_skill() {
        use crate::capability_dto::CapabilityInfo;
        let cap = SkillsCapability;
        let info = CapabilityInfo::from_core(&cap);

        assert_eq!(info.id.as_str(), "skills");
        assert!(info.is_skill, "skills capability should have is_skill=true");
        assert!(!info.is_mcp);
        assert_eq!(info.category, Some("Skills".to_string()));
        assert!(!info.tool_definitions.is_empty());
        assert!(!info.dependencies.is_empty());
    }

    // ========================================================================
    // Integration: Dependency Resolution
    // ========================================================================

    #[test]
    fn test_skills_dependency_resolution() {
        use crate::capabilities::resolve_dependencies;

        let registry = crate::capabilities::CapabilityRegistry::with_builtins();
        let resolved = resolve_dependencies(&["skills".to_string()], &registry).unwrap();

        // Should auto-include session_file_system as dependency
        assert!(
            resolved
                .resolved_ids
                .contains(&"session_file_system".to_string()),
            "skills should pull in session_file_system dependency"
        );
        assert!(resolved.resolved_ids.contains(&"skills".to_string()));
        assert!(
            resolved
                .added_as_dependencies
                .contains(&"session_file_system".to_string()),
            "session_file_system should be marked as auto-added"
        );
    }

    // ========================================================================
    // Integration: apply_capabilities
    // ========================================================================

    #[tokio::test]
    async fn test_apply_capabilities_with_skills() {
        use crate::capabilities::SystemPromptContext;
        use crate::runtime_agent::RuntimeAgentBuilder;

        let registry = crate::capabilities::CapabilityRegistry::with_builtins();
        let ctx = SystemPromptContext::without_file_store(crate::typed_id::SessionId::new());
        // Use builder pattern which resolves dependencies automatically
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["skills".to_string()], &registry, &ctx)
            .await
            .model("gpt-5.2")
            .build();

        // System prompt should include skills capability section
        assert!(
            runtime_agent
                .system_prompt
                .contains("/workspace/.agents/skills/"),
            "System prompt should mention skills path"
        );
        assert!(
            runtime_agent
                .system_prompt
                .contains("/workspace/.agents/skills/"),
            "System prompt should mention skills path with workspace prefix"
        );
        assert!(
            runtime_agent
                .system_prompt
                .contains("<capability id=\"skills\">"),
            "Should include skills capability in XML tags"
        );

        // Should include file system tools (from dependency) + skills tools
        let tool_names: Vec<&str> = runtime_agent.tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"list_skills"));
        assert!(tool_names.contains(&"activate_skill"));
        // Dependency tools (from session_file_system)
        assert!(tool_names.contains(&"read_file"));
        assert!(tool_names.contains(&"write_file"));
    }

    // ========================================================================
    // Dynamic system_prompt_contribution tests
    // ========================================================================

    #[tokio::test]
    async fn test_contribution_includes_discovered_skills() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        store.add_file(
            session_id,
            "/.agents/skills/pdf-processor/SKILL.md",
            &valid_skill_md("pdf-processor", "Process PDF files"),
        );
        store.add_file(
            session_id,
            "/.agents/skills/data-analysis/SKILL.md",
            &valid_skill_md("data-analysis", "Analyze datasets"),
        );

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("pdf-processor"));
        assert!(result.contains("data-analysis"));
        assert!(result.contains("Available skills:"));
    }

    #[tokio::test]
    async fn test_contribution_static_when_no_file_store() {
        let cap = SkillsCapability;
        let ctx = SystemPromptContext::without_file_store(SessionId::new());

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("/workspace/.agents/skills/"));
        // No "Available skills:" section
        assert!(!result.contains("Available skills:"));
    }

    #[tokio::test]
    async fn test_contribution_static_when_no_skills_dir() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());

        let ctx = SystemPromptContext {
            session_id: SessionId::new(),
            locale: None,
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("/workspace/.agents/skills/"));
        // No "Available skills:" section (dir doesn't exist)
        assert!(!result.contains("Available skills:"));
    }

    // ========================================================================
    // truncate_description tests
    // ========================================================================

    #[test]
    fn test_truncate_short_description() {
        assert_eq!(truncate_description("Short desc", 76), "Short desc");
    }

    #[test]
    fn test_truncate_exact_limit() {
        let s = "a".repeat(76);
        assert_eq!(truncate_description(&s, 76), s);
    }

    #[test]
    fn test_truncate_long_description() {
        let s = "a".repeat(100);
        let result = truncate_description(&s, 76);
        assert!(result.ends_with('…'));
        // 75 chars + "…" = 76 display chars
        assert_eq!(result.chars().count(), 76);
    }

    #[test]
    fn test_truncate_preserves_words_trimming() {
        let s = "Extract text and tables from PDF files, fill forms, merge documents, and do other cool things too";
        let result = truncate_description(s, 76);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 76);
    }

    // ========================================================================
    // System prompt caps at MAX_SKILLS_IN_PROMPT
    // ========================================================================

    #[tokio::test]
    async fn test_contribution_caps_at_max_skills() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Add 20 skills (exceeds MAX_SKILLS_IN_PROMPT = 15)
        for i in 0..20 {
            let name = format!("skill-{:02}", i);
            store.add_file(
                session_id,
                &format!("/.agents/skills/{}/SKILL.md", name),
                &valid_skill_md(&name, &format!("Description for skill {}", i)),
            );
        }

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();

        // Should contain "Available skills:"
        assert!(result.contains("Available skills:"));

        // Count how many skill entries appear (lines starting with "- **skill-")
        let skill_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.starts_with("- **skill-"))
            .collect();
        assert_eq!(skill_lines.len(), MAX_SKILLS_IN_PROMPT);

        // Should contain overflow message
        assert!(result.contains("5 more skills available"));
        assert!(result.contains("list_skills"));
    }

    #[tokio::test]
    async fn test_contribution_no_overflow_at_limit() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Add exactly MAX_SKILLS_IN_PROMPT skills
        for i in 0..MAX_SKILLS_IN_PROMPT {
            let name = format!("skill-{:02}", i);
            store.add_file(
                session_id,
                &format!("/.agents/skills/{}/SKILL.md", name),
                &valid_skill_md(&name, &format!("Description for skill {}", i)),
            );
        }

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();

        let skill_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.starts_with("- **skill-"))
            .collect();
        assert_eq!(skill_lines.len(), MAX_SKILLS_IN_PROMPT);

        // No overflow message
        assert!(!result.contains("more skills available"));
    }

    // ========================================================================
    // Integration: AttachSkillCapability mount → SkillsCapability discovery
    // ========================================================================

    /// Simulate the runtime flow: AttachSkillCapability produces a mount,
    /// we materialize it into MockFileStore, then SkillsCapability discovers it.
    fn materialize_mount_into_store(
        store: &MockFileStore,
        session_id: SessionId,
        mount: &crate::capability_types::MountPoint,
    ) {
        use crate::capability_types::MountSource;
        fn walk(store: &MockFileStore, session_id: SessionId, base: &str, source: &MountSource) {
            match source {
                MountSource::InlineFile { content, .. } => {
                    store.add_file(session_id, base, content);
                }
                MountSource::InlineDirectory { entries } => {
                    for (name, entry) in entries {
                        let path = format!("{}/{}", base, name);
                        walk(store, session_id, &path, &entry.source);
                    }
                }
            }
        }
        walk(store, session_id, &mount.path, &mount.source);
    }

    #[tokio::test]
    async fn test_attach_skill_mount_discovered_by_list_skills() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "pdf-tool".to_string(),
            "Extract text from PDFs".to_string(),
            "# Instructions\nUse pdfplumber to extract.".to_string(),
            vec![],
        );

        // Materialize mount into VFS mock
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // list_skills should discover it
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0]["name"], "pdf-tool");
                assert_eq!(skills[0]["description"], "Extract text from PDFs");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_attach_skill_mount_activatable_by_skills_capability() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "code-review".to_string(),
            "Review code for issues".to_string(),
            "# Instructions\nReview the code carefully.\n\n## Steps\n1. Check style\n2. Check logic"
                .to_string(),
            vec![],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // activate_skill should load instructions
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ActivateSkillFromVfsTool;
        let result = tool
            .execute_with_context(serde_json::json!({"name": "code-review"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "code-review");
                assert_eq!(val["description"], "Review code for issues");
                let instructions = val["instructions"].as_str().unwrap();
                assert!(instructions.contains("<skill name=\"code-review\">"));
                assert!(instructions.contains("Review the code carefully"));
                assert!(instructions.contains("Check logic"));
                assert!(instructions.contains("</skill>"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_attach_skill_with_bundled_files_discovered() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "data-pipeline".to_string(),
            "Build data pipelines".to_string(),
            "# Instructions\nUse the bundled script.".to_string(),
            vec![
                ("run.py".to_string(), "import pandas as pd".to_string()),
                ("README.md".to_string(), "# Reference docs".to_string()),
            ],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // list_skills discovers it
        let context = ToolContext::with_file_store(session_id, store.clone());
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skills"][0]["name"], "data-pipeline");
            }
            other => panic!("Expected Success, got: {:?}", other),
        }

        // activate_skill returns bundled files
        let tool = ActivateSkillFromVfsTool;
        let result = tool
            .execute_with_context(serde_json::json!({"name": "data-pipeline"}), &context)
            .await;
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["skill"], "data-pipeline");
                let bundled = val["bundled_files"].as_array().unwrap();
                // Should list non-SKILL.md files as bundled
                assert!(!bundled.is_empty());
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_multiple_attach_skills_all_discovered() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Mount 3 different skills from "registry"
        for (i, (name, desc)) in [
            ("pdf-tool", "PDF processing"),
            ("csv-analyzer", "CSV analysis"),
            ("code-review", "Code review"),
        ]
        .iter()
        .enumerate()
        {
            let skill_id =
                uuid::Uuid::parse_str(&format!("550e8400-e29b-41d4-a716-44665544000{}", i))
                    .unwrap();
            let cap = AttachSkillCapability::from_registry(
                skill_id,
                name.to_string(),
                desc.to_string(),
                format!("# {name} Instructions"),
                vec![],
            );
            for mount in cap.mounts() {
                materialize_mount_into_store(&store, session_id, &mount);
            }
        }

        // list_skills discovers all 3
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 3);
                let names: Vec<&str> = skills.iter().map(|s| s["name"].as_str().unwrap()).collect();
                assert!(names.contains(&"pdf-tool"));
                assert!(names.contains(&"csv-analyzer"));
                assert!(names.contains(&"code-review"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_attach_skill_prompt_contribution_includes_mounted_skill() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "pdf-tool".to_string(),
            "Extract text from PDFs".to_string(),
            "# Instructions".to_string(),
            vec![],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // SkillsCapability dynamic prompt should include the mounted skill
        let skills_cap = SkillsCapability;
        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
        };
        let result = skills_cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("pdf-tool"));
        assert!(result.contains("Extract text from PDFs"));
        assert!(result.contains("Available skills:"));
    }

    #[tokio::test]
    async fn test_attach_skill_description_with_special_chars_roundtrips() {
        use crate::capabilities::attach_skill::AttachSkillCapability;

        let skill_id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "tricky-skill".to_string(),
            "Description with: colons, #hashtags, and \"quotes\"".to_string(),
            "# Instructions\nDo the thing.".to_string(),
            vec![],
        );

        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();
        for mount in cap.mounts() {
            materialize_mount_into_store(&store, session_id, &mount);
        }

        // list_skills should parse the YAML correctly
        let context = ToolContext::with_file_store(session_id, store);
        let tool = ListSkillsTool;
        let result = tool
            .execute_with_context(serde_json::json!({}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(val) => {
                let skills = val["skills"].as_array().unwrap();
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0]["name"], "tricky-skill");
                assert_eq!(
                    skills[0]["description"],
                    "Description with: colons, #hashtags, and \"quotes\""
                );
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_contribution_excludes_disable_model_invocation_skills() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        // Normal skill
        store.add_file(
            session_id,
            "/.agents/skills/normal-skill/SKILL.md",
            &valid_skill_md("normal-skill", "A normal skill"),
        );

        // Skill with disable-model-invocation: true
        store.add_file(
            session_id,
            "/.agents/skills/manual-skill/SKILL.md",
            "---\nname: manual-skill\ndescription: Manual only skill\ndisable-model-invocation: true\n---\n\n# Instructions\nManual only.",
        );

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(
            result.contains("normal-skill"),
            "Normal skill should appear"
        );
        assert!(
            !result.contains("manual-skill"),
            "Skill with disable-model-invocation should not appear in system prompt"
        );
    }

    #[tokio::test]
    async fn test_contribution_truncates_long_descriptions() {
        let cap = SkillsCapability;
        let store = Arc::new(MockFileStore::new());
        let session_id = SessionId::new();

        let long_desc = "a".repeat(200);
        store.add_file(
            session_id,
            "/.agents/skills/long-desc/SKILL.md",
            &valid_skill_md("long-desc", &long_desc),
        );

        let ctx = SystemPromptContext {
            session_id,
            locale: None,
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();

        // The full 200-char description should NOT appear
        assert!(!result.contains(&long_desc));
        // Should contain truncated version with ellipsis
        assert!(result.contains('…'));
    }
}
