//! Agents that hand work to other agents ([`AgentBuilder::member`]).

// Decision: a member is an ordinary `Agent` value. The lead gets the Agents
// capability (`agent_handoff`) with one target per member, keyed by the
// member's name, so the model sees the same `spawn_agent` tool the Platform
// gives it and the package format means the same thing on both.
// Decision: a member's id is derived from its name (UUIDv5 under a fixed
// namespace). The Framework keeps no agent catalog, and a derived id is the
// same on every build and restart, so a resumed session still finds it.
// Decision: member sessions run in the lead's Engine and storage. A member
// keeps its own instructions, model and capabilities; the host (local profile,
// backends, host services, workspace) comes from the lead.
// Decision: one level, at most 32 members, like the package format.

use super::*;
use everruns_contracts::typed_id::AgentId;

/// Capability that hands work to other agents.
pub(crate) const AGENTS_CAPABILITY: &str = "agent_handoff";

/// Namespace for member ids derived from member names.
const MEMBER_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x6d2c_51a0_8f3e_4b7a_9c1d_2e4f_6a8b_0c3d);

/// The id a member agent runs under, derived from its name.
pub(crate) fn member_agent_id(name: &str) -> AgentId {
    AgentId::from_uuid(uuid::Uuid::new_v5(&MEMBER_NAMESPACE, name.as_bytes()))
}

impl AgentBuilder {
    /// What this agent does, in one line. An agent that hands work to this
    /// one sees it next to the name, so say when to use it.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add an agent this one can hand work to.
    ///
    /// The member keeps its own instructions, model and capabilities. This
    /// agent gets the `spawn_agent` tool and sees each member by its
    /// [name](Self::name) and [description](Self::description). Work handed to
    /// a member runs as its own session in the same [`Engine`](crate::Engine),
    /// in the foreground or as a background task. Members need a host that
    /// keeps child sessions: a [`SqliteHosting`](crate::sqlite::SqliteHosting)
    /// Engine or a local profile.
    ///
    /// ```no_run
    /// # fn main() -> Result<(), everruns::BuildError> {
    /// use everruns::{Agent, Model};
    ///
    /// let reviewer = Agent::builder()
    ///     .name("reviewer")
    ///     .description("Reviews a change and lists what to fix.")
    ///     .instructions("Review the change you are given. Be specific.")
    ///     .model(Model::simulated("Looks good."))
    ///     .build()?;
    /// let lead = Agent::builder()
    ///     .name("lead")
    ///     .instructions("Hand each change to reviewer before you answer.")
    ///     .model(Model::simulated("Done."))
    ///     .member(reviewer)
    ///     .build()?;
    /// assert_eq!(lead.members().len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`build`](Self::build) fails with [`BuildError::InvalidCapability`] for
    /// a member with members of its own, two members with one name, a member
    /// named like this agent, more than 32 members, or an `agent_handoff`
    /// capability added by hand next to members.
    pub fn member(mut self, agent: Agent) -> Self {
        self.members.push(agent);
        self
    }

    /// Build bundled members and attach the Agents capability for all members.
    pub(super) fn attach_members(&mut self) -> Result<Vec<Agent>, BuildError> {
        let invalid = |reason: String| BuildError::InvalidCapability {
            id: AGENTS_CAPABILITY.into(),
            reason,
        };
        let mut members = std::mem::take(&mut self.members);
        for package in std::mem::take(&mut self.bundled_members) {
            let name = package.manifest().name.clone();
            let mut builder = Agent::builder()
                .package(&package)
                .map_err(|error| invalid(format!("agent {name}: {error}")))?;
            if builder.model.is_none() {
                builder.model = self.model.clone();
            }
            builder.providers = self.providers.clone();
            builder.host_services = self.host_services.clone();
            members.push(
                builder
                    .build()
                    .map_err(|error| invalid(format!("agent {name}: {error}")))?,
            );
        }
        for linked in std::mem::take(&mut self.linked_members) {
            if !members.iter().any(|member| member.name == linked) {
                return Err(invalid(format!(
                    "the package links agent {linked}; add it with AgentBuilder::member"
                )));
            }
        }
        if members.is_empty() {
            return Ok(members);
        }
        let lead = self.name.as_deref().unwrap_or("agent");
        if members.len() > everruns_core::agent_package::MAX_MEMBERS {
            return Err(invalid("at most 32 agents".into()));
        }
        let mut seen = HashSet::new();
        for member in &members {
            if member.name == lead {
                return Err(invalid(format!("{lead} cannot hand work to itself")));
            }
            if !seen.insert(member.name.as_str()) {
                return Err(invalid(format!("two agents are named {}", member.name)));
            }
            if !member.members.is_empty() {
                return Err(invalid(format!(
                    "{} has agents of its own; agents nest one level",
                    member.name
                )));
            }
        }
        if self
            .capabilities
            .iter()
            .any(|capability| capability.capability_ref().id() == AGENTS_CAPABILITY)
        {
            return Err(invalid(
                "add agents with AgentBuilder::member or the agent_handoff capability, not both"
                    .into(),
            ));
        }
        let targets = members
            .iter()
            .map(|member| {
                let mut target = serde_json::json!({
                    "id": member.name,
                    "name": member.name,
                    "agent_id": member_agent_id(&member.name),
                });
                if let Some(description) = &member.description {
                    target["description"] = description.clone().into();
                }
                target
            })
            .collect::<Vec<_>>();
        self.capabilities.push(
            crate::CapabilityRef::new(AGENTS_CAPABILITY)
                .config(serde_json::json!({ "targets": targets }))
                .into(),
        );
        Ok(members)
    }
}

impl Agent {
    /// What the agent does, as set by [`AgentBuilder::description`].
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Agents this one hands work to, as added by [`AgentBuilder::member`]
    /// or bundled in its package.
    pub fn members(&self) -> &[Agent] {
        &self.members
    }

    /// The member running under `id`, set up to run on this agent's host.
    #[cfg_attr(not(feature = "local"), allow(dead_code))]
    pub(crate) fn member_on_host(&self, id: AgentId) -> Option<Agent> {
        let mut member = self
            .members
            .iter()
            .find(|member| member_agent_id(&member.name) == id)?
            .clone();
        #[cfg(feature = "local")]
        {
            member.local = self.local.clone();
        }
        if self.backends.is_some() {
            member.backends = self.backends.clone();
            member.state = self.state.clone();
        }
        if member.host_services.is_none() {
            member.host_services = self.host_services.clone();
        }
        member.workspace_root = self.workspace_root.clone();
        member.default_workspace = self.default_workspace.clone();
        Some(member)
    }
}

#[cfg(test)]
#[path = "members_tests.rs"]
mod tests;
