//! The stateless public API: a subset of the Everruns `/v1` session API.
//!
//! A gateway finds a session's shard through the directory (cached), then
//! talks to any node of that shard; nodes forward writes to their leader.
//! Creating and admitting go through the shard nodes' Everruns Engine
//! (`Engine::create_registered`, `Session::send_with_id`): admission returns
//! once Engine's admit command is committed on a quorum, which is the
//! durable receipt. The Everruns-shaped session calls are in
//! `gateway_v1.rs`; a session's stream comes from its stream node
//! (`stream.rs`).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns_contracts::typed_id::{MessageId, SessionId};
use parking_lot::Mutex;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::api::{Lookup, SessionView, ShardReply, Stats};
use crate::cluster::TopologySource;
use crate::node::Proposed;
use crate::state::{Command, Placed, Response, ShardId};

type ApiError = (StatusCode, String);

fn unavailable(error: impl std::fmt::Display) -> ApiError {
    (StatusCode::SERVICE_UNAVAILABLE, error.to_string())
}

/// A shard short of memory refused new work: the caller retries later
/// (`retry_after` adds the hint as a header).
fn busy(shard: ShardId, retry_after_ms: u64) -> ApiError {
    (
        StatusCode::TOO_MANY_REQUESTS,
        format!("shard {shard} is short of memory; retry after {retry_after_ms} ms"),
    )
}

/// Every 429 tells the caller when to retry.
///
/// Decision: one hint for every refusal (the shard's own, rounded up to
/// seconds), carried as a header by this layer, so the error type stays a
/// status and a message.
async fn retry_after(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        let seconds = crate::pressure::RETRY_AFTER_MS.div_ceil(1000);
        response
            .headers_mut()
            .entry(axum::http::header::RETRY_AFTER)
            .or_insert_with(|| axum::http::HeaderValue::from(seconds));
    }
    response
}

/// How long a gateway call may take, over every member it tries.
const CALL_DEADLINE: Duration = Duration::from_secs(45);

/// How long one try at one member may take.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Gateway {
    topology: TopologySource,
    /// The directory shard's shard registry, refreshed every second.
    registry: Mutex<crate::api::Registry>,
    /// Each shard's load, refreshed with the registry; placement and the
    /// rebalancer read it.
    loads: Mutex<BTreeMap<ShardId, crate::api::ShardLoad>>,
    pub(crate) http: reqwest::Client,
    cache: Mutex<HashMap<SessionId, ShardId>>,
    next: AtomicUsize,
    /// Messages admitted (`accepted`, `duplicate`) and sessions created.
    pub admitted: crate::prom::Counters,
    /// The last minute of admissions, gateway to answer.
    pub admit_window: crate::prom::Window,
    pub created: crate::prom::Counters,
    /// Moves the rebalancer (on this node) started.
    pub rebalanced: crate::prom::Counters,
    /// Org tokens, when required (`org_token`).
    org_tokens: std::sync::OnceLock<Arc<crate::org_token::OrgTokens>>,
    /// Assigned tenants never change; an unassigned session is not cached.
    tenants: Mutex<HashMap<SessionId, String>>,
    /// The link's token, required on `/dp/v1` (`AGENTD_DP_TOKEN`).
    link_token: Mutex<crate::node_control::LinkToken>,
    /// The data plane's key, when linked (`ControlPlane::dp_key`).
    dp_key: Mutex<Option<everruns_contracts::dataplane_seal::Key>>,
    /// Each shard's stream view, for finding a session's stream node.
    pub(crate) views: crate::stream::Deliver,
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
            admit_window: Default::default(),
            created: Default::default(),
            rebalanced: Default::default(),
            org_tokens: std::sync::OnceLock::new(),
            tenants: Mutex::default(),
            link_token: Mutex::new(crate::node_control::link_token().map(Arc::from)),
            dp_key: Mutex::new(crate::executor::data_plane_key().ok().flatten()),
            views: Default::default(),
        });
        if let Some(tokens) = crate::org_token::OrgTokens::from_env() {
            gateway.require_org_tokens(tokens);
        }
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

    /// Require `token` on `/dp/v1`; takes effect for routers built after.
    pub fn set_link_token(&self, token: Option<String>) {
        *self.link_token.lock() = token.map(Arc::from);
    }

    /// Use `key` as the data plane's key.
    pub fn set_dp_key(&self, key: Option<everruns_contracts::dataplane_seal::Key>) {
        *self.dp_key.lock() = key;
    }

    /// The data plane's key; none when this data plane is not linked.
    pub fn dp_key(&self) -> Option<everruns_contracts::dataplane_seal::Key> {
        *self.dp_key.lock()
    }

    pub fn link_token(&self) -> crate::node_control::LinkToken {
        self.link_token.lock().clone()
    }

    /// Require an org token on every `/v1` call (`org_token`).
    pub fn require_org_tokens(&self, tokens: Arc<crate::org_token::OrgTokens>) {
        let _ = self.org_tokens.set(tokens);
    }

    /// `session`'s tenant, read through the log, then cached once assigned.
    pub(crate) async fn session_tenant(
        &self,
        session: SessionId,
    ) -> Result<Option<String>, ApiError> {
        if let Some(tenant) = self.tenants.lock().get(&session) {
            return Ok(Some(tenant.clone()));
        }
        // Create and Assign are separate commits. A follower's local view
        // may still be unassigned after create returned, so authorization
        // must read through the leader rather than cache that stale view.
        let tenant: Option<String> = self
            .with_shard(session, |shard| async move {
                match self.propose(shard, &Command::Tenant { session }).await? {
                    Response::Value { value } => serde_json::from_value(value)
                        .map(Lookup::Here)
                        .map_err(unavailable),
                    Response::NotFound => Ok(Lookup::Missing),
                    Response::Moved { shard } => Ok(Lookup::Moved { shard }),
                    other => Err(unavailable(format!("tenant: {other:?}"))),
                }
            })
            .await?;
        // A request concurrent with creation may precede Assign even in
        // log order. Only an assigned tenant is immutable and cacheable.
        if let Some(tenant) = &tenant {
            let mut tenants = self.tenants.lock();
            if tenants.len() > 100_000 {
                tenants.clear();
            }
            tenants.insert(session, tenant.clone());
        }
        Ok(tenant)
    }

    /// The fleet's topology: the file (seed nodes and the directory shard)
    /// overlaid with the registry, which holds each shard's live membership.
    ///
    /// Decision: the file only seeds. A shard is added by starting its nodes
    /// (they register with the directory), and a shard's nodes change with
    /// Raft membership changes, so nothing has to edit the file or restart
    /// a gateway.
    pub fn topology(&self) -> Result<crate::cluster::Topology, ApiError> {
        let mut topology = self.topology.read().map_err(unavailable)?;
        let registry = self.registry.lock();
        for (shard, members) in &registry.shards {
            if !members.is_empty() {
                topology.shards.insert(*shard, members.clone());
            }
        }
        // A retired shard is gone even if the seed file still names it.
        topology
            .shards
            .retain(|shard, _| !registry.retired.contains(shard));
        Ok(topology)
    }

    pub async fn refresh_registry(&self) {
        let Ok(topology) = self.topology() else {
            return;
        };
        for addr in topology.nodes(topology.directory_shard) {
            let response = self
                .http
                .get(format!("http://{addr}/shard/registry"))
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
        for shard in topology.shards.keys().copied() {
            for addr in topology.nodes(shard) {
                let response = self
                    .http
                    .get(format!("http://{addr}/shard/load"))
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await;
                if let Ok(response) = response
                    && response.status().is_success()
                    && let Ok(load) = response.json::<crate::api::ShardLoad>().await
                {
                    loads.insert(shard, load);
                    break;
                }
            }
        }
        *self.loads.lock() = loads;
    }

    /// Shards being emptied: no new sessions, and the rebalancer moves
    /// theirs out.
    pub fn draining(&self) -> std::collections::BTreeSet<ShardId> {
        self.registry.lock().draining.clone()
    }

    pub async fn set_draining(&self, shard: ShardId, drain: bool) -> Result<Json<Value>, ApiError> {
        let directory = self.directory_shard()?;
        if drain && shard == directory {
            // Decision: the directory shard holds the directory; retiring it
            // would need the directory itself to move. Not supported.
            return Err((
                StatusCode::CONFLICT,
                "the directory shard cannot be drained".into(),
            ));
        }
        match self
            .propose(directory, &Command::DrainShard { shard, drain })
            .await?
        {
            Response::Ok | Response::Duplicate => {}
            Response::NotFound => {
                return Err((StatusCode::NOT_FOUND, format!("unknown shard {shard}")));
            }
            other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
        }
        self.refresh_registry().await;
        let load: crate::api::ShardLoad = self.call(shard, "/shard/load", None).await?;
        Ok(Json(
            json!({ "shard": shard, "draining": drain, "load": load }),
        ))
    }

    /// The last known load of every reachable shard.
    pub fn loads(&self) -> BTreeMap<ShardId, crate::api::ShardLoad> {
        self.loads.lock().clone()
    }

    /// Sessions the rebalancer may move out of `shard` (see `/shard/movable`).
    pub async fn movable(
        &self,
        shard: ShardId,
        n: usize,
        kind: &str,
    ) -> Result<Vec<SessionId>, ApiError> {
        self.call(shard, &format!("/shard/movable?n={n}&kind={kind}"), None)
            .await
    }

    /// Where a new session goes: the less loaded of two random shards.
    ///
    /// Decision: two random choices, not the global minimum. Every gateway
    /// places from a load view up to a second old; all of them sending
    /// every new session to the one emptiest shard would overfill it before
    /// the next refresh, while two choices spread a burst and still steer
    /// clearly away from full shards. The cached load is bumped on each
    /// placement so one gateway's own burst counts at once.
    ///
    /// A shard short of memory (`ShardLoad::closed`) gets none while any other
    /// shard takes them.
    fn place(&self, shards: &[ShardId]) -> ShardId {
        let mut loads = self.loads.lock();
        let open: Vec<ShardId> = shards
            .iter()
            .copied()
            .filter(|c| !loads.get(c).is_some_and(|l| l.closed))
            .collect();
        let shards = if open.is_empty() { shards } else { &open };
        let pick = if shards.len() < 2 {
            shards[0]
        } else {
            let i = rand::random_range(0..shards.len());
            let mut j = rand::random_range(0..shards.len() - 1);
            if j >= i {
                j += 1;
            }
            let (a, b) = (shards[i], shards[j]);
            // A shard with no known load is new or unreachable: prefer the
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

    pub(crate) fn nodes(&self, shard: ShardId) -> Result<Vec<String>, ApiError> {
        let topology = self.topology()?;
        let nodes = topology.nodes(shard);
        if nodes.is_empty() {
            return Err((StatusCode::NOT_FOUND, format!("unknown shard {shard}")));
        }
        Ok(nodes)
    }

    /// Call `path` on the shard's nodes, starting from a rotating one, until
    /// one answers.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        shard: ShardId,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T, ApiError> {
        self.call_within(shard, path, body, ATTEMPT_TIMEOUT).await
    }

    /// [`call`](Self::call) for a request that may take up to `attempt` at
    /// one member (a long poll).
    pub(crate) async fn call_within<T: DeserializeOwned>(
        &self,
        shard: ShardId,
        path: &str,
        body: Option<&Value>,
        attempt_timeout: Duration,
    ) -> Result<T, ApiError> {
        let nodes = self.nodes(shard)?;
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        // Decision: each try gets at most `ATTEMPT_TIMEOUT`, so a member
        // that accepts connections but never answers (a hung or stopped
        // process; the kernel still completes the handshake) costs one try,
        // not the whole call. Before, one try could outlast the deadline
        // and the gateway never reached the healthy members (found by
        // `prove_follower_catchup.py`). A retried proposal is safe: every
        // command a gateway proposes is idempotent by its id or seed.
        let deadline = Instant::now() + CALL_DEADLINE.max(attempt_timeout * 2);
        let mut last_error = String::new();
        let mut attempt = 0usize;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let addr = &nodes[(start + attempt) % nodes.len()];
            attempt += 1;
            let url = format!("http://{addr}{path}");
            let request = match body {
                Some(body) => self.http.post(url).json(body),
                None => self.http.get(url),
            };
            match request.timeout(left.min(attempt_timeout)).send().await {
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

    pub async fn propose(&self, shard: ShardId, command: &Command) -> Result<Response, ApiError> {
        let body = serde_json::to_value(command).map_err(unavailable)?;
        let proposed: Proposed = self.call(shard, "/shard/propose", Some(&body)).await?;
        Ok(proposed.response)
    }

    pub fn directory_shard(&self) -> Result<ShardId, ApiError> {
        Ok(self.topology()?.directory_shard)
    }

    /// The shard holding `session`'s directory entry.
    pub fn directory_of(&self, session: SessionId) -> Result<ShardId, ApiError> {
        let slot = crate::state::slot_of(session);
        let owner = self.registry.lock().slots.get(&slot).copied();
        match owner {
            Some(shard) => Ok(shard),
            None => self.directory_shard(),
        }
    }

    /// Record that `session` lives in `shard` at `generation`, over the
    /// entry at `previous` (see `Command::Place`).
    pub async fn place_entry(
        &self,
        session: SessionId,
        shard: ShardId,
        generation: u64,
        previous: u64,
    ) -> Result<Response, ApiError> {
        let command = Command::Place {
            session,
            shard,
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

    /// Propose `command` to the shard holding `session`'s directory slot,
    /// following a slot that is being handed over.
    async fn directory_command(
        &self,
        session: SessionId,
        command: &Command,
    ) -> Result<Response, ApiError> {
        let mut owner = self.directory_of(session)?;
        for _ in 0..5 {
            match self.propose(owner, command).await? {
                Response::Moved { shard: next } => owner = next,
                other => return Ok(other),
            }
        }
        Err(unavailable("directory slot keeps moving"))
    }

    /// The shard a session lives in, from the cache or the directory.
    pub async fn locate(&self, session: SessionId) -> Result<ShardId, ApiError> {
        let cached = self.cache.lock().get(&session).copied();
        if let Some(shard) = cached {
            // A cached shard that has since retired is stale.
            if self.topology()?.shards.contains_key(&shard) {
                return Ok(shard);
            }
            self.forget(session);
        }
        for attempt in 0..2 {
            if attempt > 0 {
                // The slot may have moved since the last registry refresh.
                self.refresh_registry().await;
            }
            let directory = self.directory_of(session)?;
            // A replica may not have applied a fresh placement yet; ask each.
            for addr in self.nodes(directory)? {
                let url = format!("http://{addr}/shard/directory/{session}");
                if let Ok(response) = self
                    .http
                    .get(url)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    && let Ok(Some(Placed { shard, .. })) = response.json::<Option<Placed>>().await
                {
                    self.cache.lock().insert(session, shard);
                    return Ok(shard);
                }
            }
        }
        Err((
            StatusCode::NOT_FOUND,
            format!("session {session} not found"),
        ))
    }

    pub(crate) fn forget(&self, session: SessionId) {
        self.cache.lock().remove(&session);
    }

    /// Run `f` against the session's shard, following moves.
    pub(crate) async fn with_shard<T, F, Fut>(
        &self,
        session: SessionId,
        f: F,
    ) -> Result<T, ApiError>
    where
        F: Fn(ShardId) -> Fut,
        Fut: std::future::Future<Output = Result<Lookup<T>, ApiError>>,
    {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let shard = self.locate(session).await?;
            match f(shard).await? {
                Lookup::Here(value) => return Ok(value),
                Lookup::Moved { shard } => {
                    self.cache.lock().insert(session, shard);
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
        shard: Option<ShardId>,
        id: Option<SessionId>,
        tenant: Option<String>,
    ) -> Result<Value, ApiError> {
        self.create_running(definition, shard, id, tenant, None, Joining::default())
            .await
    }

    /// [`Self::create`] for a session of the control plane's agent
    /// `agent_ref` ([`crate::platform::Platform::agent_ref`]), joining it
    /// as `joining` says (its Memory, whether it is being imported).
    pub async fn create_running(
        &self,
        definition: Value,
        shard: Option<ShardId>,
        id: Option<SessionId>,
        tenant: Option<String>,
        agent_ref: Option<String>,
        joining: Joining,
    ) -> Result<Value, ApiError> {
        let Joining {
            memory,
            servers,
            importing,
            subagent,
            owner,
        } = joining;
        // Reject a malformed definition here, not at its first turn. Whether
        // a node has the tools and provider it names shows at the turn.
        let agent = crate::host::package(&definition)
            .map_err(|error| (StatusCode::BAD_REQUEST, format!("agent: {error}")))?
            .manifest()
            .name
            .clone();
        // A known id keeps its shard; placing and creating again are no-ops.
        let known = match id {
            Some(session) => self.locate(session).await.ok(),
            None => None,
        };
        let topology = self.topology()?;
        let draining = self.draining();
        let mut shards: Vec<ShardId> = topology
            .shards
            .keys()
            .copied()
            .filter(|c| !draining.contains(c))
            .collect();
        let session = id.unwrap_or_default();
        // Place first: a session the directory cannot find is unreachable,
        // and an orphan placement costs nothing. A shard that started
        // draining since the last registry refresh refuses; pick another.
        // A known session is already placed.
        let requested = shard;
        let shard = loop {
            if let Some(shard) = known {
                break shard;
            }
            if shards.is_empty() {
                return Err(unavailable("no shard takes new sessions"));
            }
            let shard = requested.unwrap_or_else(|| self.place(&shards));
            let placed = self.place_entry(session, shard, 1, 0).await?;
            match placed {
                Response::Ok | Response::Duplicate => break shard,
                // Placed before (a retry the lookup missed): it stays there.
                Response::Placement {
                    current: Some(Placed { shard, .. }),
                } => break shard,
                other if requested.is_some() || shards.len() < 2 => {
                    return Err((StatusCode::CONFLICT, format!("{other:?}")));
                }
                _ => shards.retain(|c| *c != shard),
            }
        };
        let body = json!({
            "definition": definition,
            "tenant": tenant,
            "agent_ref": agent_ref,
            "memory": memory,
            "servers": servers,
            "importing": importing,
            "subagent": subagent,
            "owner": owner,
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let reply: ShardReply = self
                .call(
                    shard,
                    &format!("/shard/sessions/{session}/create"),
                    Some(&body),
                )
                .await?;
            match reply {
                ShardReply::Ok { .. } => break,
                ShardReply::Busy { retry_after_ms } => return Err(busy(shard, retry_after_ms)),
                ShardReply::Frozen | ShardReply::NotFound if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                // A known session that moved since: it exists.
                ShardReply::Moved { shard } if known.is_some() => {
                    self.cache.lock().insert(session, shard);
                    return Ok(json!({ "id": session, "agent": agent, "shard": shard }));
                }
                other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
            }
        }
        self.cache.lock().insert(session, shard);
        if known.is_none() {
            self.created.inc(&[]);
        }
        Ok(json!({ "id": session, "agent": agent, "shard": shard }))
    }

    pub async fn admit(
        &self,
        session: SessionId,
        text: String,
        id: Option<MessageId>,
    ) -> Result<Value, ApiError> {
        self.admit_parts(session, text, Vec::new(), id).await
    }

    /// [`Gateway::admit`] for a message with parts other than text (an
    /// image, a file); `parts` replace `text` when given.
    pub async fn admit_parts(
        &self,
        session: SessionId,
        text: String,
        parts: Vec<everruns_core::ContentPart>,
        id: Option<MessageId>,
    ) -> Result<Value, ApiError> {
        self.admit_from(session, text, parts, id, None).await
    }

    /// [`Gateway::admit_parts`] for a message `sender` (a user id, from the
    /// caller's verified org token) wrote, so the session's turns can act
    /// as them (`senders.rs`).
    pub async fn admit_from(
        &self,
        session: SessionId,
        text: String,
        parts: Vec<everruns_core::ContentPart>,
        id: Option<MessageId>,
        sender: Option<String>,
    ) -> Result<Value, ApiError> {
        let started = Instant::now();
        let message_id = id.unwrap_or_default();
        let body = json!({ "text": text, "parts": parts, "id": message_id, "sender": sender });
        let value = self.session_command(session, "messages", &body).await?;
        self.admitted.inc(&[("status", "accepted")]);
        self.admit_window.observe(started.elapsed());
        Ok(value)
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
        self.session_command(session, "cancel", &body).await
    }

    /// Run a session command (`POST /shard/sessions/{id}/{command}`, which
    /// answers a [`ShardReply`]) on the session's shard, following moves and
    /// waiting out a move's freeze. A refusal is a 409 with its reason.
    pub(crate) async fn session_command(
        &self,
        session: SessionId,
        command: &str,
        body: &Value,
    ) -> Result<Value, ApiError> {
        let deadline = Instant::now() + Duration::from_secs(60);
        // A shard may not know a session its directory entry already names
        // for a moment (a create or a move in flight); longer is missing.
        let missing = Instant::now() + Duration::from_secs(10);
        loop {
            let shard = self.locate(session).await?;
            let reply: ShardReply = self
                .call(
                    shard,
                    &format!("/shard/sessions/{session}/{command}"),
                    Some(body),
                )
                .await?;
            match reply {
                ShardReply::Ok { value } => return Ok(value),
                ShardReply::Moved { shard } => {
                    self.cache.lock().insert(session, shard);
                }
                ShardReply::Frozen if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                ShardReply::NotFound if Instant::now() < missing => {
                    self.forget(session);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                ShardReply::NotFound => {
                    return Err((
                        StatusCode::NOT_FOUND,
                        format!("session {session} not found"),
                    ));
                }
                ShardReply::Refused { reason } => return Err((StatusCode::CONFLICT, reason)),
                ShardReply::Busy { retry_after_ms } => {
                    if command == "messages" {
                        self.admitted.inc(&[("status", "shed")]);
                    }
                    return Err(busy(shard, retry_after_ms));
                }
                other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
            }
        }
    }

    /// Move a session to `to`. The source shard records the move and its
    /// leader finishes it once the running turn ends (see
    /// `Node::maybe_move`); this waits until the session answers from `to`.
    pub async fn move_session(&self, session: SessionId, to: ShardId) -> Result<Value, ApiError> {
        let from = self.locate(session).await?;
        if from == to {
            return Ok(json!({ "moved": false, "shard": to }));
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
            Response::Moved { shard } if shard == to => {}
            other => return Err((StatusCode::CONFLICT, format!("{other:?}"))),
        }
        loop {
            let view: Lookup<SessionView> = self
                .call(from, &format!("/shard/sessions/{session}"), None)
                .await?;
            match view {
                Lookup::Moved { shard } => {
                    self.cache.lock().insert(session, shard);
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
        .route("/v1/sessions/{session}/agent", get(agent))
        .route("/v1/sessions/{session}/move", post(move_session))
        .route("/v1/sessions/{session}/wait", get(wait))
        .route(
            "/v1/sessions/{session}/messages/{message}/cancel",
            post(cancel),
        )
        .route("/v1/shards", get(shards))
        .route("/v1/tools", get(tools))
        .route("/v1/metrics/targets", get(targets))
        .route("/v1/shards/{shard}", axum::routing::delete(retire_shard))
        .route(
            "/v1/shards/{shard}/drain",
            post(drain_shard).delete(undrain_shard),
        )
        .route("/v1/shards/{shard}/nodes", post(add_node))
        .route(
            "/v1/shards/{shard}/nodes/{node}",
            axum::routing::delete(remove_node),
        )
        .merge(crate::gateway_v1::routes())
        .merge(crate::gateway_fs::routes())
        .layer(axum::extract::DefaultBodyLimit::max(max_request_bytes()))
        .layer(axum::middleware::from_fn_with_state(
            gateway.clone(),
            check_org_token,
        ))
        .layer(axum::middleware::from_fn(retry_after))
        .with_state(gateway.clone())
        .merge(crate::gateway_dp::router(gateway))
}

/// With org tokens required: a session call needs a token that may drive
/// the session's tenant, and every other `/v1` call needs `admin`. A
/// session of another org answers as a missing one.
async fn check_org_token(
    State(gateway): State<Arc<Gateway>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use crate::org_token::{ADMIN, OrgTokenClaimsExt, SESSIONS, bearer};
    use axum::response::IntoResponse;
    let Some(tokens) = gateway.org_tokens.get().cloned() else {
        return next.run(request).await;
    };
    let Some(token) = bearer(request.headers()).map(str::to_owned) else {
        return (StatusCode::UNAUTHORIZED, "org token: none sent").into_response();
    };
    let claims = match tokens.verify(&token).await {
        Ok(claims) => claims,
        Err(error) => return error.into_response(),
    };
    let path: Vec<String> = request
        .uri()
        .path()
        .trim_matches('/')
        .split('/')
        .map(str::to_owned)
        .collect();
    let refused = match path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["v1", "sessions"] => (!claims.has(SESSIONS) && !claims.has(ADMIN)).then(|| {
            (
                StatusCode::FORBIDDEN,
                "org token: no sessions:write scope".to_owned(),
            )
        }),
        ["v1", "sessions", session, ..] => match parse(session) {
            Err(error) => Some(error),
            Ok(id) => match gateway.session_tenant(id).await {
                Err(error) => Some(error),
                Ok(tenant) => (!claims.may_drive(tenant.as_deref()))
                    .then(|| (StatusCode::NOT_FOUND, format!("no session {id}"))),
            },
        },
        ["v1", "workspaces", workspace, ..] => match crate::gateway_fs::session_of(workspace) {
            Err(error) => Some(error),
            Ok(id) => match gateway.session_tenant(id).await {
                Err(error) => Some(error),
                Ok(tenant) => (!claims.may_drive(tenant.as_deref()))
                    .then(|| (StatusCode::NOT_FOUND, format!("no session {id}"))),
            },
        },
        _ => (!claims.has(ADMIN))
            .then(|| (StatusCode::FORBIDDEN, "org token: needs admin".to_owned())),
    };
    if let Some(error) = refused {
        return error.into_response();
    }
    request.extensions_mut().insert(claims);
    next.run(request).await
}

/// The largest request body the gateway takes (`AGENTD_MAX_REQUEST_MB`,
/// default 2 MB).
/// How a control plane session joins its shard, beyond its agent.
#[derive(Debug, Clone, Default)]
pub struct Joining {
    /// Its Memory mounts ([`crate::platform::Platform::memory`]).
    pub memory: Vec<String>,
    /// The control plane's servers it calls
    /// ([`crate::platform::Platform::servers`]).
    pub servers: crate::platform::Servers,
    /// A moved v1 session being imported: closed to messages until the
    /// control plane activates it (`state::Command::Importing`).
    pub importing: bool,
    /// The session that started it, when it is a subagent
    /// ([`crate::platform::Platform::origin`]).
    pub subagent: Option<everruns_contracts::dataplane::SubagentOrigin>,
    /// The person who owns it ([`crate::platform::Platform::owner`]).
    pub owner: Option<String>,
}

pub(crate) fn max_request_bytes() -> usize {
    std::env::var("AGENTD_MAX_REQUEST_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2)
        * 1024
        * 1024
}

pub(crate) fn parse(session: &str) -> Result<SessionId, ApiError> {
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
    #[serde(default)]
    agent: Option<Value>,
    /// Instead of `agent`: an agent of `org` by id, at its current
    /// revision from the control plane's config (see `gateway_dp`). The
    /// session is billed to the org.
    #[serde(default)]
    org: Option<String>,
    #[serde(default)]
    agent_ref: Option<String>,
    #[serde(default)]
    shard: Option<ShardId>,
    #[serde(default)]
    id: Option<SessionId>,
    /// The tenant the session's spend is billed to (see `allowance`).
    #[serde(default)]
    tenant: Option<String>,
}

async fn create(
    State(gateway): State<Arc<Gateway>>,
    claims: Option<axum::Extension<crate::org_token::OrgTokenClaims>>,
    Json(mut body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    use crate::org_token::{ADMIN, OrgTokenClaimsExt};
    // A token's org is the session's tenant, unless the caller is admin.
    if let Some(axum::Extension(claims)) = &claims
        && !claims.has(ADMIN)
    {
        let mine = |named: &Option<String>| named.as_ref().is_none_or(|o| *o == claims.org);
        if !mine(&body.org) || !mine(&body.tenant) {
            return Err((
                StatusCode::FORBIDDEN,
                "org token: a session of another org".into(),
            ));
        }
        body.tenant = Some(claims.org.clone());
        if body.agent_ref.is_some() {
            body.org = Some(claims.org.clone());
        }
    }
    let (definition, tenant) = match (body.agent, body.org, body.agent_ref) {
        (Some(agent), _, None) => {
            // THREAT[TM-TENANT]: an org's credentials are filled per call
            // for whatever its sessions' definitions name (`mcp.rs`), so a
            // definition that names them comes from the control plane only
            // (by reference, or its own admin calls). A member writing their
            // own could point another agent's credential at their server.
            if let Some(axum::Extension(claims)) = &claims
                && !claims.has(ADMIN)
                && crate::mcp::names_org_credentials(&agent)
            {
                return Err((
                    StatusCode::FORBIDDEN,
                    "org token: a definition naming the org's credentials comes from the control plane"
                        .into(),
                ));
            }
            (agent, body.tenant)
        }
        (None, Some(org), Some(agent)) => {
            let revision = gateway.agent_revision(&org, &agent).await?;
            (revision.package, Some(body.tenant.unwrap_or(org)))
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "give `agent`, or `org` and `agent_ref`".into(),
            ));
        }
    };
    Ok((
        StatusCode::CREATED,
        Json(
            gateway
                .create(definition, body.shard, body.id, tenant)
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
        .with_shard(id, |shard| {
            let gw = gw.clone();
            async move { gw.call(shard, &format!("/shard/sessions/{id}"), None).await }
        })
        .await?;
    Ok(Json(view))
}

/// `GET /v1/tools`: the tools nodes offer, ready to copy into an agent
/// definition's `tools`. Every node of a fleet runs the same build.
async fn tools(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    let directory = gateway.directory_shard()?;
    Ok(Json(gateway.call(directory, "/shard/tools", None).await?))
}

/// `GET /v1/sessions/{id}/agent`: the agent definition the session runs.
async fn agent(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = parse(&session)?;
    let gw = gateway.clone();
    let definition = gateway
        .with_shard(id, |shard| {
            let gw = gw.clone();
            async move {
                gw.call(shard, &format!("/shard/sessions/{id}/agent"), None)
                    .await
            }
        })
        .await?;
    Ok(Json(definition))
}

async fn move_session(
    State(gateway): State<Arc<Gateway>>,
    Path(session): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let session = parse(&session)?;
    let to = body["shard"]
        .as_u64()
        .ok_or((StatusCode::BAD_REQUEST, "shard required".to_string()))? as ShardId;
    Ok(Json(gateway.move_session(session, to).await?))
}

/// `POST /v1/shards/{shard}/nodes {"node": 4, "addr": "host:port"}`: add a
/// started node (run with `--join`) to a running shard.
async fn add_node(
    State(gateway): State<Arc<Gateway>>,
    Path(shard): Path<ShardId>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        gateway.call(shard, "/shard/members", Some(&body)).await?,
    ))
}

/// `POST /v1/shards/{shard}/drain`: stop placing new sessions in `shard` and
/// let the rebalancer move its sessions, running turns included, to the
/// other shards. Returns the shard's load; drained when `sessions` is 0.
async fn drain_shard(
    State(gateway): State<Arc<Gateway>>,
    Path(shard): Path<ShardId>,
) -> Result<Json<Value>, ApiError> {
    gateway.set_draining(shard, true).await
}

/// `DELETE /v1/shards/{shard}/drain`: take the shard back into placement.
async fn undrain_shard(
    State(gateway): State<Arc<Gateway>>,
    Path(shard): Path<ShardId>,
) -> Result<Json<Value>, ApiError> {
    gateway.set_draining(shard, false).await
}

/// `DELETE /v1/shards/{shard}`: retire a drained shard. Refused (409) while
/// it still holds sessions. Afterwards gateways stop routing to it, the
/// scraper stops scraping it, and its nodes can be shut down.
async fn retire_shard(
    State(gateway): State<Arc<Gateway>>,
    Path(shard): Path<ShardId>,
) -> Result<Json<Value>, ApiError> {
    let directory = gateway.directory_shard()?;
    let load: crate::api::ShardLoad = gateway.call(shard, "/shard/load", None).await?;
    let slots = gateway
        .registry
        .lock()
        .slots
        .values()
        .filter(|c| **c == shard)
        .count();
    if slots > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!("shard {shard} still holds {slots} directory slots; drain it first"),
        ));
    }
    if load.sessions > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "shard {shard} still holds {} sessions; drain it first",
                load.sessions
            ),
        ));
    }
    match gateway
        .propose(directory, &Command::RetireShard { shard })
        .await?
    {
        Response::Ok | Response::Duplicate => {
            gateway.refresh_registry().await;
            Ok(Json(json!({ "shard": shard, "retired": true })))
        }
        other => Err((StatusCode::CONFLICT, format!("{other:?}"))),
    }
}

/// `DELETE /v1/shards/{shard}/nodes/{node}`: take a node out of a shard.
async fn remove_node(
    State(gateway): State<Arc<Gateway>>,
    Path((shard, node)): Path<(ShardId, crate::types::NodeId)>,
) -> Result<Json<Value>, ApiError> {
    let nodes = gateway.nodes(shard)?;
    let mut last = String::new();
    for _ in 0..50 {
        for addr in &nodes {
            match gateway
                .http
                .delete(format!("http://{addr}/shard/members/{node}"))
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
/// every shard, labelled, so a scraper follows the fleet as nodes and shards
/// come and go.
async fn targets(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    // No target labels: every series already carries `shard` (and per-node
    // ones `node`), and a `node` label on a shard's leader-only series would
    // split it at every leader change.
    let groups: Vec<Value> = gateway
        .topology()?
        .shards
        .values()
        .flat_map(|nodes| nodes.values())
        .map(|addr| json!({ "targets": [addr], "labels": {} }))
        .collect();
    Ok(Json(json!(groups)))
}

async fn shards(State(gateway): State<Arc<Gateway>>) -> Result<Json<Value>, ApiError> {
    let topology = gateway.topology()?;
    let mut out = BTreeMap::new();
    for (shard, nodes) in &topology.shards {
        let mut stats = Vec::new();
        for addr in nodes.values() {
            let url = format!("http://{addr}/shard/stats");
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
        out.insert(shard.to_string(), stats);
    }
    Ok(Json(json!(out)))
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
        .with_shard(id, |shard| {
            let gw = gw.clone();
            async move {
                gw.call_within(
                    shard,
                    &format!("/shard/sessions/{id}/wait?turns={turns}&timeout_ms={timeout}"),
                    None,
                    Duration::from_millis(timeout) + Duration::from_secs(5),
                )
                .await
            }
        })
        .await?;
    Ok(Json(view))
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;
