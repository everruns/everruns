// LocalBackends — composable construction of runtime backends + local stores.
//
// Composability requirement (EVE-594): an embedder MUST be able to keep its own
// event bus and its own `SessionFileSystemFactory`. `LocalBackends` therefore
// takes the runtime backend bundle (which already carries the caller's event
// bus) as input, and the file system factory is configured on the embedder's
// `HostComposition` — never forced here. The individual local stores are
// exposed so an embedder can wire only the pieces it wants.

use std::sync::Arc;

use everruns_core::error::Result;
use everruns_core::traits::SessionScheduleStore;
use everruns_core::typed_id::{PrincipalId, SessionId};
use everruns_host::{HostBackends, PlatformStoreFactory, ScheduleStoreFactory};
use everruns_platform::PlatformStore;

use crate::db::SqliteDb;
use crate::platform_store::{LocalPlatformStore, LocalSessionRunner};
use crate::profile::LocalProfile;
use crate::schedule_runner::{
    LocalScheduleRunner, LocalScheduleRunnerConfig, LocalScheduleRunnerHandle,
};
use crate::schedule_store::LocalScheduleStore;
use crate::task_registry::LocalSessionTaskRegistry;

/// The local stores plus ready-to-use `HostBackends`.
///
/// `runtime_backends` carries whatever store set / event bus the caller passed
/// in, plus the local SQLite-backed task registry and schedule store factory
/// attached here. The platform store factory is attached separately via
/// [`LocalBackends::with_platform_runner`] once the embedder supplies a
/// `LocalSessionRunner` (the runner usually wraps the runtime built from these
/// backends, hence the two-step wiring).
pub struct LocalBackends {
    /// Composable runtime backend bundle (caller bus preserved).
    pub runtime_backends: HostBackends,
    /// Shared SQLite handle backing the local stores.
    pub db: SqliteDb,
    /// SQLite-backed task registry (also installed on `runtime_backends`).
    pub task_registry: Arc<LocalSessionTaskRegistry>,
    /// Profile used to build the stores.
    pub profile: LocalProfile,
    org_id: i64,
}

impl LocalBackends {
    /// Build local backends from a profile and caller-provided composable
    /// pieces. The event bus and any custom stores already configured on
    /// `runtime_backends` are preserved; this only attaches the local task
    /// registry and schedule store factory.
    ///
    /// The SQLite database is opened at `profile.db_path()`; the data directory
    /// is created if needed.
    pub fn new(profile: LocalProfile, runtime_backends: HostBackends) -> Result<Self> {
        profile
            .ensure_dirs()
            .map_err(|e| everruns_core::AgentLoopError::config(e.to_string()))?;
        let db = SqliteDb::open(profile.db_path()).map_err(everruns_core::AgentLoopError::from)?;
        Self::with_db(profile, runtime_backends, db)
    }

    /// Build local backends over an already-open SQLite handle (e.g. an
    /// in-memory DB for tests).
    pub fn with_db(
        profile: LocalProfile,
        runtime_backends: HostBackends,
        db: SqliteDb,
    ) -> Result<Self> {
        let org_id = everruns_host::in_process_internal_org_id(&profile.org_public_id);
        let task_registry = Arc::new(LocalSessionTaskRegistry::new(db.clone())?);

        // Ensure the schedule schema exists once up front (propagating any
        // error), so the per-(org) factory on the act path stays cheap and
        // cannot panic. `new` here both creates the schema and validates the DB.
        LocalScheduleStore::new(db.clone(), org_id, profile.owner_principal_id)?;
        let schedule_db = db.clone();
        let owner = profile.owner_principal_id;
        let schedule_factory: ScheduleStoreFactory = Arc::new(move |org_id: i64| {
            // One schedule store per org, over the shared DB handle. Schema is
            // already initialized above, so this is infallible.
            Arc::new(LocalScheduleStore::scoped(
                schedule_db.clone(),
                org_id,
                owner,
            )) as Arc<dyn SessionScheduleStore>
        });

        let runtime_backends = runtime_backends
            .with_session_task_registry(task_registry.clone())
            .with_schedule_store_factory(schedule_factory);

        Ok(Self {
            runtime_backends,
            db,
            task_registry,
            profile,
            org_id,
        })
    }

    /// Build a `LocalScheduleStore` directly (for the embedder that wants the
    /// concrete type and its additive metadata methods).
    pub fn schedule_store(&self) -> Result<LocalScheduleStore> {
        LocalScheduleStore::new(
            self.db.clone(),
            self.org_id,
            self.profile.owner_principal_id,
        )
    }

    /// Start the in-process schedule runner with production defaults.
    ///
    /// Retain the returned handle for the host lifetime and call
    /// [`LocalScheduleRunnerHandle::shutdown`] during graceful shutdown.
    pub fn start_schedule_runner(
        &self,
        runner: Arc<dyn LocalSessionRunner>,
    ) -> Result<LocalScheduleRunnerHandle> {
        self.start_schedule_runner_with_config(runner, LocalScheduleRunnerConfig::default())
    }

    /// Start the in-process schedule runner with custom polling and claim settings.
    pub fn start_schedule_runner_with_config(
        &self,
        runner: Arc<dyn LocalSessionRunner>,
        config: LocalScheduleRunnerConfig,
    ) -> Result<LocalScheduleRunnerHandle> {
        LocalScheduleRunner::new(self.schedule_store()?, runner)
            .with_config(config)
            .start()
            .map_err(everruns_core::AgentLoopError::from)
    }

    /// Attach a platform store factory built from a caller-supplied
    /// `LocalSessionRunner`. Call this after the runtime (and therefore the
    /// runner) exists. The factory hands out a [`LocalPlatformStore`] per
    /// (org, session); the runner is the source of truth for local session
    /// state, so no extra store handles are required.
    pub fn with_platform_runner(mut self, runner: Arc<dyn LocalSessionRunner>) -> Self {
        let base_url = self.profile.base_url.clone();
        let factory: PlatformStoreFactory =
            Arc::new(move |_org_id: i64, _session_id: SessionId| {
                Arc::new(LocalPlatformStore::new(runner.clone(), base_url.clone()))
                    as Arc<dyn PlatformStore>
            });
        self.runtime_backends = self.runtime_backends.with_platform_store_factory(factory);
        self
    }

    /// Internal org id this backend set is scoped to.
    pub fn org_id(&self) -> i64 {
        self.org_id
    }

    pub fn owner_principal_id(&self) -> PrincipalId {
        self.profile.owner_principal_id
    }
}
