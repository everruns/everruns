// What a plugin needs from the agent that runs it.
//
// Decisions (knowledge/integrations/plugins.md, "Environment"):
// - Agent Plugins 1.0.0 has no field for this, and guessing it from a
//   plugin's files is unreliable (a skill that only reads docs looks like one
//   that runs scripts). So it is declared, in the one place the spec leaves
//   for vendors: `extensions["com.everruns"].environment`.
// - The four values follow the harness tree: Conversation (no workspace, no
//   shell), Worker (files and skills), Bashkit Worker (a virtual shell), and
//   Sandbox Worker (real processes and packages). Each includes the ones
//   before it, so they are ordered.
// - A declaration is checked only for contradictions: a plugin with skills
//   needs at least `workspace`, a skill that ships scripts at least
//   `virtual-shell`. Local (stdio) MCP servers are not run, so they set no
//   bound; a plugin that relies on one declares `sandbox`. A contradiction
//   raises the requirement to the bound and adds a warning, so a wrong
//   declaration can only make a plugin harder to load, never easier.
// - No declaration means `sandbox`: the strictest, until a person (the
//   directory's reviewer, or an org admin) says otherwise.

use serde::{Deserialize, Serialize};

use crate::capabilities::DeclarativeCapabilityDefinition;

/// Vendor extension key in `plugin.json`'s `extensions`.
pub const EVERRUNS_EXTENSION: &str = "com.everruns";

use everruns_contracts::runtime::capabilities::PluginEnvironment;

/// A plugin's requirement: what it declares, raised to what its contents
/// need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRequirement {
    /// What the agent must provide.
    pub environment: PluginEnvironment,
    /// Whether the plugin declared it (`extensions["com.everruns"].environment`).
    pub declared: bool,
    /// Tools the plugin says it needs beyond its own (`extensions["com.everruns"].tools`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

/// The `com.everruns` extension of a manifest, if any.
fn extension(manifest: &serde_json::Value) -> Option<&serde_json::Value> {
    manifest.get("extensions")?.get(EVERRUNS_EXTENSION)
}

/// The lowest environment a compiled plugin's contents need, with why.
fn content_bound(
    definition: &DeclarativeCapabilityDefinition,
) -> (PluginEnvironment, Option<&'static str>) {
    let scripts = definition.skills.iter().any(|skill| {
        skill
            .files
            .iter()
            .any(|file| file.path.starts_with("scripts/") || file.path.contains("/scripts/"))
    });
    if scripts {
        return (PluginEnvironment::VirtualShell, Some("skill scripts"));
    }
    if !definition.skills.is_empty() || !definition.files.is_empty() {
        return (PluginEnvironment::Workspace, Some("skills"));
    }
    (PluginEnvironment::Conversation, None)
}

/// A plugin's requirement from its stored manifest and compiled definition.
/// Warnings go to `warnings`.
pub fn plugin_requirement(
    manifest: &serde_json::Value,
    definition: &DeclarativeCapabilityDefinition,
    warnings: &mut Vec<String>,
) -> PluginRequirement {
    let extension = extension(manifest);
    let declared = extension
        .and_then(|extension| extension.get("environment"))
        .and_then(serde_json::Value::as_str);
    let tools = extension
        .and_then(|extension| extension.get("tools"))
        .and_then(serde_json::Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let (bound, reason) = content_bound(definition);
    match declared.map(|value| (value, PluginEnvironment::parse(value))) {
        Some((_, Some(environment))) if environment >= bound => PluginRequirement {
            environment,
            declared: true,
            tools,
        },
        Some((_, Some(environment))) => {
            warnings.push(format!(
                "declares environment '{environment}', but {} need '{bound}'; using '{bound}'",
                reason.unwrap_or("its contents")
            ));
            PluginRequirement {
                environment: bound,
                declared: true,
                tools,
            }
        }
        Some((value, None)) => {
            warnings.push(format!(
                "unknown com.everruns environment '{value}'; using 'sandbox'"
            ));
            PluginRequirement {
                environment: PluginEnvironment::Sandbox,
                declared: false,
                tools,
            }
        }
        None => PluginRequirement {
            environment: PluginEnvironment::Sandbox,
            declared: false,
            tools,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{DeclarativeCapabilitySkill, DeclarativeCapabilitySkillFile};

    fn definition(skill_files: Option<&[&str]>) -> DeclarativeCapabilityDefinition {
        DeclarativeCapabilityDefinition {
            name: "p".into(),
            description: "p".into(),
            skills: skill_files
                .map(|files| {
                    vec![DeclarativeCapabilitySkill {
                        name: "s".into(),
                        description: "s".into(),
                        instructions: "do".into(),
                        files: files
                            .iter()
                            .map(|path| DeclarativeCapabilitySkillFile {
                                path: path.to_string(),
                                content: String::new(),
                            })
                            .collect(),
                        user_invocable: true,
                        disable_model_invocation: false,
                    }]
                })
                .unwrap_or_default(),
            ..DeclarativeCapabilityDefinition::default()
        }
    }

    fn manifest(environment: Option<&str>) -> serde_json::Value {
        match environment {
            Some(environment) => serde_json::json!({
                "name": "p",
                "extensions": {"com.everruns": {"environment": environment, "tools": ["web_fetch"]}}
            }),
            None => serde_json::json!({"name": "p"}),
        }
    }

    #[test]
    fn declared_environment_is_used_when_contents_allow_it() {
        let mut warnings = Vec::new();
        let requirement = plugin_requirement(
            &manifest(Some("workspace")),
            &definition(Some(&["references/a.md"])),
            &mut warnings,
        );
        assert_eq!(requirement.environment, PluginEnvironment::Workspace);
        assert!(requirement.declared);
        assert_eq!(requirement.tools, ["web_fetch"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_contradiction_raises_the_requirement_and_warns() {
        for (declared, files, expected) in [
            ("conversation", &[][..], PluginEnvironment::Workspace),
            (
                "workspace",
                &["scripts/run.sh"][..],
                PluginEnvironment::VirtualShell,
            ),
        ] {
            let mut warnings = Vec::new();
            let requirement = plugin_requirement(
                &manifest(Some(declared)),
                &definition(Some(files)),
                &mut warnings,
            );
            assert_eq!(requirement.environment, expected, "{declared}");
            assert_eq!(warnings.len(), 1, "{declared}: {warnings:?}");
        }
    }

    #[test]
    fn missing_or_unknown_declaration_needs_a_sandbox() {
        let mut warnings = Vec::new();
        let missing = plugin_requirement(&manifest(None), &definition(None), &mut warnings);
        assert_eq!(missing.environment, PluginEnvironment::Sandbox);
        assert!(!missing.declared);
        assert!(warnings.is_empty());
        let unknown =
            plugin_requirement(&manifest(Some("cloud")), &definition(None), &mut warnings);
        assert_eq!(unknown.environment, PluginEnvironment::Sandbox);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn capabilities_provide_the_highest_level_they_reach() {
        assert_eq!(
            PluginEnvironment::provided_by(["ask_user", "message_metadata"]),
            PluginEnvironment::Conversation
        );
        assert_eq!(
            PluginEnvironment::provided_by(["session_file_system", "skills"]),
            PluginEnvironment::Workspace
        );
        assert_eq!(
            PluginEnvironment::provided_by(["session_file_system", "bashkit_shell"]),
            PluginEnvironment::VirtualShell
        );
        assert_eq!(
            PluginEnvironment::provided_by(["session_sandbox"]),
            PluginEnvironment::Sandbox
        );
        assert!(PluginEnvironment::Workspace.satisfied_by(PluginEnvironment::VirtualShell));
        assert!(!PluginEnvironment::VirtualShell.satisfied_by(PluginEnvironment::Workspace));
    }

    #[test]
    fn serde_uses_the_declared_spelling() {
        assert_eq!(
            serde_json::to_value(PluginEnvironment::VirtualShell).unwrap(),
            "virtual-shell"
        );
        for environment in [
            PluginEnvironment::Conversation,
            PluginEnvironment::Workspace,
            PluginEnvironment::VirtualShell,
            PluginEnvironment::Sandbox,
        ] {
            assert_eq!(
                PluginEnvironment::parse(environment.as_str()),
                Some(environment)
            );
        }
    }
}
