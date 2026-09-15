//! Building the agent for a selected environment.
//!
//! One agent definition, one task, two capability sets. The environment is not
//! a prompt variable: it is the set of capabilities the agent is given, which
//! is exactly what the profile in `environment.rs` is derived from.

use everruns::{Agent, BashkitShell, BuildError, OpenAI, WorkspacePolicy};
use std::path::Path;

use crate::environment::Target;

pub const MODEL: &str = "gpt-5.6-terra";

/// A target an agent can actually be built for.
///
/// `Target::Host` has no variant here on purpose. Nothing routes agent tool
/// calls through `Environment::compute()` yet, so a host agent would carry a
/// declared target and no way to use it; an absent variant is the honest form
/// of that, and it keeps the impossible case out of the builder entirely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runnable {
    Bashkit,
    Files,
}

impl Runnable {
    pub fn from_target(target: Target) -> Option<Self> {
        match target {
            Target::Bashkit => Some(Self::Bashkit),
            Target::Files => Some(Self::Files),
            Target::Host => None,
        }
    }
}

pub fn build(provider: OpenAI, workspace: &Path, target: Runnable) -> Result<Agent, BuildError> {
    let builder = Agent::builder()
        .name("environment-agent")
        .instructions(include_str!("instructions.md"))
        .provider(provider)
        .model(MODEL)
        .max_iterations(16)
        // One real host directory becomes the session's /workspace. The
        // read/write policy is an explicit opt-in; the default is read-only.
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_write());

    match target {
        // The shell is what makes this a `vfs` target rather than no target at
        // all, which is why the profile names `bashkit_shell` as its source.
        // It supplies the session filesystem itself, so adding `FileSystem`
        // alongside it would register `session_file_system` twice.
        Runnable::Bashkit => builder.capability(BashkitShell::new()).build(),
        // Nothing executes here. `workspace` already registers the session
        // file tools, so this is the whole surface, which is why the profile
        // reports no target rather than a contained one.
        Runnable::Files => builder.build(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_runnable_targets_build_without_contacting_the_provider() {
        let workspace = tempfile::tempdir().expect("temp dir");
        for target in [Runnable::Bashkit, Runnable::Files] {
            build(OpenAI::new("test-key"), workspace.path(), target)
                .unwrap_or_else(|error| panic!("{target:?} should build: {error:?}"));
        }
    }

    #[test]
    fn the_host_target_has_no_runnable_form() {
        assert_eq!(Runnable::from_target(Target::Host), None);
        assert_eq!(
            Runnable::from_target(Target::Bashkit),
            Some(Runnable::Bashkit)
        );
    }
}
