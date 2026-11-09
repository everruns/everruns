//! Agent definitions as data (data plane v2 plan step 3).
//!
//! Decision: an Engine that hosts sessions for agents defined by data (the
//! platform, a cell) does not register code per agent and does not ask
//! anyone to build one. The authority keeps every revision of a definition
//! with the sessions that use it ([`SessionHosting::publish_definition`]),
//! and the Engine that activates a session reads the revision the session
//! is bound to from that authority and hydrates it into an [`Agent`]
//! ([`AgentHydrator`], by default the agent-package mapping). Hydrated
//! agents are cached by `(agent, revision)` in a bounded cache, so a warm
//! activation costs a map lookup and a cold one a parse and a build, with no
//! call off the session's own authority. Code-registered builds
//! ([`EngineBuilder::register`](crate::EngineBuilder::register)) are
//! unchanged and always win.
//!
//! A revision is immutable: publishing different content under the same
//! `(agent, revision)` fails, so a retained turn always rebuilds exactly the
//! behavior it started with (`TurnRevision`). Publishing makes the revision
//! the agent's current one; a session created from a definition moves to
//! the current revision when its next turn starts, never in the middle of a
//! turn.
//!
//! [`SessionHosting::publish_definition`]: super::SessionHosting::publish_definition

use super::AgentBuild;
use crate::{Agent, AgentPackage, Harness, PackageFormat};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spi::error::{AgentLoopError, Result};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use super::spi;

/// Most bytes a published definition may take, as JSON. Matches the
/// portable package limit.
pub const MAX_DEFINITION_BYTES: usize = 10 * 1024 * 1024;

/// One revision of an agent, as data: what the authority keeps and the
/// Engine hydrates. Secret-free: credentials stay host-owned and are bound
/// by the [`AgentHydrator`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// The agent's stable key and this revision (`build_id`). Sessions bind
    /// to it as their [`AgentBuild`].
    #[serde(flatten)]
    pub build: AgentBuild,
    /// The portable agent package manifest, as JSON
    /// ([`AgentPackage::to_string`] with [`PackageFormat::Json`]).
    pub package: Value,
}

impl AgentDefinition {
    /// A revision of `agent` from a portable package.
    pub fn from_package(
        agent: impl Into<String>,
        revision: impl Into<String>,
        package: &AgentPackage,
    ) -> Result<Self> {
        let package = serde_json::from_str(
            &package
                .to_string(PackageFormat::Json)
                .map_err(|error| AgentLoopError::config(error.to_string()))?,
        )
        .map_err(|error| AgentLoopError::config(error.to_string()))?;
        Ok(Self {
            build: AgentBuild::new(agent, revision),
            package,
        })
    }

    /// The portable package this revision holds.
    pub fn agent_package(&self) -> Result<AgentPackage> {
        AgentPackage::parse(&self.package.to_string(), PackageFormat::Json)
            .map_err(|error| AgentLoopError::config(error.to_string()))
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let valid = |key: &str| {
            !key.is_empty()
                && key.len() <= 256
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        };
        if !valid(&self.build.agent) || !valid(&self.build.build_id) {
            return Err(AgentLoopError::config(
                "definition agent and revision allow only [A-Za-z0-9_.:-]",
            ));
        }
        if serde_json::to_vec(&self.package)
            .map_err(|error| AgentLoopError::config(error.to_string()))?
            .len()
            > MAX_DEFINITION_BYTES
        {
            return Err(AgentLoopError::config("definition exceeds 10 MiB"));
        }
        Ok(())
    }
}

/// What a definition hydrates into: the Agent and the Harness its
/// sessions run with.
pub struct Hydrated {
    /// Executable behavior.
    pub agent: Agent,
    /// The session Harness; `None` for the host default.
    pub harness: Option<Harness>,
}

/// Turns a definition into executable behavior. Pure CPU: it must not call
/// out (the control plane is off the turn path), and it must return the
/// same behavior for the same definition every time. A host whose
/// definitions name capabilities or credentials only it can supply (the
/// platform) installs its own; the default is the agent-package mapping.
/// Stability: experimental.
pub trait AgentHydrator: Send + Sync + 'static {
    /// Build `definition`, or explain why this process cannot.
    fn hydrate(&self, definition: &AgentDefinition) -> Result<Hydrated>;
}

impl<F> AgentHydrator for F
where
    F: Fn(&AgentDefinition) -> Result<Hydrated> + Send + Sync + 'static,
{
    fn hydrate(&self, definition: &AgentDefinition) -> Result<Hydrated> {
        self(definition)
    }
}

/// The default hydrator: the portable package through
/// [`AgentBuilder::package`](crate::AgentBuilder::package), with the
/// package's standard Harness. A package names its model by provider key;
/// give the hydrator the host services whose providers serve those keys.
#[derive(Clone, Default)]
pub struct PackageHydrator {
    services: Option<super::services::HostServices>,
}

impl PackageHydrator {
    /// Hydrate with these host services (model providers, host capabilities).
    pub fn with_services(services: super::services::HostServices) -> Self {
        Self {
            services: Some(services),
        }
    }
}

impl AgentHydrator for PackageHydrator {
    fn hydrate(&self, definition: &AgentDefinition) -> Result<Hydrated> {
        let package = definition.agent_package()?;
        let config = |error: crate::PackageError| AgentLoopError::config(error.to_string());
        let mut builder = Agent::builder();
        if let Some(services) = &self.services {
            builder = builder.host_services(services.clone());
        }
        let agent = package
            .apply_to(builder)
            .map_err(config)?
            .build()
            .map_err(|error| AgentLoopError::config(error.to_string()))?;
        Ok(Hydrated {
            agent,
            harness: package.session_harness().map_err(config)?,
        })
    }
}

/// Default number of hydrated revisions an Engine keeps.
pub(crate) const DEFAULT_HYDRATED_CAPACITY: usize = 256;

/// Bounded cache of hydrated revisions, least recently used dropped first
/// (in batches down to three quarters, as the session cache does). A
/// dropped revision is hydrated again from the authority's copy.
pub(crate) struct HydratedCache<T> {
    capacity: usize,
    clock: AtomicU64,
    entries: Mutex<HashMap<AgentBuild, (T, u64)>>,
}

impl<T: Clone> HydratedCache<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            clock: AtomicU64::new(1),
            entries: Mutex::default(),
        }
    }

    pub(crate) fn get(&self, build: &AgentBuild) -> Option<T> {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let entry = entries.get_mut(build)?;
        entry.1 = self.clock.fetch_add(1, Ordering::Relaxed);
        Some(entry.0.clone())
    }

    /// Keep `value` for `build`; an entry another task inserted first wins,
    /// so every caller sees one behavior per revision.
    pub(crate) fn insert(&self, build: AgentBuild, value: T) -> T {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let used = self.clock.fetch_add(1, Ordering::Relaxed);
        let kept = entries
            .entry(build)
            .and_modify(|entry| entry.1 = used)
            .or_insert((value, used))
            .0
            .clone();
        if entries.len() > self.capacity {
            let target = self.capacity - self.capacity / 4;
            let mut by_use: Vec<(u64, AgentBuild)> = entries
                .iter()
                .map(|(build, (_, used))| (*used, build.clone()))
                .collect();
            by_use.sort_unstable_by_key(|(used, _)| *used);
            for (_, build) in by_use.into_iter().take(entries.len() - target) {
                entries.remove(&build);
            }
        }
        kept
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

/// Authority-side store of definitions, for the built-in profiles: every
/// published revision, and each agent's current one.
#[derive(Default)]
pub(crate) struct DefinitionStore {
    revisions: HashMap<AgentBuild, AgentDefinition>,
    current: HashMap<String, String>,
}

impl DefinitionStore {
    /// Keep `definition` and make it current. Same content again is a
    /// no-op success; different content under a published revision fails.
    pub(crate) fn publish(&mut self, definition: &AgentDefinition) -> Result<()> {
        definition.validate()?;
        match self.revisions.get(&definition.build) {
            Some(kept) if kept != definition => {
                // THREAT[TM-DURABLE-012]: a retained revision names one
                // immutable behavior; it is never rebound.
                return Err(AgentLoopError::config(
                    "agent revision is already published with different content",
                ));
            }
            Some(_) => {}
            None => {
                self.revisions
                    .insert(definition.build.clone(), definition.clone());
            }
        }
        self.current.insert(
            definition.build.agent.clone(),
            definition.build.build_id.clone(),
        );
        Ok(())
    }

    pub(crate) fn get(&self, build: &AgentBuild) -> Option<AgentDefinition> {
        self.revisions.get(build).cloned()
    }

    /// Move a session bound to an older revision of a defined agent to the
    /// current one, with the current revision's Harness, for its next
    /// turn. Returns whether the binding changed. The Environment a
    /// session was created with never changes: a revision that needs
    /// another one is reported as unavailable configuration at activation.
    pub(crate) fn advance(&self, binding: &mut super::SessionBinding) -> bool {
        let Some(current) = self.current.get(&binding.behavior.agent) else {
            return false;
        };
        if current == &binding.behavior.build_id {
            return false;
        }
        let build = AgentBuild::new(binding.behavior.agent.clone(), current.clone());
        let harness = self.revisions.get(&build).and_then(|definition| {
            match definition.agent_package().ok()?.session_harness().ok()? {
                Some(harness) => serde_json::to_value(harness).ok().map(Some),
                None => Some(None),
            }
        });
        let Some(harness) = harness else {
            return false;
        };
        binding.behavior = build;
        binding.harness = harness;
        true
    }
}

#[cfg(test)]
#[path = "definitions_tests.rs"]
mod tests;
