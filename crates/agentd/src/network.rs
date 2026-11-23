//! Raft RPC between the nodes of a shard, over HTTP: JSON bodies, except a
//! snapshot chunk's bytes, which travel raw (see [`SnapshotChunk`]).

use std::sync::Arc;
use std::time::Duration;

use openraft::BasicNode;
use openraft::error::{
    InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError, Unreachable,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::catchup::CatchUp;
use crate::types::{NodeId, Raft, TypeConfig};

/// Most bytes of log entries one append carries.
const APPEND_BUDGET_BYTES: usize = 256 * 1024;

/// Header carrying a snapshot chunk's metadata; the body is its raw bytes.
pub const SNAPSHOT_HEADER: &str = "x-raft-snapshot";

/// A snapshot chunk without its bytes (`InstallSnapshotRequest` minus
/// `data`), sent as JSON in [`SNAPSHOT_HEADER`].
///
/// Decision: snapshot bytes travel raw, not inside JSON, where a byte array
/// is a list of numbers about four times its size to write and parse. A
/// follower that fell behind the leader's purged log can only catch up by
/// snapshot, and while one chunk is slower than the election timeout it
/// calls an election, cancels the transfer and starts over: a 75 000
/// session shard (a 150 MB snapshot) never recovered from losing a node.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SnapshotChunk {
    pub vote: openraft::Vote<NodeId>,
    pub meta: openraft::SnapshotMeta<NodeId, BasicNode>,
    pub offset: u64,
    pub done: bool,
}

/// Creates one HTTP client per peer.
#[derive(Clone)]
pub struct Network {
    http: reqwest::Client,
    /// Peers serve Raft on their address's port plus this (0: the same
    /// listener as everything else). See [`Isolation`].
    port_offset: u16,
}

impl Network {
    #[expect(
        clippy::expect_used,
        reason = "a client with no TLS config only fails to build when the TLS backend cannot start, and a node is useless without one"
    )]
    pub fn new(port_offset: u16) -> Self {
        Self {
            http: reqwest::Client::builder()
                .no_proxy() // fleet traffic is direct, whatever the system proxy says
                .pool_max_idle_per_host(64)
                .tcp_nodelay(true)
                .build()
                .expect("http client"),
            port_offset,
        }
    }
}

/// Where a member serves Raft: its address, with `offset` added to the port.
pub fn raft_addr(addr: &str, offset: u16) -> String {
    if offset == 0 {
        return addr.to_string();
    }
    match addr.rsplit_once(':') {
        Some((host, port)) => match port.parse::<u16>() {
            Ok(port) => format!("{host}:{}", port.saturating_add(offset)),
            Err(_) => addr.to_string(),
        },
        None => addr.to_string(),
    }
}

pub struct Peer {
    http: reqwest::Client,
    target: NodeId,
    addr: String,
}

impl RaftNetworkFactory<TypeConfig> for Network {
    type Network = Peer;

    async fn new_client(&mut self, target: NodeId, node: &BasicNode) -> Self::Network {
        Peer {
            http: self.http.clone(),
            target,
            addr: raft_addr(&node.addr, self.port_offset),
        }
    }
}

impl Peer {
    async fn call<Req, Resp, E>(
        &self,
        path: &str,
        body: &Req,
        ttl: Duration,
    ) -> Result<Resp, RPCError<NodeId, BasicNode, RaftError<NodeId, E>>>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
        E: std::error::Error + DeserializeOwned,
    {
        let url = format!("http://{}/raft/{path}", self.addr);
        let response = self
            .http
            .post(url)
            .json(body)
            .timeout(ttl)
            .send()
            .await
            .map_err(|error| {
                if error.is_connect() {
                    RPCError::Unreachable(Unreachable::new(&error))
                } else {
                    RPCError::Network(NetworkError::new(&error))
                }
            })?;
        let result: Result<Resp, RaftError<NodeId, E>> = response
            .json()
            .await
            .map_err(|error| RPCError::Network(NetworkError::new(&error)))?;
        result.map_err(|error| RPCError::RemoteError(RemoteError::new(self.target, error)))
    }
}

impl RaftNetwork<TypeConfig> for Peer {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        // Decision: openraft bounds every append by the heartbeat interval and
        // retries the same range after a timeout, so a range too big to ship
        // and fsync in one interval (a follower catching up, a loaded host)
        // would never land. Send a prefix that fits a byte budget and report
        // it as a partial success; the next append carries on from there.
        let mut rpc = rpc;
        let mut sent = rpc.entries.len();
        let mut bytes = 0;
        for (i, entry) in rpc.entries.iter().enumerate() {
            bytes += serde_json::to_vec(entry).map(|b| b.len()).unwrap_or(0);
            if i > 0 && bytes > APPEND_BUDGET_BYTES {
                sent = i;
                break;
            }
        }
        let partial = sent < rpc.entries.len();
        if partial {
            rpc.entries.truncate(sent);
        }
        let last = rpc.entries.last().map(|entry| entry.log_id);
        let response: AppendEntriesResponse<NodeId> =
            self.call("append", &rpc, option.hard_ttl()).await?;
        Ok(match response {
            AppendEntriesResponse::Success if partial => {
                AppendEntriesResponse::PartialSuccess(last)
            }
            other => other,
        })
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<NodeId>,
        RPCError<NodeId, BasicNode, RaftError<NodeId, InstallSnapshotError>>,
    > {
        let chunk = SnapshotChunk {
            vote: rpc.vote,
            meta: rpc.meta,
            offset: rpc.offset,
            done: rpc.done,
        };
        let header = serde_json::to_string(&chunk)
            .map_err(|error| RPCError::Network(NetworkError::new(&error)))?;
        let response = self
            .http
            .post(format!("http://{}/raft/snapshot", self.addr))
            .header(SNAPSHOT_HEADER, header)
            .body(rpc.data)
            .timeout(option.hard_ttl().max(Duration::from_secs(30)))
            .send()
            .await
            .map_err(|error| {
                if error.is_connect() {
                    RPCError::Unreachable(Unreachable::new(&error))
                } else {
                    RPCError::Network(NetworkError::new(&error))
                }
            })?;
        let result: Result<
            InstallSnapshotResponse<NodeId>,
            RaftError<NodeId, InstallSnapshotError>,
        > = response
            .json()
            .await
            .map_err(|error| RPCError::Network(NetworkError::new(&error)))?;
        result.map_err(|error| RPCError::RemoteError(RemoteError::new(self.target, error)))
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<NodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        self.call("vote", &rpc, option.hard_ttl()).await
    }
}

/// How many threads a node with its own Raft listener gives Raft.
const RAFT_THREADS: usize = 2;

/// Raft on its own listener and threads.
///
/// Decision: Raft traffic does not share a node's HTTP listener and Tokio
/// threads with client traffic. openraft gives a follower one heartbeat
/// interval to take an append, and two seconds at most to answer a vote.
/// In EKS run 43774871232, on a 48-vCPU host running flat out, those
/// requests waited in the same queues as tens of thousands of client
/// requests and missed their deadlines: three shards stopped committing for
/// about 20 minutes each, with elections every few seconds, while client
/// requests piled up until nodes ran out of memory. Raft's own threads and
/// listener answer in milliseconds whatever the client load, and the
/// operating system schedules them like any other thread.
///
/// The threads belong to the node: dropping it stops them, as a lost node
/// would.
pub struct Isolation {
    runtime: Option<tokio::runtime::Runtime>,
}

impl Isolation {
    /// Threads for a node whose Raft listens at `offset` above its port;
    /// none when `offset` is 0 (Raft shares the caller's).
    pub fn new(offset: u16) -> anyhow::Result<Self> {
        let runtime = (offset > 0)
            .then(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(RAFT_THREADS)
                    .thread_name("shard-raft")
                    .enable_all()
                    .build()
            })
            .transpose()?;
        Ok(Self { runtime })
    }

    /// Run `future` on Raft's threads (the caller's when shared). Raft's
    /// tasks run where it is created.
    pub async fn run<F>(&self, future: F) -> anyhow::Result<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        match &self.runtime {
            Some(runtime) => Ok(runtime.spawn(future).await?),
            None => Ok(future.await),
        }
    }

    /// Serve Raft on `listener` on Raft's threads.
    pub fn serve(
        &self,
        listener: std::net::TcpListener,
        raft: Raft,
        catchup: Arc<CatchUp>,
    ) -> anyhow::Result<()> {
        let Some(runtime) = &self.runtime else {
            anyhow::bail!("raft shares the node's listener");
        };
        let _entered = runtime.enter();
        let listener = tokio::net::TcpListener::from_std(listener)?;
        runtime.spawn(async move {
            if let Err(error) =
                axum::serve(listener, router(raft, catchup).into_make_service()).await
            {
                tracing::error!(%error, "raft listener stopped");
            }
        });
        Ok(())
    }
}

impl Drop for Isolation {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Raft's routes. Their state is the Raft handle and what this node heard
/// from its leader (`catchup.rs`), so a listener on Raft's own threads holds
/// nothing that keeps the node alive.
pub fn router(raft: Raft, catchup: Arc<CatchUp>) -> axum::Router {
    use axum::routing::post;
    axum::Router::new()
        .route("/raft/append", post(append))
        .route("/raft/vote", post(vote))
        .route("/raft/snapshot", post(snapshot))
        // Decision: no body limit. A snapshot chunk is 16 MB and an append
        // of 256 entries can be as large; axum's default 2 MB refused them.
        // Moving these routes off the client router (#5440) dropped the
        // layer that lifted it, so a follower that fell behind could never
        // take a snapshot again: it stayed 25 000 entries behind for 70
        // minutes in EKS run 43803384547 (`prove_rejoin_under_load.py`).
        // The shard's network is trusted.
        .layer(axum::extract::DefaultBodyLimit::disable())
        .with_state((raft, catchup))
}

type RaftState = axum::extract::State<(Raft, Arc<CatchUp>)>;

async fn append(
    axum::extract::State((raft, catchup)): RaftState,
    axum::Json(rpc): axum::Json<AppendEntriesRequest<TypeConfig>>,
) -> impl axum::response::IntoResponse {
    if let Some(commit) = rpc.leader_commit {
        catchup.heard(commit.index);
    }
    axum::Json(raft.append_entries(rpc).await)
}

async fn vote(
    axum::extract::State((raft, _)): RaftState,
    axum::Json(rpc): axum::Json<VoteRequest<NodeId>>,
) -> impl axum::response::IntoResponse {
    axum::Json(raft.vote(rpc).await)
}

async fn snapshot(
    axum::extract::State((raft, catchup)): RaftState,
    headers: axum::http::HeaderMap,
    data: axum::body::Bytes,
) -> Result<impl axum::response::IntoResponse, (axum::http::StatusCode, String)> {
    let chunk: SnapshotChunk = headers
        .get(SNAPSHOT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| serde_json::from_str(value).ok())
        .ok_or_else(|| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                "missing snapshot chunk header".into(),
            )
        })?;
    if let Some(last) = chunk.meta.last_log_id {
        catchup.heard(last.index);
    }
    let rpc = InstallSnapshotRequest::<TypeConfig> {
        vote: chunk.vote,
        meta: chunk.meta,
        offset: chunk.offset,
        data: data.to_vec(),
        done: chunk.done,
    };
    Ok(axum::Json(raft.install_snapshot(rpc).await))
}

#[cfg(test)]
mod tests {
    use super::raft_addr;

    #[test]
    fn raft_listens_at_an_offset_from_the_node_port() {
        assert_eq!(raft_addr("shard0-1.shard0:7000", 1), "shard0-1.shard0:7001");
        assert_eq!(raft_addr("127.0.0.1:7101", 1000), "127.0.0.1:8101");
        assert_eq!(raft_addr("127.0.0.1:7101", 0), "127.0.0.1:7101");
        assert_eq!(raft_addr("no-port", 5), "no-port");
    }
}
