use std::path::Path;

use everruns::{Agent, BashkitShell, BuildError, Provider, WorkspacePolicy};

pub const MODEL: &str = "gpt-5.6-terra";

pub fn build(provider: impl Into<Provider>, workspace: &Path) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("bashkit-repo-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(provider)
        .model(MODEL)
        .max_iterations(12)
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_write())
        .capability(BashkitShell::new())
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
