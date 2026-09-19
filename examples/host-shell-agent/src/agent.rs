use std::path::Path;

use everruns::{Agent, BuildError, ContainmentMode, HostShell, Provider, WorkspacePolicy};

pub const MODEL: &str = "gpt-5.6-terra";

/// An agent with one capability: a contained shell on this machine.
///
/// `workspace-write` is the whole point of the example. The agent needs to
/// compile and run a test binary, which no virtual shell can do, and it must not
/// be able to write anywhere else while doing it. The boundary is a kernel
/// policy rather than a rule in the prompt, so a model that ignores the
/// instructions still cannot escape it.
pub fn build(provider: impl Into<Provider>, workspace: &Path) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("host-shell-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(provider)
        .model(MODEL)
        .max_iterations(16)
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_write())
        .capability(HostShell::new().containment(ContainmentMode::WorkspaceWrite))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_without_contacting_the_provider() {
        let workspace = tempfile::tempdir().unwrap();
        assert!(build(everruns::OpenAI::new("test-key"), workspace.path()).is_ok());
    }
}
