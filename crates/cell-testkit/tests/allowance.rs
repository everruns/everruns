//! Spend allowances end to end on a one-node cell: a metered session runs
//! only the provider calls its tenant's grant covers, through the Engine's
//! own budget gate, and the cell's ledger shows every reservation closed.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use everruns_cell::cluster::{Topology, TopologySource};
use everruns_cell::node::NodeConfig;
use everruns_cell::store::Store;
use serde_json::{Value, json};

struct Cell {
    base: String,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl Cell {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let addr = format!("127.0.0.1:{port}");
        let members = BTreeMap::from([(1, addr.clone())]);
        let topology = Topology {
            directory_cell: 0,
            cells: BTreeMap::from([(0, members.clone())]),
        };
        let path = dir.path().join("topology.json");
        std::fs::write(&path, serde_json::to_vec(&topology).unwrap()).unwrap();
        let cfg = NodeConfig {
            cell: 0,
            node_id: 1,
            members,
            data_dir: dir.path().join("node"),
            store: Store::open(&format!("file://{}", dir.path().join("store").display())).unwrap(),
            memory_budget: 1 << 30,
            unload_idle: None,
            claim_ttl: Duration::from_secs(10),
            max_turns_per_node: 64,
            snapshot_every: 100_000,
            keep_log: 256,
            executors: Default::default(),
            control_plane: None,
        };
        let host = Arc::new(everruns_cell_testkit::host());
        let listen = addr.parse().unwrap();
        tokio::spawn(async move {
            everruns_cell::cli::serve_node(cfg, host, TopologySource { path }, listen, false).await
        });
        let cell = Self {
            base: format!("http://{addr}"),
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
            _dir: dir,
        };
        for _ in 0..200 {
            if cell
                .http
                .get(cell.url("/v1/cells"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return cell;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the cell never came up");
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
        let definition = everruns_cell_testkit::definition("worker").unwrap();
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
    let cell = Cell::start().await;
    let far = 4_000_000_000_000i64;
    // "poor" cannot cover one call; "rich" covers many.
    for (tenant, granted) in [("poor", 50), ("rich", 1_000_000)] {
        cell.post(
            &format!("/cell/tenants/{tenant}/grants"),
            json!({
                "grant_id": format!("{tenant}-1"),
                "granted_micros": granted,
                "per_call_micros": 100,
                "spend_until_ms": far,
            }),
        )
        .await;
    }

    let poor = cell.turn("poor").await;
    let kinds = types(&poor);
    assert!(kinds.contains(&"budget.exhausted"), "{kinds:?}");
    assert!(
        !kinds.contains(&"llm.generation"),
        "no call started: {kinds:?}"
    );
    assert!(kinds.contains(&"turn.failed"), "{kinds:?}");

    let rich = cell.turn("rich").await;
    let kinds = types(&rich);
    let calls = kinds.iter().filter(|k| **k == "llm.generation").count();
    assert!(
        calls >= 2,
        "the worker calls a tool, then answers: {kinds:?}"
    );
    assert!(kinds.contains(&"turn.completed"), "{kinds:?}");

    let ledger = cell.get("/cell/allowances").await;
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
