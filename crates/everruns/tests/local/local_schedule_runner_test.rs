use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use everruns::local::{
    LocalScheduleRunner, LocalScheduleRunnerConfig, LocalScheduleStore, LocalSessionRunner,
    SqliteDb,
};
use everruns_capabilities::{PlatformCreateSessionRequest, PlatformMessage};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{AgentId, HarnessId, PrincipalId, ScheduleId, SessionId};
use everruns_core::session::ExecutionSession;
use everruns_core::session_schedule::{ScheduleType, SessionSchedule};
use everruns_core::session_services::SessionScheduleStore;
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
    ) -> Result<ExecutionSession> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn create_session_with_options(
        &self,
        _request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
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
    ) -> Result<Vec<ExecutionSession>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_session(&self, _session_id: SessionId) -> Result<Option<ExecutionSession>> {
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

#[derive(Default)]
struct RoutingRunner {
    active_sessions: Mutex<HashSet<SessionId>>,
    attempts: AtomicUsize,
    attempted_sessions: Mutex<Vec<SessionId>>,
    delivered: Mutex<Vec<(SessionId, String)>>,
    delivery_notify: Notify,
}

impl RoutingRunner {
    fn activate(&self, session_id: SessionId) {
        self.active_sessions.lock().insert(session_id);
    }

    fn attempts_for(&self, session_id: SessionId) -> usize {
        self.attempted_sessions
            .lock()
            .iter()
            .filter(|attempted| **attempted == session_id)
            .count()
    }

    async fn wait_for_deliveries(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.delivered.lock().len() < expected {
                self.delivery_notify.notified().await;
            }
        })
        .await
        .expect("timed out waiting for routed delivery");
    }
}

#[async_trait]
impl LocalSessionRunner for RoutingRunner {
    async fn routable_session_ids(&self) -> Result<Option<Vec<SessionId>>> {
        Ok(Some(self.active_sessions.lock().iter().copied().collect()))
    }

    async fn create_session(
        &self,
        _harness_id: HarnessId,
        _agent_id: Option<AgentId>,
        _title: Option<&str>,
        _locale: Option<&str>,
        _parent_session_id: Option<SessionId>,
    ) -> Result<ExecutionSession> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.attempted_sessions.lock().push(session_id);
        if !self.active_sessions.lock().contains(&session_id) {
            return Err(AgentLoopError::tool("session is inactive"));
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
    ) -> Result<Vec<ExecutionSession>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_session(&self, _session_id: SessionId) -> Result<Option<ExecutionSession>> {
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
    ) -> Result<ExecutionSession> {
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
    ) -> Result<Vec<ExecutionSession>> {
        Err(AgentLoopError::tool("unused in schedule runner test"))
    }

    async fn get_session(&self, _session_id: SessionId) -> Result<Option<ExecutionSession>> {
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
        .with_config(config(
            Duration::from_millis(10),
            Duration::from_millis(100),
        ))
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(session_runner.attempts.load(Ordering::SeqCst), 1);

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
async fn inactive_session_is_skipped_until_it_becomes_routable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("scoped-schedules.db");
    let store_a = store(SqliteDb::open(&path).unwrap());
    let store_b = store(SqliteDb::open(&path).unwrap());
    let inactive_session_id = SessionId::new();
    let active_session_id = SessionId::new();
    let inactive_schedule = store_a
        .create_schedule(
            inactive_session_id,
            "later".into(),
            None,
            Some(chrono::Utc::now() - chrono::Duration::milliseconds(10)),
            "UTC".into(),
        )
        .await
        .unwrap();
    store_a
        .create_schedule(
            active_session_id,
            "now".into(),
            None,
            Some(chrono::Utc::now()),
            "UTC".into(),
        )
        .await
        .unwrap();
    let session_runner = Arc::new(RoutingRunner::default());
    session_runner.activate(active_session_id);
    let mut runner_config = config(Duration::from_millis(10), Duration::from_secs(1));
    runner_config.batch_size = 1;
    let handle_a = LocalScheduleRunner::new(store_a.clone(), session_runner.clone())
        .with_config(runner_config.clone())
        .start()
        .unwrap();
    let handle_b = LocalScheduleRunner::new(store_b, session_runner.clone())
        .with_config(runner_config)
        .start()
        .unwrap();

    session_runner.wait_for_deliveries(1).await;
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert_eq!(session_runner.attempts_for(inactive_session_id), 0);
    assert_eq!(session_runner.attempts_for(active_session_id), 1);
    let pending = store_a
        .list_schedules(inactive_session_id)
        .await
        .unwrap()
        .remove(0);
    assert!(pending.enabled);
    assert_eq!(pending.trigger_count, 0);
    assert!(pending.next_trigger_at.is_some());

    session_runner.activate(inactive_session_id);
    session_runner.wait_for_deliveries(2).await;
    handle_a.shutdown().await.unwrap();
    handle_b.shutdown().await.unwrap();

    assert_eq!(session_runner.attempts_for(inactive_session_id), 1);
    assert_eq!(session_runner.attempts_for(active_session_id), 1);
    let mut delivered = session_runner.delivered.lock().clone();
    delivered.sort_by_key(|(session_id, _)| session_id.to_string());
    let mut expected = vec![
        (inactive_session_id, "later".into()),
        (active_session_id, "now".into()),
    ];
    expected.sort_by_key(|(session_id, _)| session_id.to_string());
    assert_eq!(delivered, expected);
    let completed = store_a
        .list_schedules(inactive_session_id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(completed.id, inactive_schedule.id);
    assert!(!completed.enabled);
    assert_eq!(completed.trigger_count, 1);
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

// ---------------------------------------------------------------------------
// Schedules as timers in the session log (actor-based design, step 3)
// ---------------------------------------------------------------------------

mod journal {
    use super::*;
    use everruns_contracts::runtime::events::{
        EventContext, EventData, EventRequest, TimerCancelledData, TimerFiredData,
    };
    use everruns_core::host::{
        EventLog, EventReadLimit, EventReadRequest, EventReader, InMemoryEventLog,
    };

    fn journaled(db: SqliteDb, log: &Arc<InMemoryEventLog>) -> LocalScheduleStore {
        store(db).with_journal(log.clone() as Arc<dyn EventLog>)
    }

    async fn timer_events(log: &InMemoryEventLog, session_id: SessionId) -> Vec<EventData> {
        let page = log
            .read_page(EventReadRequest::new(
                session_id,
                EventReadLimit::new(100).unwrap(),
            ))
            .await
            .unwrap();
        page.events
            .into_iter()
            .map(|event| event.data)
            .filter(|data| data.event_type().starts_with("timer."))
            .collect()
    }

    async fn append(log: &InMemoryEventLog, session_id: SessionId, data: impl Into<EventData>) {
        log.append(EventRequest::new(session_id, EventContext::empty(), data))
            .await
            .unwrap();
    }

    async fn run_briefly(store: &LocalScheduleStore, runner: &Arc<RecordingRunner>) {
        let handle = LocalScheduleRunner::new(store.clone(), runner.clone())
            .with_config(config(Duration::from_millis(10), Duration::from_secs(1)))
            .start()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn create_fire_and_cancel_are_written_to_the_session_log() {
        let log = Arc::new(InMemoryEventLog::new());
        let store = journaled(SqliteDb::open_in_memory().unwrap(), &log);
        let session_id = SessionId::new();
        let once = store
            .create_schedule(
                session_id,
                "wake up".into(),
                None,
                Some(chrono::Utc::now()),
                "UTC".into(),
            )
            .await
            .unwrap();
        let daily = store
            .create_schedule(
                session_id,
                "daily".into(),
                Some("0 9 * * *".into()),
                None,
                "UTC".into(),
            )
            .await
            .unwrap();
        let runner = Arc::new(RecordingRunner::default());
        run_briefly(&store, &runner).await;
        store.cancel_schedule(session_id, daily.id).await.unwrap();

        let kinds: Vec<(&str, String)> = timer_events(&log, session_id)
            .await
            .iter()
            .map(|data| {
                let id = match data {
                    EventData::TimerSet(d) => d.timer_id.clone(),
                    EventData::TimerFired(d) => d.timer_id.clone(),
                    EventData::TimerCancelled(d) => d.timer_id.clone(),
                    other => panic!("unexpected {other:?}"),
                };
                (data.event_type(), id)
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("timer.set", once.id.to_string()),
                ("timer.set", daily.id.to_string()),
                ("timer.fired", once.id.to_string()),
                ("timer.cancelled", daily.id.to_string()),
            ]
        );
    }

    /// Register `session_id` in the Framework session catalog of `db`.
    fn catalog(db: &SqliteDb, session_id: SessionId) {
        everruns::local::LocalSessionStore::new(db.clone()).unwrap();
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO framework_sessions (session_id) VALUES (?1)",
                [session_id.to_string()],
            )
        })
        .unwrap();
    }

    #[tokio::test]
    async fn a_schedule_the_index_lost_is_rebuilt_from_the_log_and_delivered() {
        let log = Arc::new(InMemoryEventLog::new());
        let session_id = SessionId::new();
        let original = journaled(SqliteDb::open_in_memory().unwrap(), &log)
            .create_schedule(
                session_id,
                "still due".into(),
                None,
                Some(chrono::Utc::now()),
                "UTC".into(),
            )
            .await
            .unwrap();

        // A fresh database: the index is gone, the log is not.
        let db = SqliteDb::open_in_memory().unwrap();
        catalog(&db, session_id);
        let store = journaled(db, &log);
        assert!(store.list_schedules(session_id).await.unwrap().is_empty());

        let runner = Arc::new(RecordingRunner::default());
        run_briefly(&store, &runner).await;

        assert_eq!(
            runner.delivered.lock().as_slice(),
            &[(session_id, "still due".into())]
        );
        let rebuilt = store.list_schedules(session_id).await.unwrap();
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt[0].id, original.id);
        assert!(!rebuilt[0].enabled);
        assert_eq!(rebuilt[0].trigger_count, 1);
    }

    #[tokio::test]
    async fn an_occurrence_the_log_records_is_not_delivered_again() {
        let log = Arc::new(InMemoryEventLog::new());
        let store = journaled(SqliteDb::open_in_memory().unwrap(), &log);
        let session_id = SessionId::new();
        let schedule = store
            .create_schedule(
                session_id,
                "once".into(),
                None,
                Some(chrono::Utc::now()),
                "UTC".into(),
            )
            .await
            .unwrap();
        // The process recorded the occurrence, then died before the index
        // write: the row still looks due.
        append(
            &log,
            session_id,
            TimerFiredData {
                timer_id: schedule.id.to_string(),
                fired_at: chrono::Utc::now(),
                next_fire_at: None,
            },
        )
        .await;

        let runner = Arc::new(RecordingRunner::default());
        run_briefly(&store, &runner).await;

        assert!(runner.delivered.lock().is_empty());
        let row = &store.list_schedules(session_id).await.unwrap()[0];
        assert!(!row.enabled);
        assert_eq!(row.trigger_count, 1);
    }

    #[tokio::test]
    async fn a_cancellation_the_log_records_disables_the_row() {
        let log = Arc::new(InMemoryEventLog::new());
        let store = journaled(SqliteDb::open_in_memory().unwrap(), &log);
        let session_id = SessionId::new();
        let schedule = store
            .create_schedule(
                session_id,
                "daily".into(),
                Some("0 9 * * *".into()),
                None,
                "UTC".into(),
            )
            .await
            .unwrap();
        append(
            &log,
            session_id,
            TimerCancelledData {
                timer_id: schedule.id.to_string(),
            },
        )
        .await;

        let report = store.reconcile_journal().await.unwrap();

        assert_eq!(report.advanced, 1);
        assert!(!store.list_schedules(session_id).await.unwrap()[0].enabled);
    }

    #[tokio::test]
    async fn a_schedule_created_before_journaling_is_written_into_the_log() {
        let db = SqliteDb::open_in_memory().unwrap();
        let session_id = SessionId::new();
        let schedule = store(db.clone())
            .create_schedule(
                session_id,
                "legacy".into(),
                Some("0 9 * * *".into()),
                None,
                "UTC".into(),
            )
            .await
            .unwrap();
        let log = Arc::new(InMemoryEventLog::new());
        let store = journaled(db, &log);

        let first = store.reconcile_journal().await.unwrap();
        let second = store.reconcile_journal().await.unwrap();

        assert_eq!(first.journaled, 1);
        assert_eq!(second, Default::default(), "reconcile is idempotent");
        match timer_events(&log, session_id).await.as_slice() {
            [EventData::TimerSet(set)] => {
                assert_eq!(set.timer_id, schedule.id.to_string());
                assert_eq!(set.cron_expression.as_deref(), Some("0 9 * * *"));
            }
            other => panic!("expected one timer.set, got {other:?}"),
        }
    }
}
