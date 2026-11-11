//! Everruns' own qualification of a session authority, run against a cell.
//!
//! The scenarios come from `everruns::hosting::conformance`; a single-node
//! cell stands in for a replicated one (every authority rule runs in Raft
//! apply either way). The node's dispatcher is not started: the scenarios
//! claim and activate sessions themselves.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use everruns_cell::node::{Node, NodeConfig};
use everruns_cell::store::Store;

async fn cell() -> (Arc<Node>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = NodeConfig {
        cell: 0,
        node_id: 1,
        members: BTreeMap::from([(1, "127.0.0.1:1".to_string())]),
        data_dir: dir.path().to_path_buf(),
        store: Store::open("memory://").unwrap(),
        memory_budget: 1 << 30,
        unload_idle: None,
        claim_ttl: Duration::from_secs(10),
        max_turns_per_node: 64,
        snapshot_every: 100_000,
        keep_log: 2048,
    };
    let node = Node::start(cfg, Arc::new(everruns_cell::StandardHost::new()))
        .await
        .unwrap();
    node.bootstrap().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !node.is_leader() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("single-node cell elected no leader");
    (node, dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn authority() {
    let (node, _dir) = cell().await;
    everruns::hosting::conformance::authority(&node.hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lease_expiry() {
    let (node, _dir) = cell().await;
    everruns::hosting::conformance::lease_expiry(&node.hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_wait_recovery() {
    let (node, _dir) = cell().await;
    everruns::hosting::conformance::engine_wait_recovery(node.hosting.clone()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_build_recovery() {
    let (node, _dir) = cell().await;
    everruns::hosting::conformance::retained_build_recovery(node.hosting.clone()).await;
}
