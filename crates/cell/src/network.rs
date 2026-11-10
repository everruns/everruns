//! Raft RPC between the nodes of a cell, over HTTP with JSON bodies.

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

use crate::types::{NodeId, TypeConfig};

/// Most bytes of log entries one append carries.
const APPEND_BUDGET_BYTES: usize = 256 * 1024;

/// Creates one HTTP client per peer.
#[derive(Clone)]
pub struct Network {
    http: reqwest::Client,
}

impl Network {
    #[expect(
        clippy::expect_used,
        reason = "a client with no TLS config only fails to build when the TLS backend cannot start, and a node is useless without one"
    )]
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .no_proxy() // fleet traffic is direct, whatever the system proxy says
                .pool_max_idle_per_host(64)
                .tcp_nodelay(true)
                .build()
                .expect("http client"),
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Self::new()
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
            addr: node.addr.clone(),
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
        self.call(
            "snapshot",
            &rpc,
            option.hard_ttl().max(Duration::from_secs(30)),
        )
        .await
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<NodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<NodeId>, RPCError<NodeId, BasicNode, RaftError<NodeId>>> {
        self.call("vote", &rpc, option.hard_ttl()).await
    }
}
