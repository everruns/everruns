use super::{
    CapabilityStatus, MountAccess, MountPoint, RiskLevel, SKILLS_DISCOVERY_PATH, SkillContribution,
};
use crate::capability_types::MountSource;
use crate::{CapabilityInfo, ScopedMcpServers, validate_skill_name};
use everruns_capability::{CapabilityId, plugin_capability_id};
use serde::{Deserialize, Serialize};

pub const DECLARATIVE_CAPABILITY_PREFIX: &str = "declarative:";
// Capability refs are persisted in existing VARCHAR(50) capability columns.
// `declarative:` is 12 bytes, leaving 38 bytes for the unique name.
const MAX_NAME_BYTES: usize = 38;
const MAX_DISPLAY_NAME_BYTES: usize = 80;
const MAX_PROMPT_BYTES: usize = 64 * 1024;
const MAX_FILES: usize = 32;
const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_SKILLS: usize = 16;
const MAX_SKILL_BYTES: usize = 64 * 1024;
const MAX_MCP_SERVERS: usize = 16;

pub fn declarative_capability_id(name: &str) -> String {
    format!("{DECLARATIVE_CAPABILITY_PREFIX}{name}")
}

pub fn is_declarative_capability(capability_id: &str) -> bool {
    capability_id.starts_with(DECLARATIVE_CAPABILITY_PREFIX)
}

pub fn parse_declarative_capability_id(capability_id: &str) -> Option<&str> {
    capability_id.strip_prefix(DECLARATIVE_CAPABILITY_PREFIX)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeCapabilityDefinition {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub description: String,
    #[serde(default = "default_status")]
    pub status: CapabilityStatus,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub mcp_servers: Option<ScopedMcpServers>,
    #[serde(default)]
    pub skills: Vec<DeclarativeCapabilitySkill>,
    #[serde(default)]
    pub files: Vec<DeclarativeCapabilityFile>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default = "default_risk_level")]
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeCapabilityFile {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub access: MountAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeCapabilitySkill {
    pub name: String,
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub files: Vec<DeclarativeCapabilitySkillFile>,
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    #[serde(default)]
    pub disable_model_invocation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeCapabilitySkillFile {
    pub path: String,
    pub content: String,
}

fn default_true() -> bool {
    true
}

fn default_status() -> CapabilityStatus {
    CapabilityStatus::Available
}

fn default_risk_level() -> RiskLevel {
    RiskLevel::Low
}

impl Default for DeclarativeCapabilityDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            display_name: None,
            description: String::new(),
            status: CapabilityStatus::Available,
            icon: Some("puzzle".to_string()),
            category: Some("Declarative".to_string()),
            system_prompt: None,
            mcp_servers: None,
            skills: Vec::new(),
            files: Vec::new(),
            dependencies: Vec::new(),
            features: Vec::new(),
            risk_level: RiskLevel::Low,
        }
    }
}

impl DeclarativeCapabilityDefinition {
    pub fn mounts(&self, capability_id: &str) -> Vec<MountPoint> {
        self.files
            .iter()
            .map(|file| {
                let source = MountSource::text_file(file.content.clone());
                match file.access {
                    MountAccess::ReadOnly => {
                        MountPoint::readonly(file.path.clone(), source, capability_id)
                    }
                    MountAccess::ReadWrite => {
                        MountPoint::readwrite(file.path.clone(), source, capability_id)
                    }
                }
            })
            .collect()
    }

    pub fn skill_contributions(&self) -> Vec<SkillContribution> {
        self.skills
            .iter()
            .map(|skill| {
                SkillContribution::new(
                    skill.name.clone(),
                    skill.description.clone(),
                    skill.instructions.clone(),
                )
                .with_files(
                    skill
                        .files
                        .iter()
                        .map(|file| (file.path.clone(), file.content.clone()))
                        .collect(),
                )
                .with_user_invocable(skill.user_invocable)
                .with_disable_model_invocation(skill.disable_model_invocation)
            })
            .collect()
    }
}

pub fn hydrate_declarative_capability_config(
    _config: serde_json::Value,
    definition: &DeclarativeCapabilityDefinition,
) -> serde_json::Value {
    serde_json::to_value(definition).unwrap_or_default()
}

pub fn declarative_capability_info(
    name: &str,
    definition: DeclarativeCapabilityDefinition,
) -> CapabilityInfo {
    CapabilityInfo {
        id: CapabilityId::new(declarative_capability_id(name)),
        name: definition.display_name.unwrap_or(definition.name),
        description: definition.description,
        status: definition.status,
        icon: definition.icon.or_else(|| Some("puzzle".to_string())),
        category: definition
            .category
            .or_else(|| Some("Declarative".to_string())),
        system_prompt: definition.system_prompt,
        tool_definitions: Vec::new(),
        is_mcp: false,
        is_skill: false,
        is_guardrail: false,
        dependencies: definition.dependencies,
        features: definition.features,
        config_schema: None,
        config_ui_schema: None,
        risk_level: definition.risk_level,
        agent_count: 0,
        harness_count: 0,
        docs_slug: None,
        localizations: Default::default(),
    }
}

/// Hydrate a `plugin:` capability config: same logic as the declarative counterpart.
///
/// The per-agent config for a `plugin:` capability ref is the serialized
/// `DeclarativeCapabilityDefinition` produced by the compiler. Hydration simply
/// re-serializes the definition so callers get a canonical JSON value.
pub fn hydrate_plugin_capability_config(
    config: serde_json::Value,
    definition: &DeclarativeCapabilityDefinition,
) -> serde_json::Value {
    // Identical to the declarative path: discard the incoming config and
    // return the canonical definition. The `plugin:` namespace keeps refs
    // from colliding with `declarative:` refs.
    hydrate_declarative_capability_config(config, definition)
}

/// Build a `CapabilityInfo` DTO for a plugin capability identity.
///
/// Server installs pass their public ID; standalone runtime plugins pass their
/// manifest name. Both remain distinct from `declarative:{name}`.
pub fn plugin_capability_info(
    identity: &str,
    definition: DeclarativeCapabilityDefinition,
) -> CapabilityInfo {
    CapabilityInfo {
        id: CapabilityId::new(plugin_capability_id(identity)),
        name: definition
            .display_name
            .clone()
            .unwrap_or_else(|| definition.name.clone()),
        description: definition.description.clone(),
        status: definition.status,
        icon: definition
            .icon
            .clone()
            .or_else(|| Some("puzzle".to_string())),
        category: definition
            .category
            .clone()
            .or_else(|| Some("Plugin".to_string())),
        system_prompt: definition.system_prompt.clone(),
        tool_definitions: Vec::new(),
        is_mcp: false,
        is_skill: false,
        is_guardrail: false,
        dependencies: definition.dependencies.clone(),
        features: definition.features.clone(),
        config_schema: None,
        config_ui_schema: None,
        risk_level: definition.risk_level,
        agent_count: 0,
        harness_count: 0,
        docs_slug: None,
        localizations: Default::default(),
    }
}

pub fn validate_declarative_capability_definition(
    definition: &DeclarativeCapabilityDefinition,
) -> Result<(), String> {
    validate_name(&definition.name)?;
    if let Some(display_name) = &definition.display_name {
        validate_non_empty("display_name", display_name, MAX_DISPLAY_NAME_BYTES)?;
    }
    validate_non_empty("description", &definition.description, 512)?;

    if let Some(prompt) = &definition.system_prompt {
        validate_size("system_prompt", prompt, MAX_PROMPT_BYTES)?;
    }
    if let Some(servers) = &definition.mcp_servers
        && servers.len() > MAX_MCP_SERVERS
    {
        return Err(format!(
            "mcp_servers cannot contain more than {MAX_MCP_SERVERS} entries"
        ));
    }
    if definition.files.len() > MAX_FILES {
        return Err(format!(
            "files cannot contain more than {MAX_FILES} entries"
        ));
    }
    if definition.skills.len() > MAX_SKILLS {
        return Err(format!(
            "skills cannot contain more than {MAX_SKILLS} entries"
        ));
    }

    for dependency in &definition.dependencies {
        if is_declarative_capability(dependency) {
            return Err("declarative capability dependencies cannot reference other declarative capabilities".to_string());
        }
    }

    for file in &definition.files {
        validate_mount_path(&file.path)?;
        validate_size(
            &format!("file {}", file.path),
            &file.content,
            MAX_FILE_BYTES,
        )?;
        if file.path.starts_with(SKILLS_DISCOVERY_PATH) {
            return Err(format!(
                "file path {} is reserved; use skills[] for skill contributions",
                file.path
            ));
        }
    }

    for skill in &definition.skills {
        validate_skill_name(&skill.name).map_err(|errors| {
            format!("invalid skill name '{}': {}", skill.name, errors.join("; "))
        })?;
        validate_non_empty("skill.description", &skill.description, 512)?;
        validate_size(
            &format!("skill {} instructions", skill.name),
            &skill.instructions,
            MAX_SKILL_BYTES,
        )?;
        for file in &skill.files {
            validate_relative_path(&file.path)?;
            validate_size(
                &format!("skill {} file {}", skill.name, file.path),
                &file.content,
                MAX_FILE_BYTES,
            )?;
        }
    }

    Ok(())
}

fn validate_non_empty(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} is required"));
    }
    validate_size(field, value, max)
}

fn validate_name(name: &str) -> Result<(), String> {
    validate_non_empty("name", name, MAX_NAME_BYTES)?;
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("name is required".to_string());
    };
    if !first.is_ascii_lowercase() {
        return Err("name must start with a lowercase letter".to_string());
    }
    if !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-') {
        return Err("name may contain only lowercase letters, digits, '_' and '-'".to_string());
    }
    if name.ends_with('_') || name.ends_with('-') {
        return Err("name cannot end with '_' or '-'".to_string());
    }
    Ok(())
}

fn validate_size(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(format!("{field} cannot exceed {max} bytes"));
    }
    Ok(())
}

fn validate_mount_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') || path.contains("..") || path.contains("//") {
        return Err(format!("invalid mount path: {path}"));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.starts_with('/') || path.contains("..") || path.contains("//") || path.trim().is_empty()
    {
        return Err(format!("invalid relative file path: {path}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_definition() -> DeclarativeCapabilityDefinition {
        DeclarativeCapabilityDefinition {
            name: "research_pack".to_string(),
            display_name: Some("Research Pack".to_string()),
            description: "Curated research behavior".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn declarative_capability_ref_uses_unique_name() {
        assert_eq!(
            declarative_capability_id("research_pack"),
            "declarative:research_pack"
        );
        assert_eq!(
            parse_declarative_capability_id("declarative:research_pack"),
            Some("research_pack")
        );
    }

    #[test]
    fn validation_accepts_name_and_display_name() {
        validate_declarative_capability_definition(&valid_definition()).unwrap();
    }

    #[test]
    fn validation_rejects_names_that_do_not_fit_capability_ref_columns() {
        let mut definition = valid_definition();
        definition.name = "a".repeat(MAX_NAME_BYTES + 1);
        let err = validate_declarative_capability_definition(&definition).unwrap_err();
        assert!(err.contains("name cannot exceed"));
    }

    #[test]
    fn capability_info_uses_display_name_for_title() {
        let info = declarative_capability_info("research_pack", valid_definition());
        assert_eq!(info.id.as_str(), "declarative:research_pack");
        assert_eq!(info.name, "Research Pack");
    }
}
