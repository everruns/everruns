//! A local fleet for integration tests: shards of nodes and executors,
//! each in its own runtime so a test can kill it outright, optionally
//! reporting to a fake control plane.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

pub mod org;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use everruns_agentd::cluster::{Topology, TopologySource};
use everruns_agentd::executor::{Executor, ExecutorConfig};
use everruns_agentd::node::{ExecutorMode, NodeConfig};
use everruns_agentd::remote::EXECUTOR_IDS;
use everruns_agentd::store::Store;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpSocket};
use tokio::runtime::Runtime;

/// A socket that may share its port with one other bound the same way:
/// a node's listener beside the fleet's reservation of that port.
fn shareable() -> TcpSocket {
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_reuseaddr(true).unwrap();
    // Linux lets two SO_REUSEADDR sockets bind one port while at most one
    // listens; BSDs (macOS) need SO_REUSEPORT for that.
    #[cfg(all(unix, not(target_os = "linux")))]
    socket.set_reuseport(true).unwrap();
    socket
}

/// Hold a fresh local port for a node for as long as the fleet lives.
///
/// Decision: the port is reserved by a bound socket that never listens,
/// so connecting while its node is down is refused, as with a dead
/// process, and nobody else can take the port meanwhile. Picking a port
/// by binding `:0` and dropping it (what this harness did before) raced:
/// two nodes of a fleet could draw the same port, or a killed node's port
/// could go elsewhere before its restart, and the node that lost sat out
/// the whole test while the fleet waited on it ("shard 1 elected no
/// leader").
fn reserve() -> TcpSocket {
    let socket = shareable();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    socket
}

/// A node's port and, with a Raft port offset, its Raft port as well.
fn reserve_with(offset: u16) -> (TcpSocket, Option<TcpSocket>) {
    loop {
        let port = reserve();
        if offset == 0 {
            return (port, None);
        }
        let mut raft = port.local_addr().unwrap();
        let Some(raft_port) = raft.port().checked_add(offset) else {
            continue;
        };
        raft.set_port(raft_port);
        let socket = shareable();
        if socket.bind(raft).is_ok() {
            return (port, Some(socket));
        }
    }
}

/// Listen on `listen`, a reserved port. A killed predecessor's listener
/// closes as its runtime winds down, so wait for it briefly; nobody else
/// can hold the port.
fn listen_on(listen: SocketAddr) -> TcpListener {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let socket = shareable();
        match socket.bind(listen) {
            Ok(()) => return socket.listen(1024).unwrap(),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                assert!(Instant::now() < deadline, "{listen} stayed in use");
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("bind {listen}: {error}"),
        }
    }
}

pub fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

pub struct Fleet {
    dir: tempfile::TempDir,
    topology: TopologySource,
    /// (shard, node) -> its address and runtime (None once killed).
    nodes: BTreeMap<(u32, u64), (String, Option<Runtime>)>,
    executors: BTreeMap<u64, (Arc<Executor>, Option<Runtime>)>,
    pub http: reqwest::Client,
    configs: BTreeMap<(u32, u64), NodeConfig>,
    /// Each node's port, held for the fleet's life (see [`reserve`]).
    ports: BTreeMap<(u32, u64), TcpSocket>,
    /// Each node's Raft port, when Raft has its own listener.
    raft_ports: Vec<TcpSocket>,
    /// Why nodes stopped serving on their own; any entry fails the test.
    failed: Arc<Mutex<Vec<String>>>,
    /// Set before the test stops a node, so the error its serve returns
    /// as the runtime winds down (a cancelled task) is not a failure.
    stopping: BTreeMap<(u32, u64), Arc<AtomicBool>>,
}

impl Drop for Fleet {
    fn drop(&mut self) {
        for (_, rt) in self.nodes.values_mut() {
            if let Some(rt) = rt.take() {
                rt.shutdown_background();
            }
        }
        for (_, rt) in self.executors.values_mut() {
            if let Some(rt) = rt.take() {
                rt.shutdown_background();
            }
        }
    }
}

impl Fleet {
    /// Shard 0 of three nodes and shard 1 of one, both handing turns to
    /// executors; no executor yet.
    pub async fn start() -> Self {
        Self::start_with(None).await
    }

    /// [`start`](Self::start), the shards reporting to `control_plane`.
    pub async fn start_with(control_plane: Option<String>) -> Self {
        Self::start_with_cp(control_plane.map(everruns_agentd::node_control::ControlPlane::new))
            .await
    }

    /// [`start`](Self::start), the shards linked to `control_plane` as given.
    pub async fn start_with_cp(
        control_plane: Option<everruns_agentd::node_control::ControlPlane>,
    ) -> Self {
        Self::start_opts(control_plane, 0).await
    }

    /// [`start`](Self::start), every node serving Raft on its own listener
    /// (its port plus one) and threads.
    pub async fn start_isolated() -> Self {
        Self::start_opts(None, 1).await
    }

    async fn start_opts(
        control_plane: Option<everruns_agentd::node_control::ControlPlane>,
        raft_port_offset: u16,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut topology = Topology {
            directory_shard: 0,
            shards: BTreeMap::new(),
        };
        let mut ports = BTreeMap::new();
        let mut raft_ports = Vec::new();
        for (shard, size) in [(0u32, 3u64), (1, 1)] {
            let mut members = BTreeMap::new();
            for node in 1..=size {
                let (port, raft) = reserve_with(raft_port_offset);
                members.insert(node, port.local_addr().unwrap().to_string());
                ports.insert((shard, node), port);
                raft_ports.extend(raft);
            }
            topology.shards.insert(shard, members);
        }
        let path = dir.path().join("topology.json");
        std::fs::write(&path, serde_json::to_vec(&topology).unwrap()).unwrap();
        let source = TopologySource { path };
        let mut fleet = Self {
            topology: source.clone(),
            nodes: BTreeMap::new(),
            executors: BTreeMap::new(),
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
            configs: BTreeMap::new(),
            ports,
            raft_ports,
            failed: Arc::default(),
            stopping: BTreeMap::new(),
            dir,
        };
        for (shard, members) in &topology.shards {
            for (node, addr) in members {
                let cfg = NodeConfig {
                    shard: *shard,
                    node_id: *node,
                    members: members.clone(),
                    data_dir: fleet.dir.path().join(format!("node-{shard}-{node}")),
                    store: fleet.store(),
                    memory_budget: 1 << 30,
                    // Every node shares the test process: measure against
                    // no real limit unless a test sets one.
                    memory_limit: 1 << 40,
                    raft_port_offset,
                    unload_idle: None,
                    claim_ttl: Duration::from_secs(2),
                    max_turns_per_node: 64,
                    pressure_turns: 1024,
                    snapshot_every: 100_000,
                    keep_log: 256,
                    executors: ExecutorMode::Pull { max_running: 64 },
                    control_plane: control_plane.clone(),
                };
                fleet.configs.insert((*shard, *node), cfg);
                fleet.nodes.insert((*shard, *node), (addr.clone(), None));
                fleet.start_node(*shard, *node);
            }
        }
        for shard in [0, 1] {
            fleet.leader(shard).await;
        }
        fleet
    }

    /// (Re)start a node from its data directory, on its reserved port.
    pub fn start_node(&mut self, shard: u32, node: u64) {
        let cfg = self.configs[&(shard, node)].clone();
        let (addr, slot) = self.nodes.get_mut(&(shard, node)).unwrap();
        let listen: SocketAddr = addr.parse().unwrap();
        let rt = runtime();
        let listener = {
            let _runtime = rt.enter();
            listen_on(listen)
        };
        let host = Arc::new(everruns_agentd_testkit::host());
        let topology = self.topology.clone();
        let failed = self.failed.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        self.stopping.insert((shard, node), stopping.clone());
        rt.spawn(async move {
            let served =
                everruns_agentd::cli::serve_node_on(cfg, host, topology, listener, false).await;
            if let Err(error) = served
                && !stopping.load(Ordering::SeqCst)
            {
                failed
                    .lock()
                    .unwrap()
                    .push(format!("node {shard}/{node} on {listen}: {error:#}"));
            }
        });
        *slot = Some(rt);
    }

    /// Fail now, with the reason, if a node stopped serving on its own,
    /// rather than at whatever timeout its absence runs into later.
    pub fn check(&self) {
        let failed = self.failed.lock().unwrap();
        assert!(failed.is_empty(), "the fleet lost nodes: {failed:?}");
    }

    /// `POST /shard/propose` on `shard`, any member; the shard's response.
    pub async fn propose(&self, shard: u32, command: Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.check();
            for ((c, _), (addr, rt)) in &self.nodes {
                if *c != shard || rt.is_none() {
                    continue;
                }
                if let Ok(response) = self
                    .http
                    .post(format!("http://{addr}/shard/propose"))
                    .json(&command)
                    .send()
                    .await
                    && response.status().is_success()
                {
                    let proposed: Value = response.json().await.unwrap();
                    return proposed["response"].clone();
                }
            }
            assert!(
                Instant::now() < deadline,
                "no member of shard {shard} took {command}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// `/shard/stats` of `shard`'s leader.
    pub async fn stats(&self, shard: u32) -> Value {
        let leader = self.leader(shard).await;
        let (addr, _) = &self.nodes[&(shard, leader)];
        self.http
            .get(format!("http://{addr}/shard/stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// Wait until `check` holds, or fail with `what`.
    pub async fn until<F, Fut>(&self, what: &str, check: F)
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !check().await {
            self.check();
            assert!(Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub fn store(&self) -> Store {
        Store::open(&format!(
            "file://{}",
            self.dir.path().join("store").display()
        ))
        .unwrap()
    }

    /// Start executor `n` (ids from `EXECUTOR_IDS`).
    pub fn executor(&mut self, n: u64) -> Arc<Executor> {
        self.executor_with(n, 8)
    }

    /// Executor `n`, running up to `slots` turns at once.
    pub fn executor_with(&mut self, n: u64, slots: usize) -> Arc<Executor> {
        self.executor_limited(n, slots, 0)
    }

    /// Executor `n` that admits turns against a memory limit of `limit`
    /// bytes (measured as this process's resident set).
    pub fn executor_limited(&mut self, n: u64, slots: usize, limit: u64) -> Arc<Executor> {
        let rt = runtime();
        let cfg = ExecutorConfig {
            id: EXECUTOR_IDS + n,
            topology: self.topology.clone(),
            store: self.store().with_cache(16 << 20),
            slots,
            mirror_sessions: 64,
            pull_wait: Duration::from_secs(2),
            dream_sweep: Duration::from_millis(500),
            memory_limit: limit,
        };
        let executor = {
            let _guard = rt.enter();
            let executor = Executor::start(cfg, Arc::new(everruns_agentd_testkit::host())).unwrap();
            executor.spawn();
            executor
        };
        self.executors.insert(n, (executor.clone(), Some(rt)));
        executor
    }

    /// Kill executor `n` outright: its tasks stop mid-step, nothing hands off.
    pub fn kill_executor(&mut self, n: u64) {
        if let Some(rt) = self.executors.get_mut(&n).and_then(|(_, rt)| rt.take()) {
            rt.shutdown_background();
        }
    }

    /// Restart a node with its memory pressure measured against `bytes`.
    pub fn restart_with_memory_limit(&mut self, shard: u32, node: u64, bytes: u64) {
        self.restart_with(shard, node, |cfg| cfg.memory_limit = bytes);
    }

    /// Restart a node with its configuration changed by `change`.
    pub fn restart_with(&mut self, shard: u32, node: u64, change: impl FnOnce(&mut NodeConfig)) {
        self.kill_node(shard, node);
        if let Some(cfg) = self.configs.get_mut(&(shard, node)) {
            change(cfg);
        }
        self.start_node(shard, node);
    }

    /// POST without retrying: the status, the `Retry-After` header and the
    /// body (JSON, or the text as a string).
    pub async fn post_once(&self, path: &str, body: Value) -> (u16, Option<String>, Value) {
        let response = self
            .http
            .post(format!("{}{path}", self.base()))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let retry = response
            .headers()
            .get("retry-after")
            .map(|v| v.to_str().unwrap().to_string());
        let text = response.text().await.unwrap_or_default();
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, retry, body)
    }

    pub fn kill_node(&mut self, shard: u32, node: u64) {
        if let Some(stopping) = self.stopping.get(&(shard, node)) {
            stopping.store(true, Ordering::SeqCst);
        }
        if let Some(rt) = self
            .nodes
            .get_mut(&(shard, node))
            .and_then(|(_, rt)| rt.take())
        {
            rt.shutdown_background();
        }
    }

    /// Lose every node of `shard`, disks and all, then rebuild it from the
    /// object store as of `at_ms` (its newest without one) on new nodes at
    /// the same addresses, and wait for a leader.
    pub async fn rebuild(
        &mut self,
        shard: u32,
        at_ms: Option<i64>,
    ) -> everruns_agentd::restore::Rebuilt {
        self.rebuild_from(shard, at_ms, None).await
    }

    /// Lose two zones of `shard`: every node stops, and only `survivor`'s
    /// disk is left. Rebuild from the object store plus that disk on new
    /// nodes at the same addresses, and wait for a leader.
    pub async fn rebuild_with_survivor(
        &mut self,
        shard: u32,
        survivor: u64,
    ) -> everruns_agentd::restore::Rebuilt {
        self.rebuild_from(shard, None, Some(survivor)).await
    }

    async fn rebuild_from(
        &mut self,
        shard: u32,
        at_ms: Option<i64>,
        survivor: Option<u64>,
    ) -> everruns_agentd::restore::Rebuilt {
        let nodes: Vec<u64> = self
            .configs
            .keys()
            .filter(|(c, _)| *c == shard)
            .map(|(_, n)| *n)
            .collect();
        for node in &nodes {
            self.kill_node(shard, *node);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (mut members, mut local) = (BTreeMap::new(), BTreeMap::new());
        let mut kept = None;
        for node in &nodes {
            let dir = self.configs[&(shard, *node)].data_dir.clone();
            if dir.exists() {
                let suffix = if survivor == Some(*node) {
                    "survivor"
                } else {
                    "lost"
                };
                let aside = dir.with_extension(format!("{suffix}-{}", now_ms()));
                std::fs::rename(&dir, &aside).unwrap();
                if survivor == Some(*node) {
                    kept = Some(aside);
                }
            }
            members.insert(*node, self.nodes[&(shard, *node)].0.clone());
            local.insert(*node, dir);
        }
        let rebuilt = everruns_agentd::restore::rebuild(
            &self.store(),
            shard,
            &members,
            &local,
            at_ms,
            kept.as_deref(),
        )
        .await
        .unwrap();
        for node in &nodes {
            self.start_node(shard, *node);
        }
        self.leader(shard).await;
        rebuilt
    }

    /// The fleet's topology file.
    pub fn topology(&self) -> TopologySource {
        self.topology.clone()
    }

    /// `shard`'s live nodes: (node id, address).
    pub fn addrs(&self, shard: u32) -> Vec<(u64, String)> {
        self.nodes
            .iter()
            .filter(|((c, _), (_, rt))| *c == shard && rt.is_some())
            .map(|((_, node), (addr, _))| (*node, addr.clone()))
            .collect()
    }

    /// Every live node's address.
    pub fn bases(&self) -> Vec<String> {
        self.nodes
            .values()
            .filter(|(_, rt)| rt.is_some())
            .map(|(addr, _)| format!("http://{addr}"))
            .collect()
    }

    /// A live node's base URL; its gateway reaches every shard.
    pub fn base(&self) -> String {
        let (addr, _) = self
            .nodes
            .values()
            .find(|(_, rt)| rt.is_some())
            .expect("a live node");
        format!("http://{addr}")
    }

    /// The node id leading `shard`, once it has one.
    pub async fn leader(&self, shard: u32) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            self.check();
            for ((c, node), (addr, rt)) in &self.nodes {
                if *c != shard || rt.is_none() {
                    continue;
                }
                if let Ok(response) = self
                    .http
                    .get(format!("http://{addr}/shard/stats"))
                    .send()
                    .await
                    && let Ok(stats) = response.json::<Value>().await
                    && stats["leader"].as_u64() == Some(*node)
                {
                    return *node;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("shard {shard} elected no leader");
    }

    pub async fn post(&self, path: &str, body: Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.check();
            let url = format!("{}{path}", self.base());
            match self.http.post(&url).json(&body).send().await {
                Ok(response) if response.status().is_success() => {
                    return response.json().await.unwrap();
                }
                Ok(response) if Instant::now() > deadline => {
                    panic!("POST {path}: {}", response.text().await.unwrap_or_default())
                }
                Err(error) if Instant::now() > deadline => panic!("POST {path}: {error}"),
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }

    pub async fn get(&self, path: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.check();
            let url = format!("{}{path}", self.base());
            match self.http.get(&url).send().await {
                Ok(response) if response.status().is_success() => {
                    return response.json().await.unwrap();
                }
                _ if Instant::now() > deadline => panic!("GET {path} kept failing"),
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }

    pub async fn create(&self, agent: &str) -> String {
        let definition = everruns_agentd_testkit::definition(agent).unwrap();
        let created = self
            .post("/v1/sessions", json!({ "agent": definition }))
            .await;
        created["id"].as_str().unwrap().to_string()
    }

    pub async fn create_in(&self, agent: &str, shard: u32) -> String {
        let definition = everruns_agentd_testkit::definition(agent).unwrap();
        let created = self
            .post(
                "/v1/sessions",
                json!({ "agent": definition, "shard": shard }),
            )
            .await;
        created["id"].as_str().unwrap().to_string()
    }

    /// Admit `text`; the admitted message's id.
    pub async fn say(&self, session: &str, text: &str) -> Value {
        self.post(
            &format!("/v1/sessions/{session}/messages"),
            json!({ "text": text }),
        )
        .await
    }

    /// Wait until `session` completed `turns` turns.
    pub async fn turns(&self, session: &str, turns: u64) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let view = self
                .get(&format!(
                    "/v1/sessions/{session}/wait?turns={turns}&timeout_ms=5000"
                ))
                .await;
            if view["turns_completed"].as_u64() >= Some(turns) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{session} did not finish turn {turns}: {view}"
            );
        }
    }

    pub async fn events(&self, session: &str) -> Vec<Value> {
        self.get(&format!("/v1/sessions/{session}/events")).await["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Wait until an executor holds `session`; its id.
    pub async fn held(&self, session: &str) -> u64 {
        self.holder(session).await["node"].as_u64().unwrap()
    }

    /// Wait until an executor holds `session`; the fence of its epoch.
    pub async fn fence(&self, session: &str) -> u64 {
        self.holder(session).await["fence"].as_u64().unwrap()
    }

    /// Wait until an executor holds `session`; its epoch (`node`, `fence`).
    ///
    /// Decision: take every field of the epoch from one view. The gateway
    /// spreads reads over the shard's members and each answers from its own
    /// applied state, so a second GET can reach a follower that has not
    /// applied the claim the first one saw, and find no epoch at all.
    pub async fn holder(&self, session: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let view = self.get(&format!("/v1/sessions/{session}")).await;
            if view["active"]["node"].is_u64() {
                return view["active"].clone();
            }
            assert!(Instant::now() < deadline, "nobody took {session}: {view}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

pub fn types(events: &[Value]) -> Vec<&str> {
    events.iter().filter_map(|e| e["type"].as_str()).collect()
}

/// The agent's answers, in order.
pub fn answers(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["data"]["message"]["role"] == "agent")
        .filter_map(|e| e["data"]["message"]["content"][0]["text"].as_str())
        .map(str::to_string)
        .collect()
}

pub fn pulled(executor: &Executor) -> u64 {
    executor.stats.get(&[("event", "pulled")])
}

/// Wall-clock milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
