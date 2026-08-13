//! Neutral skill-capability values shared by capability implementations.
//!
//! Concrete skill discovery and attachment capabilities live in
//! `everruns-builtins`. Core retains the stable `skill:` identity namespace,
//! mount contribution DTOs, and SKILL.md normalization used by declarative and
//! custom capabilities.

use crate::capability_types::{CapabilityId, MountDirectoryBuilder, MountPoint};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Skill capability ID prefix.
pub const SKILL_CAPABILITY_PREFIX: &str = "skill:";

/// Default path for filesystem-based skill discovery.
pub const SKILLS_DISCOVERY_PATH: &str = "/.agents/skills";

/// Maximum number of skills in a single capability.
pub const MAX_SKILLS_PER_CAPABILITY: usize = 50;

/// Generate the stable capability ID for a skill.
pub fn skill_capability_id(skill_id: Uuid) -> String {
    format!("{SKILL_CAPABILITY_PREFIX}{skill_id}")
}

/// Check whether an ID uses the stable skill-capability namespace.
pub fn is_skill_capability(capability_id: &str) -> bool {
    capability_id.starts_with(SKILL_CAPABILITY_PREFIX)
}

/// Parse the skill UUID from a stable capability ID.
pub fn parse_skill_capability_id(capability_id: &str) -> Option<Uuid> {
    capability_id
        .strip_prefix(SKILL_CAPABILITY_PREFIX)
        .and_then(|value| Uuid::parse_str(value).ok())
}

/// Metadata for a discovered skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    /// Skill name from SKILL.md frontmatter.
    pub name: String,
    /// Skill description from SKILL.md frontmatter.
    pub description: String,
    /// Source location.
    pub source: SkillSource,
    /// Whether this skill appears as a user slash command.
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    /// Whether the model is prevented from invoking this skill automatically.
    #[serde(default)]
    pub disable_model_invocation: bool,
}

fn default_true() -> bool {
    true
}

/// Where a skill was discovered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillSource {
    /// Discovered in the session filesystem.
    Filesystem { path: String },
    /// Loaded from a registry.
    Registry { skill_id: String },
}

/// Full skill content loaded on activation.
#[derive(Debug, Clone)]
pub struct SkillInstructions {
    /// Full SKILL.md body.
    pub instructions: String,
    /// Bundled files as path/content pairs.
    pub files: Vec<(String, String)>,
}

/// A skill contributed by a capability in code.
#[derive(Debug, Clone)]
pub struct SkillContribution {
    /// Skill name and mount-directory name.
    pub name: String,
    /// Short description shown in capability catalogs.
    pub description: String,
    /// SKILL.md instruction body.
    pub instructions: String,
    /// Bundled files mounted alongside SKILL.md.
    pub files: Vec<(String, String)>,
    /// Whether this skill is user-invocable.
    pub user_invocable: bool,
    /// Whether automatic model invocation is disabled.
    pub disable_model_invocation: bool,
}

impl SkillContribution {
    /// Create a contribution with user invocation enabled and model invocation allowed.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            instructions: instructions.into(),
            files: Vec::new(),
            user_invocable: true,
            disable_model_invocation: false,
        }
    }

    /// Attach bundled files mounted alongside SKILL.md.
    pub fn with_files(mut self, files: Vec<(String, String)>) -> Self {
        self.files = files;
        self
    }

    /// Set whether the skill is user-invocable.
    pub fn with_user_invocable(mut self, flag: bool) -> Self {
        self.user_invocable = flag;
        self
    }

    /// Set whether model invocation is disabled.
    pub fn with_disable_model_invocation(mut self, flag: bool) -> Self {
        self.disable_model_invocation = flag;
        self
    }

    /// Normalize the contribution into the read-only mount consumed by skill implementations.
    pub fn to_mount(&self, owner_id: &str) -> MountPoint {
        let skill_md = reconstruct_skill_md(
            &self.name,
            &self.description,
            &self.instructions,
            self.user_invocable,
            self.disable_model_invocation,
        );
        let mut builder = MountDirectoryBuilder::new().file("SKILL.md", &skill_md);
        for (path, content) in &self.files {
            builder = builder.file(path, content);
        }
        MountPoint::readonly(
            format!("{SKILLS_DISCOVERY_PATH}/{}", self.name),
            builder.build(),
            owner_id,
        )
    }
}

/// Reconstruct a canonical SKILL.md document from stored fields.
pub fn reconstruct_skill_md(
    name: &str,
    description: &str,
    instructions: &str,
    user_invocable: bool,
    disable_model_invocation: bool,
) -> String {
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

/// Parse SKILL.md files discovered in the session VFS.
pub fn discover_skills_from_entries(
    entries: &[(String, String)],
) -> Vec<(SkillMeta, SkillInstructions)> {
    let mut results = Vec::new();
    for (path, content) in entries {
        match crate::skill::parse_skill_md(content) {
            Ok(parsed) => results.push((
                SkillMeta {
                    name: parsed.name,
                    description: parsed.description,
                    source: SkillSource::Filesystem { path: path.clone() },
                    user_invocable: parsed.user_invocable,
                    disable_model_invocation: parsed.disable_model_invocation,
                },
                SkillInstructions {
                    instructions: parsed.instructions,
                    files: Vec::new(),
                },
            )),
            Err(errors) => tracing::warn!(
                path = %path,
                errors = ?errors,
                "Skipping invalid SKILL.md"
            ),
        }
    }
    results
}

/// Stable `skill:` namespace helpers for [`CapabilityId`].
pub trait SkillCapabilityIdExt: Sized {
    /// Check whether this ID names a skill capability.
    fn is_skill(&self) -> bool;
    /// Create an ID for a skill.
    fn skill(skill_id: Uuid) -> Self;
    /// Parse the skill UUID from this ID.
    fn skill_id(&self) -> Option<Uuid>;
}

impl SkillCapabilityIdExt for CapabilityId {
    fn is_skill(&self) -> bool {
        is_skill_capability(self.as_str())
    }

    fn skill(skill_id: Uuid) -> Self {
        Self::new(skill_capability_id(skill_id))
    }

    fn skill_id(&self) -> Option<Uuid> {
        parse_skill_capability_id(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_identity_round_trips() {
        let skill_id = Uuid::new_v4();
        let capability_id = skill_capability_id(skill_id);
        assert!(is_skill_capability(&capability_id));
        assert_eq!(parse_skill_capability_id(&capability_id), Some(skill_id));
        assert_eq!(CapabilityId::skill(skill_id).skill_id(), Some(skill_id));
    }

    #[test]
    fn contribution_mount_uses_stable_discovery_path() {
        let mount = SkillContribution::new("ops", "Operations", "Run safely.").to_mount("fixture");
        assert_eq!(mount.path, "/.agents/skills/ops");
    }
}
