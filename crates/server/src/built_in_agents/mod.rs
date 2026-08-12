//! Built-in agent definitions.
//!
//! The agent counterpart to [`crate::harnesses`]. Built-in agents are supplied
//! by the platform definition, protected from API mutation (EVE-865), and
//! reconciled into every org by [`crate::org_init`].
//!
//! Only platform-essential agents belong here. An agent an org could plausibly
//! want to edit should be a harness example adopted on demand instead — once an
//! agent is built in, the org can only copy it, never edit it.

mod platform_chat;

use everruns_platform::BuiltInAgentDefinition;

pub use platform_chat::{AGENT_NAME as PLATFORM_CHAT_AGENT_NAME, HARNESS_NAME};

/// All built-in agent definitions in provisioning order.
pub fn built_in_agents() -> Vec<BuiltInAgentDefinition> {
    vec![platform_chat::definition()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn built_in_agent_names_are_unique() {
        let agents = built_in_agents();
        let names: HashSet<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names.len(),
            agents.len(),
            "built-in agents are reconciled by name; duplicates would fight over the same row"
        );
    }

    #[test]
    fn every_built_in_agent_names_a_built_in_harness() {
        let harness_names: HashSet<String> = crate::harnesses::built_in_harnesses()
            .into_iter()
            .map(|h| h.name)
            .collect();
        for agent in built_in_agents() {
            assert!(
                harness_names.contains(&agent.harness_name),
                "built-in agent '{}' names harness '{}', which is not provisioned. \
                 Bootstrap would fail to resolve it.",
                agent.name,
                agent.harness_name
            );
        }
    }
}
