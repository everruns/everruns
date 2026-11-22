//! Spend allowances end to end on a one-node shard: a metered session runs
//! only the provider calls its tenant's grant covers, through the Engine's
//! own budget gate, and the shard's ledger shows every reservation closed.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use everruns_agentd::cluster::{Topology, TopologySource};
use everruns_agentd::node::NodeConfig;
use everruns_agentd::store::Store;
use serde_json::{Value, json};

struct Shard {
    base: String,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl Shard {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        // Bound before the topology names it, and kept: a port picked and
        // released first could be someone else's by the time the node binds.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let members = BTreeMap::from([(1, addr.clone())]);
        let topology = Topology {
            directory_shard: 0,
            shards: BTreeMap::from([(0, members.clone())]),
        };
        let path = dir.path().join("topology.json");
        std::fs::write(&path, serde_json::to_vec(&topology).unwrap()).unwrap();
        let cfg = NodeConfig {
            shard: 0,
            node_id: 1,
            members,
            data_dir: dir.path().join("node"),
            store: Store::open(&format!("file://{}", dir.path().join("store").display())).unwrap(),
            memory_budget: 1 << 30,
            memory_limit: 0,
            raft_port_offset: 0,
            unload_idle: None,
            claim_ttl: Duration::from_secs(10),
            max_turns_per_node: 64,
            pressure_turns: 1024,
            snapshot_every: 100_000,
            keep_log: 256,
            executors: Default::default(),
            control_plane: None,
        };
        let host = Arc::new(everruns_agentd_testkit::host());
        let served = tokio::spawn(async move {
            everruns_agentd::cli::serve_node_on(cfg, host, TopologySource { path }, listener, false)
                .await
        });
        let shard = Self {
            base: format!("http://{addr}"),
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
            _dir: dir,
        };
        for _ in 0..200 {
            if served.is_finished() {
                panic!("the shard stopped: {:?}", served.await);
            }
            if shard
                .http
                .get(shard.url("/v1/shards"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return shard;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the shard never came up");
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        for _ in 0..100 {
            if let Ok(response) = self.http.post(self.url(path)).json(&body).send().await
                && response.status().is_success()
            {
                return response.json().await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("POST {path} kept failing");
    }

    async fn get(&self, path: &str) -> Value {
        self.http
            .get(self.url(path))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// Run one turn of a new `worker` session billed to `tenant`; its events.
    async fn turn(&self, tenant: &str) -> Vec<Value> {
        let definition = everruns_agentd_testkit::definition("worker").unwrap();
        let created = self
            .post(
                "/v1/sessions",
                json!({ "agent": definition, "tenant": tenant }),
            )
            .await;
        let id = created["id"].as_str().unwrap().to_string();
        self.post(
            &format!("/v1/sessions/{id}/messages"),
            json!({ "text": "go" }),
        )
        .await;
        self.get(&format!("/v1/sessions/{id}/wait?turns=1&timeout_ms=30000"))
            .await;
        self.get(&format!("/v1/sessions/{id}/events")).await["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}

fn types(events: &[Value]) -> Vec<&str> {
    events.iter().filter_map(|e| e["type"].as_str()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_runs_only_the_calls_its_allowance_covers() {
    let shard = Shard::start().await;
    let far = 4_000_000_000_000i64;
    // "poor" cannot cover one call; "rich" covers many.
    for (tenant, granted) in [("poor", 50), ("rich", 1_000_000)] {
        shard
            .post(
                &format!("/shard/tenants/{tenant}/grants"),
                json!({
                    "grant_id": format!("{tenant}-1"),
                    "granted_micros": granted,
                    "per_call_micros": 100,
                    "spend_until_ms": far,
                }),
            )
            .await;
    }

    let poor = shard.turn("poor").await;
    let kinds = types(&poor);
    assert!(kinds.contains(&"budget.exhausted"), "{kinds:?}");
    assert!(
        !kinds.contains(&"llm.generation"),
        "no call started: {kinds:?}"
    );
    assert!(kinds.contains(&"turn.failed"), "{kinds:?}");

    let rich = shard.turn("rich").await;
    let kinds = types(&rich);
    let calls = kinds.iter().filter(|k| **k == "llm.generation").count();
    assert!(
        calls >= 2,
        "the worker calls a tool, then answers: {kinds:?}"
    );
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");

    let ledger = shard.get("/shard/allowances").await;
    for tenant in ["poor", "rich"] {
        assert_eq!(
            ledger["tenants"][tenant]["reserved"],
            json!({}),
            "{tenant}: every reservation settled"
        );
    }
    // The simulated model reports no cost: settled at zero, no overrun.
    assert_eq!(
        ledger["tenants"]["rich"]["grants"]["rich-1"]["used_micros"],
        0
    );
}
