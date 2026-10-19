//! Product guidance shared with the public skills, mounted through `platform`.

use everruns_core::capabilities::SkillContribution;
use std::sync::LazyLock;

const FILES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/platform_skills.rs"));

static SKILLS: LazyLock<Vec<SkillContribution>> = LazyLock::new(|| {
    FILES
        .iter()
        .filter(|(path, _)| path.ends_with("/SKILL.md") && path.split('/').count() == 2)
        .map(|(path, text)| {
            // Invalid product guidance is an error, never silently omitted.
            #[expect(
                clippy::expect_used,
                reason = "embedded authored skills are validated by tests"
            )]
            let parsed = everruns_contracts::runtime::skill::parse_skill_md(text)
                .expect("valid embedded platform skill");
            let prefix = path.strip_suffix("SKILL.md").unwrap_or_default();
            let files = FILES
                .iter()
                .filter_map(|(path, content)| {
                    let relative = path.strip_prefix(prefix)?;
                    (relative != "SKILL.md").then(|| (relative.to_string(), content.to_string()))
                })
                .collect();
            SkillContribution::new(parsed.name, parsed.description, parsed.instructions)
                .with_files(files)
                .with_user_invocable(parsed.user_invocable)
                .with_disable_model_invocation(parsed.disable_model_invocation)
        })
        .collect()
});

pub(super) fn contributions() -> Vec<SkillContribution> {
    SKILLS.clone()
}

#[cfg(all(test, everruns_has_workspace_skills))]
mod tests {
    use super::*;
    use crate::capabilities::platform::PlatformCapability;
    use everruns_contracts::CapabilityRef;
    use everruns_contracts::runtime::capability_types::MountSource;
    use everruns_core::capabilities::{
        Capability, CapabilityRegistry, SystemPromptContext, collect_capabilities_with_configs,
        resolve_capability_configs,
    };
    use serde_json::json;

    struct FileSystemDependency;

    impl Capability for FileSystemDependency {
        fn id(&self) -> &str {
            "session_file_system"
        }
        fn name(&self) -> &str {
            "Host filesystem"
        }
        fn description(&self) -> &str {
            "Host-provided filesystem dependency for skill mounts."
        }
    }

    #[tokio::test]
    async fn platform_skills_are_available_on_both_surfaces() {
        let mut registry = CapabilityRegistry::new();
        registry.register(PlatformCapability);
        registry.register(everruns_core::builtins::skills::SkillsCapability);
        registry.register(FileSystemDependency);
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        for config in [json!({}), json!({"surface": "shell"})] {
            let configs = resolve_capability_configs(
                &[CapabilityRef::with_config("platform", config)],
                &registry,
            )
            .unwrap();
            let collected = collect_capabilities_with_configs(&configs, &registry, &ctx).await;
            let mounts: Vec<_> = collected
                .mounts
                .iter()
                .filter(|mount| mount.path.starts_with("/.agents/skills/"))
                .collect();
            assert_eq!(mounts.len(), 3);
            for name in ["everruns", "everruns-cli", "everruns-agent-authoring"] {
                let mount = mounts
                    .iter()
                    .find(|mount| mount.path == format!("/.agents/skills/{name}"))
                    .unwrap();
                assert!(mount.is_readonly());
                assert_eq!(mount.capability_id, "platform");
                let MountSource::InlineDirectory { entries } = &mount.source else {
                    panic!("expected inline skill directory");
                };
                let MountSource::InlineFile { content, .. } = &entries["SKILL.md"].source else {
                    panic!("expected skill text");
                };
                let mounted = everruns_contracts::runtime::skill::parse_skill_md(content).unwrap();
                let source_path = format!("{name}/SKILL.md");
                let source = FILES
                    .iter()
                    .find(|(path, _)| *path == source_path)
                    .unwrap()
                    .1;
                let authored = everruns_contracts::runtime::skill::parse_skill_md(source).unwrap();
                assert_eq!(mounted.name, name);
                assert_eq!(mounted.description, authored.description);
                assert_eq!(mounted.instructions, authored.instructions);
            }
            let tools: Vec<_> = collected.tools.iter().map(|tool| tool.name()).collect();
            assert!(tools.contains(&"activate_skill"));
            assert!(tools.contains(&"list_skills"));
        }
    }
}
