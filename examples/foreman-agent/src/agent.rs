//! The two agents on the factory floor.
//!
//! Both are ordinary Framework agents on the Bashkit shell. The supervisor does
//! not reach inside either one: it never picks their tools, their files, or
//! their next step. It watches what they emit, and it owns exactly one thing —
//! whether a session keeps running.
//!
//! The asymmetry that matters is the workspace policy. The coding worker mounts
//! the repository read-write. The verifier mounts the same repository under the
//! default read-only policy, so "independent check" is a property of the mount
//! rather than a request in a prompt a model may decline to honor.

use std::path::Path;

use everruns::{Agent, BashkitShell, BuildError, Model, WorkspacePolicy};

/// The coding worker's model.
pub const WORKER_MODEL: &str = "gpt-5.6-terra";
/// The supervising classifier's model.
pub const FOREMAN_MODEL: &str = "jev-latest";

/// A worker that edits the repository.
pub fn worker(model: Model, workspace: &Path) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("factory-worker")
        .instructions(include_str!("resources/worker.md"))
        .model(model)
        .max_iterations(16)
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_write())
        .capability(BashkitShell::new())
        .build()
}

/// A verifier that can read the repository and nothing else.
pub fn verifier(model: Model, workspace: &Path) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("factory-verifier")
        .instructions(include_str!("resources/verifier.md"))
        .model(model)
        .max_iterations(10)
        .workspace(workspace)
        .capability(BashkitShell::new())
        .build()
}

/// The mission a coding worker is started on.
///
/// Deliberately broad. Foreman's bet is that supervision belongs outside the
/// worker's loop, so the mission stays the job — not a decomposition of it.
pub fn coding_mission(job: &str) -> String {
    format!(
        "Original job:\n{job}\n\n\
         You are a coding worker operating on this repository.\n\
         Inspect the existing repository and previous work before changing anything.\n\
         Continue working toward fully satisfying the original job.\n\
         Do not assume previous workers completed the job correctly.\n\
         Check your work before finishing.\n\
         Report what you did, what remains unresolved, and any problems you encountered.\n"
    )
}

/// The mission an independent verification pass is started on.
pub fn verification_mission(job: &str) -> String {
    format!(
        "Original job:\n{job}\n\n\
         You are an independent verification worker with read-only access.\n\
         Inspect the current repository against the original job.\n\
         Verify whether the implementation actually satisfies the request.\n\
         Look for missing requirements, incorrect behavior, incomplete implementation,\n\
         insufficient tests, and failures hidden by previous workers.\n\
         Do not assume previous workers were correct, and do not attempt repairs.\n\
         Report your findings clearly, and state plainly whether the job is satisfied.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_agents_build_without_contacting_a_provider() {
        let workspace = tempfile::tempdir().unwrap();
        let model = Model::simulated("ready");
        assert!(worker(model.clone(), workspace.path()).is_ok());
        assert!(verifier(model, workspace.path()).is_ok());
    }

    #[test]
    fn missions_carry_the_job_and_differ_in_posture() {
        let job = "Add tiered shipping rates.";
        assert!(coding_mission(job).contains(job));
        assert!(verification_mission(job).contains(job));
        assert!(verification_mission(job).contains("do not attempt repairs"));
    }
}
