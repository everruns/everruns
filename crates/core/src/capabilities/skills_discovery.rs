// Skills Discovery Capability (built-in)
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
// Individual database-registered skills use `skill:{uuid}` capabilities
// (handled by SkillCapability). This capability handles dynamic VFS discovery.

use super::{Capability, CapabilityStatus, SystemPromptContext};
use crate::tool_types::{BuiltinTool, ToolDefinition, ToolPolicy};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::Value;

/// Skills Discovery capability ID (built-in)
pub const SKILLS_DISCOVERY_CAPABILITY_ID: &str = "skills";

/// Path in session VFS where skills are discovered
const SKILLS_PATH: &str = "/.agents/skills";

/// Built-in Skills Discovery Capability
///
/// Provides the generic skills mechanism. No skills are bundled — users upload
/// SKILL.md files to `/.agents/skills/{name}/SKILL.md` in the session VFS.
pub struct SkillsDiscoveryCapability;

/// Static skills system prompt (used by sync callers and as fallback)
const SKILLS_SYSTEM_PROMPT: &str = "You have access to an agent skills system. Skills are instruction packages \
that can be discovered from the session filesystem.\n\n\
Skills location: `/.agents/skills/{skill-name}/SKILL.md`\n\n\
Use `list_skills` to discover available skills. When a skill is relevant to the \
user's task, use `activate_skill` to load its full instructions into your context. \
Only activate skills that are relevant to the current task.";

#[async_trait]
impl Capability for SkillsDiscoveryCapability {
    fn id(&self) -> &str {
        SKILLS_DISCOVERY_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Agent Skills"
    }

    fn description(&self) -> &str {
        r#"Discover and activate skills from the session filesystem.

Skills are instruction packages (SKILL.md files) that teach the agent new abilities. Upload skills to `/.agents/skills/{name}/SKILL.md` and the agent will discover them automatically.

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
                    discovered_skills.push((parsed.name, parsed.description));
                }
            }
        }

        let mut prompt = String::from(SKILLS_SYSTEM_PROMPT);

        if !discovered_skills.is_empty() {
            prompt.push_str("\n\nAvailable skills:\n");
            for (name, description) in &discovered_skills {
                prompt.push_str(&format!("- **{}**: {}\n", name, description));
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
                    Scans /.agents/skills/ for SKILL.md files and returns their names \
                    and descriptions."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
                policy: ToolPolicy::Auto,
            }),
            ToolDefinition::Builtin(BuiltinTool {
                name: "activate_skill".to_string(),
                display_name: Some("Activate Skill".to_string()),
                description: "Activate a skill by name to load its full instructions. \
                    The skill must exist at /.agents/skills/{name}/SKILL.md in the \
                    session filesystem."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The skill directory name (e.g., 'pdf-processing')"
                        }
                    },
                    "required": ["name"]
                }),
                policy: ToolPolicy::Auto,
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
                    "message": "No skills found. Upload skills to /.agents/skills/{name}/SKILL.md"
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
                            "path": skill_md_path,
                            "version": parsed.version,
                        }));
                    }
                    Err(errors) => {
                        skills.push(serde_json::json!({
                            "name": entry.name,
                            "path": skill_md_path,
                            "error": format!("Invalid SKILL.md: {}", errors.join(", ")),
                        }));
                    }
                }
            }
        }

        ToolExecutionResult::success(serde_json::json!({
            "skills": skills,
            "count": skills.len(),
            "skills_path": SKILLS_PATH,
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
                    "Skill '{name}' not found at {skill_md_path}. \
                     Use list_skills to see available skills."
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
                let instructions = format!(
                    "<skill name=\"{}\">\n{}\n</skill>",
                    parsed.name, parsed.instructions
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
                        .map(|e| e.path.clone())
                        .collect::<Vec<_>>(),
                    Err(_) => vec![],
                };

                let mut result = serde_json::json!({
                    "skill": parsed.name,
                    "instructions": instructions,
                    "description": parsed.description,
                });

                if !bundled_files.is_empty() {
                    result["bundled_files"] = serde_json::json!(bundled_files);
                }

                ToolExecutionResult::success(result)
            }
            Err(errors) => ToolExecutionResult::tool_error(format!(
                "Invalid SKILL.md at {skill_md_path}: {}",
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
    fn test_skills_discovery_capability_metadata() {
        let cap = SkillsDiscoveryCapability;

        assert_eq!(cap.id(), "skills");
        assert_eq!(cap.name(), "Agent Skills");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("wand"));
        assert_eq!(cap.category(), Some("Skills"));
    }

    #[test]
    fn test_skills_discovery_has_system_prompt() {
        let cap = SkillsDiscoveryCapability;
        let prompt = cap.system_prompt_addition().unwrap();

        assert!(prompt.contains("/.agents/skills/"));
        assert!(prompt.contains("list_skills"));
        assert!(prompt.contains("activate_skill"));
    }

    #[test]
    fn test_skills_discovery_provides_tools() {
        let cap = SkillsDiscoveryCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name(), "list_skills");
        assert_eq!(tools[1].name(), "activate_skill");
    }

    #[test]
    fn test_skills_discovery_tool_definitions() {
        let cap = SkillsDiscoveryCapability;
        let defs = cap.tool_definitions();
        assert_eq!(defs.len(), 2);

        let names: Vec<&str> = defs.iter().map(|d| d.name()).collect();
        assert!(names.contains(&"list_skills"));
        assert!(names.contains(&"activate_skill"));
    }

    #[test]
    fn test_skills_discovery_dependencies() {
        let cap = SkillsDiscoveryCapability;
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
                assert!(paths.contains(&"/.agents/skills/data-tool/README.md"));
                // scripts/run.py is nested so won't be a direct child
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
        let cap = SkillsDiscoveryCapability;
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
            runtime_agent.system_prompt.contains("list_skills"),
            "System prompt should mention list_skills tool"
        );
        assert!(
            runtime_agent.system_prompt.contains("/.agents/skills/"),
            "System prompt should mention skills path"
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
        let cap = SkillsDiscoveryCapability;
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
        let cap = SkillsDiscoveryCapability;
        let ctx = SystemPromptContext::without_file_store(SessionId::new());

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("list_skills"));
        // No "Available skills:" section
        assert!(!result.contains("Available skills:"));
    }

    #[tokio::test]
    async fn test_contribution_static_when_no_skills_dir() {
        let cap = SkillsDiscoveryCapability;
        let store = Arc::new(MockFileStore::new());

        let ctx = SystemPromptContext {
            session_id: SessionId::new(),
            file_store: Some(store),
        };

        let result = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(result.contains("<capability id=\"skills\">"));
        assert!(result.contains("list_skills"));
        // No "Available skills:" section (dir doesn't exist)
        assert!(!result.contains("Available skills:"));
    }
}
