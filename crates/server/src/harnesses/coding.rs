//! Provider-neutral coding harness.
//!
//! Decisions:
//! - Tagged `beta`: code navigation and multi-repository work are new, and the
//!   tag is how the harness list says so.
//! - Language servers are on (they start on first use), as are structural
//!   search and edit and the repository map.
//! - `repositories` comes with no repositories listed. An agent lists its
//!   usual ones, a session may start with its own set, and the agent can add
//!   one it turns out to need (read-only unless the agent's config says
//!   otherwise).
//! - `github` is on with pull requests allowed, since a coding session that
//!   pushed a branch ends in one. Without a writable repository nothing can be
//!   pushed, so nothing can be opened either.

use crate::domains::harnesses::record::{BuiltInCapabilityDefinition, BuiltInHarnessDefinition};

pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "coding",
        "Coding",
        "Coding harness for software development on a full sandbox, across one or several GitHub repositories, with language servers, structural search and a repository map. The Agent's sandbox policy selects Daytona, E2B, Modal or a container without changing tool names.",
        super::coding_prompt::CODING_SYSTEM_PROMPT,
    )
    .with_icon("terminal")
    .with_parent_name("sandbox-worker")
    .with_tags(["coding", "environment", "built-in", "beta"])
    .with_capabilities([
        BuiltInCapabilityDefinition::new("github_scout"),
        BuiltInCapabilityDefinition::new("session_tasks"),
        BuiltInCapabilityDefinition::new("repositories"),
        BuiltInCapabilityDefinition::with_config(
            "github",
            serde_json::json!({ "allow_pull_requests": true }),
        ),
        BuiltInCapabilityDefinition::new("lsp"),
        BuiltInCapabilityDefinition::new("ast_grep"),
        BuiltInCapabilityDefinition::new("ast_edit"),
        BuiltInCapabilityDefinition::new("repo_map"),
    ])
}
