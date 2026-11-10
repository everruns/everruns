//! The stateless public API: a subset of the Everruns `/v1` session API.
//!
//! A gateway finds a session's cell through the directory (cached), then
//! talks to any node of that cell; nodes forward writes to their leader.
//! Creating and admitting go through the cell nodes' Everruns Engine
//! (`Engine::create_registered`, `Session::send_with_id`): admission returns
//! once Engine's admit command is committed on a quorum, which is the
//! durable receipt. SSE merges the streams of every node in the cell.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::events::Event;
use futures::{Stream, StreamExt};
use parking_lot::Mutex;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::api::{CellReply, Lookup, SessionView, Stats, sse_event};
use crate::cluster::TopologySource;
use crate::node::Proposed;
use crate::state::{CellId, Command, Placed, Response};

type ApiError = (StatusCode, String);

fn unavailable(error: impl std::fmt::Display) -> ApiError {
    (StatusCode::SERVICE_UNAVAILABLE, error.to_string())
}

pub struct Gateway {
    topology: TopologySource,
    /// The directory cell's cell registry, refreshed every second.
    registry: Mutex<crate::api::Registry>,
    /// Each cell's load, refreshed with the registry; placement and the
    /// rebalancer read it.
    loads: Mutex<BTreeMap<CellId, crate::api::CellLoad>>,
    http: reqwest::Client,
    cache: Mutex<HashMap<SessionId, CellId>>,
    next: AtomicUsize,
    /// Messages admitted (`accepted`, `duplicate`) and sessions created.
    pub admitted: crate::prom::Counters,
    pub created: crate::prom::Counters,
    /// Moves the rebalancer (on this node) started.
    pub rebalanced: crate::prom::Counters,
}

impl Gateway {
    #[expect(
        clippy::expect_used,
        reason = "a client with no TLS config only fails to build when the TLS backend cannot start, and a gateway is useless without one"
    )]
    pub fn new(topology: TopologySource) -> Arc<Self> {
        let gateway = Arc::new(Self {
            topology,
            registry: Mutex::default(),
            loads: Mutex::default(),
            http: reqwest::Client::builder()
                .no_proxy() // fleet traffic is direct, whatever the system proxy says
                .pool_max_idle_per_host(512)
                .tcp_nodelay(true)
                .build()
                .expect("http client"),
            cache: Mutex::default(),
            next: AtomicUsize::new(0),
            admitted: Default::default(),
            created: Default::default(),
            rebalanced: Default::default(),
        });
        let weak = Arc::downgrade(&gateway);
        tokio::spawn(async move {
            while let Some(gateway) = weak.upgrade() {
                gateway.refresh_registry().await;
                gateway.refresh_loads().await;
                drop(gateway);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        gateway
    }

    /// The fleet's topology: the file (seed nodes and the directory cell)
    /// overlaid with the registry, which holds each cell's live membership.
    ///
    /// Decision: the file only seeds. A cell is added by starting its nodes
    /// (they register with the directory), and a cell's nodes change with
    /// Raft membership changes, so nothing has to edit the file or restart
    /// a gateway.
    pub fn topology(&self) -> Result<crate::cluster::Topology, ApiError> {
        let mut topology = self.topology.read().map_err(unavailable)?;
        let registry = self.registry.lock();
        for (cell, members) in &registry.cells {
            if !members.is_empty() {
                topology.cells.insert(*cell, members.clone());
            }
        }
        // A retired cell is gone even if the seed file still names it.
        topology
            .cells
            .retain(|cell, _| !registry.retired.contains(cell));
        Ok(topology)
    }

    pub async fn refresh_registry(&self) {
        let Ok(topology) = self.topology() else {
            return;
        };
        for addr in topology.nodes(topology.directory_cell) {
            let response = self
                .http
                .get(format!("http://{addr}/cell/registry"))
                .timeout(Duration::from_secs(2))
                .send()
                .await;
            if let Ok(response) = response
                && response.status().is_success()
                && let Ok(registry) = response.json().await
            {
                *self.registry.lock() = registry;
                return;
            }
        }
    }

    async fn refresh_loads(&self) {
        let Ok(topology) = self.topology() else {
            return;
        };
        let mut loads = BTreeMap::new();
        for cell in topology.cells.keys().copied() {
            for addr in topology.nodes(cell) {
                let response = self
                    .http
                    .get(format!("http://{addr}/cell/load"))
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await;
                if let Ok(response) = response
                    && response.status().is_success()
                    && let Ok(load) = response.json::<crate::api::CellLoad>().await
                {
                    loads.insert(cell, load);
                    break;
                }
            }
        }
        *self.loads.lock() = loads;
    }

    /// Cells being emptied: no new sessions, and the rebalancer moves
    /// theirs out.
    pub fn draining(&self) -> std::collections::BTreeSet<CellId> {
        self.registry.lock().draining.clone()
    }

    pub async fn set_draining(&self, cell: CellId, drain: bool) -> Result<Json<Value>, ApiError> {
        let directory = self.directory_cell()?;
        if drain && cell == directory {
            // Decision: the directory cell holds the directory; retiring it
            // would need the directory itself to move. Not supported.
            return Err((
                StatusCode::CONFLICT,
                "the directory cell cannot be drained".into(),
            ));
        }
        match self
            .propose(directory, &Command::DrainCell { cell, drain })
            .await?
        {
            Response::Ok | Response::Duplicate => {}
            Response::NotFound => {
                return Err((StatusCode::NOT_FOUND, format!("unknown cell {cell}")));
            }
            other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
        }
        self.refresh_registry().await;
        let load: crate::api::CellLoad = self.call(cell, "/cell/load", None).await?;
        Ok(Json(
            json!({ "cell": cell, "draining": drain, "load": load }),
        ))
    }

    /// The last known load of every reachable cell.
    pub fn loads(&self) -> BTreeMap<CellId, crate::api::CellLoad> {
        self.loads.lock().clone()
    }

    /// Sessions the rebalancer may move out of `cell` (see `/cell/movable`).
    pub async fn movable(
        &self,
        cell: CellId,
        n: usize,
        kind: &str,
    ) -> Result<Vec<SessionId>, ApiError> {
        self.call(cell, &format!("/cell/movable?n={n}&kind={kind}"), None)
            .await
    }

    /// Where a new session goes: the less loaded of two random cells.
    ///
    /// Decision: two random choices, not the global minimum. Every gateway
    /// places from a load view up to a second old; all of them sending
    /// every new session to the one emptiest cell would overfill it before
    /// the next refresh, while two choices spread a burst and still steer
    /// clearly away from full cells. The cached load is bumped on each
    /// placement so one gateway's own burst counts at once.
    fn place(&self, cells: &[CellId]) -> CellId {
        let mut loads = self.loads.lock();
        let pick = if cells.len() < 2 {
            cells[0]
        } else {
            let i = rand::random_range(0..cells.len());
            let mut j = rand::random_range(0..cells.len() - 1);
            if j >= i {
                j += 1;
            }
            let (a, b) = (cells[i], cells[j]);
            // A cell with no known load is new or unreachable: prefer the
            // known one unless both are unknown.
            match (loads.get(&a), loads.get(&b)) {
                (Some(la), Some(lb)) => {
                    if lb.score() < la.score() {
                        b
                    } else {
                        a
                    }
                }
                (Some(_), None) | (None, None) => a,
                (None, Some(_)) => b,
            }
        };
        if let Some(load) = loads.get_mut(&pick) {
            load.sessions += 1;
        }
        pick
    }

    fn nodes(&self, cell: CellId) -> Result<Vec<String>, ApiError> {
        let topology = self.topology()?;
        let nodes = topology.nodes(cell);
        if nodes.is_empty() {
            return Err((StatusCode::NOT_FOUND, format!("unknown cell {cell}")));
        }
        Ok(nodes)
    }

    /// Call `path` on the cell's nodes, starting from a rotating one, until
    /// one answers.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        cell: CellId,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T, ApiError> {
        let nodes = self.nodes(cell)?;
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut last_error = String::new();
        let mut attempt = 0usize;
        while Instant::now() < deadline {
            let addr = &nodes[(start + attempt) % nodes.len()];
            attempt += 1;
            let url = format!("http://{addr}{path}");
            let request = match body {
                Some(body) => self.http.post(url).json(body),
                None => self.http.get(url),
            };
            match request.timeout(Duration::from_secs(45)).send().await {
                Ok(response) if response.status().is_success() => {
                    return response.json().await.map_err(unavailable);
                }
                Ok(response) => last_error = format!("{addr}: {}", response.status()),
                Err(error) => last_error = format!("{addr}: {error}"),
            }
            if attempt.is_multiple_of(nodes.len()) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Err(unavailable(last_error))
    }

    pub async fn propose(&self, cell: CellId, command: &Command) -> Result<Response, ApiError> {
        let body = serde_json::to_value(command).map_err(unavailable)?;
        let proposed: Proposed = self.call(cell, "/cell/propose", Some(&body)).await?;
        Ok(proposed.response)
    }

    pub fn directory_cell(&self) -> Result<CellId, ApiError> {
        Ok(self.topology()?.directory_cell)
    }

    /// The cell holding `session`'s directory entry.
    pub fn directory_of(&self, session: SessionId) -> Result<CellId, ApiError> {
        let shard = crate::state::shard_of(session);
        let owner = self.registry.lock().shards.get(&shard).copied();
        match owner {
            Some(cell) => Ok(cell),
            None => self.directory_cell(),
        }
    }

    /// Record that `session` lives in `cell` at `generation`, over the
    /// entry at `previous` (see `Command::Place`).
    pub async fn place_entry(
        &self,
        session: SessionId,
        cell: CellId,
        generation: u64,
        previous: u64,
    ) -> Result<Response, ApiError> {
        let command = Command::Place {
            session,
            cell,
            generation,
            previous,
        };
        self.directory_command(session, &command).await
    }

    /// `session`'s directory entry, read through the log.
    pub async fn placement(&self, session: SessionId) -> Result<Option<Placed>, ApiError> {
        match self
            .directory_command(session, &Command::Locate { session })
            .await?
        {
            Response::Placement { current } => Ok(current),
            other => Err(unavailable(format!("locate: {other:?}"))),
        }
    }

    /// Propose `command` to the cell holding `session`'s directory shard,
    /// following a shard that is being handed over.
    async fn directory_command(
        &self,
        session: SessionId,
        command: &Command,
    ) -> Result<Response, ApiError> {
        let mut owner = self.directory_of(session)?;
        for _ in 0..5 {
            match self.propose(owner, command).await? {
                Response::Moved { cell: next } => owner = next,
                other => return Ok(other),
            }
        }
        Err(unavailable("directory shard keeps moving"))
    }

    /// The cell a session lives in, from the cache or the directory.
    pub async fn locate(&self, session: SessionId) -> Result<CellId, ApiError> {
        let cached = self.cache.lock().get(&session).copied();
        if let Some(cell) = cached {
            // A cached cell that has since retired is stale.
            if self.topology()?.cells.contains_key(&cell) {
                return Ok(cell);
            }
            self.forget(session);
        }
        for attempt in 0..2 {
            if attempt > 0 {
                // The shard may have moved since the last registry refresh.
                self.refresh_registry().await;
            }
            let directory = self.directory_of(session)?;
            // A replica may not have applied a fresh placement yet; ask each.
            for addr in self.nodes(directory)? {
                let url = format!("http://{addr}/cell/directory/{session}");
                if let Ok(response) = self
                    .http
                    .get(url)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    && let Ok(Some(Placed { cell, .. })) = response.json::<Option<Placed>>().await
                {
                    self.cache.lock().insert(session, cell);
                    return Ok(cell);
                }
            }
        }
        Err((
            StatusCode::NOT_FOUND,
            format!("session {session} not found"),
        ))
    }

    fn forget(&self, session: SessionId) {
        self.cache.lock().remove(&session);
    }

    /// Run `f` against the session's cell, following moves.
    async fn with_cell<T, F, Fut>(&self, session: SessionId, f: F) -> Result<T, ApiError>
    where
        F: Fn(CellId) -> Fut,
        Fut: std::future::Future<Output = Result<Lookup<T>, ApiError>>,
    {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let cell = self.locate(session).await?;
            match f(cell).await? {
                Lookup::Here(value) => return Ok(value),
                Lookup::Moved { cell } => {
                    self.cache.lock().insert(session, cell);
                }
                Lookup::Missing => {
                    self.forget(session);
                    if Instant::now() > deadline {
                        return Err((StatusCode::NOT_FOUND, "session not found".into()));
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }

    /// Create a session running the agent `definition` describes (an
    /// Everruns agent package manifest). With `id`, creating is idempotent:
    /// a retry, or a parent re-delegating after a crash, finds the session
    /// it made.
    pub async fn create(
        &self,
        definition: Value,
        cell: Option<CellId>,
        id: Option<SessionId>,
        tenant: Option<String>,
    ) -> Result<Value, ApiError> {
        // Reject a malformed definition here, not at its first turn. Whether
        // a node has the tools and provider it names shows at the turn.
        let agent = crate::host::package(&definition)
            .map_err(|error| (StatusCode::BAD_REQUEST, format!("agent: {error}")))?
            .manifest()
            .name
            .clone();
        // A known id keeps its cell; placing and creating again are no-ops.
        let known = match id {
            Some(session) => self.locate(session).await.ok(),
            None => None,
        };
        let topology = self.topology()?;
        let draining = self.draining();
        let mut cells: Vec<CellId> = topology
            .cells
            .keys()
            .copied()
            .filter(|c| !draining.contains(c))
            .collect();
        let session = id.unwrap_or_default();
        // Place first: a session the directory cannot find is unreachable,
        // and an orphan placement costs nothing. A cell that started
        // draining since the last registry refresh refuses; pick another.
        // A known session is already placed.
        let requested = cell;
        let cell = loop {
            if let Some(cell) = known {
                break cell;
            }
            if cells.is_empty() {
                return Err(unavailable("no cell takes new sessions"));
            }
            let cell = requested.unwrap_or_else(|| self.place(&cells));
            let placed = self.place_entry(session, cell, 1, 0).await?;
            match placed {
                Response::Ok | Response::Duplicate => break cell,
                // Placed before (a retry the lookup missed): it stays there.
                Response::Placement {
                    current: Some(Placed { cell, .. }),
                } => break cell,
                other if requested.is_some() || cells.len() < 2 => {
                    return Err((StatusCode::CONFLICT, format!("{other:?}")));
                }
                _ => cells.retain(|c| *c != cell),
            }
        };
        let body = json!({ "definition": definition, "tenant": tenant });
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let reply: CellReply = self
                .call(
                    cell,
                    &format!("/cell/sessions/{session}/create"),
                    Some(&body),
                )
                .await?;
            match reply {
                CellReply::Ok { .. } => break,
                CellReply::Frozen | CellReply::NotFound if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                // A known session that moved since: it exists.
                CellReply::Moved { cell } if known.is_some() => {
                    self.cache.lock().insert(session, cell);
                    return Ok(json!({ "id": session, "agent": agent, "cell": cell }));
                }
                other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
            }
        }
        self.cache.lock().insert(session, cell);
        if known.is_none() {
            self.created.inc(&[("agent", agent.as_str())]);
        }
        Ok(json!({ "id": session, "agent": agent, "cell": cell }))
    }

    pub async fn admit(
        &self,
        session: SessionId,
        text: String,
        id: Option<MessageId>,
    ) -> Result<Value, ApiError> {
        let message_id = id.unwrap_or_default();
        let body = json!({ "text": text, "id": message_id });
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let cell = self.locate(session).await?;
            let reply: CellReply = self
                .call(
                    cell,
                    &format!("/cell/sessions/{session}/messages"),
                    Some(&body),
                )
                .await?;
            match reply {
                CellReply::Ok { value } => {
                    self.admitted.inc(&[("status", "accepted")]);
                    return Ok(value);
                }
                CellReply::Moved { cell } => {
                    self.cache.lock().insert(session, cell);
                }
                CellReply::Frozen | CellReply::NotFound if Instant::now() < deadline => {
                    self.forget(session);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                CellReply::NotFound => {
                    return Err((
                        StatusCode::NOT_FOUND,
                        format!("session {session} not found"),
                    ));
                }
                other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
            }
        }
    }

    /// Cancel the turn of admitted message `input` (Engine's targeted
    /// cancel: it never hits whichever turn happens to run later).
    pub async fn cancel(
        &self,
        session: SessionId,
        input: MessageId,
        id: Option<uuid::Uuid>,
    ) -> Result<Value, ApiError> {
        let body = json!({ "input": input, "id": id.unwrap_or_else(uuid::Uuid::new_v4) });
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let cell = self.locate(session).await?;
            let reply: CellReply = self
                .call(
                    cell,
                    &format!("/cell/sessions/{session}/cancel"),
                    Some(&body),
                )
                .await?;
            match reply {
                CellReply::Ok { value } => return Ok(value),
                CellReply::Moved { cell } => {
                    self.cache.lock().insert(session, cell);
                }
                CellReply::Frozen if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                CellReply::NotFound => {
                    return Err((
                        StatusCode::NOT_FOUND,
                        format!("session {session} not found"),
                    ));
                }
                other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
            }
        }
    }

    /// Move a session to `to`. The source cell records the move and its
    /// leader finishes it once the running turn ends (see
    /// `Node::maybe_move`); this waits until the session answers from `to`.
    pub async fn move_session(&self, session: SessionId, to: CellId) -> Result<Value, ApiError> {
        let from = self.locate(session).await?;
        if from == to {
            return Ok(json!({ "moved": false, "cell": to }));
        }
        let started = Instant::now();
        match self
            .propose(
                from,
                &Command::Freeze {
                    session,
                    to: Some(to),
                },
            )
            .await?
        {
            Response::Ok => {}
            Response::Moved { cell } if cell == to => {}
            other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
        }
        loop {
            let view: Lookup<SessionView> = self
                .call(from, &format!("/cell/sessions/{session}"), None)
                .await?;
            match view {
                Lookup::Moved { cell } => {
                    self.cache.lock().insert(session, cell);
                    break;
                }
                Lookup::Missing => return Err((StatusCode::NOT_FOUND, "session left".into())),
                Lookup::Here(_) if started.elapsed() > Duration::from_secs(300) => {
                    return Err((StatusCode::ACCEPTED, "move recorded, still running".into()));
                }
                Lookup::Here(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        Ok(json!({
            "moved": true,
            "from": from,
            "to": to,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        }))
    }
}

pub fn router(gateway: Arc<Gateway>) -> Router {
    Router::new()
        .route("/v1/sessions", post(create))
        .route("/v1/sessions/{session}", get(session))
        .route("/v1/sessions/{session}/messages", post(message))
        .route("/v1/sessions/{session}/events", get(events))
        .route("/v1/sessions/{session}/agent", get(agent))
        .route("/v1/sessions/{session}/sse", get(sse))
        .route("/v1/sessions/{session}/move", post(move_session))
        .route("/v1/sessions/{session}/wait", get(wait))
        .route(
            "/v1/sessions/{session}/messages/{message}/cancel",
            post(cancel),
        )
        .route("/v1/cells", get(cells))
        .route("/v1/tools", get(tools))
        .route("/v1/metrics/targets", get(targets))
        .route("/v1/cells/{cell}", axum::routing::delete(retire_cell))
        .route(
            "/v1/cells/{cell}/drain",
            post(drain_cell).delete(undrain_cell),
        )
        .route("/v1/cells/{cell}/nodes", post(add_node))
        .route(
            "/v1/cells/{cell}/nodes/{node}",
            axum::routing::delete(remove_node),
        )
        .layer(axum::extract::DefaultBodyLimit::max(max_request_bytes()))
        .with_state(gateway)
}

/// The largest request body the gateway takes (`CELL_MAX_REQUEST_MB`,
/// default 2 MB).
fn max_request_bytes() -> usize {
    std::env::var("CELL_MAX_REQUEST_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2)
        * 1024
        * 1024
}

fn parse(session: &str) -> Result<SessionId, ApiError> {
    SessionId::parse(session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize, Default)]
struct CancelBody {
    /// The cancel command's id, for retrying it safely.
    #[serde(default)]
    id: Option<uuid::Uuid>,
}

/// `POST /v1/sessions/{id}/messages/{message}/cancel`: cancel the turn of
/// an admitted message.
async fn cancel(
    State(gateway): State<Arc<Gateway>>,
    Path((session, message)): Path<(String, String)>,
    body: Option<Json<CancelBody>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let session = parse(&session)?;
    let message =
        MessageId::parse(&message).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let id = body.and_then(|Json(b)| b.id);
    Ok((
        StatusCode::ACCEPTED,
        Json(gateway.cancel(session, message, id).await?),
    ))
}

#[derive(Deserialize)]
struct CreateBody {
    /// The agent's definition: an Everruns agent package manifest.
    agent: Value,
    #[serde(default)]
    cell: Option<CellId>,
    #[serde(default)]
    id: Option<SessionId>,
    /// The tenant the session's spend is billed to (see `allowance`).
    #[serde(default)]
    tenant: Option<String>,
}

async fn create(
    State(gateway): State<Arc<Gateway>>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            gateway
                .create(body.agent, body.cell, body.id, body.tenant)
                .await?,
        ),
    ))
}

async fn session(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<SessionView>, ApiError> {
    let id = parse(&session)?;
    let gw = gateway.clone();
    let view = gateway
        .with_cell(id, |cell| {
            let gw = gw.clone();
            async move { gw.call(cell, &format!("/cell/sessions/{id}"), None).await }
        })
        .await?;
    Ok(Json(view))
}

/// `GET /v1/tools`: the tools nodes offer, ready to copy into an agent
/// definition's `tools`. Every node of a fleet runs the same build.
async fn tools(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    let directory = gateway.directory_cell()?;
    Ok(Json(gateway.call(directory, "/cell/tools", None).await?))
}

/// `GET /v1/sessions/{id}/agent`: the agent definition the session runs.
async fn agent(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = parse(&session)?;
    let gw = gateway.clone();
    let definition = gateway
        .with_cell(id, |cell| {
            let gw = gw.clone();
            async move {
                gw.call(cell, &format!("/cell/sessions/{id}/agent"), None)
                    .await
            }
        })
        .await?;
    Ok(Json(definition))
}

/// `{"message": {"content": [{"type": "text", "text": ...}]}}` as the
/// Everruns API takes it, or `{"text": ...}`; `id` makes a retry idempotent.
#[derive(Deserialize)]
struct MessageBody {
    #[serde(default)]
    message: Option<Value>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

async fn message(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Json(body): Json<MessageBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let session = parse(&session)?;
    let text = match (body.text, body.message) {
        (Some(text), _) => text,
        (None, Some(message)) => message["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
        (None, None) => return Err((StatusCode::BAD_REQUEST, "message or text required".into())),
    };
    let id = body
        .id
        .map(|id| MessageId::parse(&id).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string())))
        .transpose()?;
    Ok((
        StatusCode::ACCEPTED,
        Json(gateway.admit(session, text, id).await?),
    ))
}

#[derive(Deserialize, Default)]
struct Cursor {
    #[serde(default)]
    after_sequence: Option<i32>,
    #[serde(default)]
    since_id: Option<String>,
}

async fn events(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Result<Json<Value>, ApiError> {
    let id = parse(&session)?;
    let after = cursor.after_sequence.unwrap_or(0);
    let gw = gateway.clone();
    let events: Vec<Event> = gateway
        .with_cell(id, |cell| {
            let gw = gw.clone();
            async move {
                gw.call(
                    cell,
                    &format!("/cell/sessions/{id}/events?after={after}"),
                    None,
                )
                .await
            }
        })
        .await?;
    Ok(Json(json!({ "data": events })))
}

async fn move_session(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let to = body["cell"]
        .as_u64()
        .ok_or((StatusCode::BAD_REQUEST, "cell required".to_string()))? as CellId;
    Ok(Json(gateway.move_session(session, to).await?))
}

/// `POST /v1/cells/{cell}/nodes {"node": 4, "addr": "host:port"}`: add a
/// started node (run with `--join`) to a running cell.
async fn add_node(
    State(gateway): State<Arc<Gateway>>,
    Path(cell): Path<CellId>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        gateway.call(cell, "/cell/members", Some(&body)).await?,
    ))
}

/// `POST /v1/cells/{cell}/drain`: stop placing new sessions in `cell` and
/// let the rebalancer move its sessions, running turns included, to the
/// other cells. Returns the cell's load; drained when `sessions` is 0.
async fn drain_cell(
    State(gateway): State<Arc<Gateway>>,
    Path(cell): Path<CellId>,
) -> Result<Json<Value>, ApiError> {
    gateway.set_draining(cell, true).await
}

/// `DELETE /v1/cells/{cell}/drain`: take the cell back into placement.
async fn undrain_cell(
    State(gateway): State<Arc<Gateway>>,
    Path(cell): Path<CellId>,
) -> Result<Json<Value>, ApiError> {
    gateway.set_draining(cell, false).await
}

/// `DELETE /v1/cells/{cell}`: retire a drained cell. Refused (409) while
/// it still holds sessions. Afterwards gateways stop routing to it, the
/// scraper stops scraping it, and its nodes can be shut down.
async fn retire_cell(
    State(gateway): State<Arc<Gateway>>,
    Path(cell): Path<CellId>,
) -> Result<Json<Value>, ApiError> {
    let directory = gateway.directory_cell()?;
    let load: crate::api::CellLoad = gateway.call(cell, "/cell/load", None).await?;
    let shards = gateway
        .registry
        .lock()
        .shards
        .values()
        .filter(|c| **c == cell)
        .count();
    if shards > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!("cell {cell} still holds {shards} directory shards; drain it first"),
        ));
    }
    if load.sessions > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "cell {cell} still holds {} sessions; drain it first",
                load.sessions
            ),
        ));
    }
    match gateway
        .propose(directory, &Command::RetireCell { cell })
        .await?
    {
        Response::Ok | Response::Duplicate => {
            gateway.refresh_registry().await;
            Ok(Json(json!({ "cell": cell, "retired": true })))
        }
        other => Err((StatusCode::CONFLICT, format!("{other:?}"))),
    }
}

/// `DELETE /v1/cells/{cell}/nodes/{node}`: take a node out of a cell.
async fn remove_node(
    State(gateway): State<Arc<Gateway>>,
    Path((cell, node)): Path<(CellId, crate::types::NodeId)>,
) -> Result<Json<Value>, ApiError> {
    let nodes = gateway.nodes(cell)?;
    let mut last = String::new();
    for _ in 0..50 {
        for addr in &nodes {
            match gateway
                .http
                .delete(format!("http://{addr}/cell/members/{node}"))
                .timeout(Duration::from_secs(30))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    return Ok(Json(r.json().await.map_err(unavailable)?));
                }
                Ok(r) => last = format!("{addr}: {}", r.status()),
                Err(e) => last = format!("{addr}: {e}"),
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(unavailable(last))
}

/// Prometheus HTTP service discovery (`http_sd_configs`): every node of
/// every cell, labelled, so a scraper follows the fleet as nodes and cells
/// come and go.
async fn targets(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    // No target labels: every series already carries `cell` (and per-node
    // ones `node`), and a `node` label on a cell's leader-only series would
    // split it at every leader change.
    let groups: Vec<Value> = gateway
        .topology()?
        .cells
        .values()
        .flat_map(|nodes| nodes.values())
        .map(|addr| json!({ "targets": [addr], "labels": {} }))
        .collect();
    Ok(Json(json!(groups)))
}

async fn cells(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    let topology = gateway.topology()?;
    let mut out = BTreeMap::new();
    for (cell, nodes) in &topology.cells {
        let mut stats = Vec::new();
        for addr in nodes.values() {
            let url = format!("http://{addr}/cell/stats");
            let value: Option<Stats> = match gateway
                .http
                .get(url)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(r) => r.json().await.ok(),
                Err(_) => None,
            };
            stats.push(json!({ "addr": addr, "stats": value }));
        }
        out.insert(cell.to_string(), stats);
    }
    Ok(Json(json!(out)))
}

/// `GET /v1/sessions/{id}/sse`: durable events after the cursor, then live
/// durable and ephemeral events, as `event: <type>`, `id: <event id>`
/// (durable only), `data: <Event>`.
async fn sse(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Query(cursor): Query<Cursor>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let id = parse(&session)?;
    let cell = gateway.locate(id).await?;
    let mut after = cursor.after_sequence.unwrap_or(0);
    if let Some(since) = cursor.since_id {
        let gw = gateway.clone();
        let events: Vec<Event> = gateway
            .with_cell(id, |cell| {
                let gw = gw.clone();
                async move {
                    gw.call(cell, &format!("/cell/sessions/{id}/events?after=0"), None)
                        .await
                }
            })
            .await?;
        after = events
            .iter()
            .find(|e| e.id.to_string() == since)
            .and_then(|e| e.sequence)
            .ok_or((StatusCode::BAD_REQUEST, "unknown since_id".to_string()))?;
    }
    let nodes = gateway.nodes(cell)?;
    let (tx, rx) = mpsc::channel::<Event>(1024);
    let (merged_tx, merged_rx) = mpsc::channel::<Event>(1024);
    for addr in nodes {
        let http = gateway.http.clone();
        let tx = tx.clone();
        let url = format!("http://{addr}/cell/sessions/{id}/stream?after={after}");
        tokio::spawn(async move { follow(http, url, tx).await });
    }
    drop(tx);
    // Merge: durable events in sequence order, once; ephemeral as they come.
    tokio::spawn(async move {
        let mut rx = rx;
        let mut last = after;
        while let Some(event) = rx.recv().await {
            match event.sequence {
                Some(seq) if seq == last + 1 => {
                    last = seq;
                    if merged_tx.send(event).await.is_err() {
                        return;
                    }
                }
                Some(_) => {}
                None => {
                    if merged_tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
        }
    });
    let connected = futures::stream::once(async {
        Ok(SseEvent::default()
            .event("connected")
            .data(r#"{"status":"connected"}"#))
    });
    let stream = connected.chain(ReceiverStream::new(merged_rx).map(|event| Ok(sse_event(&event))));
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}

/// Follow one node's stream, reconnecting from the last durable sequence
/// it delivered, until the merged stream closes.
async fn follow(http: reqwest::Client, base: String, tx: mpsc::Sender<Event>) {
    let mut after: Option<i32> = None;
    loop {
        if tx.is_closed() {
            return;
        }
        let url = match after {
            Some(after) => {
                let (head, _) = base.split_once("?after=").unwrap_or((&base, ""));
                format!("{head}?after={after}")
            }
            None => base.clone(),
        };
        if let Ok(response) = http.get(&url).send().await {
            let mut body = response.bytes_stream();
            let mut buffer = String::new();
            while let Some(Ok(chunk)) = body.next().await {
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(end) = buffer.find("\n\n") {
                    let frame: String = buffer.drain(..end + 2).collect();
                    let data: String = frame
                        .lines()
                        .filter_map(|line| line.strip_prefix("data:"))
                        .map(|d| d.trim_start())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if data.is_empty() {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<Event>(&data) {
                        if let Some(seq) = event.sequence {
                            after = Some(seq);
                        }
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[derive(Deserialize)]
struct WaitQuery {
    turns: u64,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// Long-poll until the session completed `turns` turns. Not part of the
/// Everruns API; the load driver uses it instead of polling.
async fn wait(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Query(query): Query<WaitQuery>,
) -> Result<Json<SessionView>, ApiError> {
    let id = parse(&session)?;
    let timeout = query.timeout_ms.unwrap_or(30_000);
    let turns = query.turns;
    let gw = gateway.clone();
    let view = gateway
        .with_cell(id, |cell| {
            let gw = gw.clone();
            async move {
                gw.call(
                    cell,
                    &format!("/cell/sessions/{id}/wait?turns={turns}&timeout_ms={timeout}"),
                    None,
                )
                .await
            }
        })
        .await?;
    Ok(Json(view))
}
