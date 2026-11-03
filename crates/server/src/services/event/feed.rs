//! Delivers stored events to the server's listeners from the database, so a
//! listener sees every committed event of its types, in each session's
//! order, across restarts, deploys and NATS outages.
//!
//! Decisions (`knowledge/execution/event-listeners.md`):
//! - Exporters that pair per-process state with the events they see stay
//!   in-process (`listener_dispatch`, queued after the commit as before):
//!   OpenTelemetry ends a turn's anchor span, started when the turn's first
//!   event was prepared, when it sees that event; Braintrust and Prometheus
//!   keep per-process state and counters. They are observability, so a crash
//!   losing their work is the accepted cost.
//! - Two consumers. Near (polled every 250 ms and woken by this replica's own
//!   writes): coordination, budgets, A2A push, notifications, MCP events,
//!   sandbox lifecycle, and listeners an embedder adds. Later (polled every
//!   2 s, larger pages): usage, tracing and metrics exporters, titles, run
//!   summaries, turn latency, observer matching, approval audit. Each keeps
//!   its own cursor, so a slow exporter never delays a coordination wake-up.
//! - Each consumer is split into partitions by session
//!   (`storage::repositories::feed`). A replica runs a partition only while
//!   it holds the partition's advisory lock on its lock connection; others
//!   retry every few seconds, so a stopped replica's partitions move within
//!   that time.
//! - Within a pass, sessions run concurrently and each session's events run
//!   one after another, every listener in registration order, each in its own
//!   task (a panic is contained) and bounded by [`LISTENER_TIMEOUT`].
//! - Delivery is at least once: a replica that stops mid-pass hands the pass
//!   over again. Listeners key their effects on the event (usage on
//!   `llm_generations.event_id`, budgets on the journal source, notifications
//!   on a dedupe key, observer scores on the turn, MCP events on the event id)
//!   or apply state rather than deltas (A2A push, coordination, sandbox).
//!   Exporters (OpenTelemetry, Braintrust, customer tracing, metrics) may
//!   repeat a span or a sample after a crash.
//! - Listeners see a session's events only after the pass reads them, not
//!   when the caller is answered. Callers that need a listener's effect wait
//!   for it explicitly ([`EventFeed::settled`]); nothing on the worker or API
//!   path does.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use everruns_core::{Event, EventListener};
use futures::StreamExt;
use sqlx::Connection;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::storage::StorageBackend;
use crate::storage::repositories::feed::{FEED_PARTITIONS, feed_partition, feed_try_lock};

/// A listener call that runs longer is abandoned (with an error log) so one
/// hung listener cannot hold its partition.
const LISTENER_TIMEOUT: Duration = Duration::from_secs(30);
/// How often a replica tries to take partitions it does not hold.
const LOCK_RETRY: Duration = Duration::from_secs(5);
/// A partition with nothing to deliver still moves its read point this often,
/// so its window does not grow with other consumers' event types.
const IDLE_ADVANCE: Duration = Duration::from_secs(5);
/// Sessions of one pass that run at the same time.
const SESSION_CONCURRENCY: usize = 32;

/// Listeners that run within a second of the commit. Anything not listed in
/// [`LATER`] or [`IN_PROCESS`] runs here too.
#[cfg_attr(not(test), allow(dead_code))]
const NEAR: &[&str] = &[
    "coordination_thread_turns",
    "BudgetService",
    "A2aPushListener",
    "NotificationEventListener",
    "McpEventsListener",
    "SessionSandboxEventListener",
];
/// Listeners that stay in-process (see the module decisions).
const IN_PROCESS: &[&str] = &[
    "OtelEventListener",
    "BraintrustListener",
    "PrometheusMetricsListener",
];
/// Listeners that run within seconds, in larger pages.
const LATER: &[&str] = &[
    "UsageTrackingListener",
    "CustomerTracing",
    "TurnLatencyListener",
    "automatic_session_titles",
    "run_summary",
    "ObserverMatchListener",
    "ApprovalAuditListener",
];

struct Consumer {
    name: &'static str,
    listeners: Vec<Arc<dyn EventListener>>,
    /// Every type a listener of this consumer wants.
    event_types: Vec<String>,
    poll: Duration,
    page: i64,
    /// Per partition: woken when this replica stores an event there.
    wakes: Vec<Notify>,
}

impl Consumer {
    fn new(
        name: &'static str,
        listeners: Vec<Arc<dyn EventListener>>,
        poll: Duration,
        page: i64,
    ) -> Option<Self> {
        if listeners.is_empty() {
            return None;
        }
        let mut event_types: Vec<String> = Vec::new();
        for listener in &listeners {
            match listener.event_types() {
                Some(types) => event_types.extend(types.iter().map(|t| t.to_string())),
                // A listener of every type: the feed still delivers stored
                // events only (ephemeral deltas never reach the database).
                None => {
                    event_types.clear();
                    break;
                }
            }
        }
        event_types.sort();
        event_types.dedup();
        Some(Self {
            name,
            listeners,
            event_types,
            poll,
            page,
            wakes: (0..FEED_PARTITIONS).map(|_| Notify::new()).collect(),
        })
    }

    fn wants(&self, listener: &dyn EventListener, event: &Event) -> bool {
        listener
            .event_types()
            .is_none_or(|types| types.contains(&event.event_type.as_str()))
    }
}

/// The listener feed of one server process. Cheap to clone.
#[derive(Clone)]
pub struct EventFeed {
    inner: Arc<Inner>,
}

struct Inner {
    db: Arc<StorageBackend>,
    consumers: Vec<Consumer>,
    /// `held[consumer][partition]`: this replica holds the partition.
    held: Vec<Vec<AtomicBool>>,
    started: OnceLock<()>,
    /// Callers of [`EventFeed::settled`] waiting: idle passes move their read
    /// point at once instead of every [`IDLE_ADVANCE`].
    settling: std::sync::atomic::AtomicUsize,
    stop: CancellationToken,
    tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

enum Pass {
    /// Another replica holds the partition.
    NotHeld,
    /// Nothing to deliver.
    Idle,
    Delivered {
        more: bool,
    },
}

impl EventFeed {
    /// The feed for `listeners` (`None` when none of them is fed from the
    /// database), and the listeners that stay in-process.
    pub fn split(
        db: Arc<StorageBackend>,
        listeners: &[Arc<dyn EventListener>],
    ) -> (Option<Self>, Vec<Arc<dyn EventListener>>) {
        let mut near = Vec::new();
        let mut later = Vec::new();
        let mut in_process = Vec::new();
        for listener in listeners {
            let name = listener.name();
            if IN_PROCESS.contains(&name) {
                in_process.push(listener.clone());
            } else if LATER.contains(&name) {
                later.push(listener.clone());
            } else {
                // NEAR, and anything not classified (an embedder's listener):
                // nearer is the safe default.
                near.push(listener.clone());
            }
        }
        (Self::new(db, near, later), in_process)
    }

    fn new(
        db: Arc<StorageBackend>,
        near: Vec<Arc<dyn EventListener>>,
        later: Vec<Arc<dyn EventListener>>,
    ) -> Option<Self> {
        let consumers: Vec<Consumer> = [
            Consumer::new("near", near, Duration::from_millis(250), 200),
            Consumer::new("later", later, Duration::from_secs(2), 1000),
        ]
        .into_iter()
        .flatten()
        .collect();
        if consumers.is_empty() {
            return None;
        }
        let held = consumers
            .iter()
            .map(|_| {
                (0..FEED_PARTITIONS)
                    .map(|_| AtomicBool::new(false))
                    .collect()
            })
            .collect();
        let feed = Self {
            inner: Arc::new(Inner {
                db,
                consumers,
                held,
                started: OnceLock::new(),
                settling: std::sync::atomic::AtomicUsize::new(0),
                stop: CancellationToken::new(),
                tasks: std::sync::Mutex::new(Vec::new()),
            }),
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            feed.start();
        }
        Some(feed)
    }

    /// Start the lock keeper and one loop per consumer and partition. Runs
    /// once; later calls do nothing.
    pub fn start(&self) {
        self.inner.started.get_or_init(|| {
            let mut tasks = self.inner.tasks.lock().unwrap();
            tasks.push(tokio::spawn(keep_locks(self.inner.clone())));
            for consumer in 0..self.inner.consumers.len() {
                for partition in 0..FEED_PARTITIONS {
                    tasks.push(tokio::spawn(run_partition(
                        self.inner.clone(),
                        consumer,
                        partition,
                    )));
                }
            }
        });
    }

    /// An event of `session_id` was stored: wake its near partition here.
    pub fn wake(&self, session_id: Uuid) {
        let partition = feed_partition(session_id) as usize;
        for consumer in &self.inner.consumers {
            if consumer.poll < Duration::from_secs(1) {
                consumer.wakes[partition].notify_one();
            }
        }
    }

    /// Wait until every event committed before this call reached its
    /// listeners, at most `limit`. For tests and callers that read a
    /// listener's effect. `false` at the limit.
    pub async fn settled(&self, limit: Duration) -> bool {
        self.inner.settling.fetch_add(1, Ordering::SeqCst);
        let settled = self.settle(limit).await;
        self.inner.settling.fetch_sub(1, Ordering::SeqCst);
        settled
    }

    async fn settle(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let target: i64 = match sqlx::query_scalar(
            "SELECT pg_snapshot_xmax(pg_current_snapshot())::text::bigint",
        )
        .fetch_one(self.inner.db.pool())
        .await
        {
            Ok(target) => target,
            Err(error) => {
                tracing::warn!(%error, "event feed: could not read the commit point");
                return false;
            }
        };
        for consumer in 0..self.inner.consumers.len() {
            for partition in 0..FEED_PARTITIONS {
                loop {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    let cursor = self
                        .inner
                        .db
                        .feed_cursor(self.inner.consumers[consumer].name, partition)
                        .await
                        .unwrap_or(0);
                    if cursor >= target {
                        break;
                    }
                    // Nudge the owner (here, or poll on another replica).
                    self.inner.consumers[consumer].wakes[partition as usize].notify_one();
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
        true
    }

    /// Stop every loop, letting a pass in progress finish, at most `limit`.
    pub async fn stop(&self, limit: Duration) -> bool {
        self.inner.stop.cancel();
        let tasks: Vec<_> = std::mem::take(&mut *self.inner.tasks.lock().unwrap());
        tokio::time::timeout(limit, futures::future::join_all(tasks))
            .await
            .is_ok()
    }
}

/// Holds this replica's partitions on one connection. Takes free partitions
/// every [`LOCK_RETRY`]; when the connection fails, lets every partition go
/// and starts over, so another replica can take them.
async fn keep_locks(inner: Arc<Inner>) {
    let options = inner.db.pool().connect_options();
    loop {
        if inner.stop.is_cancelled() {
            return;
        }
        let mut conn = match sqlx::PgConnection::connect_with(&options).await {
            Ok(conn) => conn,
            Err(error) => {
                tracing::warn!(%error, "event feed: lock connection failed");
                if sleep_or_stop(&inner.stop, LOCK_RETRY).await {
                    return;
                }
                continue;
            }
        };
        let lost = loop {
            let mut failed = false;
            for (index, consumer) in inner.consumers.iter().enumerate() {
                for partition in 0..FEED_PARTITIONS {
                    let held = &inner.held[index][partition as usize];
                    if held.load(Ordering::SeqCst) {
                        continue;
                    }
                    match feed_try_lock(&mut conn, consumer.name, partition).await {
                        Ok(true) => {
                            held.store(true, Ordering::SeqCst);
                            consumer.wakes[partition as usize].notify_one();
                        }
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "event feed: taking a partition failed");
                            failed = true;
                        }
                    }
                }
            }
            if failed || conn.ping().await.is_err() {
                break true;
            }
            if sleep_or_stop(&inner.stop, LOCK_RETRY).await {
                break false;
            }
        };
        for held in inner.held.iter().flatten() {
            held.store(false, Ordering::SeqCst);
        }
        // Closing the connection releases every partition it held.
        let _ = conn.close().await;
        if !lost {
            return;
        }
    }
}

/// `true` when stopped.
async fn sleep_or_stop(stop: &CancellationToken, duration: Duration) -> bool {
    tokio::select! {
        _ = stop.cancelled() => true,
        _ = tokio::time::sleep(duration) => false,
    }
}

async fn run_partition(inner: Arc<Inner>, consumer: usize, partition: u8) {
    let spec = &inner.consumers[consumer];
    let held = &inner.held[consumer][partition as usize];
    let mut cursor: Option<i64> = None;
    let mut last_advance = Instant::now();
    loop {
        if !held.load(Ordering::SeqCst) {
            cursor = None;
            tokio::select! {
                _ = inner.stop.cancelled() => return,
                _ = spec.wakes[partition as usize].notified() => {}
                _ = tokio::time::sleep(LOCK_RETRY) => {}
            }
            continue;
        }
        let outcome = pass(&inner, spec, partition, &mut cursor, &mut last_advance).await;
        let wait = match outcome {
            Ok(Pass::Delivered { more: true }) => continue,
            Ok(Pass::Delivered { more: false } | Pass::Idle | Pass::NotHeld) => spec.poll,
            Err(error) => {
                tracing::warn!(consumer = spec.name, partition, %error, "event feed pass failed");
                // Read the cursor again: a failed commit may have landed.
                cursor = None;
                Duration::from_secs(1)
            }
        };
        tokio::select! {
            _ = inner.stop.cancelled() => return,
            _ = spec.wakes[partition as usize].notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

async fn pass(
    inner: &Inner,
    spec: &Consumer,
    partition: u8,
    cursor: &mut Option<i64>,
    last_advance: &mut Instant,
) -> anyhow::Result<Pass> {
    let db = inner.db.database();
    let from = match *cursor {
        Some(cursor) => cursor,
        None => {
            let read = db.feed_cursor(spec.name, partition).await?;
            *cursor = Some(read);
            read
        }
    };
    let batch = db
        .feed_read(spec.name, partition, from, &spec.event_types, spec.page)
        .await?;
    if batch.sessions.is_empty() {
        if batch.next_cursor > from
            && (last_advance.elapsed() >= IDLE_ADVANCE || inner.settling.load(Ordering::SeqCst) > 0)
        {
            if !db
                .feed_commit(spec.name, partition, from, batch.next_cursor, &[])
                .await?
            {
                *cursor = None;
                return Ok(Pass::NotHeld);
            }
            *cursor = Some(batch.next_cursor);
            *last_advance = Instant::now();
        }
        return Ok(Pass::Idle);
    }

    let delivered: Vec<(Uuid, i32)> = futures::stream::iter(batch.sessions)
        .map(|(session, rows)| async move {
            let last = rows.last().map(|row| row.sequence).unwrap_or(0);
            for row in rows {
                let event = super::EventService::row_to_event(row);
                for listener in &spec.listeners {
                    if spec.wants(listener.as_ref(), &event) {
                        run_listener(listener.clone(), &event).await;
                    }
                }
            }
            (session, last)
        })
        .buffer_unordered(SESSION_CONCURRENCY)
        .collect()
        .await;

    if !db
        .feed_commit(spec.name, partition, from, batch.next_cursor, &delivered)
        .await?
    {
        tracing::warn!(
            consumer = spec.name,
            partition,
            "event feed: cursor moved under a pass; another replica held the partition"
        );
        *cursor = None;
        return Ok(Pass::NotHeld);
    }
    *cursor = Some(batch.next_cursor);
    *last_advance = Instant::now();
    Ok(Pass::Delivered { more: batch.more })
}

async fn run_listener(listener: Arc<dyn EventListener>, event: &Event) {
    let name = listener.name();
    let event = Arc::new(event.clone());
    let handle = tokio::spawn(async move { listener.on_event(&event).await });
    match tokio::time::timeout(LISTENER_TIMEOUT, handle).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::error!(listener = name, %error, "EventListener panicked or was cancelled");
        }
        Err(_) => {
            tracing::error!(
                listener = name,
                timeout_s = LISTENER_TIMEOUT.as_secs(),
                "EventListener timed out; the feed moved on"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_listener_has_one_place() {
        for name in NEAR.iter().chain(LATER).chain(IN_PROCESS) {
            let places = [NEAR, LATER, IN_PROCESS]
                .iter()
                .filter(|list| list.contains(name))
                .count();
            assert_eq!(places, 1, "{name} is listed {places} times");
        }
    }
}
