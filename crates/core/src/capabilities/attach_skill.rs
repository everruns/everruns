// Attach Skill Virtual Capability
//
// Mounts a database-registered skill into the session VFS so that the
// built-in SkillsCapability can discover it alongside user-uploaded skills.
//
// Design decisions:
// - Follows MCP capability pattern: virtual capability wrapping external resources
// - Capability ID format: "skill:{skill_uuid}" for registry-based skills
// - Does NOT contribute to system prompt or provide tools — SkillsCapability
//   handles discovery, prompt injection, and the activate_skill tool.
// - Mounts reconstructed SKILL.md + bundled files to /.agents/skills/{name}/
// - Depends on `session_file_system` for VFS mounting

#[cfg(test)]
use crate::capability_types::CapabilityStatus;
use crate::capability_types::{CapabilityId, MountDirectoryBuilder, MountPoint};

use super::Capability;
use serde::{Deserialize, Serialize};
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
    /// Whether this skill appears as a /slash command for users
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    /// Whether the model is prevented from auto-invoking this skill
    #[serde(default)]
    pub disable_model_invocation: bool,
}

fn default_true() -> bool {
    true
}

/// Where a skill was discovered from
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillSource {
    /// Discovered from `.agents/skills/` in session filesystem
    Filesystem { path: String },
    /// Loaded from the database-backed registry
    Registry { skill_id: String },
}

/// Full skill content loaded on activation
#[derive(Debug, Clone)]
pub struct SkillInstructions {
    /// Full SKILL.md body (markdown instructions)
    pub instructions: String,
    /// Bundled files (path -> content), for VFS mounting
    pub files: Vec<(String, String)>,
}

/// Attach Skill Virtual Capability.
///
/// Mounts a database-registered skill into `/.agents/skills/{name}/` in the
/// session VFS. The built-in `SkillsCapability` then discovers and serves it
/// through its `list_skills` / `activate_skill` tools.
///
/// This capability does NOT contribute to the system prompt or provide tools.
#[derive(Debug, Clone)]
pub struct AttachSkillCapability {
    /// Unique capability ID: "skill:{uuid}"
    capability_id: String,
    /// Skill name (used for display + mount path)
    skill_name: String,
    /// Skill description (for display)
    skill_description: String,
    /// Reconstructed SKILL.md content (frontmatter + instructions)
    skill_md_content: String,
    /// Bundled files (path -> content)
    files: Vec<(String, String)>,
    /// Whether this skill is user-invocable as a /slash command
    user_invocable: bool,
    /// Whether the model is prevented from auto-invoking this skill
    disable_model_invocation: bool,
}

impl AttachSkillCapability {
    /// Create an attach capability for a registry-based skill.
    ///
    /// Reconstructs a valid SKILL.md and prepares mount points so that
    /// SkillsCapability can discover the skill from the VFS.
    pub fn from_registry(
        skill_id: Uuid,
        name: String,
        description: String,
        instructions: String,
        files: Vec<(String, String)>,
    ) -> Self {
        Self::from_registry_with_options(
            skill_id,
            name,
            description,
            instructions,
            files,
            true,
            false,
        )
    }

    pub fn from_registry_with_invocable(
        skill_id: Uuid,
        name: String,
        description: String,
        instructions: String,
        files: Vec<(String, String)>,
        user_invocable: bool,
    ) -> Self {
        Self::from_registry_with_options(
            skill_id,
            name,
            description,
            instructions,
            files,
            user_invocable,
            false,
        )
    }

    pub fn from_registry_with_options(
        skill_id: Uuid,
        name: String,
        description: String,
        instructions: String,
        files: Vec<(String, String)>,
        user_invocable: bool,
        disable_model_invocation: bool,
    ) -> Self {
        let skill_md_content = reconstruct_skill_md(
            &name,
            &description,
            &instructions,
            user_invocable,
            disable_model_invocation,
        );

        Self {
            capability_id: skill_capability_id(skill_id),
            skill_name: name,
            skill_description: description,
            skill_md_content,
            files,
            user_invocable,
            disable_model_invocation,
        }
    }

    /// Get the skill name
    pub fn skill_name(&self) -> &str {
        &self.skill_name
    }

    /// Whether this skill is user-invocable as a /slash command
    pub fn user_invocable(&self) -> bool {
        self.user_invocable
    }

    /// Whether the model is prevented from auto-invoking this skill
    pub fn disable_model_invocation(&self) -> bool {
        self.disable_model_invocation
    }

    /// Build mount points for the skill directory.
    ///
    /// Mounts SKILL.md + bundled files under `/.agents/skills/{name}/`.
    fn build_mounts(&self) -> Vec<MountPoint> {
        let mut builder = MountDirectoryBuilder::new();
        builder = builder.file("SKILL.md", &self.skill_md_content);

        for (path, content) in &self.files {
            builder = builder.file(path, content);
        }

        vec![MountPoint::readonly(
            format!("{}/{}", SKILLS_DISCOVERY_PATH, self.skill_name),
            builder.build(),
            &self.capability_id,
        )]
    }
}

impl Capability for AttachSkillCapability {
    fn id(&self) -> &str {
        Box::leak(self.capability_id.clone().into_boxed_str())
    }

    fn name(&self) -> &str {
        Box::leak(self.skill_name.clone().into_boxed_str())
    }

    fn description(&self) -> &str {
        Box::leak(self.skill_description.clone().into_boxed_str())
    }

    fn icon(&self) -> Option<&str> {
        Some("wand")
    }

    fn category(&self) -> Option<&str> {
        Some("Skills")
    }

    fn mounts(&self) -> Vec<MountPoint> {
        self.build_mounts()
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }
}

/// Reconstruct a valid SKILL.md from stored fields.
///
/// Produces content that `parse_skill_md` can round-trip:
/// ```text
/// ---
/// name: skill-name
/// description: "Skill description here."
/// ---
///
/// <instructions body>
/// ```
fn reconstruct_skill_md(
    name: &str,
    description: &str,
    instructions: &str,
    user_invocable: bool,
    disable_model_invocation: bool,
) -> String {
    // Quote description to handle YAML-special characters (:, #, etc.)
    let safe_description = format!("\"{}\"", description.replace('"', "\\\""));
    let invocable_line = if user_invocable {
        String::new()
    } else {
        "user-invocable: false\n".to_string()
    };
    let model_invocation_line = if disable_model_invocation {
        "disable-model-invocation: true\n".to_string()
    } else {
        String::new()
    };
    format!(
        "---\nname: {name}\ndescription: {safe_description}\n{invocable_line}{model_invocation_line}---\n\n{instructions}"
    )
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
                    user_invocable: parsed.user_invocable,
                    disable_model_invocation: parsed.disable_model_invocation,
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
    fn test_attach_skill_from_registry() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
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
    }

    #[test]
    fn test_attach_skill_no_system_prompt() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "test-skill".to_string(),
            "A test".to_string(),
            "# Instructions".to_string(),
            vec![],
        );

        assert!(cap.system_prompt_addition().is_none());
    }

    #[test]
    fn test_attach_skill_no_tools() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "test-skill".to_string(),
            "A test".to_string(),
            "# Instructions".to_string(),
            vec![],
        );

        assert!(cap.tools().is_empty());
        assert!(cap.tool_definitions().is_empty());
    }

    #[test]
    fn test_attach_skill_mounts_skill_md() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "pdf-tool".to_string(),
            "Extract text from PDFs".to_string(),
            "# Instructions\nUse pdfplumber.".to_string(),
            vec![],
        );

        let mounts = cap.mounts();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].path, "/.agents/skills/pdf-tool");
        assert!(mounts[0].is_readonly());
    }

    #[test]
    fn test_attach_skill_mounts_with_bundled_files() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry(
            skill_id,
            "data-skill".to_string(),
            "Analyze data".to_string(),
            "# Instructions".to_string(),
            vec![
                ("scripts/run.py".to_string(), "print('hi')".to_string()),
                ("references/REF.md".to_string(), "# Ref".to_string()),
            ],
        );

        let mounts = cap.mounts();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].path, "/.agents/skills/data-skill");

        // Verify directory contains SKILL.md + bundled files
        use crate::capability_types::MountSource;
        match &mounts[0].source {
            MountSource::InlineDirectory { entries } => {
                assert!(entries.contains_key("SKILL.md"));
                assert!(entries.contains_key("scripts/run.py"));
                assert!(entries.contains_key("references/REF.md"));
                assert_eq!(entries.len(), 3);
            }
            _ => panic!("Expected InlineDirectory"),
        }
    }

    #[test]
    fn test_reconstruct_skill_md_roundtrips() {
        let content = reconstruct_skill_md(
            "test-skill",
            "A test skill",
            "# Instructions\nDo the thing.",
            true,
            false,
        );

        // Should be parseable by parse_skill_md
        let parsed = crate::skill::parse_skill_md(&content).unwrap();
        assert_eq!(parsed.name, "test-skill");
        assert_eq!(parsed.description, "A test skill");
        assert!(parsed.instructions.contains("# Instructions"));
        assert!(parsed.user_invocable);
    }

    #[test]
    fn test_reconstruct_skill_md_escapes_description() {
        let content = reconstruct_skill_md(
            "test-skill",
            "Description with: colons and \"quotes\"",
            "# Body",
            true,
            false,
        );

        let parsed = crate::skill::parse_skill_md(&content).unwrap();
        assert_eq!(parsed.name, "test-skill");
        assert_eq!(
            parsed.description,
            "Description with: colons and \"quotes\""
        );
    }

    #[test]
    fn test_reconstruct_skill_md_not_invocable() {
        let content =
            reconstruct_skill_md("bg-skill", "Background context", "# Body", false, false);

        let parsed = crate::skill::parse_skill_md(&content).unwrap();
        assert_eq!(parsed.name, "bg-skill");
        assert!(!parsed.user_invocable);
    }

    #[test]
    fn test_reconstruct_skill_md_disable_model_invocation() {
        let content = reconstruct_skill_md("manual-skill", "Manual only", "# Body", true, true);

        let parsed = crate::skill::parse_skill_md(&content).unwrap();
        assert_eq!(parsed.name, "manual-skill");
        assert!(parsed.user_invocable);
        assert!(parsed.disable_model_invocation);
    }

    #[test]
    fn test_reconstruct_skill_md_both_flags() {
        let content = reconstruct_skill_md("both-flags", "Both flags set", "# Body", false, true);

        let parsed = crate::skill::parse_skill_md(&content).unwrap();
        assert!(!parsed.user_invocable);
        assert!(parsed.disable_model_invocation);
    }

    #[test]
    fn test_attach_skill_with_disable_model_invocation() {
        let skill_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let cap = AttachSkillCapability::from_registry_with_options(
            skill_id,
            "manual-skill".to_string(),
            "Manual only".to_string(),
            "# Instructions".to_string(),
            vec![],
            true,
            true,
        );

        assert_eq!(cap.name(), "manual-skill");
        assert!(cap.user_invocable());
    }

    #[test]
    fn test_attach_skill_dependencies() {
        let cap = AttachSkillCapability::from_registry(
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            "test".to_string(),
            "test".to_string(),
            "body".to_string(),
            vec![],
        );
        assert_eq!(cap.dependencies(), vec!["session_file_system"]);
    }

    #[test]
    fn test_skill_meta_serialization() {
        let meta = SkillMeta {
            name: "test-skill".to_string(),
            description: "A test".to_string(),
            source: SkillSource::Registry {
                skill_id: "abc".to_string(),
            },
            user_invocable: true,
            disable_model_invocation: false,
        };

        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("test-skill"));

        let parsed: SkillMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test-skill");
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
}
