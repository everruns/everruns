//! Conservative recovery of persisted executions whose owner is known lost.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TaskExecutor, TaskMessage, TaskOwnership,
    find_task_executor, recover_orphaned_task,
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::task_observer::{TaskTransition, TaskTransitionObserver};
use parking_lot::{Mutex, RwLock};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use super::{LocalSessionTaskRegistry, SqliteDb};

type ExecutorLookup = Arc<dyn Fn(&str) -> Option<Arc<dyn TaskExecutor>> + Send + Sync>;

/// Recovery service and registry decorator for a local profile.
///
/// Reads and cancellation reconcile persisted executions before returning them.
/// Recovery uses the underlying registry's guarded update and the bound wake observer;
/// it never injects parent messages or interprets task age as ownership loss.
/// The underlying registry must not itself call this decorator.
pub struct LocalTaskRecovery {
    db: SqliteDb,
    registry: Arc<dyn SessionTaskRegistry>,
    executor_lookup: ExecutorLookup,
    maintenance: Mutex<Option<JoinHandle<()>>>,
    wake_observer: RwLock<Option<Arc<dyn TaskTransitionObserver>>>,
    wake_delivery: AsyncMutex<()>,
}

impl LocalTaskRecovery {
    /// Wrap an initialized local registry over the same database.
    pub fn new(db: SqliteDb, registry: Arc<dyn SessionTaskRegistry>) -> Self {
        Self {
            db,
            registry,
            executor_lookup: Arc::new(find_task_executor),
            maintenance: Mutex::new(None),
            wake_observer: RwLock::new(None),
            wake_delivery: AsyncMutex::new(()),
        }
    }

    /// Bind the runtime's existing task wake observer before recovering tasks.
    /// The raw registry does not notify this observer on recovery writes.
    /// Delivery is at least once across crashes: durable observers must deduplicate
    /// by session, task ID and terminal attempt before acknowledging the wake.
    pub fn set_wake_observer(&self, observer: Arc<dyn TaskTransitionObserver>) {
        *self.wake_observer.write() = Some(observer);
    }

    /// Reconcile one observed execution, with a snapshot guard against races.
    pub async fn recover_task(&self, task: &SessionTask) -> Result<Option<SessionTask>> {
        if !matches!(task.state, SessionTaskState::Running) {
            return Ok(None);
        }
        let ownership = self.ownership(task);
        let recovered = recover_orphaned_task(self.registry.as_ref(), task, ownership).await?;
        // The guarded registry transaction persists the terminal wake with settlement.
        Ok(recovered)
    }

    fn ownership(&self, task: &SessionTask) -> TaskOwnership {
        let Some(executor) = (self.executor_lookup)(&task.kind) else {
            return TaskOwnership::Unknown;
        };
        let ownership = executor.ownership(task);
        if ownership != TaskOwnership::Unknown {
            return ownership;
        }
        executor
            .local_owner_process(task)
            .map(process_ownership)
            .unwrap_or(TaskOwnership::Unknown)
    }

    /// Reconcile tasks when a session is reopened or resumed.
    /// Returns only tasks settled by this call, not stale or unchanged snapshots.
    pub async fn recover_session(&self, session_id: SessionId) -> Result<Vec<SessionTask>> {
        let recovered = self.recover_session_tasks(session_id).await?;
        self.deliver_recovery_wakes(Some(session_id)).await?;
        Ok(recovered)
    }

    async fn recover_session_tasks(&self, session_id: SessionId) -> Result<Vec<SessionTask>> {
        let filter = SessionTaskFilter {
            state: Some(SessionTaskState::Running),
            ..Default::default()
        };
        let tasks = self.registry.list(session_id, Some(&filter)).await?;
        let mut recovered = Vec::new();
        for task in tasks {
            if let Some(task) = self.recover_task(&task).await? {
                recovered.push(task);
            }
        }
        Ok(recovered)
    }

    /// Reconcile every session with persisted tasks in this local database.
    /// Useful at startup and for hosts that run their own maintenance scheduler.
    pub async fn recover_all(&self) -> Result<Vec<SessionTask>> {
        let state = SessionTaskState::Running.to_string();
        let session_ids: Vec<String> = self.db.with_conn(|conn| {
            let mut statement =
                conn.prepare("SELECT DISTINCT session_id FROM local_tasks WHERE state = ?1")?;
            statement
                .query_map([state], |row| row.get::<_, String>(0))?
                .collect()
        })?;
        let mut recovered = Vec::new();
        for session_id in session_ids {
            let session_id = session_id.parse().map_err(|error| {
                AgentLoopError::store(format!("Invalid persisted task session id: {error}"))
            })?;
            recovered.extend(self.recover_session_tasks(session_id).await?);
        }
        self.deliver_recovery_wakes(None).await?;
        Ok(recovered)
    }

    // Do not hold SQLite during callbacks. A reentrant observer may inspect this registry;
    // try_lock avoids recursively waiting for its own pending wake delivery.
    async fn deliver_recovery_wakes(&self, session_id: Option<SessionId>) -> Result<()> {
        let Some(observer) = self.wake_observer.read().clone() else {
            return Ok(());
        };
        let Ok(_delivery) = self.wake_delivery.try_lock() else {
            return Ok(());
        };
        for task in LocalSessionTaskRegistry::pending_recovery_wakes(&self.db, session_id)? {
            let current = self.registry.get(task.session_id, &task.id).await?;
            if !current.is_some_and(|current| {
                current.attempt == task.attempt && current.state == task.state
            }) {
                // An explicit new attempt supersedes the old terminal wake.
                LocalSessionTaskRegistry::acknowledge_recovery_wake(&self.db, &task)?;
                continue;
            }
            match observer
                .on_transition(&task, TaskTransition::Terminal)
                .await
            {
                Ok(()) => LocalSessionTaskRegistry::acknowledge_recovery_wake(&self.db, &task)?,
                Err(error) => tracing::warn!(
                    %error, task_id = %task.id, attempt = task.attempt,
                    "task recovery wake remains pending for retry"
                ),
            }
        }
        Ok(())
    }

    /// Start low-frequency local maintenance on the current Tokio runtime.
    /// Repeated calls are harmless. Dropping this service stops maintenance.
    pub fn start_maintenance(self: &Arc<Self>) {
        let mut maintenance = self.maintenance.lock();
        if maintenance.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        let weak = Arc::downgrade(self);
        *maintenance = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Startup recovery is explicit; do not run a second immediate sweep.
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(recovery) = weak.upgrade() else {
                    break;
                };
                if let Err(error) = recovery.recover_all().await {
                    tracing::warn!(%error, "local task recovery failed");
                }
            }
        }));
    }
}

impl Drop for LocalTaskRecovery {
    fn drop(&mut self) {
        if let Some(task) = self.maintenance.get_mut().take() {
            task.abort();
        }
    }
}

/// A PID is only a negative proof. A live or reused PID cannot establish who
/// owns an attempt. Only an executor-validated owner marker reaches this probe.
#[cfg(unix)]
fn process_ownership(pid: u32) -> TaskOwnership {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return TaskOwnership::Unknown;
    };
    if pid <= 0 {
        return TaskOwnership::Unknown;
    }
    // SAFETY: signal zero does not send a signal, and the PID is positive.
    if unsafe { libc::kill(pid, 0) } == -1
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    {
        TaskOwnership::OwnerLost
    } else {
        TaskOwnership::Unknown
    }
}

#[cfg(not(unix))]
fn process_ownership(_pid: u32) -> TaskOwnership {
    TaskOwnership::Unknown
}

#[async_trait]
impl SessionTaskRegistry for LocalTaskRecovery {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        self.registry.create(input).await
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        if let Some(task) = self.registry.get(session_id, task_id).await? {
            self.recover_task(&task).await?;
            self.deliver_recovery_wakes(Some(session_id)).await?;
            // A concurrent completion or restart can make the observed snapshot stale.
            self.registry.get(session_id, task_id).await
        } else {
            Ok(None)
        }
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        // Reconcile before filtering, so terminal-state filters see recovered tasks.
        self.recover_session(session_id).await?;
        self.registry.list(session_id, filter).await
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        self.registry.update(session_id, task_id, update).await
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        if let Some(task) = self.registry.request_cancel(session_id, task_id).await? {
            self.recover_task(&task).await?;
            self.deliver_recovery_wakes(Some(session_id)).await?;
            self.registry.get(session_id, task_id).await
        } else {
            Ok(None)
        }
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        self.registry
            .record_message(session_id, task_id, message)
            .await
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        self.registry
            .list_messages(session_id, task_id, limit, after_id)
            .await
    }
}

#[cfg(test)]
#[path = "task_recovery_tests.rs"]
mod tests;
