use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::platform_store::{PlatformCreateSessionRequest, PlatformMessage};
use everruns_core::session::Session;
use everruns_core::session_schedule::{ScheduleType, SessionSchedule};
use everruns_core::traits::SessionScheduleStore;
use everruns_core::typed_id::{AgentId, HarnessId, PrincipalId, ScheduleId, SessionId};
use everruns_local::{
    LocalScheduleRunner, LocalScheduleRunnerConfig, LocalScheduleStore, LocalSessionRunner,
    SqliteDb,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

#[derive(Default)]
struct RecordingRunner {
    attempts: AtomicUsize,
    failures_remaining: AtomicUsize,
    delivered: Mutex<Vec<(SessionId, String)>>,
    attempt_notify: Notify,
    delivery_notify: Notify,
}

impl RecordingRunner {
    fn failing(times: usize) -> Self {
        Self {
            failures_remaining: AtomicUsize::new(times),
            ..Self::default()
        }
    }

    async fn wait_for_attempts(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.attempts.load(Ordering::SeqCst) < expected {
                self.attempt_notify.notified().await;
            }
        })
        .await
        .expect("timed out waiting for delivery attempt");
    }

    async fn wait_for_deliveries(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.delivered.lock().len() < expected {
                self.delivery_notify.notified().await;
            }
        })
        .await
        .expect("timed out waiting for scheduled delivery");
    }
}

#[async_trait]
impl LocalSessionRunner for RecordingRunner {
    async fn create_session(
        &self,
        _harness_id: HarnessId,
        _agent_id: Option<AgentId>,
        _title: Option<&str>,
        _locale: Option<&str>,
        _parent_session_id: Option<SessionId>,
    ) -> Result<Session> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn create_session_with_options(
        &self,
        _request: PlatformCreateSessionRequest,
    ) -> Result<Session> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.attempt_notify.notify_waiters();
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(AgentLoopError::tool("injected delivery failure"));
        }
        self.delivered
            .lock()
            .push((session_id, content.to_string()));
        self.delivery_notify.notify_waiters();
        Ok(())
    }

    async fn list_sessions(
        &self,
        _limit: Option<usize>,
        _agent_id: Option<AgentId>,
    ) -> Result<Vec<Session>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_session(&self, _session_id: SessionId) -> Result<Option<Session>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_messages(
        &self,
        _session_id: SessionId,
        _limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_session_status(&self, _session_id: SessionId) -> Result<Option<String>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }
}

struct BlockingRunner {
    attempts: AtomicUsize,
    entered: Notify,
    block: Notify,
}

impl Default for BlockingRunner {
    fn default() -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            entered: Notify::new(),
            block: Notify::new(),
        }
    }
}

#[async_trait]
impl LocalSessionRunner for BlockingRunner {
    async fn create_session(
        &self,
        _harness_id: HarnessId,
        _agent_id: Option<AgentId>,
        _title: Option<&str>,
        _locale: Option<&str>,
        _parent_session_id: Option<SessionId>,
    ) -> Result<Session> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn send_message(&self, _session_id: SessionId, _content: &str) -> Result<()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_waiters();
        self.block.notified().await;
        Ok(())
    }

    async fn list_sessions(
        &self,
        _limit: Option<usize>,
        _agent_id: Option<AgentId>,
    ) -> Result<Vec<Session>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_session(&self, _session_id: SessionId) -> Result<Option<Session>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_messages(
        &self,
        _session_id: SessionId,
        _limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_session_status(&self, _session_id: SessionId) -> Result<Option<String>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }
}

fn config(poll_interval: Duration, claim_timeout: Duration) -> LocalScheduleRunnerConfig {
    LocalScheduleRunnerConfig {
        poll_interval,
        claim_timeout,
        batch_size: 8,
    }
}

fn store(db: SqliteDb) -> LocalScheduleStore {
    LocalScheduleStore::new(db, 1, PrincipalId::from_seed(1)).unwrap()
}

#[tokio::test]
async fn one_shot_delivers_once_and_disables_schedule() {
    let store = store(SqliteDb::open_in_memory().unwrap());
    let session_id = SessionId::new();
    let schedule = store
        .create_schedule(
            session_id,
            "wake up".into(),
            None,
            Some(chrono::Utc::now() + chrono::Duration::milliseconds(50)),
            "UTC".into(),
        )
        .await
        .unwrap();
    let session_runner = Arc::new(RecordingRunner::default());
    let handle = LocalScheduleRunner::new(store.clone(), session_runner.clone())
        .with_config(config(Duration::from_millis(10), Duration::from_secs(1)))
        .start()
        .unwrap();

    session_runner.wait_for_deliveries(1).await;
    tokio::time::sleep(Duration::from_millis(75)).await;
    handle.shutdown().await.unwrap();

    assert_eq!(
        session_runner.delivered.lock().as_slice(),
        &[(session_id, "wake up".into())]
    );
    let persisted = store
        .list_schedules(session_id)
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == schedule.id)
        .unwrap();
    assert!(!persisted.enabled);
    assert!(persisted.next_trigger_at.is_none());
    assert!(persisted.last_triggered_at.is_some());
    assert_eq!(persisted.trigger_count, 1);
}

#[tokio::test]
async fn recurring_schedule_advances_and_delivers_more_than_once() {
    let store = store(SqliteDb::open_in_memory().unwrap());
    let session_id = SessionId::new();
    let schedule = store
        .create_schedule(
            session_id,
            "tick".into(),
            Some("*/1 * * * * *".into()),
            None,
            "America/Chicago".into(),
        )
        .await
        .unwrap();
    let first_trigger = schedule.next_trigger_at.unwrap();
    let session_runner = Arc::new(RecordingRunner::default());
    let handle = LocalScheduleRunner::new(store.clone(), session_runner.clone())
        .with_config(config(Duration::from_millis(20), Duration::from_secs(1)))
        .start()
        .unwrap();

    session_runner.wait_for_deliveries(2).await;
    handle.shutdown().await.unwrap();

    let persisted = store.list_schedules(session_id).await.unwrap().remove(0);
    assert!(persisted.enabled);
    assert!(persisted.trigger_count >= 2);
    assert!(persisted.last_triggered_at.is_some());
    assert!(persisted.next_trigger_at.unwrap() > first_trigger);
}

#[tokio::test]
async fn concurrent_runners_do_not_duplicate_an_occurrence() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("schedules.db");
    let store_a = store(SqliteDb::open(&path).unwrap());
    let store_b = store(SqliteDb::open(&path).unwrap());
    let session_id = SessionId::new();
    store_a
        .create_schedule(
            session_id,
            "once".into(),
            None,
            Some(chrono::Utc::now()),
            "UTC".into(),
        )
        .await
        .unwrap();
    let session_runner = Arc::new(RecordingRunner::default());
    let runner_config = config(Duration::from_millis(5), Duration::from_secs(1));
    let handle_a = LocalScheduleRunner::new(store_a, session_runner.clone())
        .with_config(runner_config.clone())
        .start()
        .unwrap();
    let handle_b = LocalScheduleRunner::new(store_b, session_runner.clone())
        .with_config(runner_config)
        .start()
        .unwrap();

    session_runner.wait_for_deliveries(1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle_a.shutdown().await.unwrap();
    handle_b.shutdown().await.unwrap();
    assert_eq!(session_runner.delivered.lock().len(), 1);
}

#[tokio::test]
async fn active_claim_heartbeat_prevents_slow_delivery_from_being_reclaimed() {
    let store = store(SqliteDb::open_in_memory().unwrap());
    let session_id = SessionId::new();
    store
        .create_schedule(
            session_id,
            "slow".into(),
            None,
            Some(chrono::Utc::now()),
            "UTC".into(),
        )
        .await
        .unwrap();
    let session_runner = Arc::new(BlockingRunner::default());
    let runner_config = config(Duration::from_millis(5), Duration::from_millis(75));
    let handle_a = LocalScheduleRunner::new(store.clone(), session_runner.clone())
        .with_config(runner_config.clone())
        .start()
        .unwrap();
    let handle_b = LocalScheduleRunner::new(store, session_runner.clone())
        .with_config(runner_config)
        .start()
        .unwrap();

    tokio::time::timeout(Duration::from_secs(2), session_runner.entered.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(session_runner.attempts.load(Ordering::SeqCst), 1);

    session_runner.block.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;
    handle_a.shutdown().await.unwrap();
    handle_b.shutdown().await.unwrap();
    assert_eq!(session_runner.attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stale_claim_is_recovered_after_interrupted_runner() {
    let store = store(SqliteDb::open_in_memory().unwrap());
    let session_id = SessionId::new();
    store
        .create_schedule(
            session_id,
            "recover".into(),
            None,
            Some(chrono::Utc::now()),
            "UTC".into(),
        )
        .await
        .unwrap();
    let blocked = Arc::new(BlockingRunner::default());
    let runner_config = config(Duration::from_millis(10), Duration::from_millis(100));
    let interrupted = LocalScheduleRunner::new(store.clone(), blocked.clone())
        .with_config(runner_config.clone())
        .start()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), blocked.entered.notified())
        .await
        .unwrap();
    interrupted.abort();

    let recovered = Arc::new(RecordingRunner::default());
    let recovery_handle = LocalScheduleRunner::new(store.clone(), recovered.clone())
        .with_config(runner_config)
        .start()
        .unwrap();
    recovered.wait_for_deliveries(1).await;
    recovery_handle.shutdown().await.unwrap();

    assert_eq!(blocked.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(recovered.delivered.lock().len(), 1);
    assert_eq!(
        store.list_schedules(session_id).await.unwrap()[0].trigger_count,
        1
    );
}

#[tokio::test]
async fn failed_delivery_is_recorded_and_retried() {
    let store = store(SqliteDb::open_in_memory().unwrap());
    let session_id = SessionId::new();
    let schedule = store
        .create_schedule(
            session_id,
            "retry".into(),
            None,
            Some(chrono::Utc::now()),
            "UTC".into(),
        )
        .await
        .unwrap();
    let session_runner = Arc::new(RecordingRunner::failing(1));
    let handle = LocalScheduleRunner::new(store.clone(), session_runner.clone())
        .with_config(config(Duration::from_millis(200), Duration::from_secs(1)))
        .start()
        .unwrap();

    session_runner.wait_for_attempts(1).await;
    let recorded = store
        .last_delivery_error(schedule.id)
        .await
        .unwrap()
        .unwrap();
    assert!(recorded.contains("injected delivery failure"));
    let retryable = store.list_schedules(session_id).await.unwrap().remove(0);
    assert!(retryable.enabled);
    assert_eq!(retryable.trigger_count, 0);
    assert!(retryable.next_trigger_at.is_some());

    session_runner.wait_for_deliveries(1).await;
    handle.shutdown().await.unwrap();
    let completed = store.list_schedules(session_id).await.unwrap().remove(0);
    assert_eq!(completed.trigger_count, 1);
    assert!(!completed.enabled);
    assert!(
        store
            .last_delivery_error(schedule.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn existing_recurring_schedule_is_migrated_and_executed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy.db");
    let session_id = SessionId::new();
    let now = chrono::Utc::now();
    let legacy = SessionSchedule {
        id: ScheduleId::new(),
        session_id,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        owner: None,
        effective_owner: None,
        description: "legacy recurring".into(),
        cron_expression: Some("*/1 * * * * *".into()),
        scheduled_at: None,
        timezone: "UTC".into(),
        enabled: true,
        schedule_type: ScheduleType::Recurring,
        next_trigger_at: None,
        last_triggered_at: None,
        trigger_count: 0,
        created_at: now,
        updated_at: now,
    };
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE local_schedules (
                id TEXT PRIMARY KEY,
                org_id INTEGER NOT NULL,
                session_id TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                snapshot TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}'
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_schedules (id, org_id, session_id, enabled, snapshot, metadata)
             VALUES (?1, 1, ?2, 1, ?3, '{}')",
            rusqlite::params![
                legacy.id.to_string(),
                session_id.to_string(),
                serde_json::to_string(&legacy).unwrap(),
            ],
        )
        .unwrap();
    }

    let store = store(SqliteDb::open(&path).unwrap());
    assert!(
        store.list_schedules(session_id).await.unwrap()[0]
            .next_trigger_at
            .is_some()
    );
    let session_runner = Arc::new(RecordingRunner::default());
    let handle = LocalScheduleRunner::new(store, session_runner.clone())
        .with_config(config(Duration::from_millis(10), Duration::from_secs(1)))
        .start()
        .unwrap();
    session_runner.wait_for_deliveries(1).await;
    handle.shutdown().await.unwrap();
}
