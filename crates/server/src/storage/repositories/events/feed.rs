//! The listener feed's reads and writes: committed events of one partition,
//! handed to a consumer in order, at least once.
//!
//! Decisions (`knowledge/execution/event-listeners.md`):
//! - Every event row records the id of the transaction that wrote it
//!   (`events.xact_id`, defaulted to `pg_current_xact_id()`). A pass reads the
//!   events between its cursor and the oldest transaction still running
//!   (`pg_snapshot_xmin`), in one statement so both come from one snapshot.
//!   Every transaction below that point has committed or rolled back, so a
//!   late commit is read on a later pass, never skipped.
//! - A session's events can commit out of transaction id order: a
//!   transaction that took its id earlier can wait for a session's sequence
//!   lock held by a later one. The feed keeps each consumer's last delivered
//!   sequence per session, delivers a session's committed lower-sequence
//!   events that sit above the window together with the window (in sequence
//!   order), and skips them when the read point reaches them. So a consumer
//!   sees a session's events in sequence order.
//! - One replica runs a partition at a time: it holds the partition's
//!   session-level advisory lock on one connection kept for that purpose
//!   (`services::event::feed`), so passes need no open transaction while the
//!   consumer runs and borrow pool connections only per statement.
//! - The cursor advances in the same transaction that records the delivered
//!   sequences, after the consumer ran, and only from the value the pass
//!   started at: a crash in between hands the same events over again (at
//!   least once), never skips them, and two passes that overlapped (a lock
//!   connection lost) cannot both move it.

use anyhow::Result;
use uuid::Uuid;

use super::Database;
use super::rows::EventRow;
use crate::kernel_imports::contracts::typed_id::{EventId, SessionId};

/// Partitions per consumer. Part of `idx_events_feed`'s expression: changing
/// it needs a new index.
pub(crate) const FEED_PARTITIONS: u8 = 8;

/// A session's partition, matching `idx_events_feed`'s expression
/// `get_byte(uuid_send(session_id), 15) % 8`.
pub(crate) fn feed_partition(session_id: Uuid) -> u8 {
    session_id.as_bytes()[15] % FEED_PARTITIONS
}

/// The event columns of `e`, as a literal for `concat!`.
macro_rules! event_columns {
    () => {
        "e.id, e.session_id, e.sequence, e.event_type, e.ts, e.context, \
         e.data, e.metadata, e.tags, e.created_at"
    };
}

/// What a pass hands to its consumer.
pub(crate) struct FeedBatch {
    /// Each session's undelivered events, in sequence order. Sessions are in
    /// the order their first event was read.
    pub sessions: Vec<(Uuid, Vec<EventRow>)>,
    /// The read point after this batch.
    pub next_cursor: i64,
    /// Whether events of the window were left for the next pass.
    pub more: bool,
}

#[derive(sqlx::FromRow)]
struct WindowRow {
    hi: i64,
    xact: Option<i64>,
    #[sqlx(flatten)]
    event: OptionalEventRow,
}

#[derive(sqlx::FromRow)]
struct OptionalEventRow {
    id: Option<EventId>,
    session_id: Option<SessionId>,
    sequence: Option<i32>,
    event_type: Option<String>,
    ts: Option<chrono::DateTime<chrono::Utc>>,
    context: Option<serde_json::Value>,
    data: Option<serde_json::Value>,
    metadata: Option<serde_json::Value>,
    tags: Option<Vec<String>>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl OptionalEventRow {
    fn into_row(self) -> Option<EventRow> {
        Some(EventRow {
            id: self.id?,
            session_id: self.session_id?,
            sequence: self.sequence?,
            event_type: self.event_type?,
            ts: self.ts?,
            context: self.context?,
            data: self.data?,
            metadata: self.metadata,
            tags: self.tags,
            created_at: self.created_at?,
        })
    }
}

/// Try to take `consumer`'s `partition` on the lock connection. Held until
/// released or the connection closes.
pub(crate) async fn feed_try_lock(
    conn: &mut sqlx::PgConnection,
    consumer: &str,
    partition: u8,
) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT pg_try_advisory_lock(hashtextextended('event-feed:' || $1 || ':' || $2::text, 0))",
    )
    .bind(consumer)
    .bind(partition as i16)
    .fetch_one(conn)
    .await?)
}

impl Database {
    /// Where `consumer` reads `partition` next. A consumer without a cursor
    /// starts at the beginning: rows written before the feed existed have no
    /// transaction id and are never read.
    pub(crate) async fn feed_cursor(&self, consumer: &str, partition: u8) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT next_xact_id::text::bigint FROM event_feed_cursors \
             WHERE consumer = $1 AND partition = $2",
        )
        .bind(consumer)
        .bind(partition as i16)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0))
    }

    /// Which of `partitions` (with their cursors) have a committed event of
    /// `event_types` (every type when empty) to deliver. One statement for
    /// all of a replica's partitions, so an idle replica costs one cheap read
    /// per consumer per poll instead of one per partition.
    pub(crate) async fn feed_probe(
        &self,
        partitions: &[(u8, i64)],
        event_types: &[String],
    ) -> Result<Vec<u8>> {
        let parts: Vec<i32> = partitions.iter().map(|(p, _)| *p as i32).collect();
        let cursors: Vec<String> = partitions.iter().map(|(_, c)| c.to_string()).collect();
        let ready: Vec<i32> = sqlx::query_scalar(
            "WITH point AS MATERIALIZED ( \
                 SELECT pg_snapshot_xmin(pg_current_snapshot()) AS hi \
             ) \
             SELECT p.part FROM point, UNNEST($1::int[], $2::text[]) AS p(part, cur) \
             WHERE EXISTS ( \
                 SELECT 1 FROM events ev \
                 WHERE (get_byte(uuid_send(ev.session_id), 15) % 8) = p.part \
                   AND ev.xact_id IS NOT NULL \
                   AND ev.xact_id >= p.cur::xid8 \
                   AND ev.xact_id < point.hi \
                   AND (cardinality($3::text[]) = 0 OR ev.event_type = ANY($3)) \
             )",
        )
        .bind(&parts)
        .bind(&cursors)
        .bind(event_types)
        .fetch_all(&self.pool)
        .await?;
        Ok(ready.into_iter().map(|p| p as u8).collect())
    }

    /// Read up to `limit` committed events of `event_types` (every type when
    /// empty) from `partition` at `cursor`, plus the out-of-order ones their
    /// sessions need first.
    pub(crate) async fn feed_read(
        &self,
        consumer: &str,
        partition: u8,
        cursor: i64,
        event_types: &[String],
        limit: i64,
    ) -> Result<FeedBatch> {
        let window: Vec<WindowRow> = sqlx::query_as(concat!(
            "WITH point AS MATERIALIZED ( \
                 SELECT pg_snapshot_xmin(pg_current_snapshot()) AS hi \
             ) \
             SELECT p.hi::text::bigint AS hi, e.xact_id::text::bigint AS xact, ",
            event_columns!(),
            " \
             FROM point p \
             LEFT JOIN LATERAL ( \
                 SELECT ev.id, ev.session_id, ev.sequence, ev.event_type, ev.ts, ev.context, \
                        ev.data, ev.metadata, ev.tags, ev.created_at, ev.xact_id \
                 FROM events ev \
                 WHERE (get_byte(uuid_send(ev.session_id), 15) % 8) = $1 \
                   AND ev.xact_id IS NOT NULL \
                   AND ev.xact_id >= $2::text::xid8 \
                   AND ev.xact_id < p.hi \
                   AND (cardinality($3::text[]) = 0 OR ev.event_type = ANY($3))  \
                 ORDER BY ev.xact_id, ev.session_id, ev.sequence \
                 LIMIT $4 \
             ) e ON true",
        ))
        .bind(partition as i32)
        .bind(cursor.to_string())
        .bind(event_types)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let hi = window.first().map(|row| row.hi).unwrap_or(cursor);
        let mut rows: Vec<(i64, EventRow)> = window
            .into_iter()
            .filter_map(|row| Some((row.xact?, row.event.into_row()?)))
            .collect();

        // A full page may end inside a transaction: leave that transaction
        // for the next pass, unless it is the only one on the page.
        let mut more = false;
        let mut next_cursor = hi.max(cursor);
        if rows.len() as i64 >= limit
            && let Some(&(last, _)) = rows.last()
        {
            more = true;
            if rows.first().is_some_and(|(first, _)| *first == last) {
                // One transaction larger than a page: take all of it.
                rows = self.feed_transaction(partition, last, event_types).await?;
                next_cursor = last + 1;
            } else {
                rows.retain(|(xact, _)| *xact < last);
                next_cursor = last;
            }
        }

        let mut order: Vec<Uuid> = Vec::new();
        let mut by_session: std::collections::HashMap<Uuid, Vec<EventRow>> =
            std::collections::HashMap::new();
        for (_, row) in rows {
            let session = row.session_id.uuid();
            by_session
                .entry(session)
                .or_insert_with(|| {
                    order.push(session);
                    Vec::new()
                })
                .push(row);
        }
        if order.is_empty() {
            return Ok(FeedBatch {
                sessions: Vec::new(),
                next_cursor,
                more,
            });
        }

        // What each session already handed over.
        let delivered: std::collections::HashMap<Uuid, i32> = sqlx::query_as::<_, (Uuid, i32)>(
            "SELECT session_id, delivered_sequence FROM event_feed_sessions \
             WHERE consumer = $1 AND session_id = ANY($2)",
        )
        .bind(consumer)
        .bind(&order)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .collect();

        // Committed events above the window that come before a windowed one
        // in their session's order.
        let mut after = Vec::with_capacity(order.len());
        let mut before = Vec::with_capacity(order.len());
        for session in &order {
            after.push(delivered.get(session).copied().unwrap_or(0));
            before.push(
                by_session[session]
                    .iter()
                    .map(|row| row.sequence)
                    .max()
                    .unwrap_or(0),
            );
        }
        let early: Vec<OptionalEventRow> = sqlx::query_as(concat!(
            "SELECT ",
            event_columns!(),
            " FROM events e \
             JOIN UNNEST($1::uuid[], $2::int[], $3::int[]) AS s(session_id, after_seq, before_seq) \
               ON e.session_id = s.session_id \
              AND e.sequence > s.after_seq AND e.sequence < s.before_seq \
             WHERE e.xact_id >= $4::text::xid8 \
               AND (cardinality($5::text[]) = 0 OR e.event_type = ANY($5))",
        ))
        .bind(&order)
        .bind(&after)
        .bind(&before)
        .bind(next_cursor.to_string())
        .bind(event_types)
        .fetch_all(&self.pool)
        .await?;
        for row in early.into_iter().filter_map(OptionalEventRow::into_row) {
            if let Some(events) = by_session.get_mut(&row.session_id.uuid()) {
                events.push(row);
            }
        }

        let sessions = order
            .into_iter()
            .filter_map(|session| {
                let floor = delivered.get(&session).copied().unwrap_or(0);
                let mut events = by_session.remove(&session)?;
                events.retain(|row| row.sequence > floor);
                events.sort_by_key(|row| row.sequence);
                events.dedup_by_key(|row| row.sequence);
                (!events.is_empty()).then_some((session, events))
            })
            .collect();
        Ok(FeedBatch {
            sessions,
            next_cursor,
            more,
        })
    }

    async fn feed_transaction(
        &self,
        partition: u8,
        xact: i64,
        event_types: &[String],
    ) -> Result<Vec<(i64, EventRow)>> {
        let rows: Vec<OptionalEventRow> = sqlx::query_as(concat!(
            "SELECT ",
            event_columns!(),
            " FROM events e \
             WHERE (get_byte(uuid_send(e.session_id), 15) % 8) = $1 \
               AND e.xact_id IS NOT NULL AND e.xact_id = $2::text::xid8 \
               AND (cardinality($3::text[]) = 0 OR e.event_type = ANY($3)) \
             ORDER BY e.session_id, e.sequence",
        ))
        .bind(partition as i32)
        .bind(xact.to_string())
        .bind(event_types)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| Some((xact, row.into_row()?)))
            .collect())
    }

    /// Record what was handed over and move `partition`'s read point from
    /// `from` to `to`. `false` when the cursor was no longer at `from` (an
    /// overlapping pass moved it): nothing is recorded.
    pub(crate) async fn feed_commit(
        &self,
        consumer: &str,
        partition: u8,
        from: i64,
        to: i64,
        delivered: &[(Uuid, i32)],
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        if !delivered.is_empty() {
            let (sessions, sequences): (Vec<Uuid>, Vec<i32>) = delivered.iter().copied().unzip();
            sqlx::query(
                "INSERT INTO event_feed_sessions (consumer, session_id, delivered_sequence) \
                 SELECT $1, s, q FROM UNNEST($2::uuid[], $3::int[]) AS t(s, q) ORDER BY s \
                 ON CONFLICT (consumer, session_id) DO UPDATE \
                 SET delivered_sequence = GREATEST(event_feed_sessions.delivered_sequence, \
                                                   EXCLUDED.delivered_sequence), \
                     updated_at = now()",
            )
            .bind(consumer)
            .bind(&sessions)
            .bind(&sequences)
            .execute(&mut *tx)
            .await?;
        }
        let moved = sqlx::query(
            "INSERT INTO event_feed_cursors (consumer, partition, next_xact_id) \
             VALUES ($1, $2, $3::text::xid8) \
             ON CONFLICT (consumer, partition) DO UPDATE \
             SET next_xact_id = EXCLUDED.next_xact_id, updated_at = now() \
             WHERE event_feed_cursors.next_xact_id = $4::text::xid8",
        )
        .bind(consumer)
        .bind(partition as i16)
        .bind(to.to_string())
        .bind(from.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if moved == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use sqlx::Connection;

    const TYPES: &[&str] = &["turn.started"];

    fn types() -> Vec<String> {
        TYPES.iter().map(|t| t.to_string()).collect()
    }

    async fn insert(conn: &mut sqlx::PgConnection, session: Uuid, sequence: i32) {
        sqlx::query(
            "INSERT INTO events (session_id, sequence, event_type) VALUES ($1, $2, 'turn.started')",
        )
        .bind(session)
        .bind(sequence)
        .execute(conn)
        .await
        .unwrap();
    }

    async fn connect(db: &StorageBackend) -> sqlx::PgConnection {
        sqlx::PgConnection::connect_with(&db.pool().connect_options())
            .await
            .unwrap()
    }

    /// One pass for `session`'s partition: what it hands over, in order.
    /// Records the delivery like a consumer that ran.
    async fn pass(db: &StorageBackend, session: Uuid) -> Vec<i32> {
        let partition = feed_partition(session);
        let cursor = db.feed_cursor("test", partition).await.unwrap();
        let batch = db
            .feed_read("test", partition, cursor, &types(), 100)
            .await
            .unwrap();
        let mut seen = Vec::new();
        let mut delivered = Vec::new();
        for (id, rows) in batch.sessions {
            if id == session {
                seen.extend(rows.iter().map(|row| row.sequence));
            }
            delivered.push((id, rows.last().unwrap().sequence));
        }
        assert!(
            db.feed_commit("test", partition, cursor, batch.next_cursor, &delivered)
                .await
                .unwrap()
        );
        seen
    }

    /// Passes until `want` events of `session` came through. Other tests'
    /// transactions share the read point (it is cluster-wide), so a pass may
    /// briefly hand over nothing.
    async fn passes(db: &StorageBackend, session: Uuid, want: usize) -> Vec<i32> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut seen = Vec::new();
        while seen.len() < want && std::time::Instant::now() < deadline {
            seen.extend(pass(db, session).await);
            if seen.len() < want {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
        seen
    }

    #[tokio::test]
    async fn a_late_commit_is_read_on_a_later_pass_never_skipped() {
        let db = StorageBackend::test_database();
        let session = db.create_test_session().await.uuid();
        let mut slow = connect(&db).await;
        let mut fast = connect(&db).await;

        // The first transaction takes its id, then the second commits first.
        sqlx::query("BEGIN").execute(&mut slow).await.unwrap();
        insert(&mut slow, session, 1).await;
        insert(&mut fast, session, 2).await;
        assert_eq!(
            pass(&db, session).await,
            Vec::<i32>::new(),
            "nothing past a transaction still running"
        );

        sqlx::query("COMMIT").execute(&mut slow).await.unwrap();
        assert_eq!(passes(&db, session, 2).await, vec![1, 2]);
        assert_eq!(pass(&db, session).await, Vec::<i32>::new(), "once");
    }

    #[tokio::test]
    async fn a_session_stays_in_order_when_an_earlier_transaction_commits_later() {
        let db = StorageBackend::test_database();
        let session = db.create_test_session().await.uuid();
        let other = db.create_test_session().await.uuid();
        let mut early = connect(&db).await;
        let mut holder = connect(&db).await;
        let mut late = connect(&db).await;

        // `early` takes the smallest id, `holder` the next and stays open,
        // `late` the largest.
        sqlx::query("BEGIN").execute(&mut early).await.unwrap();
        insert(&mut early, other, 1).await;
        sqlx::query("BEGIN").execute(&mut holder).await.unwrap();
        sqlx::query("SELECT pg_current_xact_id()")
            .execute(&mut holder)
            .await
            .unwrap();
        // `late` writes the session's first event and commits; `early` its
        // second. By transaction id, the second comes first.
        insert(&mut late, session, 1).await;
        insert(&mut early, session, 2).await;
        sqlx::query("COMMIT").execute(&mut early).await.unwrap();

        // The window ends at `holder` at the latest: it holds `early`'s event
        // only, and the pass brings `late`'s committed, earlier event along,
        // first.
        assert_eq!(passes(&db, session, 2).await, vec![1, 2]);

        // Once the window reaches `late`'s event, it is not handed over again.
        sqlx::query("COMMIT").execute(&mut holder).await.unwrap();
        assert_eq!(pass(&db, session).await, Vec::<i32>::new());
    }

    #[tokio::test]
    async fn an_unrecorded_pass_is_handed_over_again() {
        let db = StorageBackend::test_database();
        let session = db.create_test_session().await.uuid();
        let mut conn = connect(&db).await;
        insert(&mut conn, session, 1).await;
        let partition = feed_partition(session);

        // A replica read the event and stopped before recording it.
        db.feed_read("test", partition, 0, &types(), 100)
            .await
            .unwrap();

        assert_eq!(passes(&db, session, 1).await, vec![1]);
    }

    #[tokio::test]
    async fn a_moved_cursor_refuses_the_older_pass() {
        let db = StorageBackend::test_database();
        let session = db.create_test_session().await.uuid();
        let mut conn = connect(&db).await;
        insert(&mut conn, session, 1).await;
        let partition = feed_partition(session);
        let batch = db
            .feed_read("test", partition, 0, &types(), 100)
            .await
            .unwrap();
        assert!(
            db.feed_commit("test", partition, 0, batch.next_cursor, &[(session, 1)])
                .await
                .unwrap()
        );
        // An overlapping pass that started at the same cursor.
        assert!(
            !db.feed_commit("test", partition, 0, batch.next_cursor, &[(session, 1)])
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn a_probe_names_only_partitions_with_something_to_deliver() {
        let db = StorageBackend::test_database();
        let session = db.create_test_session().await.uuid();
        let partition = feed_partition(session);
        let other = (partition + 1) % FEED_PARTITIONS;
        let mut conn = connect(&db).await;
        insert(&mut conn, session, 1).await;
        let cursor = db.feed_cursor("test", partition).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let ready = db
                .feed_probe(&[(partition, cursor), (other, i64::MAX / 2)], &types())
                .await
                .unwrap();
            assert!(!ready.contains(&other), "nothing above the other cursor");
            if ready == vec![partition] {
                break;
            }
            // The read point is cluster-wide: other tests' transactions can
            // hold it back for a moment.
            assert!(std::time::Instant::now() < deadline, "never reported");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let ready = db
            .feed_probe(&[(partition, cursor)], &["no.such.type".to_string()])
            .await
            .unwrap();
        assert!(ready.is_empty(), "other event types do not count");
    }

    #[tokio::test]
    async fn one_lock_connection_holds_a_partition() {
        let db = StorageBackend::test_database();
        let mut first = connect(&db).await;
        let mut second = connect(&db).await;
        assert!(feed_try_lock(&mut first, "test", 3).await.unwrap());
        assert!(!feed_try_lock(&mut second, "test", 3).await.unwrap());
        assert!(feed_try_lock(&mut second, "test", 4).await.unwrap());
        // Closing the connection lets the partition go, once the server
        // process behind it has exited, which follows the close a moment
        // later.
        first.close().await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !feed_try_lock(&mut second, "test", 3).await.unwrap() {
            assert!(
                std::time::Instant::now() < deadline,
                "the closed connection's lock was never released"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}
