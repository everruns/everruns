//! The install's stream out (`GET /v1/stream`) and the control plane an
//! install runs for its own shards (agentd design, "Stream").
//!
//! Decisions:
//! - The stream is the feed. Every shard leader already sends its feed to a
//!   control plane and, once stored, archives it in the object store
//!   (`node_control.rs`). An install is that control plane for its own
//!   shards: it stores nothing and answers every batch as stored, so the
//!   archive is the stream, and a consumer reads it with its own cursor.
//!   agentd tracks no consumer; two can read at once.
//! - The same built-in control plane answers allowance requests from the
//!   group's spend ledger (`group.rs`), so shards meter spend exactly as
//!   they do on the hosted service.
//! - A cursor is each shard's position (incarnation, index, ordinal), so
//!   records come once and in order per shard; across shards they merge
//!   by time. A shard rebuilt from a restore starts a new incarnation,
//!   read after the old one.
//! - The archive keeps [`RETENTION`]. Older objects are deleted and a mark
//!   records how far, so a cursor from before it gets an explicit gap
//!   (410) instead of silently missing records. Each session keeps its
//!   own totals for life (`Record::Session`), so a consumer that fell
//!   behind can still bill from the newest summary.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use futures::TryStreamExt;
use object_store::ObjectStoreExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dp::{AllowanceGrant, AllowanceRequest, FeedAck, FeedBatch, FeedRecord};
use crate::gateway::Gateway;
use crate::group::GroupOp;
use crate::state::{Command, Response as Reply};
use crate::store::Store;

type ApiError = (StatusCode, String);

/// How long the stream keeps records.
pub const RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);
/// How often a long poll looks at the archive again.
const POLL: Duration = Duration::from_millis(500);
/// The most a long poll waits.
const MAX_WAIT: Duration = Duration::from_secs(30);

/// The control plane an install runs for its own shards, served on the
/// nodes' internal address with the link's token.
pub fn control_plane(gateway: Arc<Gateway>) -> Router {
    Router::new()
        .route("/dp/v1/feed", post(feed))
        .route("/dp/v1/tenants/{tenant}/allowances", post(allowance))
        .with_state(gateway)
}

/// Every batch is stored as sent: the shard archives it next, which is
/// what the stream reads.
async fn feed(Json(batch): Json<FeedBatch>) -> Json<FeedAck> {
    let through = batch
        .records
        .last()
        .map_or((0, 0), |r| (r.index, r.ordinal));
    Json(FeedAck {
        source: batch.source,
        incarnation: batch.incarnation,
        stored_through: through,
    })
}

async fn allowance(
    State(gateway): State<Arc<Gateway>>,
    Path(tenant): Path<String>,
    Json(request): Json<AllowanceRequest>,
) -> Result<Json<AllowanceGrant>, ApiError> {
    let directory = gateway.directory_shard()?;
    let command = Command::Group {
        org: tenant,
        change: GroupOp::Ask { request },
        now_ms: crate::node::now_ms(),
    };
    match gateway.propose(directory, &command).await? {
        Reply::Value { value } => serde_json::from_value(value)
            .map(Json)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        other => Err((StatusCode::CONFLICT, format!("{other:?}"))),
    }
}

/// `/v1/stream`, on a gateway that runs an install with a store.
pub fn routes(store: Store) -> Router<Arc<Gateway>> {
    Router::new().route(
        "/v1/stream",
        get(move |state, query, headers| stream(state, query, headers, store.clone())),
    )
}

/// Each shard's position: `source` → (incarnation, index, ordinal).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor(pub BTreeMap<String, (u64, u64, u32)>);

impl Cursor {
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(&self.0).unwrap_or_default();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    pub fn decode(text: &str) -> Result<Self, ApiError> {
        if text.is_empty() {
            return Ok(Self::default());
        }
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(text)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .map(Self)
            .ok_or((StatusCode::BAD_REQUEST, "not a stream cursor".into()))
    }
}

/// What a read asks for.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub labels: BTreeMap<String, String>,
    /// Record types (`Usage`, `Session`, ...) or a session event's own type
    /// (`turn.completed`, ...), which an `Events` pointer names among its
    /// types; its events are read from the session. Empty: every one.
    pub types: Vec<String>,
}

impl Filter {
    fn wants(&self, record: &FeedRecord) -> bool {
        if !self
            .labels
            .iter()
            .all(|(k, v)| record.labels.get(k) == Some(v))
        {
            return false;
        }
        if self.types.is_empty() {
            return true;
        }
        let kind = record.record.kind();
        let event_types: Vec<&str> = match &record.record {
            crate::dp::Record::Event { event, .. } => event["type"].as_str().into_iter().collect(),
            crate::dp::Record::Events { types, .. } => types.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        self.types
            .iter()
            .any(|t| t == kind || event_types.contains(&t.as_str()))
    }
}

/// One record as the stream gives it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamRecord {
    pub shard: String,
    pub incarnation: u64,
    pub group: String,
    #[serde(flatten)]
    pub record: FeedRecord,
}

/// A page of the stream after `cursor`, at most `limit` records.
pub async fn read(
    store: &Store,
    sources: &[String],
    cursor: &Cursor,
    filter: &Filter,
    limit: usize,
) -> Result<(Vec<StreamRecord>, Cursor), ApiError> {
    let mut found: Vec<(String, u64, FeedRecord)> = Vec::new();
    let mut scanned = cursor.clone();
    for source in sources {
        let at = cursor.0.get(source).copied();
        // Checked against the mark, not the listing: a fully trimmed
        // incarnation has no objects left to list.
        if let Some((incarnation, index, _)) = at
            && trimmed(store, source, incarnation)
                .await
                .is_some_and(|t| index + 1 < t)
        {
            return Err((
                StatusCode::GONE,
                format!(
                    "the stream keeps {} days; this cursor is older (start again without one)",
                    RETENTION.as_secs() / 86_400
                ),
            ));
        }
        for incarnation in incarnations(store, source).await.map_err(unavailable)? {
            if at.is_some_and(|(inc, _, _)| incarnation < inc) {
                continue;
            }
            let after = at.filter(|(inc, _, _)| *inc == incarnation);
            let from = after.map_or(0, |(_, index, _)| index);
            let records = store
                .feed_since(source, incarnation, from)
                .await
                .map_err(unavailable)?;
            for record in records {
                if after.is_some_and(|(_, index, ordinal)| {
                    (record.index, record.ordinal) <= (index, ordinal)
                }) {
                    continue;
                }
                found.push((source.clone(), incarnation, record));
            }
        }
    }
    // Per shard in order; across shards by time.
    found.sort_by(|a, b| {
        a.2.at_unix_ms
            .cmp(&b.2.at_unix_ms)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| (a.1, a.2.index, a.2.ordinal).cmp(&(b.1, b.2.index, b.2.ordinal)))
    });
    // A shard's records must come in its own order whatever their times: a
    // shard's clock only moves forward, but keep the order exact.
    let mut out = Vec::new();
    for (source, incarnation, record) in found {
        if out.len() >= limit {
            break;
        }
        let position = (incarnation, record.index, record.ordinal);
        if scanned.0.get(&source).is_some_and(|p| *p >= position) {
            continue;
        }
        scanned.0.insert(source.clone(), position);
        if filter.wants(&record) {
            out.push(StreamRecord {
                shard: source,
                incarnation,
                group: record.org_id.clone(),
                record,
            });
        }
    }
    Ok((out, scanned))
}

fn unavailable(error: impl std::fmt::Display) -> ApiError {
    (StatusCode::SERVICE_UNAVAILABLE, error.to_string())
}

/// The incarnations `source` has archived, oldest first.
async fn incarnations(store: &Store, source: &str) -> anyhow::Result<Vec<u64>> {
    let dir = store.key(&format!("feed/{source}"));
    let listed = store.inner.list_with_delimiter(Some(&dir)).await?;
    let mut out: Vec<u64> = listed
        .common_prefixes
        .iter()
        .filter_map(|p| p.filename()?.parse().ok())
        .collect();
    out.sort_unstable();
    Ok(out)
}

/// The mark of how far `source`'s archive was trimmed.
fn trim_mark(source: &str, incarnation: u64) -> String {
    format!("feed-trimmed/{source}/{incarnation}.json")
}

async fn trimmed(store: &Store, source: &str, incarnation: u64) -> Option<u64> {
    let bytes = store
        .get_object(&trim_mark(source, incarnation))
        .await
        .ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?["through_index"].as_u64()
}

/// Delete archived records older than `retention`, marking how far.
pub async fn trim(store: &Store, source: &str, retention: Duration) -> anyhow::Result<usize> {
    let cutoff =
        chrono::Utc::now() - chrono::Duration::from_std(retention).unwrap_or(chrono::Duration::MAX);
    let mut deleted = 0;
    for incarnation in incarnations(store, source).await? {
        let dir = store.key(&format!("feed/{source}/{incarnation}"));
        let listed: Vec<_> = store.inner.list(Some(&dir)).try_collect().await?;
        let mut through = trimmed(store, source, incarnation).await.unwrap_or(0);
        for meta in listed.iter().filter(|m| m.last_modified < cutoff) {
            let last = meta
                .location
                .filename()
                .and_then(|f| f.split('-').nth(2))
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0);
            through = through.max(last + 1);
            store.inner.delete(&meta.location).await?;
            deleted += 1;
        }
        if deleted > 0 {
            store
                .put_object(
                    &trim_mark(source, incarnation),
                    serde_json::to_vec(&json!({ "through_index": through }))?,
                )
                .await?;
        }
    }
    Ok(deleted)
}

/// [`trim_loop`] as a task that never ends.
pub async fn trim_loop_task(gateway: Arc<Gateway>, store: Store) -> anyhow::Result<()> {
    trim_loop(gateway, store).await;
    Ok(())
}

/// Trim every shard's archive once an hour.
pub async fn trim_loop(gateway: Arc<Gateway>, store: Store) {
    loop {
        if let Ok(topology) = gateway.topology() {
            for shard in topology.shards.keys() {
                let source = format!("shard-{shard}");
                if let Err(error) = trim(&store, &source, RETENTION).await {
                    tracing::warn!(%source, %error, "stream archive not trimmed");
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn stream(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<Vec<(String, String)>>,
    headers: HeaderMap,
    store: Store,
) -> Response {
    match stream_inner(gateway, query, headers, store).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn stream_inner(
    gateway: Arc<Gateway>,
    query: Vec<(String, String)>,
    headers: HeaderMap,
    store: Store,
) -> Result<Response, ApiError> {
    let param = |name: &str| {
        query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    let filter = Filter {
        labels: crate::install::label_filter(&query)?,
        types: query
            .iter()
            .filter(|(k, _)| k == "type")
            .flat_map(|(_, v)| v.split(',').map(str::to_owned))
            .filter(|t| !t.is_empty())
            .collect(),
    };
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut cursor = Cursor::decode(&param("after").or(last_event_id).unwrap_or_default())?;
    let limit = param("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(500)
        .clamp(1, 5000);
    let wait = param("wait")
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(Duration::ZERO, Duration::from_secs)
        .min(MAX_WAIT);
    let sources = || -> Result<Vec<String>, ApiError> {
        Ok(gateway
            .topology()?
            .shards
            .keys()
            .map(|s| format!("shard-{s}"))
            .collect())
    };
    let sse = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/event-stream"));
    if sse {
        // First read here, so a bad cursor answers with its status.
        let (first, next) = read(&store, &sources()?, &cursor, &filter, limit).await?;
        // Each event's id is the cursor just past it, so a client that
        // reconnects with `Last-Event-ID` resumes there. Records filtered out
        // after the last one sent are read again then, and skipped again.
        let events = futures::stream::unfold(
            (first, cursor, next, gateway, store, filter),
            move |(mut pending, mut sent, mut next, gateway, store, filter)| async move {
                loop {
                    if !pending.is_empty() {
                        let record = pending.remove(0);
                        sent.0.insert(
                            record.shard.clone(),
                            (
                                record.incarnation,
                                record.record.index,
                                record.record.ordinal,
                            ),
                        );
                        let id = if pending.is_empty() { &next } else { &sent };
                        let event = axum::response::sse::Event::default()
                            .id(id.encode())
                            .json_data(&record)
                            .unwrap_or_default();
                        return Some((
                            Ok::<_, std::convert::Infallible>(event),
                            (pending, sent, next, gateway, store, filter),
                        ));
                    }
                    sent = next.clone();
                    tokio::time::sleep(POLL).await;
                    let sources: Vec<String> = match gateway.topology() {
                        Ok(t) => t.shards.keys().map(|s| format!("shard-{s}")).collect(),
                        Err(_) => continue,
                    };
                    match read(&store, &sources, &next, &filter, 500).await {
                        Ok((page, after)) => {
                            pending = page;
                            next = after;
                        }
                        Err(_) => return None,
                    }
                }
            },
        );
        return Ok(axum::response::Sse::new(events)
            .keep_alive(axum::response::sse::KeepAlive::default())
            .into_response());
    }
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let (records, next) = read(&store, &sources()?, &cursor, &filter, limit).await?;
        if !records.is_empty() || tokio::time::Instant::now() >= deadline {
            return Ok(Json(json!({ "data": records, "cursor": next.encode() })).into_response());
        }
        // Nothing yet: what was scanned past (filtered out) stays skipped.
        cursor = next;
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dp::Record;

    fn record(index: u64, at: i64, labels: &[(&str, &str)]) -> FeedRecord {
        FeedRecord {
            index,
            ordinal: 0,
            org_id: "g".into(),
            at_unix_ms: at,
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            record: Record::ConfigApplied { version: index },
        }
    }

    #[tokio::test]
    async fn a_cursor_reads_each_record_once_in_order_across_shards() {
        let store = Store::open("memory://").unwrap();
        store
            .put_feed("shard-0", 1, &[record(1, 10, &[]), record(2, 30, &[])])
            .await
            .unwrap();
        store
            .put_feed("shard-1", 1, &[record(5, 20, &[("org", "a")])])
            .await
            .unwrap();
        let sources = vec!["shard-0".to_owned(), "shard-1".to_owned()];
        let all = Filter::default();
        let (page, cursor) = read(&store, &sources, &Cursor::default(), &all, 2)
            .await
            .unwrap();
        let times: Vec<i64> = page.iter().map(|r| r.record.at_unix_ms).collect();
        assert_eq!(times, vec![10, 20]);
        let (page, cursor) = read(&store, &sources, &cursor, &all, 10).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].record.at_unix_ms, 30);
        let (page, _) = read(&store, &sources, &cursor, &all, 10).await.unwrap();
        assert!(page.is_empty());
        // Round trip through its text form.
        assert_eq!(Cursor::decode(&cursor.encode()).unwrap(), cursor);
    }

    #[tokio::test]
    async fn labels_filter_and_a_trimmed_cursor_is_a_gap() {
        let store = Store::open("memory://").unwrap();
        store
            .put_feed(
                "shard-0",
                1,
                &[
                    record(1, 10, &[("org", "a")]),
                    record(2, 11, &[("org", "b")]),
                ],
            )
            .await
            .unwrap();
        let sources = vec!["shard-0".to_owned()];
        let only_a = Filter {
            labels: [("org".to_owned(), "a".to_owned())].into(),
            types: vec![],
        };
        let (page, cursor) = read(&store, &sources, &Cursor::default(), &only_a, 10)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].record.labels["org"], "a");
        // Both were scanned: the cursor is past them.
        assert_eq!(cursor.0["shard-0"], (1, 2, 0));
        assert_eq!(trim(&store, "shard-0", Duration::ZERO).await.unwrap(), 1);
        let old = Cursor([("shard-0".to_owned(), (1, 1, 0))].into());
        let error = read(&store, &sources, &old, &Filter::default(), 10)
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::GONE);
    }
}
