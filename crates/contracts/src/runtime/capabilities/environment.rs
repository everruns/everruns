// What a plugin needs from the agent that runs it.
//
// Lives here, not in core with the rest of plugin compilation, because the
// shared collector enforces it: a plugin definition carries the environment it
// needs, and collection drops it when the agent's full capability list
// (harness, agent and session together) does not provide that. Core's
// `plugins::environment` derives the requirement from a manifest.

use serde::{Deserialize, Serialize};

/// What a plugin needs from the agent that runs it, weakest first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum PluginEnvironment {
    /// Chat only: no workspace and no shell. MCP tools and prompts.
    Conversation,
    /// Files and skills (the Worker level).
    Workspace,
    /// A virtual shell, for skills that run scripts (Bashkit Worker).
    VirtualShell,
    /// A full sandbox: real processes, packages, local MCP servers.
    #[default]
    Sandbox,
}

impl PluginEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Workspace => "workspace",
            Self::VirtualShell => "virtual-shell",
            Self::Sandbox => "sandbox",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "conversation" => Some(Self::Conversation),
            "workspace" => Some(Self::Workspace),
            "virtual-shell" => Some(Self::VirtualShell),
            "sandbox" => Some(Self::Sandbox),
            _ => None,
        }
    }

    /// The level that provides it, as people see it.
    pub fn level_name(self) -> &'static str {
        match self {
            Self::Conversation => "Conversation",
            Self::Workspace => "Worker",
            Self::VirtualShell => "Bashkit Worker",
            Self::Sandbox => "Sandbox Worker",
        }
    }

    /// What an agent with these (resolved, canonical) capabilities provides.
    ///
    /// `session_sandbox` is a real sandbox; `bashkit_shell` a virtual shell;
    /// `session_file_system` a workspace, where skills live.
    pub fn provided_by<'a>(capability_ids: impl IntoIterator<Item = &'a str>) -> Self {
        let mut provided = Self::Conversation;
        for id in capability_ids {
            let level = match id {
                crate::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID => Self::Sandbox,
                "bashkit_shell" | "virtual_bash" => Self::VirtualShell,
                "session_file_system" => Self::Workspace,
                _ => continue,
            };
            provided = provided.max(level);
        }
        provided
    }

    /// Whether an agent that provides `provided` can run a plugin that
    /// needs `self`.
    pub fn satisfied_by(self, provided: Self) -> bool {
        provided >= self
    }
}

impl std::fmt::Display for PluginEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
