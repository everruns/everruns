//! Background, child-session and coordination obligations in the native
//! authority (EVE-1298).
//!
//! Decision: reuse the local profile's task registry and recovery service
//! over the authority's own database file, rather than writing a second task
//! store. Task records, message channels and recovery wakes then share the
//! authority's durability domain. What changes is delivery:
//!
//! - A wake (a thread report to its coordinator, or a recovered task) is an
//!   authority admission with an identity derived from the task, its attempt
//!   and the transition, so a repeated delivery replays instead of starting a
//!   second turn, and a wake accepted before process loss still runs.
//! - Child sessions (subagents, coordination threads) are ordinary sessions of
//!   the same retained build, created through Engine, with their parent link
//!   in the binding and the session record.
//! - On every activation, running tasks whose owner is proved gone are
//!   re-attached when their executor supports it, otherwise failed
//!   conservatively with a wake. Uncertain effects are never replayed: an
//!   executor re-attaches only work it declares safe to run again.
use super::*;
use crate::local::{
    LocalPlatformStore, LocalScheduleRunner, LocalScheduleRunnerHandle, LocalScheduleStore,
    LocalSessionRunner, LocalSessionTaskRegistry, LocalTaskRecovery, SqliteDb,
};
use everruns_capabilities::capabilities::coordination::{
    THREAD_RESOLVED_DETAIL, ThreadTurn, settle_thread_turn,
};
use everruns_capabilities::{PlatformHostBackendsExt, PlatformMessage, PlatformStore};
use everruns_contracts::turn::TurnStopReason;
use everruns_contracts::typed_id::{AgentId, HarnessId};
use everruns_core::host::SessionBuilder;
use everruns_core::session::ExecutionSession;
use everruns_core::session_task::{
    SessionTask, SessionTaskFilter, SessionTaskRegistry, SessionTaskState, SessionTaskUpdate,
    TASK_KIND_ASSIGNMENT, TaskOwnership, find_task_executor,
};
use everruns_core::{ObservingTaskRegistry, TaskTransition, TaskTransitionObserver, wake_text_for};

/// Re-attach attempts before a lost task is failed instead.
const MAX_REATTACH_ATTEMPTS: i32 = 3;
/// Stable namespace for wake admission identities.
const WAKE_NAMESPACE: uuid::Uuid = uuid::Uuid::from_u128(0x6576_6572_7275_6e73_7461_736b_7761_6b65);

/// Internal org of hosted sessions; the runtime scopes schedules by it.
pub(crate) const AUTHORITY_ORG: i64 = everruns_contracts::runtime::organization::DEFAULT_ORG_ID;

/// Principal stamped on schedules, as for the default local profile.
fn schedule_owner() -> everruns_contracts::typed_id::PrincipalId {
    everruns_contracts::typed_id::PrincipalId::from_seed(1)
}

/// Task and schedule services sharing the authority database file.
pub(super) struct AuthorityServices {
    db: SqliteDb,
    pub(super) registry: Arc<LocalSessionTaskRegistry>,
    pub(super) recovery: Arc<LocalTaskRecovery>,
}

impl AuthorityServices {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let db = SqliteDb::open(path).map_err(AgentLoopError::from)?;
        let registry = Arc::new(LocalSessionTaskRegistry::new(db.clone())?);
        let recovery = Arc::new(LocalTaskRecovery::new(db.clone(), registry.clone()));
        // Creates the schedule tables, so a cutover can write rows into them.
        LocalScheduleStore::new(db.clone(), AUTHORITY_ORG, schedule_owner())?;
        Ok(Self {
            db,
            registry,
            recovery,
        })
    }

    /// Schedules of one org. With a journal, the store records timers in the
    /// session log; the runner's store has none, since a delivery is itself
    /// an admission in that log.
    fn schedules(
        &self,
        org: i64,
        journal: Option<Arc<dyn everruns_core::host::EventLog>>,
    ) -> LocalScheduleStore {
        let store = LocalScheduleStore::scoped(self.db.clone(), org, schedule_owner());
        match journal {
            Some(journal) => store.with_journal(journal),
            None => store,
        }
    }
}

/// Whether evidence shows a legacy task is still executing somewhere.
pub(crate) fn task_is_running_elsewhere(task: &SessionTask) -> bool {
    if task.state.is_terminal() || task.state == SessionTaskState::Queued {
        return false;
    }
    match ownership(task) {
        TaskOwnership::Alive => true,
        TaskOwnership::Lost | TaskOwnership::OwnerLost => false,
        // A live local owner process is not proof, but it is not permission
        // to move the work either.
        TaskOwnership::Unknown => find_task_executor(&task.kind)
            .and_then(|executor| executor.local_owner_process(task))
            .is_some(),
    }
}

fn ownership(task: &SessionTask) -> TaskOwnership {
    let Some(executor) = find_task_executor(&task.kind) else {
        return TaskOwnership::Unknown;
    };
    match executor.ownership(task) {
        TaskOwnership::Unknown => executor
            .local_owner_process(task)
            .map(crate::local::process_ownership)
            .unwrap_or(TaskOwnership::Unknown),
        known => known,
    }
}

impl SqliteHosting {
    pub(super) fn task_registry(&self) -> Arc<dyn SessionTaskRegistry> {
        self.services.recovery.clone()
    }

    /// Attach the authority's task registry, child-session store and wakes
    /// to an activation's runtime, after reconciling lost executions.
    pub(super) async fn prepare(
        &self,
        engine: &crate::Engine,
        session: SessionId,
        environment: &crate::Environment,
        backends: HostBackends,
    ) -> Result<HostBackends> {
        self.services
            .recovery
            .set_wake_observer(Arc::new(HostedWaker::new(engine, self.clone(), true)));
        let registry: Arc<dyn SessionTaskRegistry> = Arc::new(
            ObservingTaskRegistry::new(self.task_registry())
                .with_observer(Arc::new(HostedWaker::new(engine, self.clone(), false))),
        );
        let runner: Arc<dyn LocalSessionRunner> = Arc::new(HostedSessionRunner {
            engine: engine.downgrade(),
            hosting: self.clone(),
            registry: registry.clone(),
        });
        let store: Arc<dyn PlatformStore> = Arc::new(LocalPlatformStore::new(runner));
        self.reattach(session, environment, registry.clone(), store.clone())
            .await?;
        self.services.recovery.recover_session(session).await?;
        // Schedule timers are journaled through this epoch's fenced log.
        let services = self.services.clone();
        let journal = backends.event_log.clone();
        Ok(backends
            .with_session_task_registry(registry)
            .with_platform_store_factory(Arc::new(move |_, _| store.clone()))
            .with_schedule_store_factory(Arc::new(move |org| {
                Arc::new(services.schedules(org, Some(journal.clone())))
                    as Arc<dyn everruns_core::session_services::SessionScheduleStore>
            })))
    }

    /// Fire due schedules of hosted sessions as ordinary admissions.
    pub(super) fn start_schedules(
        &self,
        engine: &crate::Engine,
    ) -> Result<LocalScheduleRunnerHandle> {
        let runner: Arc<dyn LocalSessionRunner> = Arc::new(HostedSessionRunner {
            engine: engine.downgrade(),
            hosting: self.clone(),
            registry: self.task_registry(),
        });
        LocalScheduleRunner::new(self.services.schedules(AUTHORITY_ORG, None), runner)
            .start()
            .map_err(AgentLoopError::from)
    }

    /// Re-attach running tasks whose owner is proved gone, when their
    /// executor declares the work safe to start again.
    async fn reattach(
        &self,
        session: SessionId,
        environment: &crate::Environment,
        registry: Arc<dyn SessionTaskRegistry>,
        store: Arc<dyn PlatformStore>,
    ) -> Result<()> {
        let running = self
            .services
            .registry
            .list(
                session,
                Some(&SessionTaskFilter {
                    state: Some(SessionTaskState::Running),
                    ..Default::default()
                }),
            )
            .await?;
        for task in running {
            if !matches!(
                ownership(&task),
                TaskOwnership::Lost | TaskOwnership::OwnerLost
            ) || task.attempt >= MAX_REATTACH_ATTEMPTS
            {
                continue;
            }
            let Some(executor) =
                find_task_executor(&task.kind).filter(|executor| executor.can_reattach_task(&task))
            else {
                continue;
            };
            let attempt = task.attempt + 1;
            let Some(claimed) = self
                .services
                .registry
                .update(
                    session,
                    &task.id,
                    SessionTaskUpdate {
                        worker_id: Some("authority".into()),
                        heartbeat_at: Some(chrono::Utc::now()),
                        state_detail: Some(format!("re-attached (attempt {attempt})")),
                        expected_attempt: Some(task.attempt),
                        increment_attempt: true,
                        ..Default::default()
                    },
                )
                .await?
                .filter(|claimed| claimed.attempt == attempt)
            else {
                continue;
            };
            let mut context = everruns_core::tool_context::ToolContext::with_file_store(
                session,
                environment.workspace_head().file_system(),
            )
            .with_session_task_registry(registry.clone())
            .with_subagent_delegate(Arc::new(
                everruns_capabilities::PlatformStoreSubagentDelegate(store.clone()),
            ));
            context.bind_to_turn_with_causal_context(
                everruns_core::session_task::task_origin_event_context(&claimed)
                    .unwrap_or_else(everruns_core::events::EventContext::empty),
                everruns_core::causal::task_causal_context(&claimed),
            );
            if let Err(error) = executor.start(&claimed, &context).await {
                tracing::warn!(%session, task = %claimed.id, %error, "task re-attach failed");
                self.services
                    .registry
                    .update(
                        session,
                        &claimed.id,
                        SessionTaskUpdate {
                            expected_attempt: Some(attempt),
                            state: Some(SessionTaskState::Failed),
                            state_detail: Some("Re-attach after process loss failed.".into()),
                            error: Some(everruns_core::session_task::TaskError {
                                kind: "orphaned_execution".into(),
                                message: "The task could not be resumed after its owner was lost. Inspect partial artifacts before starting new work.".into(),
                            }),
                            ..Default::default()
                        },
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// Whether `ancestor` is on `session`'s retained parent chain.
    pub(super) fn is_ancestor(&self, session: SessionId, ancestor: SessionId) -> Result<bool> {
        let mut cursor = session;
        for _ in 0..32 {
            let parent = self.transaction(cursor, |_, state| {
                Ok(state
                    .binding
                    .as_ref()
                    .and_then(|binding| binding.metadata.get("parent_session_id"))
                    .and_then(|value| value.as_str())
                    .and_then(|value| value.parse::<SessionId>().ok()))
            })?;
            match parent {
                Some(parent) if parent == ancestor => return Ok(true),
                Some(parent) => cursor = parent,
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    /// Seed a child session's catalog fields before its first activation.
    pub(super) fn seed_record(&self, session: SessionId, record: &ExecutionSession) -> Result<()> {
        self.transaction(session, |tx, state| {
            state.binding(session)?;
            if object::<Value>(tx, session, "session", "record")?.is_none() {
                put(tx, session, "session", "record", record)?;
            }
            Ok(())
        })
    }
}

/// Delivers task wakes as deduplicated authority admissions.
pub(super) struct HostedWaker {
    engine: crate::engine::WeakEngine,
    hosting: SqliteHosting,
    /// Recovery wakes cover every task kind; ordinary transitions only reach
    /// coordinators, as on the legacy lane (other kinds wake mid-turn).
    all_kinds: bool,
}

impl HostedWaker {
    pub(super) fn new(engine: &crate::Engine, hosting: SqliteHosting, all_kinds: bool) -> Self {
        Self {
            engine: engine.downgrade(),
            hosting,
            all_kinds,
        }
    }
}

#[async_trait]
impl TaskTransitionObserver for HostedWaker {
    async fn on_transition(
        &self,
        task: &SessionTask,
        transition: TaskTransition,
    ) -> anyhow::Result<()> {
        if (!self.all_kinds && task.kind != TASK_KIND_ASSIGNMENT)
            || task.state_detail.as_deref() == Some(THREAD_RESOLVED_DETAIL)
        {
            return Ok(());
        }
        let Some(text) = wake_text_for(task, transition) else {
            return Ok(());
        };
        let identity = format!(
            "{}:{}:{}:{:?}:{}",
            task.session_id, task.id, task.attempt, transition, task.state
        );
        let mut message = everruns_core::RuntimeMessage::user(text);
        message.id = MessageId::from_uuid(uuid::Uuid::new_v5(&WAKE_NAMESPACE, identity.as_bytes()));
        message.metadata = Some(everruns_core::message::task_wake_message_metadata());
        everruns_core::causal::task_causal_context(task).write_metadata(&mut message.metadata);
        self.hosting.admit(task.session_id, message, false).await?;
        if let Some(engine) = self.engine.upgrade() {
            let session = task.session_id;
            everruns_contracts::rt::spawn(async move {
                if let Err(error) = engine.activate(session).await {
                    tracing::warn!(%session, %error, "task wake activation failed");
                }
            });
        }
        Ok(())
    }
}

/// Runs child sessions and wakes through the hosting Engine.
struct HostedSessionRunner {
    engine: crate::engine::WeakEngine,
    hosting: SqliteHosting,
    registry: Arc<dyn SessionTaskRegistry>,
}

impl HostedSessionRunner {
    fn engine(&self) -> Result<crate::Engine> {
        self.engine
            .upgrade()
            .ok_or_else(|| AgentLoopError::config("hosting Engine is gone"))
    }

    async fn parent(&self, session: SessionId) -> Result<Option<SessionId>> {
        Ok(self
            .hosting
            .status(session)
            .await?
            .binding
            .metadata
            .get("parent_session_id")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse().ok()))
    }

    /// Report a finished thread turn to its coordinator, if it has one.
    async fn settle_thread(
        &self,
        coordinator: Option<SessionId>,
        session_id: SessionId,
        result: &std::result::Result<crate::Turn, crate::RunError>,
    ) {
        let Some(coordinator) = coordinator else {
            return;
        };
        let turn = match result {
            Ok(turn) if turn.success => ThreadTurn::Completed,
            Ok(turn) if turn.stop_reason == TurnStopReason::Cancelled => ThreadTurn::Cancelled,
            _ => ThreadTurn::Failed,
        };
        let _ = settle_thread_turn(self.registry.as_ref(), coordinator, session_id, turn).await;
    }
}

#[async_trait]
impl LocalSessionRunner for HostedSessionRunner {
    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        parent_session_id: Option<SessionId>,
    ) -> Result<ExecutionSession> {
        let parent = parent_session_id.ok_or_else(|| {
            AgentLoopError::tool("a hosted session only creates child sessions of its own")
        })?;
        let engine = self.engine()?;
        let build = engine.session_binding(parent).await?.behavior;
        let child = SessionId::new();
        // The child runs the parent's retained build; its link is part of the
        // immutable binding, so it survives every rebuild and restart.
        engine
            .create_registered_with_metadata(
                child,
                &build,
                serde_json::json!({"parent_session_id": parent}),
            )
            .await?;
        let mut record = SessionBuilder::new(harness_id)
            .id(child)
            .title(title.unwrap_or("Thread"))
            .build();
        record.agent_id = agent_id;
        record.locale = locale.map(str::to_string);
        record.parent_session_id = Some(parent);
        self.hosting.seed_record(child, &record)?;
        Ok(record)
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        self.send_message_with_causal_context(session_id, content, &crate::CausalContext::default())
            .await
    }

    async fn send_message_with_causal_context(
        &self,
        session_id: SessionId,
        content: &str,
        causes: &crate::CausalContext,
    ) -> Result<()> {
        let session = self
            .engine()?
            .resume(session_id)
            .await
            .map_err(|error| AgentLoopError::tool(error.to_string()))?;
        let coordinator = self.parent(session_id).await?;
        if let Some(coordinator) = coordinator {
            let _ = settle_thread_turn(
                self.registry.as_ref(),
                coordinator,
                session_id,
                ThreadTurn::Started,
            )
            .await;
        }
        let result = session
            .run(crate::InputMessage::user(content).with_causal_context(causes))
            .await;
        self.settle_thread(coordinator, session_id, &result).await;
        match result {
            Ok(turn) if turn.success => Ok(()),
            Ok(turn) => Err(AgentLoopError::tool(format!(
                "thread turn failed: {}",
                turn.error.unwrap_or_default()
            ))),
            Err(error) => Err(AgentLoopError::tool(error.to_string())),
        }
    }

    async fn deliver_occurrence(
        &self,
        session_id: SessionId,
        content: &str,
        occurrence: everruns_contracts::typed_id::MessageId,
    ) -> Result<()> {
        let session = self
            .engine()?
            .resume(session_id)
            .await
            .map_err(|error| AgentLoopError::tool(error.to_string()))?;
        let coordinator = self.parent(session_id).await?;
        if let Some(coordinator) = coordinator {
            let _ = settle_thread_turn(
                self.registry.as_ref(),
                coordinator,
                session_id,
                ThreadTurn::Started,
            )
            .await;
        }
        // The same id reopens the receipt a lost acknowledgement left behind,
        // whether its turn is pending, running or already finished (EVE-1309).
        let result = match session
            .send_with_id(occurrence, crate::InputMessage::user(content))
            .await
        {
            Ok(sent) => sent.wait().await,
            Err(error) => Err(error),
        };
        self.settle_thread(coordinator, session_id, &result).await;
        // Any committed outcome delivers the occurrence, a failed turn included:
        // retrying would only reopen that outcome, and the schedule must advance.
        result
            .map(|_| ())
            .map_err(|error| AgentLoopError::tool(error.to_string()))
    }

    async fn list_sessions(
        &self,
        _limit: Option<usize>,
        _agent_id: Option<AgentId>,
    ) -> Result<Vec<ExecutionSession>> {
        Ok(Vec::new())
    }

    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        match self
            .hosting
            .reader(session_id)
            .session_store
            .get_session(session_id)
            .await
        {
            Err(AgentLoopError::SessionNotFound(_)) => Ok(None),
            other => other,
        }
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        crate::coordination::platform_messages(
            self.hosting.reader(session_id).event_log,
            session_id,
            limit,
        )
        .await
    }

    async fn get_session_status(&self, session_id: SessionId) -> Result<Option<String>> {
        match self.hosting.status(session_id).await {
            Ok(status) => Ok(Some(status.phase().to_string())),
            Err(AgentLoopError::SessionNotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn latest_terminal_turn_status(&self, session_id: SessionId) -> Result<Option<String>> {
        crate::coordination::latest_terminal_turn_status(
            self.hosting.reader(session_id).event_log,
            session_id,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosting::AgentBuild;
    use everruns_core::execution_loading::SessionStore;

    /// Child sessions created through the hosted platform store are ordinary
    /// hosted sessions of the parent's build, with a durable parent link.
    #[tokio::test]
    async fn hosted_child_sessions_keep_their_build_and_parent_across_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authority.db");
        let build = AgentBuild::new("assistant", "child-v1");
        let agent = || {
            crate::Agent::builder()
                .instructions("Answer briefly.")
                .model(crate::Model::simulated("Hello!"))
                .build()
                .unwrap()
        };
        let profile = SqliteHosting::open(&path).unwrap();
        let engine = crate::Engine::builder()
            .hosting(profile.clone())
            .register(build.clone(), agent())
            .build();
        let parent = SessionId::new();
        engine.create_registered(parent, &build).await.unwrap();
        let runner = HostedSessionRunner {
            engine: engine.downgrade(),
            hosting: profile.clone(),
            registry: profile.task_registry(),
        };
        let child = runner
            .create_session(HarnessId::new(), None, Some("Thread"), None, Some(parent))
            .await
            .unwrap();
        let causes = crate::CausalContext::from_traceparents([
            "00-1234567890abcdef1234567890abcdef-1234567890abcdef-00",
        ]);
        runner
            .send_message_with_causal_context(child.id, "work on it", &causes)
            .await
            .unwrap();
        let events = engine
            .resume(child.id)
            .await
            .unwrap()
            .events_after(0)
            .await
            .unwrap();
        let input = events
            .iter()
            .find(|event| event.event_type() == "input.message")
            .unwrap();
        assert_eq!(
            input.canonical_json()["data"]["message"]["metadata"]["everruns.causal_links"][0],
            causes.links()[0].traceparent()
        );
        assert_eq!(runner.get_messages(child.id, None).await.unwrap().len(), 2);
        assert_eq!(
            runner
                .get_session_status(child.id)
                .await
                .unwrap()
                .as_deref(),
            Some("idle")
        );
        assert!(
            runner
                .create_session(HarnessId::new(), None, None, None, None)
                .await
                .is_err(),
            "a hosted session only creates its own children"
        );
        drop(runner);
        drop(engine);

        let restarted = SqliteHosting::open(&path).unwrap();
        let binding = restarted.status(child.id).await.unwrap().binding;
        assert_eq!(binding.behavior, build);
        assert_eq!(
            binding.metadata["parent_session_id"],
            serde_json::json!(parent.to_string())
        );
        let record = restarted
            .reader(child.id)
            .session_store
            .get_session(child.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.parent_session_id, Some(parent));
        assert_eq!(record.title.as_deref(), Some("Thread"));
    }
}

#[cfg(test)]
mod occurrence_tests {
    use super::*;
    use crate::hosting::AgentBuild;
    use everruns_contracts::runtime::session_services::SessionScheduleStore;
    use everruns_llmsim::{LlmSimConfig, ToolCallConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DESCRIPTION: &str = "Run the hourly check.";

    /// One activation of the authority file: each agent turn calls `effect`
    /// once, counted across restarts.
    fn open(
        path: &Path,
        build: &AgentBuild,
        effects: &Arc<AtomicUsize>,
    ) -> (crate::Engine, SqliteHosting, Arc<HostedSessionRunner>) {
        let counted = effects.clone();
        let effect = crate::FunctionTool::new(
            "effect",
            "Perform the scheduled side effect.",
            serde_json::json!({"type": "object", "properties": {}}),
            move |_arguments: Value| {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, String>(serde_json::json!({"done": true}))
                }
            },
        );
        let call = everruns_contracts::tool_types::ToolCall {
            id: "call_effect".into(),
            name: "effect".into(),
            arguments: serde_json::json!({}),
        };
        let model = crate::Model::simulated_with_config(LlmSimConfig {
            tool_calls: Some(ToolCallConfig::Sequence(vec![vec![call], vec![]])),
            ..LlmSimConfig::fixed("Checked.")
        });
        let agent = crate::Agent::builder()
            .instructions("Run the check.")
            .model(model)
            .tool(effect)
            .build()
            .unwrap();
        let hosting = SqliteHosting::open(path).unwrap();
        let engine = crate::Engine::builder()
            .hosting(hosting.clone())
            .register(build.clone(), agent)
            .build();
        let runner = Arc::new(HostedSessionRunner {
            engine: engine.downgrade(),
            hosting: hosting.clone(),
            registry: hosting.task_registry(),
        });
        (engine, hosting, runner)
    }

    async fn journal_count(engine: &crate::Engine, session: SessionId, event_type: &str) -> usize {
        engine
            .resume(session)
            .await
            .unwrap()
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .filter(|event| event.event_type() == event_type)
            .count()
    }

    /// A schedule occurrence retried after process loss reopens its original
    /// admission instead of running again (EVE-1309). Covers loss after the
    /// turn's terminal commit and loss after admission but before execution,
    /// both before the claim was acknowledged. The next occurrence, with the
    /// same description, is distinct and runs.
    #[tokio::test]
    async fn retried_schedule_occurrences_reopen_their_admission_across_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authority.db");
        let build = AgentBuild::new("assistant", "scheduled-v1");
        let effects = Arc::new(AtomicUsize::new(0));
        let timeout = std::time::Duration::from_secs(1);
        let session = SessionId::new();

        let (engine, hosting, runner) = open(&path, &build, &effects);
        engine.create_registered(session, &build).await.unwrap();
        let store = hosting.services.schedules(AUTHORITY_ORG, None);
        let schedule = store
            .create_schedule(
                session,
                DESCRIPTION.into(),
                Some("0 * * * *".into()),
                None,
                "UTC".into(),
            )
            .await
            .unwrap();
        let first_due = schedule.next_trigger_at.unwrap();

        // Occurrence 1: the turn commits, then the process dies before the
        // claim is acknowledged.
        let claim = store
            .claim_due("lost", first_due, timeout, 8, None)
            .unwrap();
        let first = crate::local::occurrence_message_id(&claim[0].schedule);
        runner
            .deliver_occurrence(session, DESCRIPTION, first)
            .await
            .unwrap();
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        drop((runner, store, hosting));
        engine.shutdown(std::time::Duration::from_secs(5)).await;

        let (engine, hosting, runner) = open(&path, &build, &effects);
        let store = hosting.services.schedules(AUTHORITY_ORG, None);
        let retry_at = first_due + chrono::Duration::seconds(5);
        let claim = store
            .claim_due("restarted", retry_at, timeout, 8, None)
            .unwrap();
        assert_eq!(
            claim[0].schedule.trigger_count, 0,
            "occurrence was retained"
        );
        assert_eq!(
            crate::local::occurrence_message_id(&claim[0].schedule),
            first
        );
        runner
            .deliver_occurrence(session, DESCRIPTION, first)
            .await
            .unwrap();
        assert_eq!(
            effects.load(Ordering::SeqCst),
            1,
            "retry reopened the outcome"
        );
        store
            .complete_delivery(&claim[0], "restarted", retry_at)
            .await
            .unwrap();

        // Occurrence 2, same description: admitted, then the process dies
        // before the turn executes or the claim is acknowledged.
        let second_due = store.list_schedules(session).await.unwrap()[0]
            .next_trigger_at
            .unwrap();
        let claim = store
            .claim_due("lost", second_due, timeout, 8, None)
            .unwrap();
        assert_eq!(claim[0].schedule.trigger_count, 1);
        let second = crate::local::occurrence_message_id(&claim[0].schedule);
        assert_ne!(second, first, "a later occurrence is a new admission");
        drop((runner, store));
        engine.shutdown(std::time::Duration::from_secs(5)).await;
        let mut message =
            everruns_core::host::AcceptedTurnInput::new(crate::InputMessage::user(DESCRIPTION))
                .into_message();
        message.id = second;
        hosting.admit(session, message, false).await.unwrap();
        drop(hosting);

        let (engine, hosting, runner) = open(&path, &build, &effects);
        let store = hosting.services.schedules(AUTHORITY_ORG, None);
        let retry_at = second_due + chrono::Duration::seconds(5);
        let claim = store
            .claim_due("restarted", retry_at, timeout, 8, None)
            .unwrap();
        assert_eq!(
            crate::local::occurrence_message_id(&claim[0].schedule),
            second
        );
        runner
            .deliver_occurrence(session, DESCRIPTION, second)
            .await
            .unwrap();
        store
            .complete_delivery(&claim[0], "restarted", retry_at)
            .await
            .unwrap();

        assert_eq!(
            effects.load(Ordering::SeqCst),
            2,
            "one effect per occurrence"
        );
        assert_eq!(journal_count(&engine, session, "input.message").await, 2);
        assert_eq!(journal_count(&engine, session, "turn.started").await, 2);
        let advanced = store.list_schedules(session).await.unwrap().remove(0);
        assert_eq!(advanced.trigger_count, 2, "one advancement per occurrence");
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    use crate::hosting::AgentBuild;
    use everruns_contracts::typed_id::MessageId;

    fn engine_with(
        profile: &SqliteHosting,
        build: &AgentBuild,
        model: crate::Model,
    ) -> crate::Engine {
        let agent = crate::Agent::builder()
            .instructions("Answer briefly.")
            .model(model)
            .build()
            .unwrap();
        crate::Engine::builder()
            .hosting(profile.clone())
            .register(build.clone(), agent)
            .build()
    }

    async fn child_of(
        engine: &crate::Engine,
        profile: &SqliteHosting,
        build: &AgentBuild,
    ) -> (LocalPlatformStore, Arc<HostedSessionRunner>, SessionId) {
        let parent = SessionId::new();
        engine.create_registered(parent, build).await.unwrap();
        let runner = Arc::new(HostedSessionRunner {
            engine: engine.downgrade(),
            hosting: profile.clone(),
            registry: profile.task_registry(),
        });
        let child = runner
            .create_session(HarnessId::new(), None, Some("Child"), None, Some(parent))
            .await
            .unwrap();
        (LocalPlatformStore::new(runner.clone()), runner, child.id)
    }

    /// A native child is idle after success, failure and cancellation alike.
    /// Waiting on it reports the finished turn's outcome, which is what lets a
    /// foreground subagent task settle (EVE-1310); a child that never ran a
    /// turn stays a nonterminal `idle`.
    #[tokio::test]
    async fn waiting_on_a_native_child_reports_its_turn_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();

        let ok = AgentBuild::new("assistant", "ok-v1");
        let engine = engine_with(&profile, &ok, crate::Model::simulated("Done."));
        let (store, runner, child) = child_of(&engine, &profile, &ok).await;
        assert_eq!(store.wait_for_idle(child, Some(5)).await.unwrap(), "idle");
        runner.send_message(child, "work on it").await.unwrap();
        assert_eq!(
            store.wait_for_idle(child, Some(5)).await.unwrap(),
            "completed"
        );

        let session = engine.resume(child).await.unwrap();
        let message = MessageId::new();
        session.send_with_id(message, "stop this").await.unwrap();
        let admission = session.admission(message).await.unwrap().unwrap();
        admission.cancel(uuid::Uuid::new_v4()).await.unwrap();
        engine.activate(child).await.unwrap();
        assert_eq!(
            admission.wait().await.unwrap().stop_reason,
            TurnStopReason::Cancelled
        );
        assert_eq!(
            store.wait_for_idle(child, Some(5)).await.unwrap(),
            "cancelled"
        );

        let profile = SqliteHosting::open(directory.path().join("failing.db")).unwrap();
        let failing = AgentBuild::new("assistant", "failing-v1");
        let engine = engine_with(
            &profile,
            &failing,
            crate::Model::simulated_with_config(everruns_llmsim::LlmSimConfig::error(
                "provider unavailable",
            )),
        );
        let (store, runner, child) = child_of(&engine, &profile, &failing).await;
        assert!(runner.send_message(child, "work on it").await.is_err());
        assert_eq!(store.wait_for_idle(child, Some(5)).await.unwrap(), "failed");
    }
}

#[cfg(test)]
mod causal_tests {
    use super::*;
    use everruns_core::host::{EventReadLimit, EventReadRequest};
    use everruns_core::session_task::{CreateSessionTask, TaskWakePolicy};

    #[tokio::test]
    async fn causal_authority_recovery_wake_keeps_first_causes_across_restart_and_retry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wake.db");
        let hosting = SqliteHosting::open(&path).unwrap();
        let engine = crate::Engine::builder().hosting(hosting.clone()).build();
        let session = SessionId::new();
        hosting
            .create(
                session,
                &crate::hosting::SessionBinding {
                    framework: 1,
                    behavior: crate::hosting::AgentBuild::new("assistant", "v1"),
                    harness: None,
                    environment: None,
                    metadata: Value::Null,
                },
            )
            .await
            .unwrap();
        let causes = crate::CausalContext::from_traceparents([
            "00-1234567890abcdef1234567890abcdef-1234567890abcdef-00",
        ]);
        let mut spec = serde_json::json!({});
        causes.write_task_context(&mut spec);
        let task = hosting
            .services
            .registry
            .create(CreateSessionTask {
                session_id: session,
                kind: TASK_KIND_ASSIGNMENT.into(),
                id: None,
                display_name: "done".into(),
                spec,
                state: SessionTaskState::Succeeded,
                links: Default::default(),
                wake_policy: TaskWakePolicy::OnTerminal,
            })
            .await
            .unwrap();
        let waker = HostedWaker::new(&engine, hosting.clone(), true);
        drop(engine); // Commit admissions without starting a replacement turn.
        waker
            .on_transition(&task, TaskTransition::Terminal)
            .await
            .unwrap();
        drop(waker);
        drop(hosting);

        let reopened = SqliteHosting::open(&path).unwrap();
        let engine = crate::Engine::builder().hosting(reopened.clone()).build();
        let waker = HostedWaker::new(&engine, reopened.clone(), true);
        drop(engine);
        let mut retried = reopened
            .services
            .registry
            .get(session, &task.id)
            .await
            .unwrap()
            .unwrap();
        crate::CausalContext::from_traceparents([
            "00-2234567890abcdef1234567890abcdef-2234567890abcdef-00",
        ])
        .write_task_context(&mut retried.spec);
        waker
            .on_transition(&retried, TaskTransition::Terminal)
            .await
            .unwrap();
        let page = reopened
            .reader(session)
            .event_log
            .read_page(EventReadRequest::new(
                session,
                EventReadLimit::new(100).unwrap(),
            ))
            .await
            .unwrap();
        let inputs: Vec<_> = page
            .events
            .iter()
            .filter_map(|event| match &event.data {
                EventData::InputMessage(data) => Some(&data.message),
                _ => None,
            })
            .collect();
        assert_eq!(
            inputs.len(),
            1,
            "a retried wake must reuse its committed admission"
        );
        assert_eq!(
            crate::CausalContext::from_metadata(inputs[0].metadata.as_ref()),
            causes
        );
    }
}
