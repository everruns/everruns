//! Agents that start other agents ([`AgentBuilder::spawns`],
//! [`AgentBuilder::threads_with`]).

// Decision (2026-11-25, knowledge/runtime-resources/threads.md): two ways to
// start another agent, never both on one agent. `spawns` puts the agent on the
// Spawn agents capability (`spawn_agents.agents`): a one-off call, brief in,
// result out. `threads_with` puts it on the Threads capability
// (`threads.agents`): followed work the parent messages and closes. The model
// sees the same tools the Platform gives it, and the package format means the
// same thing on both.
// Decision: an added agent is an ordinary `Agent` value. Its id is derived
// from its name (UUIDv5 under a fixed namespace). The Framework keeps no agent
// catalog, and a derived id is the same on every build and restart, so a
// resumed session still finds it.
// Decision: added agents run in the parent's Engine and storage. Each keeps
// its own instructions, model and capabilities; the host (local profile,
// backends, host services, workspace) comes from the parent.
// Decision: a `SpawnAgents` or `Threads` capability added by hand keeps its
// settings; the added agents join its `agents` list. Bundled package agents
// go on Threads when the package has Threads, otherwise on Spawn agents.
// Decision: one level, at most 32 agents, like the package format.

use super::*;
use everruns_contracts::capability::delegation::{
    LEGACY_AGENT_HANDOFF_ID, LEGACY_COORDINATION_ID, LEGACY_SUBAGENTS_ID,
    SPAWN_AGENTS_CAPABILITY_ID, THREADS_CAPABILITY_ID,
};
use everruns_contracts::typed_id::AgentId;

/// Namespace for added-agent ids derived from their names.
const MEMBER_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x6d2c_51a0_8f3e_4b7a_9c1d_2e4f_6a8b_0c3d);

/// The id an added agent runs under, derived from its name.
pub(crate) fn member_agent_id(name: &str) -> AgentId {
    AgentId::from_uuid(uuid::Uuid::new_v5(&MEMBER_NAMESPACE, name.as_bytes()))
}

/// Lets an agent make one-off calls to agents: a brief goes in, a result
/// comes back, and the child is done.
///
/// Defaults: the agent may spawn itself (a fresh copy with its own context).
/// Add other agents with [`AgentBuilder::spawns`]; their names are what the
/// model passes as `agent` to the `spawn_agent` tool.
///
/// An agent has Spawn agents or [`Threads`](crate::threads::Threads), never
/// both: threads already cover one-off work.
#[derive(Clone, Debug)]
pub struct SpawnAgents {
    run_self: bool,
    max_depth: Option<u64>,
    max_open: Option<u64>,
    max_total: Option<u64>,
}

impl Default for SpawnAgents {
    fn default() -> Self {
        Self {
            run_self: true,
            max_depth: None,
            max_open: None,
            max_total: None,
        }
    }
}

impl SpawnAgents {
    /// Spawn agents with the default limits; the agent may spawn itself.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the agent may spawn a copy of itself (default `true`). Turn
    /// it off when it should only call the agents added with
    /// [`AgentBuilder::spawns`].
    pub fn run_self(mut self, enabled: bool) -> Self {
        self.run_self = enabled;
        self
    }

    /// How deep spawned agents may nest.
    pub fn max_depth(mut self, depth: u64) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Most spawned agents working at the same time.
    pub fn max_open(mut self, limit: u64) -> Self {
        self.max_open = Some(limit);
        self
    }

    /// Most agents one session may ever spawn.
    pub fn max_total(mut self, limit: u64) -> Self {
        self.max_total = Some(limit);
        self
    }
}

impl crate::IntoCapability for SpawnAgents {
    fn into_capability(self) -> crate::CapabilitySpec {
        let mut config = serde_json::Map::new();
        config.insert("self".into(), self.run_self.into());
        for (key, value) in [
            ("max_depth", self.max_depth),
            ("max_open", self.max_open),
            ("max_total", self.max_total),
        ] {
            if let Some(value) = value {
                config.insert(key.into(), value.into());
            }
        }
        crate::CapabilityRef::new(SPAWN_AGENTS_CAPABILITY_ID)
            .config(serde_json::Value::Object(config))
            .into()
    }
}

/// How the parent reaches the agents added to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MemberKind {
    Spawn,
    Threads,
}

impl MemberKind {
    fn capability_id(self) -> &'static str {
        match self {
            Self::Spawn => SPAWN_AGENTS_CAPABILITY_ID,
            Self::Threads => THREADS_CAPABILITY_ID,
        }
    }
}

/// True for a capability id that carries added agents (`agents`).
pub(crate) fn carries_agents(id: &str) -> bool {
    id == SPAWN_AGENTS_CAPABILITY_ID || id == THREADS_CAPABILITY_ID
}

impl AgentBuilder {
    /// What this agent does, in one line. An agent that starts this one sees
    /// it next to the name, so say when to use it.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add an agent this one can call for one-off work.
    ///
    /// This agent gets [Spawn agents](SpawnAgents): the `spawn_agent` tool
    /// lists each added agent by its [name](Self::name) and
    /// [description](Self::description). A call sends a brief, the agent
    /// works it as its own session in the same [`Engine`](crate::Engine), and
    /// its result comes back, in the foreground (`wait`) or as a background
    /// task. The added agent keeps its own instructions, model and
    /// capabilities. It needs a host that keeps child sessions: a
    /// [`SqliteHosting`](crate::sqlite::SqliteHosting) Engine or a local
    /// profile.
    ///
    /// Add [`SpawnAgents`] yourself to change its settings, for example to
    /// stop the agent from spawning itself.
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
    ///     .instructions("Spawn reviewer on each change before you answer.")
    ///     .model(Model::simulated("Done."))
    ///     .spawns(reviewer)
    ///     .build()?;
    /// assert_eq!(lead.members().len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`build`](Self::build) fails with [`BuildError::InvalidCapability`] for
    /// an added agent with agents of its own, two agents with one name, an
    /// agent named like this one, more than 32 agents, or agents added with
    /// both `spawns` and [`threads_with`](Self::threads_with).
    pub fn spawns(mut self, agent: Agent) -> Self {
        self.members.push(agent);
        self.member_kind.get_or_insert(MemberKind::Spawn);
        if self.member_kind != Some(MemberKind::Spawn) {
            self.mixed_members = true;
        }
        self
    }

    /// Add an agent this one can hand followed work to, as threads.
    ///
    /// This agent gets [Threads](crate::threads::Threads): `start_thread`
    /// takes the added agent's [name](Self::name) as `agent`, and the thread
    /// runs it as its own session in the same [`Engine`](crate::Engine). The
    /// parent sends follow-ups with `message_thread` and closes the thread
    /// when the work is accepted; the thread reports progress, asks, and
    /// finishes through its own tools. Threads need a local profile
    /// ([`AgentBuilder::local`](Self::local)).
    ///
    /// Add [`Threads`](crate::threads::Threads) yourself to change its
    /// limits, and [`Coordinator`](crate::threads::Coordinator) to make this
    /// agent route work instead of doing it.
    ///
    /// [`build`](Self::build) fails for the same reasons as
    /// [`spawns`](Self::spawns).
    #[cfg(feature = "local")]
    pub fn threads_with(mut self, agent: Agent) -> Self {
        self.members.push(agent);
        self.member_kind.get_or_insert(MemberKind::Threads);
        if self.member_kind != Some(MemberKind::Threads) {
            self.mixed_members = true;
        }
        self
    }

    /// Build bundled agents and put every added agent on Spawn agents or
    /// Threads.
    pub(super) fn attach_members(&mut self) -> Result<Vec<Agent>, BuildError> {
        let has = |builder: &Self, ids: &[&str]| {
            builder
                .capabilities
                .iter()
                .any(|capability| ids.contains(&capability.capability_ref().id()))
        };
        let has_threads = has(self, &[THREADS_CAPABILITY_ID, LEGACY_COORDINATION_ID]);
        let has_spawn = has(
            self,
            &[
                SPAWN_AGENTS_CAPABILITY_ID,
                LEGACY_SUBAGENTS_ID,
                LEGACY_AGENT_HANDOFF_ID,
            ],
        );
        let kind = self.member_kind.unwrap_or(if has_threads {
            MemberKind::Threads
        } else {
            MemberKind::Spawn
        });
        let invalid = |reason: String| BuildError::InvalidCapability {
            id: kind.capability_id().into(),
            reason,
        };
        let both = || {
            invalid(
                "an agent has Spawn agents or Threads, never both; threads already cover one-off work"
                    .into(),
            )
        };
        if self.mixed_members || (has_threads && has_spawn) {
            return Err(both());
        }
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
                    "the package links agent {linked}; add it with AgentBuilder::spawns or AgentBuilder::threads_with"
                )));
            }
        }
        if members.is_empty() {
            return Ok(members);
        }
        if (kind == MemberKind::Spawn && has_threads) || (kind == MemberKind::Threads && has_spawn)
        {
            return Err(both());
        }
        let lead = self.name.as_deref().unwrap_or("agent");
        if members.len() > everruns_core::agent_package::MAX_MEMBERS {
            return Err(invalid("at most 32 agents".into()));
        }
        let mut seen = HashSet::new();
        for member in &members {
            if member.name == lead {
                return Err(invalid(format!(
                    "{lead} cannot add itself; it can always start itself"
                )));
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
        let agents = members
            .iter()
            .map(|member| {
                let mut agent = serde_json::json!({
                    "id": member.name,
                    "name": member.name,
                    "agent_id": member_agent_id(&member.name),
                });
                if let Some(description) = &member.description {
                    agent["description"] = description.clone().into();
                }
                agent
            })
            .collect::<Vec<_>>();
        // A capability added by hand keeps its settings; the agents join it.
        let id = kind.capability_id();
        let position = self
            .capabilities
            .iter()
            .position(|capability| capability.capability_ref().id() == id);
        let mut config = match position {
            Some(index) => self
                .capabilities
                .remove(index)
                .capability_ref()
                .config_value()
                .clone(),
            None => serde_json::json!({ "self": true }),
        };
        if !config.is_object() {
            config = serde_json::json!({ "self": true });
        }
        config["agents"] = serde_json::Value::Array(agents);
        let capability: crate::CapabilitySpec = crate::CapabilityRef::new(id).config(config).into();
        match position {
            Some(index) => self.capabilities.insert(index, capability),
            None => self.capabilities.push(capability),
        }
        Ok(members)
    }
}

impl Agent {
    /// What the agent does, as set by [`AgentBuilder::description`].
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Agents this one can start, as added by [`AgentBuilder::spawns`] or
    /// [`AgentBuilder::threads_with`], or bundled in its package.
    pub fn members(&self) -> &[Agent] {
        &self.members
    }

    /// The added agent running under `id`, set up to run on this agent's host.
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
        // The added agent works in this agent's workspace, so it gets the file
        // tools for it too (they come with `local`/`workspace` on the lead).
        const FILES: &str = "session_file_system";
        let has = |agent: &Agent| {
            agent
                .capabilities
                .iter()
                .any(|c| c.capability_id() == FILES)
        };
        if has(self) && !has(&member) {
            member
                .capabilities
                .push(everruns_contracts::CapabilityRef::new(FILES));
        }
        Some(member)
    }
}

#[cfg(test)]
#[path = "members_tests.rs"]
mod tests;
