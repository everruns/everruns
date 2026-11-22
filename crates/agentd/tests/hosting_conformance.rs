//! Everruns' own qualification of a session authority, run against a shard.
//!
//! The scenarios come from `everruns::hosting::conformance`; a single-node
//! shard stands in for a replicated one (every authority rule runs in Raft
//! apply either way). The node's dispatcher is not started: the scenarios
//! claim and activate sessions themselves.
//!
//! Each scenario runs twice: against the node's own hosting, and against
//! an executor's (`remote_*`), which reaches the same shard over HTTP and
//! reads its mirror. Both must hold the same contract.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use everruns_agentd::hosting::RaftHosting;
use everruns_agentd::node::{Node, NodeConfig};
use everruns_agentd::remote::{EXECUTOR_IDS, Remote};
use everruns_agentd::store::Store;

async fn shard() -> (Arc<Node>, tempfile::TempDir) {
    shard_on("127.0.0.1:1".to_string()).await
}

async fn shard_on(addr: String) -> (Arc<Node>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = NodeConfig {
        shard: 0,
        node_id: 1,
        members: BTreeMap::from([(1, addr)]),
        data_dir: dir.path().to_path_buf(),
        store: Store::open("memory://").unwrap(),
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
    let node = Node::start(cfg, Arc::new(everruns_agentd::StandardHost::new()))
        .await
        .unwrap();
    node.bootstrap().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !node.is_leader() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("single-node shard elected no leader");
    (node, dir)
}

/// A one-node shard served over HTTP, and an executor's hosting on it.
async fn remote() -> (RaftHosting, Arc<Node>, tempfile::TempDir) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (node, dir) = shard_on(addr.clone()).await;
    let app = everruns_agentd::api::router(node.clone())
        .merge(everruns_agentd::exec_api::router(node.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await });
    let remote = Remote::new(EXECUTOR_IDS, node.cfg.store.clone()).unwrap();
    remote.set_shards(BTreeMap::from([(0, BTreeMap::from([(1, addr)]))]));
    (RaftHosting::remote(remote), node, dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_authority() {
    let (hosting, _node, _dir) = remote().await;
    everruns::hosting::conformance::authority(&hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_lease_expiry() {
    let (hosting, _node, _dir) = remote().await;
    everruns::hosting::conformance::lease_expiry(&hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_engine_wait_recovery() {
    let (hosting, _node, _dir) = remote().await;
    everruns::hosting::conformance::engine_wait_recovery(hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_retained_build_recovery() {
    let (hosting, _node, _dir) = remote().await;
    everruns::hosting::conformance::retained_build_recovery(hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn authority() {
    let (node, _dir) = shard().await;
    everruns::hosting::conformance::authority(&node.hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lease_expiry() {
    let (node, _dir) = shard().await;
    everruns::hosting::conformance::lease_expiry(&node.hosting).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_wait_recovery() {
    let (node, _dir) = shard().await;
    everruns::hosting::conformance::engine_wait_recovery(node.hosting.clone()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn retained_build_recovery() {
    let (node, _dir) = shard().await;
    everruns::hosting::conformance::retained_build_recovery(node.hosting.clone()).await;
}
