//! A local fleet for integration tests: cells of nodes and executors,
//! each in its own runtime so a test can kill it outright, optionally
//! reporting to a fake control plane.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use everruns_cell::cluster::{Topology, TopologySource};
use everruns_cell::executor::{Executor, ExecutorConfig};
use everruns_cell::node::{ExecutorMode, NodeConfig};
use everruns_cell::remote::EXECUTOR_IDS;
use everruns_cell::store::Store;
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
/// the whole test while the fleet waited on it ("cell 1 elected no
/// leader").
fn reserve() -> TcpSocket {
    let socket = shareable();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    socket
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
    /// (cell, node) -> its address and runtime (None once killed).
    nodes: BTreeMap<(u32, u64), (String, Option<Runtime>)>,
    executors: BTreeMap<u64, (Arc<Executor>, Option<Runtime>)>,
    pub http: reqwest::Client,
    configs: BTreeMap<(u32, u64), NodeConfig>,
    /// Each node's port, held for the fleet's life (see [`reserve`]).
    ports: BTreeMap<(u32, u64), TcpSocket>,
    /// Why nodes stopped serving on their own; any entry fails the test.
    failed: Arc<Mutex<Vec<String>>>,
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
    /// Cell 0 of three nodes and cell 1 of one, both handing turns to
    /// executors; no executor yet.
    pub async fn start() -> Self {
        Self::start_with(None).await
    }

    /// [`start`](Self::start), the cells reporting to `control_plane`.
    pub async fn start_with(control_plane: Option<String>) -> Self {
        Self::start_with_cp(control_plane.map(everruns_cell::node_control::ControlPlane::new)).await
    }

    /// [`start`](Self::start), the cells linked to `control_plane` as given.
    pub async fn start_with_cp(
        control_plane: Option<everruns_cell::node_control::ControlPlane>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut topology = Topology {
            directory_cell: 0,
            cells: BTreeMap::new(),
        };
        let mut ports = BTreeMap::new();
        for (cell, size) in [(0u32, 3u64), (1, 1)] {
            let mut members = BTreeMap::new();
            for node in 1..=size {
                let port = reserve();
                members.insert(node, port.local_addr().unwrap().to_string());
                ports.insert((cell, node), port);
            }
            topology.cells.insert(cell, members);
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
            failed: Arc::default(),
            dir,
        };
        for (cell, members) in &topology.cells {
            for (node, addr) in members {
                let cfg = NodeConfig {
                    cell: *cell,
                    node_id: *node,
                    members: members.clone(),
                    data_dir: fleet.dir.path().join(format!("node-{cell}-{node}")),
                    store: fleet.store(),
                    memory_budget: 1 << 30,
                    unload_idle: None,
                    claim_ttl: Duration::from_secs(2),
                    max_turns_per_node: 64,
                    snapshot_every: 100_000,
                    keep_log: 256,
                    executors: ExecutorMode::Pull { max_running: 64 },
                    control_plane: control_plane.clone(),
                };
                fleet.configs.insert((*cell, *node), cfg);
                fleet.nodes.insert((*cell, *node), (addr.clone(), None));
                fleet.start_node(*cell, *node);
            }
        }
        for cell in [0, 1] {
            fleet.leader(cell).await;
        }
        fleet
    }

    /// (Re)start a node from its data directory, on its reserved port.
    pub fn start_node(&mut self, cell: u32, node: u64) {
        let cfg = self.configs[&(cell, node)].clone();
        let (addr, slot) = self.nodes.get_mut(&(cell, node)).unwrap();
        let listen: SocketAddr = addr.parse().unwrap();
        let rt = runtime();
        let listener = {
            let _runtime = rt.enter();
            listen_on(listen)
        };
        let host = Arc::new(everruns_cell_testkit::host());
        let topology = self.topology.clone();
        let failed = self.failed.clone();
        rt.spawn(async move {
            let served =
                everruns_cell::cli::serve_node_on(cfg, host, topology, listener, false).await;
            if let Err(error) = served {
                failed
                    .lock()
                    .unwrap()
                    .push(format!("node {cell}/{node} on {listen}: {error:#}"));
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

    /// `POST /cell/propose` on `cell`, any member; the cell's response.
    pub async fn propose(&self, cell: u32, command: Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.check();
            for ((c, _), (addr, rt)) in &self.nodes {
                if *c != cell || rt.is_none() {
                    continue;
                }
                if let Ok(response) = self
                    .http
                    .post(format!("http://{addr}/cell/propose"))
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
                "no member of cell {cell} took {command}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// `/cell/stats` of `cell`'s leader.
    pub async fn stats(&self, cell: u32) -> Value {
        let leader = self.leader(cell).await;
        let (addr, _) = &self.nodes[&(cell, leader)];
        self.http
            .get(format!("http://{addr}/cell/stats"))
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
        let rt = runtime();
        let cfg = ExecutorConfig {
            id: EXECUTOR_IDS + n,
            topology: self.topology.clone(),
            store: self.store().with_cache(16 << 20),
            slots,
            mirror_sessions: 64,
            pull_wait: Duration::from_secs(2),
        };
        let executor = {
            let _guard = rt.enter();
            let executor = Executor::start(cfg, Arc::new(everruns_cell_testkit::host())).unwrap();
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

    pub fn kill_node(&mut self, cell: u32, node: u64) {
        if let Some(rt) = self
            .nodes
            .get_mut(&(cell, node))
            .and_then(|(_, rt)| rt.take())
        {
            rt.shutdown_background();
        }
    }

    /// The fleet's topology file.
    pub fn topology(&self) -> TopologySource {
        self.topology.clone()
    }

    /// `cell`'s live nodes: (node id, address).
    pub fn addrs(&self, cell: u32) -> Vec<(u64, String)> {
        self.nodes
            .iter()
            .filter(|((c, _), (_, rt))| *c == cell && rt.is_some())
            .map(|((_, node), (addr, _))| (*node, addr.clone()))
            .collect()
    }

    /// A live node's base URL; its gateway reaches every cell.
    pub fn base(&self) -> String {
        let (addr, _) = self
            .nodes
            .values()
            .find(|(_, rt)| rt.is_some())
            .expect("a live node");
        format!("http://{addr}")
    }

    /// The node id leading `cell`, once it has one.
    pub async fn leader(&self, cell: u32) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            self.check();
            for ((c, node), (addr, rt)) in &self.nodes {
                if *c != cell || rt.is_none() {
                    continue;
                }
                if let Ok(response) = self
                    .http
                    .get(format!("http://{addr}/cell/stats"))
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
        panic!("cell {cell} elected no leader");
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
        let definition = everruns_cell_testkit::definition(agent).unwrap();
        let created = self
            .post("/v1/sessions", json!({ "agent": definition }))
            .await;
        created["id"].as_str().unwrap().to_string()
    }

    pub async fn create_in(&self, agent: &str, cell: u32) -> String {
        let definition = everruns_cell_testkit::definition(agent).unwrap();
        let created = self
            .post("/v1/sessions", json!({ "agent": definition, "cell": cell }))
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
    /// spreads reads over the cell's members and each answers from its own
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
