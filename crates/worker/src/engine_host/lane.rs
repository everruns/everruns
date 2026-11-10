//! The worker's Engine lane (data plane v2 step 4): a lease the control
//! plane routes to the Engine runs there; every other lease runs on the
//! worker's own host.
//!
//! Decisions:
//! - The control plane decides, at claim, between turns: it binds a session
//!   of an enrolled organization whose agent the Engine can express and
//!   keeps the rest on the worker's host (`EngineClaim::Legacy`).
//! - A revision this worker cannot build (a capability it lacks) runs on the
//!   worker's host instead of failing the turn: the session's log is the
//!   same either way. The verdict is kept per revision, so a session does
//!   not hydrate twice to learn it.
//! - One Engine per worker, over one execution-only profile, so hydrated
//!   revisions are shared by every session that runs them.

use std::sync::Arc;

use async_trait::async_trait;
use everruns::Engine;
use everruns::hosting::{AgentBuild, AgentDefinitions, AgentHydrator};
use tracing::{debug, warn};

use super::files::PlatformFiles;
use super::hosting::{LeaseSignals, PlatformHosting};
use super::hydrator::PlatformHydrator;
use crate::actor_lease_store::{LeasedActor, SessionActorStore};
use crate::actor_runtime::{EngineLane, LaneSignals};
use crate::engine_sessions::{EngineClaim, EngineFence, EngineLink};
use crate::worker_adapters::WorkerAdapters;

/// Revisions whose build verdict one worker keeps.
const VERDICTS: u64 = 10_000;

/// [`EngineLane`] over the platform's Engine session authority.
pub struct WorkerEngine<A: WorkerAdapters> {
    link: EngineLink,
    holder: String,
    hosting: PlatformHosting<A>,
    hydrator: PlatformHydrator<A>,
    files: PlatformFiles<A>,
    engine: Engine,
    buildable: moka::sync::Cache<AgentBuild, bool>,
}

impl<A: WorkerAdapters> WorkerEngine<A> {
    /// The lane for `holder`'s leases; `leases` is the store its drivers
    /// renew and release through.
    pub fn new(
        adapters: A,
        link: EngineLink,
        leases: Arc<dyn SessionActorStore>,
        holder: impl Into<String>,
    ) -> Self {
        let holder = holder.into();
        let hosting = PlatformHosting::new(
            adapters.clone(),
            link.sessions.clone(),
            leases,
            holder.clone(),
        );
        let hydrator = PlatformHydrator::new(adapters.clone());
        let files = PlatformFiles::new(adapters);
        let engine = Engine::builder()
            .hosting(hosting.clone())
            .hydrator(hydrator.clone())
            .definition_environment(files.clone())
            .build();
        Self {
            link,
            holder,
            hosting,
            hydrator,
            files,
            engine,
            buildable: moka::sync::Cache::new(VERDICTS),
        }
    }

    /// Whether this worker can build `build`.
    async fn buildable(&self, build: &AgentBuild) -> bool {
        if let Some(verdict) = self.buildable.get(build) {
            return verdict;
        }
        let verdict = match self.hosting.agent_definitions().definition(build).await {
            Ok(Some(definition)) => match self.hydrator.hydrate(&definition) {
                Ok(_) => true,
                Err(error) => {
                    warn!(agent = %build.agent, revision = %build.build_id, error = %error,
                        "Engine cannot build this revision; its sessions run on the worker host");
                    false
                }
            },
            Ok(None) => {
                warn!(agent = %build.agent, revision = %build.build_id,
                    "Engine revision is not published; its sessions run on the worker host");
                false
            }
            // A read that failed says nothing about the revision: ask again
            // next time.
            Err(error) => {
                warn!(error = %error, "Failed to read an Engine revision");
                return false;
            }
        };
        self.buildable.insert(build.clone(), verdict);
        verdict
    }
}

#[async_trait]
impl<A: WorkerAdapters> EngineLane for WorkerEngine<A> {
    async fn run(&self, actor: LeasedActor, signals: LaneSignals) -> Option<LeasedActor> {
        let session = actor.session_id;
        if !self.link.routes.take(session, actor.fence) {
            return Some(actor);
        }
        let lease = EngineFence {
            holder: self.holder.clone(),
            fence: actor.fence,
        };
        let binding = match self.link.sessions.claim(session, &lease).await {
            Ok(EngineClaim::Bound(binding)) => binding,
            Ok(EngineClaim::Legacy) => return Some(actor),
            Ok(EngineClaim::Lost) => {
                debug!(session_id = %session, "Lease lost before its Engine claim");
                return None;
            }
            Err(error) => {
                warn!(session_id = %session, error = %error,
                    "Engine claim failed; running the lease on the worker host");
                return Some(actor);
            }
        };
        self.hydrator.bind_key(&binding.agent_key, actor.org_id);
        let build = AgentBuild::new(binding.agent_key.clone(), binding.revision.clone());
        if !self.buildable(&build).await {
            return Some(actor);
        }
        self.files.bind_session(session, actor.org_id);
        self.hosting.offer(
            actor,
            *binding,
            LeaseSignals {
                renew_poke: Some(signals.renew_poke),
                hand_off: Some(signals.hand_off),
            },
        );
        if let Err(error) = self.engine.activate(session).await {
            warn!(session_id = %session, error = %error, "Engine activation failed");
        }
        self.hosting.forget(session);
        self.files.forget(session);
        None
    }
}
