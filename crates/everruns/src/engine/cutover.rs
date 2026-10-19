//! Moving a legacy local/custom-backend session to coherent hosting (EVE-1298).
//!
//! Ownership moves in three durable steps, each safe to interrupt:
//!
//! 1. Freeze: under the legacy session lease, write a `cutting_over` marker in
//!    the legacy store. From here the legacy executor refuses every turn, in
//!    this process and in any other sharing the store; nothing falls back to it
//!    automatically. [`Engine::abandon_legacy_cutover`] is the only way back,
//!    and only while the authority holds nothing for the session.
//! 2. Import: read the frozen session and commit it to the authority in one
//!    transaction together with its source boundary and ownership decision.
//!    An interrupted import commits nothing; a retry reads the same frozen
//!    source and either commits or replays.
//! 3. Confirm: mark the legacy store `cut_over` at that boundary.
//!
//! Outstanding obligations the authority cannot carry yet (non-terminal tasks,
//! active schedules, session secrets) refuse the cutover before step 2 rather
//! than drop them.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_core::events::EventData;
use everruns_core::host::{EventReadLimit, EventReadRequest, SessionLease, SessionLeases};

use super::legacy_metadata::{CutoverState, LegacyCutoverMarker, LegacyCutoverStore};
use super::*;
use crate::hosting::cutover::{CutoverBoundary, CutoverReport, LegacySessionImage};
use crate::hosting::{AgentBuild, SessionConfiguration};

/// How long the cutover holds the legacy session lease while importing.
const CUTOVER_LEASE: Duration = Duration::from_secs(120);

fn moved(session: SessionId) -> AgentLoopError {
    AgentLoopError::config(format!(
        "session {session} is owned by a coherent session authority after cutover; \
         resume it through the hosting Engine"
    ))
}

#[derive(Default)]
struct LegacyActivity {
    turns: usize,
    frozen: bool,
}

/// Legacy turns running in this process, per session. Session ids are
/// globally unique, so one table serves every Engine and backend bundle.
fn activity() -> std::sync::MutexGuard<'static, HashMap<SessionId, LegacyActivity>> {
    static TABLE: OnceLock<Mutex<HashMap<SessionId, LegacyActivity>>> = OnceLock::new();
    TABLE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn finish_turn(session: SessionId) {
    let mut table = activity();
    if let Some(entry) = table.get_mut(&session) {
        entry.turns = entry.turns.saturating_sub(1);
        if entry.turns == 0 && !entry.frozen {
            table.remove(&session);
        }
    }
}

fn unfreeze(session: SessionId) {
    let mut table = activity();
    if let Some(entry) = table.get_mut(&session) {
        entry.frozen = false;
        if entry.turns == 0 {
            table.remove(&session);
        }
    }
}

/// Refuse to build a legacy runtime for a session that was cut over.
pub(crate) async fn ensure_legacy_owner(
    store: &dyn LegacyCutoverStore,
    session: SessionId,
) -> Result<()> {
    if store.load_cutover(session).await?.is_some() {
        return Err(moved(session));
    }
    Ok(())
}

/// Wrap the legacy lane's session leases so every turn re-checks ownership
/// after taking its lease. A cutover takes the same lease before freezing, so
/// no legacy turn can start after the freeze in any process sharing the store.
pub(crate) fn fence_legacy_leases(
    inner: Arc<dyn SessionLeases>,
    store: Arc<dyn LegacyCutoverStore>,
) -> Arc<dyn SessionLeases> {
    Arc::new(LegacyLeaseFence { inner, store })
}

struct LegacyLeaseFence {
    inner: Arc<dyn SessionLeases>,
    store: Arc<dyn LegacyCutoverStore>,
}

#[async_trait]
impl SessionLeases for LegacyLeaseFence {
    async fn acquire(&self, session: SessionId, ttl: Duration) -> Result<Option<SessionLease>> {
        let Some(lease) = self.inner.acquire(session, ttl).await? else {
            return Ok(None);
        };
        let admitted = {
            let mut table = activity();
            let entry = table.entry(session).or_default();
            if entry.frozen {
                false
            } else {
                entry.turns += 1;
                true
            }
        };
        if !admitted {
            let _ = self.inner.release(&lease).await;
            return Err(moved(session));
        }
        // Fail closed: an unreadable marker is not permission to run.
        match self.store.load_cutover(session).await {
            Ok(None) => Ok(Some(lease)),
            Ok(Some(_)) | Err(_) => {
                finish_turn(session);
                let _ = self.inner.release(&lease).await;
                Err(moved(session))
            }
        }
    }

    async fn renew(&self, lease: &SessionLease, ttl: Duration) -> Result<bool> {
        self.inner.renew(lease, ttl).await
    }

    async fn release(&self, lease: &SessionLease) -> Result<()> {
        finish_turn(lease.session_id);
        self.inner.release(lease).await
    }
}

/// Which legacy lane `agent` selects, if any.
fn legacy_lane(agent: &Agent) -> Option<&'static str> {
    if agent.custom_backends().is_some() {
        return Some("custom_backends");
    }
    #[cfg(feature = "local")]
    if agent.local_config().is_some() {
        return Some("local");
    }
    None
}

impl Engine {
    /// Move an existing legacy local/custom-backend session to this Engine's
    /// coherent session hosting, keeping its identity and full history.
    ///
    /// `legacy` is the Agent the session ran with (it selects the legacy
    /// store); `build` is the registered build the authority executes it with
    /// from now on. The session must be quiescent: no legacy turn running, no
    /// non-terminal task, no active schedule and no session secret. The legacy
    /// executor is frozen first and never resumes the session afterwards.
    ///
    /// Retrying after any interruption is safe and returns the same boundary;
    /// a cutover to a different build, or of a session the authority already
    /// holds, fails without changing either side. Register `build` with an
    /// [`ImportedWorkspaceFactory`](crate::hosting::ImportedWorkspaceFactory)
    /// when the legacy session has a workspace. Stability: experimental.
    pub async fn cut_over_legacy_session(
        &self,
        session: SessionId,
        legacy: &Agent,
        build: &AgentBuild,
    ) -> Result<CutoverReport> {
        let profile = self.profile()?.clone();
        let lane = legacy_lane(legacy).ok_or_else(|| {
            AgentLoopError::config("the source Agent selects no legacy local or custom backends")
        })?;
        let registration = self.registration(build)?;
        let binding = hosting::registered_binding(build, &registration, serde_json::Value::Null)?;
        let backends = self
            .backends_for(legacy)
            .await
            .map_err(crate::agent::BackendInitError::into_agent_loop)?;
        if backends
            .host
            .session_store
            .get_session(session)
            .await?
            .is_none()
        {
            return Err(AgentLoopError::SessionNotFound(session));
        }
        let marker = backends.cutover.load_cutover(session).await?;
        if let Some(marker) = &marker
            && marker.target != *build
        {
            return Err(AgentLoopError::config(
                "the legacy session is already cut over to a different build",
            ));
        }
        // Freeze in this process first: refuse while one of its legacy turns
        // is running, and stop new ones from starting.
        {
            let mut table = activity();
            let entry = table.entry(session).or_default();
            if entry.turns > 0 {
                return Err(AgentLoopError::config(
                    "the legacy session is executing; retry the cutover when it is quiescent",
                ));
            }
            entry.frozen = true;
        }
        let leases = backends.host.session_leases.clone();
        let Some(lease) = leases.acquire(session, CUTOVER_LEASE).await? else {
            if marker.is_none() {
                unfreeze(session);
            }
            return Err(AgentLoopError::config(
                "the legacy session is executing in another process; retry the cutover when it is quiescent",
            ));
        };
        let result = self
            .import_frozen(
                session,
                lane,
                legacy,
                &backends,
                &registration,
                binding.clone(),
                marker.is_none(),
                &*profile,
            )
            .await;
        let _ = leases.release(&lease).await;
        let report = match result {
            Ok(report) => report,
            Err((error, frozen)) => {
                if !frozen {
                    unfreeze(session);
                }
                return Err(error);
            }
        };
        // This Engine's handles now route to the authority.
        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if sessions
                .get(&session)
                .is_some_and(|entry| entry.durable_binding.is_none())
            {
                sessions.remove(&session);
            }
        }
        self.cache_hosted(session, binding)?;
        Ok(report)
    }

    /// Return a session to its legacy lane after an interrupted cutover, only
    /// while the authority holds nothing for it. Never automatic: a host calls
    /// it as an explicit ownership decision. A committed cutover cannot be
    /// abandoned. Stability: experimental.
    pub async fn abandon_legacy_cutover(&self, session: SessionId, legacy: &Agent) -> Result<()> {
        legacy_lane(legacy).ok_or_else(|| {
            AgentLoopError::config("the source Agent selects no legacy local or custom backends")
        })?;
        match self.profile()?.status(session).await {
            Err(AgentLoopError::SessionNotFound(_)) => {}
            Ok(_) => {
                return Err(AgentLoopError::config(
                    "the authority already owns this session; a committed cutover cannot be abandoned",
                ));
            }
            Err(error) => return Err(error),
        }
        let backends = self
            .backends_for(legacy)
            .await
            .map_err(crate::agent::BackendInitError::into_agent_loop)?;
        backends.cutover.store_cutover(session, None).await?;
        unfreeze(session);
        Ok(())
    }

    /// Steps 1 to 3 under the legacy lease. The error side says whether the
    /// durable freeze was written, so the caller keeps the in-process freeze.
    #[allow(clippy::too_many_arguments)]
    async fn import_frozen(
        &self,
        session: SessionId,
        lane: &'static str,
        legacy: &Agent,
        backends: &EngineBackends,
        registration: &hosting::Registration,
        binding: crate::hosting::SessionBinding,
        first: bool,
        profile: &dyn crate::hosting::SessionHosting,
    ) -> std::result::Result<CutoverReport, (AgentLoopError, bool)> {
        let build = binding.behavior.clone();
        if first {
            backends
                .cutover
                .store_cutover(
                    session,
                    Some(&LegacyCutoverMarker {
                        state: CutoverState::CuttingOver,
                        target: build.clone(),
                        boundary: None,
                    }),
                )
                .await
                .map_err(|error| (error, false))?;
        }
        let image = self
            .read_frozen(
                session,
                lane,
                legacy,
                backends,
                registration,
                binding.clone(),
            )
            .await
            .map_err(|error| (error, true))?;
        let boundary: CutoverBoundary = serde_json::from_value(image.source["boundary"].clone())
            .map_err(|error| (AgentLoopError::store(error.to_string()), true))?;
        let counts = (
            image.checkpoints.len(),
            image.tasks.len(),
            image.values.len(),
        );
        let volatile_workspace = image.source["volatile_workspace"].as_bool() == Some(true);
        let replayed = profile
            .import_legacy(session, image)
            .await
            .map_err(|error| (error, true))?;
        backends
            .cutover
            .store_cutover(
                session,
                Some(&LegacyCutoverMarker {
                    state: CutoverState::CutOver,
                    target: build,
                    boundary: Some(boundary.clone()),
                }),
            )
            .await
            .map_err(|error| (error, true))?;
        Ok(CutoverReport {
            session,
            binding,
            boundary,
            replayed,
            checkpoints: counts.0,
            tasks: counts.1,
            values: counts.2,
            volatile_workspace,
        })
    }

    /// Read the frozen legacy session and validate that the target build can
    /// own it without losing or widening anything.
    async fn read_frozen(
        &self,
        session: SessionId,
        lane: &'static str,
        legacy: &Agent,
        backends: &EngineBackends,
        registration: &hosting::Registration,
        binding: crate::hosting::SessionBinding,
    ) -> Result<LegacySessionImage> {
        let host = &backends.host;
        let mut events = Vec::new();
        let mut cursor = None;
        loop {
            let mut request = EventReadRequest::new(
                session,
                EventReadLimit::new(256)
                    .map_err(|error| AgentLoopError::store(error.to_string()))?,
            );
            if let Some(cursor) = cursor.take() {
                request = request.with_cursor(cursor);
            }
            let page = host
                .event_log
                .read_page(request)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?;
            events.extend(page.events);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let boundary = CutoverBoundary {
            last_sequence: events.last().and_then(|event| event.sequence).unwrap_or(0),
            last_event_id: events.last().map(|event| event.id.to_string()),
            events: events.len(),
        };
        let record = host.session_store.get_session(session).await?;

        // Harness: a session that ran with one must keep one.
        if backends
            .harness_binding_store
            .is_required(session)
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?
            && registration.harness.is_none()
        {
            return Err(AgentLoopError::config(
                "the legacy session requires a Harness; register the build with it",
            ));
        }

        // Workspace: the target must reopen exactly the legacy head.
        let bound = backends
            .binding_store
            .load(session)
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        // The Agent's volatile in-memory workspace is not durable on either
        // lane; it is reported, not retained.
        let volatile_workspace = bound
            .as_ref()
            .is_some_and(crate::default_workspace::is_volatile_default);
        let workspace = bound.filter(|_| !volatile_workspace);
        if let Some(head) = &workspace {
            if registration.environment.is_none() {
                return Err(AgentLoopError::config(
                    "the legacy session has a workspace; register the build with an \
                     ImportedWorkspaceFactory so the authority reopens it",
                ));
            }
            let environment =
                crate::hosting::cutover::reopen_binding(&registration.agent, session, head.clone())
                    .await?;
            if let Some(harness) = &registration.harness {
                harness
                    .negotiate(&environment)
                    .map_err(|error| AgentLoopError::config(error.to_string()))?;
            }
        } else if registration.environment.is_none()
            && registration.agent.requires_hosted_workspace()
        {
            return Err(AgentLoopError::config(
                "this Agent requires a retained Environment factory",
            ));
        }

        // Obligations the authority cannot carry yet are refused, not dropped.
        if let Ok(secrets) = host.storage_store.list_secrets(session).await
            && !secrets.is_empty()
        {
            return Err(AgentLoopError::config(
                "the legacy session holds secrets; the native authority needs an external \
                 encrypted secret service before it can own them",
            ));
        }
        let mut tasks = Vec::new();
        if let Some(registry) = &host.session_task_registry {
            for task in registry.list(session, None).await? {
                if !task.state.is_terminal() {
                    return Err(AgentLoopError::config(format!(
                        "the legacy session has an outstanding {} task {}; let it finish or \
                         cancel it before cutover",
                        task.kind, task.id
                    )));
                }
                let messages = registry
                    .list_messages(session, &task.id, None, None)
                    .await?;
                tasks.push((task, messages));
            }
        }
        // The schedule store is a native host service.
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(factory), Some(record)) = (&host.schedule_store_factory, &record) {
            let org = everruns_core::host::in_process_internal_org_id(&record.organization_id);
            if factory(org).count_active_schedules(session).await? > 0 {
                return Err(AgentLoopError::config(
                    "the legacy session has active schedules; cancel them before cutover",
                ));
            }
        }

        let mut values = Vec::new();
        for key in host.storage_store.list_keys(session).await? {
            if super::legacy_metadata::is_framework_key(&key.key) {
                continue;
            }
            if let Some(value) = host.storage_store.get_value(session, &key.key).await? {
                values.push((key.key, value));
            }
        }

        // Compaction checkpoints for every provider/model the journal compacted.
        let mut pairs = HashSet::new();
        for event in &events {
            if let EventData::ContextCompacted(data) = &event.data
                && let Some(provider) = data.provider.clone().or_else(|| data.driver.clone())
            {
                pairs.insert((provider, data.model.clone()));
            }
        }
        let mut checkpoints = Vec::new();
        let mut seen = HashSet::new();
        for (provider, model) in pairs {
            for format in [
                everruns_core::COMPACTION_CHECKPOINT_FORMAT_VERSION,
                everruns_core::ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION,
            ] {
                if let Some(checkpoint) = host
                    .compaction_checkpoint_store
                    .get_latest_format(session, &provider, &model, format)
                    .await?
                    && seen.insert(checkpoint.id)
                {
                    checkpoints.push(checkpoint);
                }
            }
        }

        // Live selections the legacy lane held in this Engine (EVE-1297) and
        // the persisted capability overlay become authority configuration.
        let (model, effort) = self.live_controls(session).selections();
        let agent_capabilities = legacy.capability_ids();
        let overlay: Vec<_> = record
            .iter()
            .flat_map(|record| record.capabilities.iter())
            .filter(|capability| !agent_capabilities.contains(capability.capability_id()))
            .cloned()
            .collect();
        let configuration = SessionConfiguration {
            capabilities: (!overlay.is_empty()).then_some(overlay),
            model: model.map(Some),
            reasoning_effort: effort.map(Some),
            ..SessionConfiguration::default()
        };

        Ok(LegacySessionImage {
            binding,
            source: serde_json::json!({
                "lane": lane,
                "boundary": boundary,
                "volatile_workspace": volatile_workspace,
            }),
            events,
            record,
            configuration,
            checkpoints,
            values,
            tasks,
            workspace,
        })
    }
}
