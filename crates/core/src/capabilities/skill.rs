// Skill Virtual Capability
//
// Implements the Capability trait for Agent Skills (agentskills.io format).
// Skills are discoverable instruction packages loaded from:
// 1. The database-backed skills registry (via API)
// 2. The session filesystem at `.agents/skills/` (filesystem-based discovery)
//
// Design decisions:
// - Follows MCP capability pattern: virtual capability wrapping external resources
// - Capability ID format: "skill:{skill_uuid}" for registry-based skills
// - Progressive disclosure: names/descriptions in system prompt, full content on activation
// - `activate_skill` tool loads full SKILL.md into context on demand
// - Depends on `session_file_system` for reading skill files after activation
// - Bundled files (archive skills) mounted into VFS at `/skills/{name}/`

use crate::capability_types::{CapabilityId, CapabilityStatus, MountDirectoryBuilder, MountPoint};
use crate::tool_types::{BuiltinTool, ToolDefinition, ToolPolicy};
use crate::tools::{Tool, ToolExecutionResult};

use super::Capability;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Skill capability ID prefix
pub const SKILL_CAPABILITY_PREFIX: &str = "skill:";

/// Default path for filesystem-based skill discovery
pub const SKILLS_DISCOVERY_PATH: &str = "/.agents/skills";

/// Maximum number of skills in a single capability
pub const MAX_SKILLS_PER_CAPABILITY: usize = 50;

/// Generate capability ID for a skill
pub fn skill_capability_id(skill_id: Uuid) -> String {
    format!("{}{}", SKILL_CAPABILITY_PREFIX, skill_id)
}

/// Check if a capability ID is a skill capability
pub fn is_skill_capability(capability_id: &str) -> bool {
    capability_id.starts_with(SKILL_CAPABILITY_PREFIX)
}

/// Parse skill UUID from capability ID
pub fn parse_skill_capability_id(capability_id: &str) -> Option<Uuid> {
    if !capability_id.starts_with(SKILL_CAPABILITY_PREFIX) {
        return None;
    }
    let uuid_str = &capability_id[SKILL_CAPABILITY_PREFIX.len()..];
    Uuid::parse_str(uuid_str).ok()
}

/// Metadata for a discovered skill (lightweight, for system prompt injection)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    /// Skill name (from SKILL.md frontmatter)
    pub name: String,
    /// Skill description (from SKILL.md frontmatter)
    pub description: String,
    /// Source location (filesystem path or "registry")
    pub source: SkillSource,
}

/// Where a skill was discovered from
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillSource {
    /// Discovered from `.agents/skills/` in session filesystem
    Filesystem { path: String },
    /// Loaded from the database-backed registry
    Registry { skill_id: String },
}

/// Skill Virtual Capability.
///
/// Wraps one or more discovered skills, providing:
/// - System prompt with `<available_skills>` XML listing skill names/descriptions
/// - `activate_skill` tool for loading full instructions on demand
#[derive(Debug, Clone)]
pub struct SkillCapability {
    /// Unique capability ID (for single-skill: "skill:{uuid}", for aggregate: "skills")
    capability_id: String,
    /// Display name
    display_name: String,
    /// Available skills metadata (lightweight, for progressive disclosure)
    skills: Vec<SkillMeta>,
    /// Full SKILL.md instructions indexed by name (populated on discovery)
    skill_instructions: Arc<Mutex<std::collections::HashMap<String, SkillInstructions>>>,
}

/// Full skill content loaded on activation
#[derive(Debug, Clone)]
pub struct SkillInstructions {
    /// Full SKILL.md body (markdown instructions)
    pub instructions: String,
    /// Bundled files (path -> content), for VFS mounting
    pub files: Vec<(String, String)>,
}

impl SkillCapability {
    /// Create a skill capability for a single registry-based skill
    pub fn from_registry(
        skill_id: Uuid,
        name: String,
        description: String,
        instructions: String,
        files: Vec<(String, String)>,
    ) -> Self {
        let cap_id = skill_capability_id(skill_id);
        let mut skill_instructions = std::collections::HashMap::new();
        skill_instructions.insert(
            name.clone(),
            SkillInstructions {
                instructions,
                files,
            },
        );

        Self {
            capability_id: cap_id,
            display_name: name.clone(),
            skills: vec![SkillMeta {
                name,
                description,
                source: SkillSource::Registry {
                    skill_id: skill_id.to_string(),
                },
            }],
            skill_instructions: Arc::new(Mutex::new(skill_instructions)),
        }
    }

    /// Create a skill capability from multiple discovered skills
    pub fn from_discovered(skills: Vec<SkillMeta>) -> Self {
        Self {
            capability_id: "skills".to_string(),
            display_name: "Agent Skills".to_string(),
            skills,
            skill_instructions: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Register a skill's full instructions (called after discovery/loading)
    pub fn register_instructions(&self, name: &str, instructions: SkillInstructions) {
        let mut map = self.skill_instructions.lock().unwrap();
        map.insert(name.to_string(), instructions);
    }

    /// Get the number of registered skills
    pub fn skill_count(&self) -> usize {
        self.skills.len()
    }

    /// Get the list of skill names
    pub fn skill_names(&self) -> Vec<&str> {
        self.skills.iter().map(|s| s.name.as_str()).collect()
    }

    /// Build the `<available_skills>` XML block for the system prompt
    fn build_available_skills_xml(&self) -> String {
        let mut xml = String::from("<available_skills>\n");
        for skill in &self.skills {
            xml.push_str("  <skill>\n");
            xml.push_str(&format!("    <name>{}</name>\n", skill.name));
            xml.push_str(&format!(
                "    <description>{}</description>\n",
                skill.description
            ));
            if let SkillSource::Filesystem { ref path } = skill.source {
                xml.push_str(&format!("    <location>{}/SKILL.md</location>\n", path));
            }
            xml.push_str("  </skill>\n");
        }
        xml.push_str("</available_skills>");
        xml
    }

    /// Get skill instructions by name (for tool execution)
    pub fn get_instructions(&self, name: &str) -> Option<SkillInstructions> {
        let map = self.skill_instructions.lock().unwrap();
        map.get(name).cloned()
    }

    /// Build mount points for activated skill files
    pub fn mount_points_for_skill(
        name: &str,
        files: &[(String, String)],
        capability_id: &str,
    ) -> Vec<MountPoint> {
        if files.is_empty() {
            return vec![];
        }

        let mut builder = MountDirectoryBuilder::new();
        for (path, content) in files {
            // Handle nested paths by flattening into directory structure
            builder = builder.file(path, content);
        }

        vec![MountPoint::readonly(
            format!("/skills/{name}"),
            builder.build(),
            capability_id,
        )]
    }
}

impl Capability for SkillCapability {
    fn id(&self) -> &str {
        Box::leak(self.capability_id.clone().into_boxed_str())
    }

    fn name(&self) -> &str {
        Box::leak(self.display_name.clone().into_boxed_str())
    }

    fn description(&self) -> &str {
        let desc = if self.skills.len() == 1 {
            self.skills[0].description.clone()
        } else {
            format!(
                "Agent Skills: {} skill(s) available for on-demand activation",
                self.skills.len()
            )
        };
        Box::leak(desc.into_boxed_str())
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
        if self.skills.is_empty() {
            return None;
        }

        let prompt = format!(
            "{}\n\n\
            When a user's task matches an available skill, activate it by using the \
            `activate_skill` tool with the skill name. This loads the full instructions \
            into your context. Only activate skills that are relevant to the current task.",
            self.build_available_skills_xml()
        );

        Some(Box::leak(prompt.into_boxed_str()))
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        if self.skills.is_empty() {
            return vec![];
        }

        vec![Box::new(ActivateSkillTool {
            skill_instructions: self.skill_instructions.clone(),
            available_skills: self.skills.iter().map(|s| s.name.clone()).collect(),
        })]
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        if self.skills.is_empty() {
            return vec![];
        }

        let skill_names: Vec<&str> = self.skills.iter().map(|s| s.name.as_str()).collect();
        let names_list = skill_names.join(", ");

        vec![ToolDefinition::Builtin(BuiltinTool {
            name: "activate_skill".to_string(),
            display_name: Some("Activate Skill".to_string()),
            description: format!(
                "Activate an agent skill by name to load its full instructions. \
                Available skills: {names_list}"
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The name of the skill to activate",
                        "enum": skill_names
                    }
                },
                "required": ["name"]
            }),
            policy: ToolPolicy::Auto,
        })]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }
}

/// Tool for activating a skill (loading full instructions into context)
pub struct ActivateSkillTool {
    skill_instructions: Arc<Mutex<std::collections::HashMap<String, SkillInstructions>>>,
    available_skills: Vec<String>,
}

impl std::fmt::Debug for ActivateSkillTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivateSkillTool")
            .field("available_skills", &self.available_skills)
            .finish()
    }
}

#[async_trait]
impl Tool for ActivateSkillTool {
    fn name(&self) -> &str {
        "activate_skill"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Activate Skill")
    }

    fn description(&self) -> &str {
        "Activate an agent skill by name to load its full instructions into context."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the skill to activate"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let name = match arguments.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => {
                return ToolExecutionResult::ToolError(
                    "Missing required parameter: name".to_string(),
                );
            }
        };

        // Validate skill exists
        if !self.available_skills.contains(&name.to_string()) {
            let available = self.available_skills.join(", ");
            return ToolExecutionResult::ToolError(format!(
                "Unknown skill: '{name}'. Available skills: {available}"
            ));
        }

        // Look up instructions
        let instructions = {
            let map = self.skill_instructions.lock().unwrap();
            map.get(name).cloned()
        };

        match instructions {
            Some(skill) => {
                let mut result =
                    format!("<skill name=\"{name}\">\n{}\n</skill>", skill.instructions);

                // Note mounted files if present
                if !skill.files.is_empty() {
                    let file_list: Vec<&str> =
                        skill.files.iter().map(|(path, _)| path.as_str()).collect();
                    result.push_str(&format!(
                        "\n\nBundled files are available at /skills/{name}/:\n{}",
                        file_list
                            .iter()
                            .map(|f| format!("- /skills/{name}/{f}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ));
                }

                ToolExecutionResult::success(serde_json::json!({
                    "skill": name,
                    "instructions": result,
                    "files_mounted": !skill.files.is_empty()
                }))
            }
            None => ToolExecutionResult::ToolError(format!(
                "Skill '{name}' instructions not loaded. The skill may not have been fully initialized."
            )),
        }
    }
}

/// Parse SKILL.md files from a list of (path, content) entries discovered in the session VFS.
///
/// Each entry represents a directory under `.agents/skills/` containing a SKILL.md file.
/// Returns parsed skill metadata and instructions for registration.
pub fn discover_skills_from_entries(
    entries: &[(String, String)],
) -> Vec<(SkillMeta, SkillInstructions)> {
    let mut results = Vec::new();

    for (path, content) in entries {
        match crate::skill::parse_skill_md(content) {
            Ok(parsed) => {
                let meta = SkillMeta {
                    name: parsed.name.clone(),
                    description: parsed.description.clone(),
                    source: SkillSource::Filesystem { path: path.clone() },
                };
                let instructions = SkillInstructions {
                    instructions: parsed.instructions,
                    files: vec![], // Filesystem skills don't bundle files via this path
                };
                results.push((meta, instructions));
            }
            Err(errors) => {
                tracing::warn!(
                    path = %path,
                    errors = ?errors,
                    "Skipping invalid SKILL.md"
                );
            }
        }
    }

    results
}

/// CapabilityId helpers for skill capabilities
impl CapabilityId {
    /// Check if this capability ID is for a skill
    pub fn is_skill(&self) -> bool {
        is_skill_capability(self.as_str())
    }

    /// Create a capability ID for a skill
    pub fn skill(skill_id: Uuid) -> Self {
        Self::new(skill_capability_id(skill_id))
    }

    /// Parse skill UUID from this capability ID
    pub fn skill_id(&self) -> Option<Uuid> {
        parse_skill_capability_id(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_capability_id() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap_id = skill_capability_id(skill_id);
        assert_eq!(cap_id, "skill:550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn test_is_skill_capability() {
        assert!(is_skill_capability(
            "skill:550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(!is_skill_capability("current_time"));
        assert!(!is_skill_capability(
            "mcp:550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(!is_skill_capability("skills")); // aggregate ID
    }

    #[test]
    fn test_parse_skill_capability_id() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap_id = skill_capability_id(skill_id);
        let parsed = parse_skill_capability_id(&cap_id);
        assert_eq!(parsed, Some(skill_id));

        assert_eq!(parse_skill_capability_id("current_time"), None);
        assert_eq!(parse_skill_capability_id("skill:invalid"), None);
    }

    #[test]
    fn test_capability_id_skill_methods() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap_id = CapabilityId::skill(skill_id);

        assert!(cap_id.is_skill());
        assert_eq!(cap_id.skill_id(), Some(skill_id));

        let regular_cap = CapabilityId::new("current_time");
        assert!(!regular_cap.is_skill());
        assert_eq!(regular_cap.skill_id(), None);
    }

    #[test]
    fn test_skill_capability_from_registry() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = SkillCapability::from_registry(
            skill_id,
            "pdf-processing".to_string(),
            "Extract text from PDFs".to_string(),
            "# Instructions\nUse pdfplumber.".to_string(),
            vec![(
                "scripts/extract.py".to_string(),
                "print('hello')".to_string(),
            )],
        );

        assert_eq!(cap.id(), "skill:550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(cap.name(), "pdf-processing");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("wand"));
        assert_eq!(cap.category(), Some("Skills"));
        assert_eq!(cap.skill_count(), 1);
    }

    #[test]
    fn test_skill_capability_system_prompt() {
        let cap = SkillCapability::from_discovered(vec![
            SkillMeta {
                name: "pdf-processing".to_string(),
                description: "Extract text from PDFs".to_string(),
                source: SkillSource::Registry {
                    skill_id: "test".to_string(),
                },
            },
            SkillMeta {
                name: "data-analysis".to_string(),
                description: "Analyze datasets".to_string(),
                source: SkillSource::Filesystem {
                    path: "/.agents/skills/data-analysis".to_string(),
                },
            },
        ]);

        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("<available_skills>"));
        assert!(prompt.contains("<name>pdf-processing</name>"));
        assert!(prompt.contains("<name>data-analysis</name>"));
        assert!(prompt.contains("<description>Extract text from PDFs</description>"));
        assert!(prompt.contains("activate_skill"));
        assert!(prompt.contains("</available_skills>"));
        // Filesystem skills include location
        assert!(prompt.contains("<location>/.agents/skills/data-analysis/SKILL.md</location>"));
    }

    #[test]
    fn test_skill_capability_no_system_prompt_when_empty() {
        let cap = SkillCapability::from_discovered(vec![]);
        assert!(cap.system_prompt_addition().is_none());
        assert!(cap.tools().is_empty());
    }

    #[test]
    fn test_skill_capability_tool_definitions() {
        let cap = SkillCapability::from_discovered(vec![SkillMeta {
            name: "test-skill".to_string(),
            description: "A test skill".to_string(),
            source: SkillSource::Registry {
                skill_id: "test".to_string(),
            },
        }]);

        let defs = cap.tool_definitions();
        assert_eq!(defs.len(), 1);

        let ToolDefinition::Builtin(builtin) = &defs[0] else {
            panic!("expected Builtin variant");
        };
        assert_eq!(builtin.name, "activate_skill");
        assert!(builtin.description.contains("test-skill"));
    }

    #[test]
    fn test_skill_capability_dependencies() {
        let cap = SkillCapability::from_discovered(vec![]);
        assert_eq!(cap.dependencies(), vec!["session_file_system"]);
    }

    #[tokio::test]
    async fn test_activate_skill_tool_success() {
        let instructions = Arc::new(Mutex::new(std::collections::HashMap::new()));
        instructions.lock().unwrap().insert(
            "test-skill".to_string(),
            SkillInstructions {
                instructions: "# Test Instructions\nDo the thing.".to_string(),
                files: vec![],
            },
        );

        let tool = ActivateSkillTool {
            skill_instructions: instructions,
            available_skills: vec!["test-skill".to_string()],
        };

        let result = tool
            .execute(serde_json::json!({"name": "test-skill"}))
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["skill"], "test-skill");
                let instr = value["instructions"].as_str().unwrap();
                assert!(instr.contains("<skill name=\"test-skill\">"));
                assert!(instr.contains("# Test Instructions"));
                assert!(instr.contains("</skill>"));
                assert_eq!(value["files_mounted"], false);
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_tool_with_files() {
        let instructions = Arc::new(Mutex::new(std::collections::HashMap::new()));
        instructions.lock().unwrap().insert(
            "data-skill".to_string(),
            SkillInstructions {
                instructions: "# Data Skill".to_string(),
                files: vec![
                    ("scripts/run.py".to_string(), "print('hi')".to_string()),
                    ("references/REFERENCE.md".to_string(), "# Ref".to_string()),
                ],
            },
        );

        let tool = ActivateSkillTool {
            skill_instructions: instructions,
            available_skills: vec!["data-skill".to_string()],
        };

        let result = tool
            .execute(serde_json::json!({"name": "data-skill"}))
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["files_mounted"], true);
                let instr = value["instructions"].as_str().unwrap();
                assert!(instr.contains("/skills/data-skill/scripts/run.py"));
            }
            other => panic!("Expected Success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_tool_unknown_skill() {
        let instructions = Arc::new(Mutex::new(std::collections::HashMap::new()));

        let tool = ActivateSkillTool {
            skill_instructions: instructions,
            available_skills: vec!["known-skill".to_string()],
        };

        let result = tool.execute(serde_json::json!({"name": "unknown"})).await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Unknown skill"));
                assert!(msg.contains("known-skill"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_activate_skill_tool_missing_name() {
        let tool = ActivateSkillTool {
            skill_instructions: Arc::new(Mutex::new(std::collections::HashMap::new())),
            available_skills: vec![],
        };

        let result = tool.execute(serde_json::json!({})).await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Missing required parameter"));
            }
            other => panic!("Expected ToolError, got: {:?}", other),
        }
    }

    #[test]
    fn test_discover_skills_from_entries() {
        let entries = vec![
            (
                "/.agents/skills/test-skill".to_string(),
                "---\nname: test-skill\ndescription: A test.\n---\n\n# Instructions\nDo things."
                    .to_string(),
            ),
            (
                "/.agents/skills/bad-skill".to_string(),
                "no frontmatter here".to_string(),
            ),
        ];

        let results = discover_skills_from_entries(&entries);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "test-skill");
        assert_eq!(results[0].0.description, "A test.");
        assert!(results[0].1.instructions.contains("# Instructions"));
    }

    #[test]
    fn test_mount_points_for_skill() {
        let files = vec![
            ("scripts/run.py".to_string(), "print('hi')".to_string()),
            ("references/REFERENCE.md".to_string(), "# Ref".to_string()),
        ];

        let mounts = SkillCapability::mount_points_for_skill("test-skill", &files, "skills");
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].path, "/skills/test-skill");
        assert!(mounts[0].is_readonly());
    }

    #[test]
    fn test_mount_points_empty_files() {
        let mounts = SkillCapability::mount_points_for_skill("test-skill", &[], "skills");
        assert!(mounts.is_empty());
    }

    #[test]
    fn test_skill_meta_serialization() {
        let meta = SkillMeta {
            name: "test-skill".to_string(),
            description: "A test".to_string(),
            source: SkillSource::Registry {
                skill_id: "abc".to_string(),
            },
        };

        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("test-skill"));

        let parsed: SkillMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test-skill");
    }

    #[test]
    fn test_build_available_skills_xml() {
        let cap = SkillCapability::from_discovered(vec![SkillMeta {
            name: "my-skill".to_string(),
            description: "Does cool things".to_string(),
            source: SkillSource::Registry {
                skill_id: "id".to_string(),
            },
        }]);

        let xml = cap.build_available_skills_xml();
        assert!(xml.starts_with("<available_skills>"));
        assert!(xml.ends_with("</available_skills>"));
        assert!(xml.contains("<name>my-skill</name>"));
        assert!(xml.contains("<description>Does cool things</description>"));
        // Registry skills don't have location
        assert!(!xml.contains("<location>"));
    }

    #[test]
    fn test_register_instructions_after_creation() {
        let cap = SkillCapability::from_discovered(vec![SkillMeta {
            name: "lazy-skill".to_string(),
            description: "Lazily loaded".to_string(),
            source: SkillSource::Filesystem {
                path: "/.agents/skills/lazy-skill".to_string(),
            },
        }]);

        // Initially no instructions
        assert!(cap.get_instructions("lazy-skill").is_none());

        // Register instructions
        cap.register_instructions(
            "lazy-skill",
            SkillInstructions {
                instructions: "# Do stuff".to_string(),
                files: vec![],
            },
        );

        // Now instructions are available
        let instr = cap.get_instructions("lazy-skill").unwrap();
        assert!(instr.instructions.contains("# Do stuff"));
    }
}
