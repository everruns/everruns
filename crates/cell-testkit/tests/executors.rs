//! A thin, complete cell deployment (data plane v2 step 6): a three-node
//! cell and a one-node cell whose turns run on executors outside them, each
//! node and executor in its own runtime so a test can kill it outright.
//!
//! What it shows: turns run on executors, never on the nodes; a cancel
//! reaches a turn on an executor; a killed executor's turn finishes on
//! another; a turn survives its cell's leader dying; live deltas reach SSE
//! from an executor; the workspace survives executors and a move.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use everruns_cell::cluster::{Topology, TopologySource};
use everruns_cell::executor::{Executor, ExecutorConfig};
use everruns_cell::node::{ExecutorMode, NodeConfig};
use everruns_cell::remote::EXECUTOR_IDS;
use everruns_cell::store::Store;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

struct Fleet {
    dir: tempfile::TempDir,
    topology: TopologySource,
    /// (cell, node) -> its address and runtime (None once killed).
    nodes: BTreeMap<(u32, u64), (String, Option<Runtime>)>,
    executors: BTreeMap<u64, (Arc<Executor>, Option<Runtime>)>,
    http: reqwest::Client,
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
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut topology = Topology {
            directory_cell: 0,
            cells: BTreeMap::new(),
        };
        for (cell, size) in [(0u32, 3u64), (1, 1)] {
            let members = (1..=size)
                .map(|node| (node, format!("127.0.0.1:{}", free_port())))
                .collect();
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
                };
                let rt = runtime();
                let host = Arc::new(everruns_cell_testkit::host());
                let listen = addr.parse().unwrap();
                let topology = source.clone();
                rt.spawn(async move {
                    everruns_cell::cli::serve_node(cfg, host, topology, listen, false).await
                });
                fleet.nodes.insert((*cell, *node), (addr.clone(), Some(rt)));
            }
        }
        for cell in [0, 1] {
            fleet.leader(cell).await;
        }
        fleet
    }

    fn store(&self) -> Store {
        Store::open(&format!(
            "file://{}",
            self.dir.path().join("store").display()
        ))
        .unwrap()
    }

    /// Start executor `n` (ids from `EXECUTOR_IDS`).
    fn executor(&mut self, n: u64) -> Arc<Executor> {
        let rt = runtime();
        let cfg = ExecutorConfig {
            id: EXECUTOR_IDS + n,
            topology: self.topology.clone(),
            store: self.store().with_cache(16 << 20),
            slots: 8,
            mirror_sessions: 64,
            pull_wait: Duration::from_secs(2),
        };
        let executor = {
            let _guard = rt.enter();
            let executor =
                Executor::start(cfg, Arc::new(everruns_cell_testkit::host())).unwrap();
            executor.spawn();
            executor
        };
        self.executors.insert(n, (executor.clone(), Some(rt)));
        executor
    }

    /// Kill executor `n` outright: its tasks stop mid-step, nothing hands off.
    fn kill_executor(&mut self, n: u64) {
        if let Some(rt) = self.executors.get_mut(&n).and_then(|(_, rt)| rt.take()) {
            rt.shutdown_background();
        }
    }

    fn kill_node(&mut self, cell: u32, node: u64) {
        if let Some(rt) = self.nodes.get_mut(&(cell, node)).and_then(|(_, rt)| rt.take()) {
            rt.shutdown_background();
        }
    }

    /// A live node's base URL; its gateway reaches every cell.
    fn base(&self) -> String {
        let (addr, _) = self
            .nodes
            .values()
            .find(|(_, rt)| rt.is_some())
            .expect("a live node");
        format!("http://{addr}")
    }

    /// The node id leading `cell`, once it has one.
    async fn leader(&self, cell: u32) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
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

    async fn post(&self, path: &str, body: Value) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
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

    async fn get(&self, path: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
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

    async fn create(&self, agent: &str) -> String {
        let definition = everruns_cell_testkit::definition(agent).unwrap();
        let created = self
            .post("/v1/sessions", json!({ "agent": definition }))
            .await;
        created["id"].as_str().unwrap().to_string()
    }

    async fn create_in(&self, agent: &str, cell: u32) -> String {
        let definition = everruns_cell_testkit::definition(agent).unwrap();
        let created = self
            .post("/v1/sessions", json!({ "agent": definition, "cell": cell }))
            .await;
        created["id"].as_str().unwrap().to_string()
    }

    /// Admit `text`; the admitted message's id.
    async fn say(&self, session: &str, text: &str) -> Value {
        self.post(
            &format!("/v1/sessions/{session}/messages"),
            json!({ "text": text }),
        )
        .await
    }

    /// Wait until `session` completed `turns` turns.
    async fn turns(&self, session: &str, turns: u64) {
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

    async fn events(&self, session: &str) -> Vec<Value> {
        self.get(&format!("/v1/sessions/{session}/events"))
            .await["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    /// Wait until an executor holds `session`; its id.
    async fn held(&self, session: &str) -> u64 {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let view = self.get(&format!("/v1/sessions/{session}")).await;
            if let Some(node) = view["active"]["node"].as_u64() {
                return node;
            }
            assert!(Instant::now() < deadline, "nobody took {session}: {view}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn types(events: &[Value]) -> Vec<&str> {
    events.iter().filter_map(|e| e["type"].as_str()).collect()
}

/// The agent's answers, in order.
fn answers(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter(|e| e["data"]["message"]["role"] == "agent")
        .filter_map(|e| e["data"]["message"]["content"][0]["text"].as_str())
        .map(str::to_string)
        .collect()
}

fn pulled(executor: &Executor) -> u64 {
    executor.stats.get(&[("event", "pulled")])
}

#[tokio::test(flavor = "multi_thread")]
async fn turns_run_on_executors_not_nodes() {
    let mut fleet = Fleet::start().await;
    let executor = fleet.executor(1);
    let session = fleet.create("worker").await;
    fleet.say(&session, "go").await;
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"tool.completed"), "{kinds:?}");
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
    assert!(pulled(&executor) >= 1, "the executor pulled the turn");

    // A second turn of the same session goes back to the warm executor.
    fleet.say(&session, "again").await;
    fleet.turns(&session, 2).await;
    assert!(pulled(&executor) >= 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_reaches_a_turn_on_an_executor() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("slow").await;
    let admitted = fleet.say(&session, "go").await;
    let holder = fleet.held(&session).await;
    assert!(holder >= EXECUTOR_IDS, "held by an executor, not a node");
    let message = admitted["id"].as_str().unwrap();
    fleet
        .post(
            &format!("/v1/sessions/{session}/messages/{message}/cancel"),
            json!({}),
        )
        .await;
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"turn.cancelled"), "{kinds:?}");
    assert!(!kinds.contains(&"turn.completed"), "{kinds:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_killed_executors_turn_finishes_on_another() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("slow").await;
    fleet.say(&session, "go").await;
    assert_eq!(fleet.held(&session).await, EXECUTOR_IDS + 1);
    let survivor = fleet.executor(2);
    fleet.kill_executor(1);
    fleet.turns(&session, 1).await;
    let events = fleet.events(&session).await;
    let kinds = types(&events);
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
    assert!(pulled(&survivor) >= 1, "the survivor resumed the turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_survives_its_cells_leader_dying() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("slow", 0).await;
    fleet.say(&session, "go").await;
    fleet.held(&session).await;
    let leader = fleet.leader(0).await;
    fleet.kill_node(0, leader);
    let next = fleet.leader(0).await;
    assert_ne!(next, leader);
    fleet.turns(&session, 1).await;
    let kinds_events = fleet.events(&session).await;
    let kinds = types(&kinds_events);
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn live_deltas_from_an_executor_reach_sse() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create("chat").await;
    let url = format!("{}/v1/sessions/{session}/sse", fleet.base());
    let mut stream = fleet.http.get(&url).send().await.unwrap();
    fleet.say(&session, "tell me").await;
    let mut seen = String::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !seen.contains("turn.completed") && Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), stream.chunk()).await {
            Ok(Ok(Some(chunk))) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            _ => break,
        }
    }
    assert!(seen.contains("turn.completed"), "{seen}");
    assert!(
        seen.contains("event: output.message.delta"),
        "a live delta streamed: {seen}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_survives_executors_and_a_move() {
    let mut fleet = Fleet::start().await;
    fleet.executor(1);
    let session = fleet.create_in("files", 0).await;
    fleet.say(&session, "write /notes/plan.md step one").await;
    fleet.turns(&session, 1).await;
    // Larger than an inline entry: its content goes to the object store.
    let big = "lorem ipsum ".repeat(2_000);
    fleet
        .say(&session, &format!("write /notes/big.md {big}tail-marker"))
        .await;
    fleet.turns(&session, 2).await;

    // Another executor, in another cell.
    fleet.kill_executor(1);
    fleet.executor(2);
    let moved = fleet
        .post(&format!("/v1/sessions/{session}/move"), json!({ "cell": 1 }))
        .await;
    assert_eq!(moved["to"], 1, "{moved}");

    fleet.say(&session, "read /notes/plan.md").await;
    fleet.turns(&session, 3).await;
    fleet.say(&session, "read /notes/big.md").await;
    fleet.turns(&session, 4).await;
    let events = fleet.events(&session).await;
    let answers = answers(&events);
    assert_eq!(answers.len(), 4, "{answers:?}");
    assert!(answers[2].contains("step one"), "{}", answers[2]);
    assert!(answers[3].contains("tail-marker"), "the big file came back whole");
    assert!(
        answers.iter().all(|a| !a.contains("error")),
        "{answers:?}"
    );
}
