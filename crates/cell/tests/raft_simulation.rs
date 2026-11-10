//! Seeded simulation of one three-node cell: the real log, state machine
//! and openraft, wired through an in-memory network that drops, delays and
//! partitions messages, while nodes are killed and restarted from their
//! disks. Small snapshot thresholds make lagging and restarted nodes catch
//! up by snapshot install as well as by log.
//!
//! What must hold after every run, once the faults stop:
//! - every write the cell acknowledged is in every node's state;
//! - every node's state is the same (state machine safety);
//! - nothing appears that no client wrote;
//! - no term ever had two leaders.
//!
//! The fault schedule comes from the seed. The interleaving does not
//! replay bit for bit (openraft draws its own election timeouts and the
//! fsyncs run on blocking threads); exact replay needs a simulated runtime,
//! a follow-up the design lists.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use everruns::SessionId;
use everruns_cell::sm::{Shared, StateMachine};
use everruns_cell::state::{Command, Response};
use everruns_cell::store::Store;
use everruns_cell::types::{NodeId, Raft, TypeConfig};
use everruns_cell::wal::Wal;
use openraft::error::{
    InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError, Unreachable,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::{BasicNode, Config, SnapshotPolicy};
use parking_lot::Mutex;

/// xorshift64*: a seed replays the same fault schedule.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const NODES: [NodeId; 3] = [1, 2, 3];

/// The simulated network: who is up, who can reach whom, how lossy it is.
struct Router {
    rafts: Mutex<HashMap<NodeId, Raft>>,
    /// Pairs that cannot talk, in both directions.
    cut: Mutex<BTreeSet<(NodeId, NodeId)>>,
    /// Percent of messages lost.
    loss: Mutex<u64>,
    rng: Mutex<Rng>,
}

impl Router {
    fn reachable(&self, from: NodeId, to: NodeId) -> bool {
        let cut = self.cut.lock();
        !cut.contains(&(from, to)) && !cut.contains(&(to, from))
    }

    /// The target, if the message gets through; after a random delay.
    async fn deliver(&self, from: NodeId, to: NodeId) -> Result<Raft, std::io::Error> {
        let (delay, lost) = {
            let mut rng = self.rng.lock();
            let loss = *self.loss.lock();
            (rng.below(20), rng.chance(loss))
        };
        tokio::time::sleep(Duration::from_millis(delay)).await;
        if lost {
            return Err(std::io::Error::other("lost"));
        }
        if !self.reachable(from, to) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "partitioned",
            ));
        }
        self.rafts
            .lock()
            .get(&to)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "node down"))
    }
}

#[derive(Clone)]
struct Net {
    from: NodeId,
    router: Arc<Router>,
}

struct Peer {
    from: NodeId,
    to: NodeId,
    router: Arc<Router>,
}

impl RaftNetworkFactory<TypeConfig> for Net {
    type Network = Peer;
    async fn new_client(&mut self, target: NodeId, _node: &BasicNode) -> Peer {
        Peer {
            from: self.from,
            to: target,
            router: self.router.clone(),
        }
    }
}

fn unreachable<E: std::error::Error>(
    error: std::io::Error,
) -> RPCError<NodeId, BasicNode, RaftError<NodeId, E>> {
    if error.kind() == std::io::ErrorKind::ConnectionRefused {
        RPCError::Unreachable(Unreachable::new(&error))
    } else {
        RPCError::Network(NetworkError::new(&error))
    }
}

impl RaftNetwork<TypeConfig> for Peer {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        let raft = self
            .router
            .deliver(self.from, self.to)
            .await
            .map_err(unreachable)?;
        raft.append_entries(rpc)
            .await
            .map_err(|error| RPCError::RemoteError(RemoteError::new(self.to, error)))
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<NodeId>,
        RPCError<NodeId, BasicNode, RaftError<NodeId, InstallSnapshotError>>,
    > {
        let raft = self
            .router
            .deliver(self.from, self.to)
            .await
            .map_err(unreachable)?;
        raft.install_snapshot(rpc)
            .await
            .map_err(|error| RPCError::RemoteError(RemoteError::new(self.to, error)))
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<NodeId>,
        _option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        let raft = self
            .router
            .deliver(self.from, self.to)
            .await
            .map_err(unreachable)?;
        raft.vote(rpc)
            .await
            .map_err(|error| RPCError::RemoteError(RemoteError::new(self.to, error)))
    }
}

struct Cell {
    root: tempfile::TempDir,
    router: Arc<Router>,
    shared: Mutex<HashMap<NodeId, Arc<Shared>>>,
    /// (term, leader) pairs every node ever reported.
    leaders: Arc<Mutex<BTreeMap<u64, BTreeSet<NodeId>>>>,
}

impl Cell {
    fn dir(&self, node: NodeId) -> PathBuf {
        self.root.path().join(format!("n{node}"))
    }

    async fn start(&self, node: NodeId) {
        let shared = Shared::new(Store::open("memory:///").unwrap(), 0);
        let wal = Wal::open(self.dir(node).join("wal")).unwrap();
        let sm = StateMachine::open(self.dir(node).join("snapshots"), shared.clone()).unwrap();
        let config = Config {
            cluster_name: "sim".into(),
            heartbeat_interval: 50,
            election_timeout_min: 200,
            election_timeout_max: 400,
            max_payload_entries: 16,
            snapshot_policy: SnapshotPolicy::LogsSinceLast(40),
            max_in_snapshot_log_to_keep: 8,
            purge_batch_size: 1,
            ..Default::default()
        }
        .validate()
        .unwrap();
        let raft = Raft::new(
            node,
            Arc::new(config),
            Net {
                from: node,
                router: self.router.clone(),
            },
            wal,
            sm,
        )
        .await
        .unwrap();
        // Record every leader each node sees, per term.
        let mut metrics = raft.metrics();
        let leaders = self.leaders.clone();
        tokio::spawn(async move {
            while metrics.changed().await.is_ok() {
                let m = metrics.borrow().clone();
                if let Some(leader) = m.current_leader {
                    leaders
                        .lock()
                        .entry(m.vote.leader_id().term)
                        .or_default()
                        .insert(leader);
                }
            }
        });
        self.shared.lock().insert(node, shared);
        self.router.rafts.lock().insert(node, raft);
    }

    async fn kill(&self, node: NodeId) {
        let raft = self.router.rafts.lock().remove(&node);
        if let Some(raft) = raft {
            let _ = raft.shutdown().await;
        }
        self.shared.lock().remove(&node);
    }

    fn up(&self) -> Vec<NodeId> {
        let mut up: Vec<_> = self.router.rafts.lock().keys().copied().collect();
        up.sort_unstable();
        up
    }

    fn raft(&self, node: NodeId) -> Option<Raft> {
        self.router.rafts.lock().get(&node).cloned()
    }

    /// Write through whichever node says it leads. `Some(true)`: the cell
    /// acknowledged it; `None`: the outcome is unknown.
    async fn write(&self, command: Command, rng: &mut Rng) -> Option<bool> {
        let up = self.up();
        if up.is_empty() {
            return None;
        }
        let first = up[rng.below(up.len() as u64) as usize];
        let raft = self.raft(first)?;
        let target = raft.current_leader().await.unwrap_or(first);
        let raft = self.raft(target)?;
        let result =
            tokio::time::timeout(Duration::from_millis(1500), raft.client_write(command)).await;
        match result {
            // A retry of a write that already landed answers Duplicate.
            Ok(Ok(response)) => Some(matches!(response.data, Response::Ok | Response::Duplicate)),
            _ => None,
        }
    }

    fn directory(&self, node: NodeId) -> HashMap<SessionId, u32> {
        let shared = self.shared.lock().get(&node).cloned().unwrap();
        let state = shared.state.read();
        state.directory.iter().map(|(k, v)| (*k, v.cell)).collect()
    }
}

async fn run(seed: u64) {
    let mut rng = Rng::new(seed);
    let cell = Cell {
        root: tempfile::tempdir().unwrap(),
        router: Arc::new(Router {
            rafts: Mutex::default(),
            cut: Mutex::default(),
            loss: Mutex::new(0),
            rng: Mutex::new(Rng::new(seed ^ 0xA5A5)),
        }),
        shared: Mutex::default(),
        leaders: Arc::default(),
    };
    for node in NODES {
        cell.start(node).await;
    }
    let members: BTreeMap<NodeId, BasicNode> = NODES
        .iter()
        .map(|id| (*id, BasicNode::new(format!("n{id}"))))
        .collect();
    cell.raft(1).unwrap().initialize(members).await.unwrap();

    let mut kills = 0;
    let mut written: HashMap<SessionId, u32> = HashMap::new();
    let mut acked: HashMap<SessionId, u32> = HashMap::new();
    for step in 0..400u32 {
        // One fault decision per step.
        match rng.below(100) {
            0..=3 => {
                let node = NODES[rng.below(3) as usize];
                if cell.up().len() == 3 {
                    cell.kill(node).await;
                    kills += 1;
                }
            }
            4..=9 => {
                for node in NODES {
                    if !cell.up().contains(&node) {
                        cell.start(node).await;
                    }
                }
            }
            10..=12 => {
                let a = NODES[rng.below(3) as usize];
                let b = NODES[rng.below(3) as usize];
                if a != b {
                    cell.router.cut.lock().insert((a, b));
                }
            }
            13..=17 => cell.router.cut.lock().clear(),
            18..=19 => *cell.router.loss.lock() = rng.below(30),
            _ => {}
        }
        let session = SessionId::new();
        let target = step % 7;
        written.insert(session, target);
        // A client retries a while, as a gateway does; each try is a
        // separate proposal, so an earlier try may land too (still the
        // same placement).
        for _ in 0..5 {
            let command = Command::Place {
                session,
                cell: target,
                generation: 1,
                previous: 0,
            };
            if let Some(outcome) = cell.write(command, &mut rng).await {
                if outcome {
                    acked.insert(session, target);
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(50 + rng.below(100))).await;
        }
        tokio::time::sleep(Duration::from_millis(rng.below(30))).await;
    }

    // Heal: everything up, nothing lost or cut, then let the cell settle.
    cell.router.cut.lock().clear();
    *cell.router.loss.lock() = 0;
    for node in NODES {
        if !cell.up().contains(&node) {
            cell.start(node).await;
        }
    }
    // A blank write through the leader commits everything before it.
    let mut settled = false;
    for _ in 0..200 {
        if cell
            .write(
                Command::Place {
                    session: SessionId::new(),
                    cell: 99,
                    generation: 1,
                    previous: 0,
                },
                &mut rng,
            )
            .await
            .is_some()
        {
            settled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(settled, "seed {seed}: the healed cell elects a leader");
    let mut converged = false;
    for _ in 0..400 {
        let applied: Vec<Option<u64>> = NODES
            .iter()
            .map(|node| {
                cell.raft(*node)
                    .and_then(|raft| raft.metrics().borrow().last_applied.map(|id| id.index))
            })
            .collect();
        let last_log = NODES
            .iter()
            .filter_map(|node| {
                cell.raft(*node)
                    .and_then(|raft| raft.metrics().borrow().last_log_index)
            })
            .max();
        if applied.iter().all(|a| a.is_some() && *a == last_log) {
            converged = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(converged, "seed {seed}: every node applies the whole log");

    let reference = cell.directory(1);
    for node in NODES {
        let directory = cell.directory(node);
        assert_eq!(
            directory, reference,
            "seed {seed}: node {node} diverged from node 1"
        );
        for (session, target) in &acked {
            assert_eq!(
                directory.get(session),
                Some(target),
                "seed {seed}: node {node} lost acknowledged write {session}"
            );
        }
        for (session, target) in &directory {
            if *target == 99 {
                continue;
            }
            assert_eq!(
                written.get(session),
                Some(target),
                "seed {seed}: node {node} holds {session}, which nobody wrote"
            );
        }
    }
    for (term, leaders) in cell.leaders.lock().iter() {
        assert!(
            leaders.len() <= 1,
            "seed {seed}: term {term} had leaders {leaders:?}"
        );
    }
    assert!(
        acked.len() > 100,
        "seed {seed}: the cell made progress ({} of 400 acknowledged)",
        acked.len()
    );
    let snapshots: Vec<Option<u64>> = NODES
        .iter()
        .map(|node| {
            cell.raft(*node)
                .and_then(|raft| raft.metrics().borrow().snapshot.map(|id| id.index))
        })
        .collect();
    eprintln!(
        "seed {seed}: {} of 400 acknowledged, {} terms, {kills} kills, snapshots at {snapshots:?}",
        acked.len(),
        cell.leaders.lock().len(),
    );
    for node in NODES {
        cell.kill(node).await;
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_cell_under_kills_partitions_and_loss_keeps_every_acknowledged_write() {
    for seed in 0..12 {
        run(seed).await;
    }
}
