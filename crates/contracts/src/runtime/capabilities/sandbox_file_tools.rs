//! One file system per session: a managed sandbox owns the file tools.
//!
//! A Worker harness brings the session file system (VFS) and a managed sandbox
//! brings its own `/workspace`. Both answer to the same tool names, and the
//! registry is last-wins by name, so which file system a call hit used to
//! depend on capability order, and the VFS-only tools (`read_many_files`,
//! `list_directory`, ...) always read the VFS. A model that read a file the
//! sandbox had just cloned got "File not found".
//!
//! The VFS capability stays applied (skills and output persistence write
//! through `ctx.file_store`), but when a managed sandbox is active every
//! model-facing file tool name belongs to it: file-system capabilities lose
//! those tools, their definitions, and their schema rewrites.

use std::collections::HashSet;

use crate::runtime::tool_types::ToolDefinition;
use crate::runtime::tools::Tool;

/// Feature declared by a capability that owns a managed sandbox workspace.
pub const MANAGED_SANDBOX_FEATURE: &str = "managed_sandbox";
/// Feature declared by capabilities that read or write the session file system.
pub const FILE_SYSTEM_FEATURE: &str = "file_system";

/// Model-facing tool names of the session file system.
pub const SESSION_FILE_TOOL_NAMES: &[&str] = &[
    "read_file",
    "read_many_files",
    "write_file",
    "edit_file",
    "glob",
    "grep",
    "list_directory",
    "grep_files",
    "delete_file",
    "stat_file",
];

/// Whether a capability's file tools give way to an active managed sandbox.
pub(crate) fn yields_file_tools(features: &[&str]) -> bool {
    features.contains(&FILE_SYSTEM_FEATURE) && !features.contains(&MANAGED_SANDBOX_FEATURE)
}

pub(crate) fn is_session_file_tool(name: &str) -> bool {
    SESSION_FILE_TOOL_NAMES.contains(&name)
}

/// Keep the last tool per name, at the position of that last occurrence.
///
/// Mirrors the registry's last-wins rule so the definitions sent to the model
/// never list a tool twice or describe one the registry will not run.
pub(crate) fn dedupe_last_wins<T>(items: Vec<T>, name: impl Fn(&T) -> &str) -> Vec<T> {
    let mut seen = HashSet::new();
    let mut kept: Vec<T> = Vec::with_capacity(items.len());
    for item in items.into_iter().rev() {
        if seen.insert(name(&item).to_string()) {
            kept.push(item);
        }
    }
    kept.reverse();
    kept
}

pub(crate) fn dedupe_tools(tools: Vec<Box<dyn Tool>>) -> Vec<Box<dyn Tool>> {
    dedupe_last_wins(tools, |tool| tool.name())
}

pub(crate) fn dedupe_definitions(definitions: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
    dedupe_last_wins(definitions, |definition| definition.name())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::tests_support::{FixtureTool, test_ctx};
    use super::super::{Capability, CapabilityRegistry, ToolDefinitionHook, collect_capabilities};
    use super::*;

    /// Stands in for `session_file_system`: every VFS file tool, plus a schema
    /// rewrite keyed by tool name like the real path-presentation hook.
    struct VfsFixture;

    struct RewriteFileSchemas;

    impl ToolDefinitionHook for RewriteFileSchemas {
        fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
            tools
                .into_iter()
                .map(|tool| {
                    if is_session_file_tool(tool.name()) {
                        tool.with_category("rewritten-by-vfs")
                    } else {
                        tool
                    }
                })
                .collect()
        }
    }

    impl Capability for VfsFixture {
        fn id(&self) -> &str {
            "session_file_system"
        }
        fn name(&self) -> &str {
            "VFS"
        }
        fn description(&self) -> &str {
            "VFS fixture."
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            SESSION_FILE_TOOL_NAMES
                .iter()
                .map(|name| Box::new(FixtureTool(name)) as Box<dyn Tool>)
                .collect()
        }
        fn system_prompt_addition(&self) -> Option<&str> {
            Some("Use grep_files on the VFS.")
        }
        fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
            vec![Arc::new(RewriteFileSchemas)]
        }
        fn features(&self) -> Vec<&'static str> {
            vec![FILE_SYSTEM_FEATURE]
        }
    }

    /// Stands in for `session_sandbox`, which does not offer every VFS name.
    struct SandboxFixture;

    const SANDBOX_TOOLS: &[&str] = &[
        "bash",
        "read_file",
        "read_many_files",
        "write_file",
        "edit_file",
        "glob",
        "grep",
        "list_directory",
    ];

    impl Capability for SandboxFixture {
        fn id(&self) -> &str {
            "session_sandbox"
        }
        fn name(&self) -> &str {
            "Sandbox"
        }
        fn description(&self) -> &str {
            "Sandbox fixture."
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            SANDBOX_TOOLS
                .iter()
                .map(|name| Box::new(FixtureTool(name)) as Box<dyn Tool>)
                .collect()
        }
        fn features(&self) -> Vec<&'static str> {
            vec![MANAGED_SANDBOX_FEATURE]
        }
    }

    fn registry() -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::new();
        registry.register(VfsFixture);
        registry.register(SandboxFixture);
        registry
    }

    /// Whichever order the layers merge in, a file tool name either runs in
    /// the sandbox or is not offered at all.
    #[tokio::test]
    async fn a_managed_sandbox_owns_every_session_file_tool_name() {
        for ids in [
            ["session_file_system", "session_sandbox"],
            ["session_sandbox", "session_file_system"],
        ] {
            let ids = ids.map(String::from);
            let collected = collect_capabilities(&ids, &registry(), &test_ctx()).await;
            let mut tool_definitions = collected.tool_definitions;
            for hook in &collected.tool_definition_hooks {
                tool_definitions = hook.transform(tool_definitions);
            }

            let names: Vec<&str> = tool_definitions.iter().map(|d| d.name()).collect();
            let unique: HashSet<&str> = names.iter().copied().collect();
            assert_eq!(
                names.len(),
                unique.len(),
                "{ids:?}: duplicates in {names:?}"
            );

            for name in SESSION_FILE_TOOL_NAMES {
                let definition = tool_definitions.iter().find(|d| d.name() == *name);
                if SANDBOX_TOOLS.contains(name) {
                    let definition = definition.unwrap_or_else(|| panic!("{ids:?}: {name}"));
                    assert_eq!(
                        definition.hints().capability_id.as_deref(),
                        Some("session_sandbox"),
                        "{ids:?}: {name}"
                    );
                    assert_ne!(definition.category(), Some("rewritten-by-vfs"));
                } else {
                    assert!(definition.is_none(), "{ids:?}: VFS-only {name} offered");
                }
            }

            let tool_names: Vec<&str> = collected.tools.iter().map(|t| t.name()).collect();
            assert_eq!(tool_names.len(), names.len(), "{ids:?}");
            assert!(!tool_names.contains(&"grep_files"), "{ids:?}");
            // Its prompt describes tools the model no longer has.
            assert!(
                !collected
                    .system_prompt_parts
                    .iter()
                    .any(|part| part.contains("grep_files")),
                "{ids:?}"
            );
            // The VFS capability is still applied for skills and persistence.
            assert!(
                collected
                    .applied_ids
                    .iter()
                    .any(|id| id == "session_file_system")
            );
        }
    }

    #[tokio::test]
    async fn without_a_sandbox_the_vfs_keeps_its_tools() {
        let ids = ["session_file_system".to_string()];
        let collected = collect_capabilities(&ids, &registry(), &test_ctx()).await;
        let names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|d| d.name())
            .collect();
        assert_eq!(names, SESSION_FILE_TOOL_NAMES);
        assert_eq!(collected.tool_definition_hooks.len(), 1);
        assert!(
            collected
                .system_prompt_parts
                .iter()
                .any(|part| part.contains("grep_files"))
        );
    }

    #[test]
    fn dedupe_keeps_the_last_occurrence_in_place() {
        let names = vec!["a", "b", "a", "c", "b"];
        assert_eq!(dedupe_last_wins(names, |name| name), vec!["a", "c", "b"]);
    }

    #[test]
    fn only_plain_file_system_capabilities_yield() {
        assert!(yields_file_tools(&["file_system"]));
        assert!(!yields_file_tools(&["managed_sandbox"]));
        assert!(!yields_file_tools(&["file_system", "managed_sandbox"]));
        assert!(!yields_file_tools(&["schedules"]));
    }
}
