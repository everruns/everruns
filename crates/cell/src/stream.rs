//! Live streaming (design.md "Live streaming"): one stream node per session.
//!
//! Decisions:
//! - The stream node of a session is chosen by rendezvous hashing of the
//!   session id over the cell's committed membership view: the Raft
//!   configuration, and its log index as the view's generation. The edge
//!   and the executors compute it themselves from a cached view (no
//!   lookup), and send the generation they hashed with; a node that sees
//!   another generation, or that is not the owner, answers 409 with its
//!   own view instead of serving.
//! - Liveness: the owner is the first node in rank order that is up. A
//!   caller that cannot reach the first tries the next; a node accepts
//!   when every node ranked above it is down in its own liveness view.
//!   Both sides converge on the same node without a lookup.
//! - Deltas go straight from the executor to the owner and never touch
//!   Raft or disk. The owner drops them when nobody watches the session,
//!   when they come from an older epoch (`fence`), when their `seq` is not
//!   past the last one taken, and, per stream, once the committed event
//!   that ends their attempt has been sent (the stream task tracks that, in
//!   commit order, so a late delta never follows its own completion).
//! - Committed events come from the owner's own replica as it applies
//!   them. A cursor the replica hasn't applied yet waits for it; a cursor
//!   never moves backwards.
//! - A move ends the stream with a `moved` event; the client reconnects
//!   and the edge re-resolves. Connections cycle every five minutes with
//!   `disconnecting`, as the control plane's SSE does.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use futures::Stream;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Digest;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::api::Lookup;
use crate::node::{Node, now_ms};
use crate::state::{CellId, Change};
use crate::types::NodeId;

/// How long a stream stays open before it cycles.
pub const CYCLE: Duration = Duration::from_secs(300);
/// How long a stream waits for its replica to reach a client's cursor.
const CURSOR_WAIT: Duration = Duration::from_secs(10);
/// Deltas a watcher may fall behind before it skips them.
const WATCHER_DELTAS: usize = 1024;

/// A cell's committed membership, as the stream hash reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    /// The log index of the membership: changes when membership changes.
    pub generation: u64,
    pub members: BTreeMap<NodeId, String>,
}

impl View {
    /// Members in the session's rendezvous order, the owner first.
    pub fn rank(&self, session: SessionId) -> Vec<(NodeId, String)> {
        let mut ranked: Vec<(u64, NodeId, String)> = self
            .members
            .iter()
            .map(|(node, addr)| (score(session, *node), *node, addr.clone()))
            .collect();
        // Highest score first; the node id breaks a tie the same way
        // everywhere.
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked
            .into_iter()
            .map(|(_, node, addr)| (node, addr))
            .collect()
    }
}

fn score(session: SessionId, node: NodeId) -> u64 {
    let mut hash = sha2::Sha256::new();
    hash.update(session.uuid().as_bytes());
    hash.update(node.to_le_bytes());
    let digest = hash.finalize();
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(first)
}

/// What a node that won't serve says: hash again with this view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redirect {
    pub view: View,
    /// The node this one takes for the owner.
    pub owner: String,
}

/// One delta on the wire: its stream (the attempt it belongs to within
/// the turn), its order within the epoch, and the event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveItem {
    pub stream: String,
    pub seq: u64,
    pub event: Event,
}

/// `POST /cell/exec/live`: an epoch's deltas for the session's owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveBatch {
    pub session: SessionId,
    pub fence: u64,
    /// The view generation the sender hashed with; 0 when it has none.
    #[serde(default)]
    pub view: u64,
    pub deltas: Vec<LiveItem>,
}

/// A delta as watchers get it.
#[derive(Debug, Clone)]
pub struct Live {
    pub stream: String,
    pub event: Arc<Event>,
}

/// The deltas of the sessions this node streams.
#[derive(Default)]
pub struct LiveHub {
    sessions: Mutex<HashMap<SessionId, Slot>>,
}

struct Slot {
    tx: broadcast::Sender<Live>,
    fence: u64,
    seq: u64,
}

impl LiveHub {
    pub fn subscribe(&self, session: SessionId) -> broadcast::Receiver<Live> {
        let mut sessions = self.sessions.lock();
        sessions.retain(|_, slot| slot.tx.receiver_count() > 0);
        sessions
            .entry(session)
            .or_insert_with(|| Slot {
                tx: broadcast::channel(WATCHER_DELTAS).0,
                fence: 0,
                seq: 0,
            })
            .tx
            .subscribe()
    }

    /// Hand `deltas` of epoch `fence` to the session's watchers; how many
    /// were taken. None when nobody watches.
    pub fn publish(&self, session: SessionId, fence: u64, deltas: Vec<LiveItem>) -> usize {
        let mut sessions = self.sessions.lock();
        let Some(slot) = sessions.get_mut(&session) else {
            return 0;
        };
        if slot.tx.receiver_count() == 0 {
            sessions.remove(&session);
            return 0;
        }
        if fence < slot.fence {
            return 0;
        }
        if fence > slot.fence {
            slot.fence = fence;
            slot.seq = 0;
        }
        let mut taken = 0;
        for item in deltas {
            if item.seq <= slot.seq {
                continue;
            }
            slot.seq = item.seq;
            taken += 1;
            let _ = slot.tx.send(Live {
                stream: item.stream,
                event: Arc::new(item.event),
            });
        }
        taken
    }

    /// Sessions with a watcher here.
    pub fn watched(&self) -> usize {
        self.sessions
            .lock()
            .values()
            .filter(|slot| slot.tx.receiver_count() > 0)
            .count()
    }
}

/// The stream an event belongs to (`everruns::hosting::LiveAttempt`'s
/// `stream`, the same rule the Engine's delta tee applies).
pub fn stream_of(event: &Event) -> Option<String> {
    use everruns_core::events::EventData;
    match &event.data {
        EventData::OutputMessageDelta(data) => Some(data.message_id.to_string()),
        EventData::OutputMessageCompleted(data) => Some(data.message.id.to_string()),
        EventData::ToolOutputDelta(data) => Some(data.tool_call_id.clone()),
        EventData::ToolCompleted(data) => Some(data.tool_call_id.clone()),
        _ => event.context.exec_id.map(|exec| exec.to_string()),
    }
}

/// A committed event that ends the attempt it belongs to.
fn ends_stream(event: &Event) -> bool {
    use everruns_core::events::EventData;
    matches!(
        event.data,
        EventData::OutputMessageCompleted(_)
            | EventData::ToolCompleted(_)
            | EventData::ReasonCompleted(_)
    )
}

fn ends_turn(event: &Event) -> bool {
    use everruns_core::events::EventData;
    matches!(
        event.data,
        EventData::TurnCompleted(_) | EventData::TurnFailed(_) | EventData::TurnCancelled(_)
    )
}

impl Node {
    /// This cell's view, as Raft last committed it.
    pub fn stream_view(&self) -> View {
        let metrics = self.raft.metrics().borrow().clone();
        let generation = metrics
            .membership_config
            .log_id()
            .as_ref()
            .map_or(0, |id| id.index);
        let members: BTreeMap<NodeId, String> = metrics
            .membership_config
            .membership()
            .voter_ids()
            .filter_map(|id| {
                metrics
                    .membership_config
                    .nodes()
                    .find(|(n, _)| **n == id)
                    .map(|(n, node)| (*n, node.addr.clone()))
            })
            .collect();
        View {
            generation,
            members: if members.is_empty() {
                self.cfg.members.clone()
            } else {
                members
            },
        }
    }

    /// The session's stream node as this node sees it: the first member in
    /// rank order that is up.
    pub fn stream_owner(&self, session: SessionId) -> (NodeId, String) {
        let view = self.stream_view();
        let alive = self.alive_members();
        let ranked = view.rank(session);
        ranked
            .iter()
            .find(|(node, _)| alive.contains(node))
            .or(ranked.first())
            .cloned()
            .unwrap_or((self.cfg.node_id, String::new()))
    }

    /// Refuse (with the view to hash again with) unless this node streams
    /// `session` in view `generation`.
    pub fn check_stream_owner(
        &self,
        session: SessionId,
        generation: Option<u64>,
    ) -> Result<(), Redirect> {
        let view = self.stream_view();
        let (owner, addr) = self.stream_owner(session);
        let same_view = generation.is_none_or(|g| g == view.generation);
        if same_view && owner == self.cfg.node_id {
            Ok(())
        } else {
            Err(Redirect { view, owner: addr })
        }
    }

    /// Whether `fence` is the session's current epoch here.
    pub fn epoch_current(&self, session: SessionId, fence: u64) -> bool {
        self.shared
            .state
            .read()
            .sessions
            .get(&session)
            .is_some_and(|s| s.authority.current(fence, now_ms()))
    }

    /// Take an epoch's deltas for the session's watchers, if the epoch is
    /// still the session's current one.
    pub fn publish_live(&self, session: SessionId, fence: u64, deltas: Vec<LiveItem>) -> bool {
        let current = self.epoch_current(session, fence);
        if current {
            self.live.publish(session, fence, deltas);
        }
        current
    }
}

fn redirect(redirect: Redirect) -> Response {
    (StatusCode::CONFLICT, Json(redirect)).into_response()
}

/// `/cell/view`, `/cell/exec/live` and the session stream on a node.
pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/cell/view", get(view))
        .route("/cell/exec/live", post(live))
        .route("/cell/sessions/{session}/sse", get(sse))
        .with_state(node)
}

async fn view(State(node): State<Arc<Node>>) -> Json<View> {
    Json(node.stream_view())
}

async fn live(State(node): State<Arc<Node>>, Json(batch): Json<LiveBatch>) -> Response {
    // A superseded epoch is dropped wherever it lands: no redirect for it.
    if !node.epoch_current(batch.session, batch.fence) {
        return Json(json!({ "taken": false })).into_response();
    }
    let view = Some(batch.view).filter(|generation| *generation != 0);
    if let Err(r) = node.check_stream_owner(batch.session, view) {
        return redirect(r);
    }
    let taken = node.publish_live(batch.session, batch.fence, batch.deltas);
    Json(json!({ "taken": taken })).into_response()
}

/// What a session stream follows, from its query.
#[derive(Debug, Default, Clone)]
pub struct StreamQuery {
    pub since_id: Option<String>,
    pub after_sequence: Option<i32>,
    pub types: Vec<String>,
    pub exclude: Vec<String>,
    /// The view generation the caller hashed with.
    pub view: Option<u64>,
}

impl StreamQuery {
    pub fn parse(raw: Option<&str>) -> Result<Self, (StatusCode, String)> {
        let mut query = Self::default();
        let bad = |what: &str| (StatusCode::BAD_REQUEST, format!("invalid {what}"));
        for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
            match key.as_ref() {
                "since_id" => query.since_id = Some(value.into_owned()),
                "after_sequence" => {
                    query.after_sequence = Some(value.parse().map_err(|_| bad("after_sequence"))?)
                }
                "types" | "types[]" => query.types.push(value.into_owned()),
                "exclude" | "exclude[]" => query.exclude.push(value.into_owned()),
                "view" => query.view = Some(value.parse().map_err(|_| bad("view"))?),
                _ => {}
            }
        }
        Ok(query)
    }

    pub fn passes(&self, event_type: &str) -> bool {
        (self.types.is_empty() || self.types.iter().any(|t| t == event_type))
            && !self.exclude.iter().any(|t| t == event_type)
    }
}

async fn sse(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    RawQuery(raw): RawQuery,
) -> Response {
    let session = match SessionId::parse(&session) {
        Ok(session) => session,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let query = match StreamQuery::parse(raw.as_deref()) {
        Ok(query) => query,
        Err(error) => return error.into_response(),
    };
    if let Err(r) = node.check_stream_owner(session, query.view) {
        return redirect(r);
    }
    match serve(node, session, query).await {
        Ok(sse) => sse.into_response(),
        Err(error) => error.into_response(),
    }
}

/// The sequence of event `since` in `session`, waiting for this replica to
/// apply it.
async fn cursor_of(
    node: &Node,
    session: SessionId,
    since: &str,
) -> Result<i32, (StatusCode, String)> {
    let deadline = tokio::time::Instant::now() + CURSOR_WAIT;
    let mut changes = node.shared.watch(session);
    loop {
        if let Some(Ok(events)) = node.shared.read_events(session, 0).await
            && let Some(seq) = events
                .iter()
                .find(|e| e.id.to_string() == since)
                .and_then(|e| e.sequence)
        {
            return Ok(seq);
        }
        if let Lookup::Moved { cell } = lookup_placement(node, session) {
            return Err((StatusCode::GONE, format!("moved to cell {cell}")));
        }
        match tokio::time::timeout_at(deadline, changes.recv()).await {
            Err(_) => {
                return Err((
                    StatusCode::NOT_FOUND,
                    format!("unknown since_id {since}"),
                ));
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err((StatusCode::SERVICE_UNAVAILABLE, "closing".into()));
            }
            Ok(_) => {}
        }
    }
}

fn lookup_placement(node: &Node, session: SessionId) -> Lookup<()> {
    let state = node.shared.state.read();
    if state.sessions.contains_key(&session) {
        Lookup::Here(())
    } else if let Some(left) = state.forwarded.get(&session) {
        Lookup::Moved { cell: left.cell }
    } else {
        Lookup::Missing
    }
}

fn frame(event: &Event) -> SseEvent {
    crate::api::sse_event(event)
}

/// What one stream has sent, for dropping late deltas.
#[derive(Default)]
struct Sent {
    last: i32,
    /// Streams whose ending committed event went out, by turn.
    closed: HashSet<String>,
}

impl Sent {
    fn committed(&mut self, event: &Event) {
        if let Some(seq) = event.sequence {
            self.last = seq;
        }
        if ends_turn(event) {
            self.closed.clear();
        } else if ends_stream(event)
            && let Some(stream) = stream_of(event)
        {
            self.closed.insert(stream);
        }
    }
}

/// Serve `session`'s stream from this node: `connected`, the committed
/// events after the cursor, then committed events and deltas as they come.
pub async fn serve(
    node: Arc<Node>,
    session: SessionId,
    query: StreamQuery,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, String)> {
    if matches!(lookup_placement(&node, session), Lookup::Missing) {
        return Err((StatusCode::NOT_FOUND, format!("no session {session}")));
    }
    let after = match (&query.since_id, query.after_sequence) {
        (Some(since), _) => cursor_of(&node, session, since).await?,
        (None, Some(after)) => after.max(0),
        (None, None) => node
            .shared
            .state
            .read()
            .sessions
            .get(&session)
            .and_then(|s| s.last_sequence())
            .unwrap_or(0),
    };
    // A client with no cursor gets live events only, as the control plane
    // streams: it read the history it shows with `GET .../events`.
    let replay = query.since_id.is_some() || query.after_sequence.is_some();
    let (tx, rx) = mpsc::channel::<SseEvent>(256);
    tokio::spawn(follow(node, session, query, after, replay, tx));
    // `sequence` is where the stream starts: a relay resumes from it when
    // it reconnects before any event went out.
    let connected = futures::stream::once(async move {
        Ok(SseEvent::default()
            .event("connected")
            .data(json!({ "status": "connected", "sequence": after }).to_string())
            .retry(Duration::from_millis(100)))
    });
    let stream = futures::StreamExt::chain(
        connected,
        futures::StreamExt::map(ReceiverStream::new(rx), Ok),
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

async fn follow(
    node: Arc<Node>,
    session: SessionId,
    query: StreamQuery,
    after: i32,
    replay_first: bool,
    tx: mpsc::Sender<SseEvent>,
) {
    // Subscribe before reading, so nothing falls between the two.
    let mut changes = node.shared.watch(session);
    let mut live = node.live.subscribe(session);
    let mut sent = Sent {
        last: after,
        ..Default::default()
    };
    let started = tokio::time::Instant::now();
    let send_committed = async |sent: &mut Sent, events: Vec<Event>| -> bool {
        for event in events {
            if event.sequence.is_none_or(|seq| seq <= sent.last) {
                continue;
            }
            sent.committed(&event);
            if query.passes(&event.event_type) && tx.send(frame(&event)).await.is_err() {
                return false;
            }
        }
        true
    };
    let read_after = async |last: i32| -> Vec<Event> {
        match node.shared.read_events(session, last).await {
            Some(Ok(events)) => events,
            _ => Vec::new(),
        }
    };
    if replay_first {
        let events = read_after(sent.last).await;
        if !send_committed(&mut sent, events).await {
            return;
        }
    }
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        let cycle = CYCLE.saturating_sub(started.elapsed());
        tokio::select! {
            change = changes.recv() => match change {
                Ok(Change::Event(_, event)) => {
                    let seq = event.sequence.unwrap_or(0);
                    if seq == sent.last + 1 {
                        if !send_committed(&mut sent, vec![(*event).clone()]).await { return; }
                    } else if seq > sent.last + 1 {
                        let events = read_after(sent.last).await;
                        if !send_committed(&mut sent, events).await { return; }
                    }
                }
                Ok(Change::Session(_)) => {
                    if moved(&node, session, &tx).await { return; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let events = read_after(sent.last).await;
                    if !send_committed(&mut sent, events).await { return; }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            delta = live.recv() => match delta {
                Ok(delta) => {
                    if sent.closed.contains(&delta.stream) || !query.passes(&delta.event.event_type) {
                        continue;
                    }
                    if tx.send(frame(&delta.event)).await.is_err() { return; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = tick.tick() => {
                if moved(&node, session, &tx).await { return; }
            }
            _ = tokio::time::sleep(cycle) => {
                let _ = tx
                    .send(
                        SseEvent::default()
                            .event("disconnecting")
                            .data(r#"{"reason":"connection_cycle","retry_ms":100}"#)
                            .retry(Duration::from_millis(100)),
                    )
                    .await;
                return;
            }
            _ = tx.closed() => return,
        }
    }
}

/// End the stream with `moved` if the session left this cell.
async fn moved(node: &Node, session: SessionId, tx: &mpsc::Sender<SseEvent>) -> bool {
    match lookup_placement(node, session) {
        Lookup::Moved { cell } => {
            let _ = tx.send(moved_event(cell)).await;
            true
        }
        Lookup::Missing => true,
        Lookup::Here(()) => false,
    }
}

pub fn moved_event(cell: CellId) -> SseEvent {
    SseEvent::default()
        .event("moved")
        .data(json!({ "cell": cell, "retry_ms": 0 }).to_string())
        .retry(Duration::from_millis(0))
}

/// Where an epoch's deltas go: the session's stream node, found from a
/// cached view of its cell.
#[derive(Default)]
pub struct Deliver {
    views: Mutex<HashMap<CellId, (tokio::time::Instant, View)>>,
}

/// How long a cached view is trusted before it is read again.
const VIEW_FOR: Duration = Duration::from_secs(5);

impl Deliver {
    /// `cell`'s view, from the cache unless `refresh`; read from any of
    /// `seeds` (the cell's members) otherwise.
    pub async fn view(
        &self,
        http: &reqwest::Client,
        cell: CellId,
        seeds: &[String],
        refresh: bool,
    ) -> Option<View> {
        if !refresh
            && let Some((at, view)) = self.views.lock().get(&cell)
            && at.elapsed() < VIEW_FOR
        {
            return Some(view.clone());
        }
        for addr in seeds {
            if let Ok(reply) = http
                .get(format!("http://{addr}/cell/view"))
                .timeout(Duration::from_secs(2))
                .send()
                .await
                && reply.status().is_success()
                && let Ok(view) = reply.json::<View>().await
            {
                self.learn(cell, view.clone());
                return Some(view);
            }
        }
        self.views.lock().get(&cell).map(|(_, v)| v.clone())
    }

    pub fn learn(&self, cell: CellId, view: View) {
        self.views
            .lock()
            .insert(cell, (tokio::time::Instant::now(), view));
    }

    /// Send `batch` to its stream node. Best effort: a batch nobody takes
    /// is dropped, and the committed event that ends its stream still
    /// carries the whole text.
    pub async fn send(
        &self,
        http: &reqwest::Client,
        cell: CellId,
        seeds: &[String],
        mut batch: LiveBatch,
    ) -> bool {
        for _ in 0..3 {
            // A redirect taught the cache the newer view.
            let Some(view) = self.view(http, cell, seeds, false).await else {
                return false;
            };
            batch.view = view.generation;
            let mut targets: Vec<String> =
                view.rank(batch.session).into_iter().map(|(_, a)| a).collect();
            let mut i = 0;
            let mut redirected = false;
            while i < targets.len() {
                let addr = targets[i].clone();
                i += 1;
                let sent = http
                    .post(format!("http://{addr}/cell/exec/live"))
                    .json(&batch)
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await;
                match sent {
                    Ok(reply) if reply.status().is_success() => return true,
                    Ok(reply) if reply.status() == StatusCode::CONFLICT => {
                        let Ok(r) = reply.json::<Redirect>().await else {
                            return false;
                        };
                        let same = r.view.generation == view.generation;
                        self.learn(cell, r.view);
                        if same && !r.owner.is_empty() && r.owner != addr {
                            // Same view, another owner by that node's
                            // liveness: try the one it names next.
                            targets.retain(|t| *t != r.owner);
                            targets.insert(i, r.owner);
                            continue;
                        }
                        redirected = true;
                        break;
                    }
                    Ok(_) => return false,
                    // Down: the next in rank order takes over.
                    Err(_) => continue,
                }
            }
            if !redirected {
                return false;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(nodes: &[u64]) -> View {
        View {
            generation: 7,
            members: nodes
                .iter()
                .map(|n| (*n, format!("127.0.0.1:{}", 7000 + n)))
                .collect(),
        }
    }

    #[test]
    fn every_caller_ranks_a_session_the_same_way() {
        let session = SessionId::new();
        let a = view(&[1, 2, 3]).rank(session);
        let mut shuffled = view(&[3, 1, 2]);
        shuffled.generation = 9;
        assert_eq!(a, shuffled.rank(session));
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn sessions_spread_over_the_members() {
        let members = view(&[1, 2, 3]);
        let mut owners = BTreeMap::<NodeId, usize>::new();
        for _ in 0..3000 {
            *owners.entry(members.rank(SessionId::new())[0].0).or_default() += 1;
        }
        assert_eq!(owners.len(), 3);
        assert!(owners.values().all(|n| *n > 800), "{owners:?}");
    }

    #[test]
    fn removing_a_member_moves_only_its_sessions() {
        let before = view(&[1, 2, 3]);
        let after = view(&[1, 2]);
        for _ in 0..500 {
            let session = SessionId::new();
            let owner = before.rank(session)[0].0;
            if owner != 3 {
                assert_eq!(after.rank(session)[0].0, owner);
            }
        }
    }

    fn delta(seq: u64, stream: &str) -> LiveItem {
        use everruns_core::events::{EventContext, OutputMessageDeltaData};
        let session = SessionId::new();
        LiveItem {
            stream: stream.into(),
            seq,
            event: Event::new(
                session,
                EventContext::default(),
                OutputMessageDeltaData {
                    message_id: everruns_contracts::typed_id::MessageId::new(),
                    delta: "x".into(),
                    accumulated: "x".into(),
                    turn_id: everruns_contracts::typed_id::TurnId::new(),
                    phase: None,
                },
            ),
        }
    }

    #[test]
    fn a_stale_epoch_and_repeated_seqs_are_dropped() {
        let hub = LiveHub::default();
        let session = SessionId::new();
        assert_eq!(hub.publish(session, 1, vec![delta(1, "m")]), 0, "no watcher");
        let mut rx = hub.subscribe(session);
        assert_eq!(hub.publish(session, 2, vec![delta(1, "m"), delta(2, "m")]), 2);
        assert_eq!(hub.publish(session, 2, vec![delta(2, "m")]), 0, "repeated");
        assert_eq!(hub.publish(session, 1, vec![delta(9, "m")]), 0, "stale epoch");
        assert_eq!(hub.publish(session, 3, vec![delta(1, "n")]), 1, "a new epoch starts over");
        assert_eq!(rx.try_recv().unwrap().stream, "m");
    }
}
