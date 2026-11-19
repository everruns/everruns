//! A session's events, wherever the session runs: the one reader control
//! plane features use (data plane v2 parity, Phase A).
//!
//! Decisions:
//! - A v1 session's events are the control plane's `events` table. A v2
//!   session's events live in its cell; the control plane reads them over
//!   the link (`SessionPlane`). A feature calls [`list`] or [`advanced`]
//!   with the arguments it gave `StorageBackend::list_events` or
//!   `list_events_advanced` and gets the same rows for either plane, so no
//!   feature branches on the plane and a new reader sees v2 sessions by
//!   default. `scripts/lib/check-session-events-port.sh` keeps direct table
//!   reads to the modules that are v1-only by nature.
//! - The plane is decided per call from the session's org (identity cache)
//!   and the data plane's own record of the session; both are cached, so a
//!   v1 session pays two cache hits.
//! - The rows match the table's queries, quirks included: `list_events`
//!   with a `limit` keeps the newest `limit` before `before_sequence` and
//!   ignores the forward cursors; without one it reads forward from the
//!   cursor, at most 10 000 rows, and ignores `before_sequence`.
//! - The cell answers in pages of at most 1000 committed events in their
//!   public form, without deltas, filtered by sequence range and type.
//!   Every other filter (excluded types, context ids, tool name, time,
//!   search) is applied here over the pages. An event id cursor is found by
//!   reading the log from its start, so it costs a scan on a long session;
//!   features that page should prefer sequences.
//! - A v2 row has no metadata or tags: a tag filter matches nothing.
//! - [`wait_for`] polls this reader, so the features that wait for a turn
//!   to end (send and wait, health checks) wait the same way on both
//!   planes.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use everruns_contracts::dataplane::SessionEventRow;
use everruns_contracts::typed_id::{EventId, SessionId};

use crate::domains::data_plane::session_plane::{MAX_LOG, SessionPlane};
use crate::storage::{EventRow, ListEventsParams, StorageBackend};

/// Events the cell returns in one `events` read.
const PAGE: usize = 1_000;
/// Rows `list_events` returns at most without a `limit` (the table's
/// forward safety limit).
const FORWARD_LIMIT: usize = 10_000;

/// Where one session's events are.
pub enum SessionEvents {
    Platform(Arc<StorageBackend>, SessionId),
    DataPlane(Box<SessionPlane>),
}

impl SessionEvents {
    /// `session`'s events, on the plane that runs it.
    pub async fn of(db: &Arc<StorageBackend>, session: SessionId) -> Result<Self> {
        if let Some(org_id) = db.session_org_id(session).await?
            && let Some(plane) = SessionPlane::of(db, org_id, session).await?
        {
            return Ok(Self::DataPlane(Box::new(plane)));
        }
        Ok(Self::Platform(db.clone(), session))
    }

    /// Whether the session runs on a data plane.
    pub fn on_data_plane(&self) -> bool {
        matches!(self, Self::DataPlane(_))
    }

    /// The session's events, as `StorageBackend::list_events` takes and
    /// returns them.
    #[allow(clippy::too_many_arguments)]
    pub async fn list(
        &self,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        filter_types: &[String],
        exclude_types: &[String],
        before_sequence: Option<i32>,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        match self {
            Self::Platform(db, session) => {
                db.list_events(
                    *session,
                    since_sequence,
                    since_id,
                    filter_types,
                    exclude_types,
                    before_sequence,
                    limit,
                )
                .await
            }
            Self::DataPlane(plane) => {
                let session = plane.session;
                let params = match limit {
                    Some(limit) => ListEventsParams {
                        session_id: session,
                        before_sequence,
                        filter_types: filter_types.to_vec(),
                        exclude_types: exclude_types.to_vec(),
                        order_desc: true,
                        limit: Some(limit),
                        ..Default::default()
                    },
                    None => ListEventsParams {
                        session_id: session,
                        // The table resolves an id cursor and then ignores
                        // the sequence one.
                        since_id,
                        after_sequence: since_id.is_none().then_some(since_sequence).flatten(),
                        filter_types: filter_types.to_vec(),
                        exclude_types: exclude_types.to_vec(),
                        ..Default::default()
                    },
                };
                let cap = match limit {
                    Some(limit) => usize::try_from(limit).unwrap_or(0),
                    None => FORWARD_LIMIT,
                };
                let mut rows = select(&**plane, &params, cap).await?;
                // `list_events` answers oldest first either way.
                rows.sort_by_key(|r| r.sequence);
                Ok(rows)
            }
        }
    }

    /// The session's events, as `StorageBackend::list_events_advanced`
    /// takes and returns them.
    pub async fn advanced(&self, params: &ListEventsParams) -> Result<Vec<EventRow>> {
        match self {
            Self::Platform(db, _) => db.list_events_advanced(params).await,
            Self::DataPlane(plane) => {
                let cap = match params.around_id {
                    Some(_) => usize::MAX,
                    None => usize::try_from(params.limit.unwrap_or(1_000).clamp(1, 10_000))
                        .unwrap_or(1_000),
                };
                select(&**plane, params, cap).await
            }
        }
    }
}

/// `session`'s events on whichever plane runs it: `StorageBackend::list_events`
/// for any session.
#[allow(clippy::too_many_arguments)]
pub async fn list(
    db: &Arc<StorageBackend>,
    session: SessionId,
    since_sequence: Option<i32>,
    since_id: Option<EventId>,
    filter_types: &[String],
    exclude_types: &[String],
    before_sequence: Option<i32>,
    limit: Option<i32>,
) -> Result<Vec<EventRow>> {
    SessionEvents::of(db, session)
        .await?
        .list(
            since_sequence,
            since_id,
            filter_types,
            exclude_types,
            before_sequence,
            limit,
        )
        .await
}

/// `StorageBackend::list_events_advanced` for any session.
pub async fn advanced(
    db: &Arc<StorageBackend>,
    params: &ListEventsParams,
) -> Result<Vec<EventRow>> {
    SessionEvents::of(db, params.session_id)
        .await?
        .advanced(params)
        .await
}

/// The first of `terminal` event types after `after`, polling every
/// `every` until `timeout`; `None` when the time runs out.
pub async fn wait_for(
    db: &Arc<StorageBackend>,
    session: SessionId,
    after: i32,
    terminal: &[String],
    every: Duration,
    timeout: Duration,
) -> Result<Option<EventRow>> {
    let events = SessionEvents::of(db, session).await?;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let found = events
            .list(Some(after), None, terminal, &[], None, None)
            .await?;
        if let Some(first) = found.into_iter().next() {
            return Ok(Some(first));
        }
        if tokio::time::Instant::now() + every > deadline {
            return Ok(None);
        }
        tokio::time::sleep(every).await;
    }
}

/// A session's committed events, a page at a time: the cell's `events`
/// read (a fake in tests).
trait Pages {
    fn session(&self) -> SessionId;
    /// Events in `from..=to` of `types` (all when empty), the oldest
    /// [`PAGE`] of them, or the newest `last`.
    async fn page(
        &self,
        from: i32,
        to: i32,
        types: &[&str],
        last: Option<usize>,
    ) -> Result<Vec<SessionEventRow>>;
}

impl Pages for SessionPlane {
    fn session(&self) -> SessionId {
        self.session
    }

    async fn page(
        &self,
        from: i32,
        to: i32,
        types: &[&str],
        last: Option<usize>,
    ) -> Result<Vec<SessionEventRow>> {
        self.events(from, to, types, last).await
    }
}

/// The rows `params` selects, at most `cap`: the oldest, or the newest
/// (newest first) under `order_desc`.
async fn select(
    pages: &impl Pages,
    params: &ListEventsParams,
    cap: usize,
) -> Result<Vec<EventRow>> {
    if cap == 0 {
        return Ok(Vec::new());
    }
    let session = pages.session();
    let mut from = params
        .after_sequence
        .map_or(1, |s| s.saturating_add(1))
        .max(1);
    let mut to = params
        .before_sequence
        .map_or(i32::MAX, |b| b.saturating_sub(1));
    let mut cap = cap;
    let mut newest = params.order_desc;
    if let Some(id) = params.since_id {
        // `sequence > (the cursor's)`: no rows when the cursor is unknown.
        let Some(at) = anchor(pages, id).await? else {
            return Ok(Vec::new());
        };
        from = from.max(at.saturating_add(1));
    }
    if let Some(id) = params.around_id {
        let Some(at) = anchor(pages, id).await? else {
            return Ok(Vec::new());
        };
        let window = params.window.unwrap_or(50).clamp(1, 500);
        from = at.saturating_sub(window).max(1);
        to = at.saturating_add(window);
        cap = usize::try_from(window).unwrap_or(50) * 2 + 1;
        newest = false;
    }
    if from > to {
        return Ok(Vec::new());
    }
    let types: Vec<&str> = params.filter_types.iter().map(String::as_str).collect();
    if newest && !filters_here(params) {
        let mut rows: Vec<EventRow> = pages
            .page(from, to, &types, Some(cap))
            .await?
            .into_iter()
            .map(|r| row(session, r))
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.sequence));
        return Ok(rows);
    }
    let mut kept: VecDeque<EventRow> = VecDeque::new();
    let mut read = 0;
    let mut next = from;
    loop {
        let page = pages.page(next, to, &types, None).await?;
        let short = page.len() < PAGE;
        read += page.len();
        if let Some(last) = page.last() {
            next = last.sequence.saturating_add(1);
        }
        for r in page {
            let r = row(session, r);
            if !matches(params, &r) {
                continue;
            }
            kept.push_back(r);
            if kept.len() > cap {
                // Oldest first: the cap is reached. Newest first: drop the
                // oldest kept.
                if newest {
                    kept.pop_front();
                } else {
                    kept.pop_back();
                    return Ok(kept.into());
                }
            }
        }
        if short || read >= MAX_LOG || next > to {
            break;
        }
    }
    let mut rows: Vec<EventRow> = kept.into();
    if newest {
        rows.reverse();
    }
    Ok(rows)
}

/// The sequence of event `id`, read from the log's start.
async fn anchor(pages: &impl Pages, id: EventId) -> Result<Option<i32>> {
    let wanted = id.to_string();
    let mut next = 1;
    let mut read = 0;
    loop {
        let page = pages.page(next, i32::MAX, &[], None).await?;
        if let Some(found) = page.iter().find(|r| r.id == wanted) {
            return Ok(Some(found.sequence));
        }
        read += page.len();
        match page.last() {
            Some(last) if page.len() >= PAGE && read < MAX_LOG => {
                next = last.sequence.saturating_add(1);
            }
            _ => return Ok(None),
        }
    }
}

/// Whether `params` has filters the cell's read lacks.
fn filters_here(params: &ListEventsParams) -> bool {
    !params.exclude_types.is_empty()
        || params.from_ts.is_some()
        || params.to_ts.is_some()
        || params.turn_id.is_some()
        || params.exec_id.is_some()
        || params.trace_id.is_some()
        || !params.tags.is_empty()
        || params.tool_name.is_some()
        || params.q.is_some()
}

/// The table's filters other than sequence range and type.
fn matches(params: &ListEventsParams, r: &EventRow) -> bool {
    let context = |key: &str, want: &Option<String>| {
        want.as_ref()
            .is_none_or(|want| r.context[key].as_str() == Some(want.as_str()))
    };
    !params.exclude_types.contains(&r.event_type)
        && params.from_ts.is_none_or(|from| r.created_at >= from)
        && params.to_ts.is_none_or(|to| r.created_at <= to)
        && context("turn_id", &params.turn_id)
        && context("exec_id", &params.exec_id)
        && context("trace_id", &params.trace_id)
        // A v2 row carries no tags.
        && params.tags.is_empty()
        && params
            .tool_name
            .as_ref()
            .is_none_or(|name| r.data["tool_name"].as_str() == Some(name.as_str()))
        && params.q.as_ref().is_none_or(|q| {
            r.data
                .to_string()
                .to_lowercase()
                .contains(&q.to_lowercase())
        })
}

/// A cell's event row in the table's shape.
fn row(session: SessionId, r: SessionEventRow) -> EventRow {
    EventRow {
        id: r.id.parse().unwrap_or_default(),
        session_id: session,
        sequence: r.sequence,
        event_type: r.event_type,
        ts: r.ts,
        context: r.context,
        data: r.data,
        metadata: None,
        tags: None,
        created_at: r.ts,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A session's log, served a page at a time as the cell does.
    struct Log {
        session: SessionId,
        rows: Vec<SessionEventRow>,
    }

    impl Pages for Log {
        fn session(&self) -> SessionId {
            self.session
        }

        async fn page(
            &self,
            from: i32,
            to: i32,
            types: &[&str],
            last: Option<usize>,
        ) -> Result<Vec<SessionEventRow>> {
            let chosen: Vec<SessionEventRow> = self
                .rows
                .iter()
                .filter(|r| (from..=to).contains(&r.sequence))
                .filter(|r| types.is_empty() || types.contains(&r.event_type.as_str()))
                .cloned()
                .collect();
            let skip = last.map_or(0, |l| chosen.len().saturating_sub(l));
            Ok(chosen.into_iter().skip(skip).take(PAGE).collect())
        }
    }

    fn log(n: i32, kind: impl Fn(i32) -> &'static str) -> Log {
        Log {
            session: SessionId::new(),
            rows: (1..=n)
                .map(|sequence| SessionEventRow {
                    id: EventId::new().to_string(),
                    sequence,
                    event_type: kind(sequence).into(),
                    ts: chrono::Utc::now(),
                    context: json!({ "turn_id": format!("turn_{}", (sequence - 1) / 10) }),
                    data: json!({ "n": sequence, "tool_name": "bash" }),
                })
                .collect(),
        }
    }

    fn sequences(rows: &[EventRow]) -> Vec<i32> {
        rows.iter().map(|r| r.sequence).collect()
    }

    #[tokio::test]
    async fn a_cells_rows_read_as_the_tables() {
        let log = log(3, |_| "turn.completed");
        let params = ListEventsParams {
            session_id: log.session,
            ..Default::default()
        };
        let rows = select(&log, &params, 10).await.unwrap();
        assert_eq!(sequences(&rows), vec![1, 2, 3]);
        assert_eq!(rows[2].id.to_string(), log.rows[2].id);
        assert_eq!(rows[2].session_id, log.session);
        assert_eq!(rows[2].context["turn_id"], "turn_0");
        assert_eq!(rows[2].data["n"], 3);
    }

    #[tokio::test]
    async fn newest_first_keeps_the_newest_matches_across_pages() {
        let log = log(2_500, |s| {
            if s % 2 == 0 {
                "tool.started"
            } else {
                "turn.completed"
            }
        });
        let params = ListEventsParams {
            session_id: log.session,
            exclude_types: vec!["tool.started".into()],
            before_sequence: Some(2_400),
            order_desc: true,
            ..Default::default()
        };
        let rows = select(&log, &params, 3).await.unwrap();
        assert_eq!(sequences(&rows), vec![2_399, 2_397, 2_395]);
        // Without filters of its own the cell picks the newest itself.
        let plain = ListEventsParams {
            exclude_types: vec![],
            ..params
        };
        assert_eq!(
            sequences(&select(&log, &plain, 2).await.unwrap()),
            vec![2_399, 2_398]
        );
    }

    #[tokio::test]
    async fn oldest_first_stops_at_the_cap_and_filters_by_context() {
        let log = log(2_500, |_| "tool.completed");
        let params = ListEventsParams {
            session_id: log.session,
            after_sequence: Some(5),
            turn_id: Some("turn_120".into()),
            ..Default::default()
        };
        assert_eq!(
            sequences(&select(&log, &params, 4).await.unwrap()),
            vec![1_201, 1_202, 1_203, 1_204]
        );
        let none = ListEventsParams {
            tool_name: Some("python".into()),
            ..params
        };
        assert!(select(&log, &none, 4).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_id_cursor_starts_after_that_event_and_an_unknown_one_reads_nothing() {
        let log = log(1_500, |_| "x");
        let params = ListEventsParams {
            session_id: log.session,
            since_id: Some(log.rows[1_199].id.parse().unwrap()),
            ..Default::default()
        };
        assert_eq!(
            sequences(&select(&log, &params, 2).await.unwrap()),
            vec![1_201, 1_202]
        );
        let unknown = ListEventsParams {
            since_id: Some(EventId::new()),
            ..params
        };
        assert!(select(&log, &unknown, 2).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_window_surrounds_its_anchor() {
        let log = log(30, |_| "x");
        let params = ListEventsParams {
            session_id: log.session,
            around_id: Some(log.rows[9].id.parse().unwrap()),
            window: Some(2),
            order_desc: true,
            ..Default::default()
        };
        assert_eq!(
            sequences(&select(&log, &params, usize::MAX).await.unwrap()),
            vec![8, 9, 10, 11, 12]
        );
    }

    #[tokio::test]
    async fn tags_match_nothing_and_search_reads_the_payload() {
        let log = log(5, |_| "x");
        let tagged = ListEventsParams {
            session_id: log.session,
            tags: vec!["t".into()],
            ..Default::default()
        };
        assert!(select(&log, &tagged, 10).await.unwrap().is_empty());
        let search = ListEventsParams {
            session_id: log.session,
            q: Some("\"N\":4".into()),
            ..Default::default()
        };
        assert_eq!(
            sequences(&select(&log, &search, 10).await.unwrap()),
            vec![4]
        );
    }
}
