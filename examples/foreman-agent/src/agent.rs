//! Bashkit coding agent and independent read-only verifier.

use std::path::Path;

use everruns::{Agent, BashkitShell, BuildError, Model, WorkspacePolicy};

/// The coding worker's model, served by OpenRouter.
pub const WORKER_MODEL: &str = "meta/muse-spark-1.3-contributor";
/// The supervisor model.
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
    // THREAT[TM-BASH-029]: verification cannot repair or rewrite the repository it judges.
    Agent::builder()
        .name("factory-verifier")
        .instructions(include_str!("resources/verifier.md"))
        .model(model)
        .max_iterations(10)
        .workspace(workspace)
        .workspace_policy(WorkspacePolicy::read_only())
        .capability(BashkitShell::new())
        .build()
}

/// The mission a coding worker is started on.
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
#[path = "../tests/unit/agent.rs"]
mod tests;
