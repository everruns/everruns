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
use crate::capability_types::{MountDirectoryBuilder, MountPoint};

use super::Capability;
pub use everruns_core::capabilities::{
    MAX_SKILLS_PER_CAPABILITY, SKILL_CAPABILITY_PREFIX, SKILLS_DISCOVERY_PATH,
    SkillCapabilityIdExt, SkillContribution, SkillInstructions, SkillMeta, SkillSource,
    discover_skills_from_entries, is_skill_capability, parse_skill_capability_id,
    reconstruct_skill_md, skill_capability_id,
};
use uuid::Uuid;

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
        &self.capability_id
    }

    fn name(&self) -> &str {
        &self.skill_name
    }

    fn description(&self) -> &str {
        &self.skill_description
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_constructors_preserve_metadata_flags_and_complete_mounts() {
        use crate::capability_types::MountSource;
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        for variant in 0..6 {
            let (user, disabled) = match variant {
                0 => (true, false),
                1 => (false, false),
                2 => (false, false),
                3 => (false, true),
                4 => (true, false),
                _ => (true, true),
            };
            let files = vec![
                ("scripts/run.sh".into(), "echo α".into()),
                ("reference.md".into(), "# Reference".into()),
            ];
            let cap = match variant {
                0 => AttachSkillCapability::from_registry(
                    id,
                    "ops".into(),
                    "Operations".into(),
                    "Exact instructions.".into(),
                    files,
                ),
                1 => AttachSkillCapability::from_registry_with_invocable(
                    id,
                    "ops".into(),
                    "Operations".into(),
                    "Exact instructions.".into(),
                    files,
                    false,
                ),
                _ => AttachSkillCapability::from_registry_with_options(
                    id,
                    "ops".into(),
                    "Operations".into(),
                    "Exact instructions.".into(),
                    files,
                    user,
                    disabled,
                ),
            };
            assert_eq!(cap.id(), "skill:550e8400-e29b-41d4-a716-446655440000");
            assert_eq!(cap.name(), "ops");
            assert_eq!(cap.skill_name(), "ops");
            assert_eq!(cap.description(), "Operations");
            assert_eq!(cap.status(), CapabilityStatus::Available);
            assert_eq!(cap.icon(), Some("wand"));
            assert_eq!(cap.category(), Some("Skills"));
            assert_eq!(cap.user_invocable(), user);
            assert_eq!(cap.disable_model_invocation(), disabled);
            assert_eq!(cap.dependencies(), ["session_file_system"]);
            assert!(cap.system_prompt_addition().is_none());
            assert!(cap.tools().is_empty());
            assert!(cap.tool_definitions().is_empty());
            let mounts = cap.mounts();
            assert_eq!(mounts.len(), 1);
            assert_eq!(mounts[0].path, "/.agents/skills/ops");
            assert_eq!(
                mounts[0].capability_id,
                "skill:550e8400-e29b-41d4-a716-446655440000"
            );
            assert!(mounts[0].is_readonly());
            let MountSource::InlineDirectory { entries } = &mounts[0].source else {
                panic!("expected directory")
            };
            assert_eq!(entries.len(), 3);
            assert_eq!(
                entries["scripts/run.sh"].source,
                MountSource::text_file("echo α")
            );
            assert_eq!(
                entries["reference.md"].source,
                MountSource::text_file("# Reference")
            );
            let MountSource::InlineFile { content, encoding } = &entries["SKILL.md"].source else {
                panic!("expected SKILL.md")
            };
            assert_eq!(encoding, "text");
            let parsed = crate::skill::parse_skill_md(content).unwrap();
            assert_eq!(parsed.name, "ops");
            assert_eq!(parsed.description, "Operations");
            assert_eq!(parsed.instructions, "Exact instructions.");
            assert_eq!(parsed.user_invocable, user);
            assert_eq!(parsed.disable_model_invocation, disabled);
        }
    }
}
