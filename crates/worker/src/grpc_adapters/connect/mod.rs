//! Connecting the worker's control-plane client.
//!
//! Decision: every call on this client has a deadline. Without one a
//! control-plane call that never answers (a stalled connection, a wedged
//! handler) never errors, so the turn awaiting it hangs while its actor lease
//! keeps renewing; with one it fails as a retryable store error and the turn
//! resumes from the log. The deadline bounds the wait for the response, which
//! suits this client's calls: all unary, or a finite client stream
//! (`EmitEventStream`). The server-streaming task subscription lives on
//! `GrpcDurableStore`'s own channel. Generous by default, far past any call's
//! normal latency.

use std::{collections::BTreeSet, net::SocketAddr, time::Duration};

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_internal_protocol::{MAX_GRPC_MESSAGE_SIZE, WorkerServiceClient};
use tonic::transport::{Channel, ClientTlsConfig};

use super::checkpoint_availability::CheckpointAvailability;
use super::{GrpcClient, SharedClient};
use crate::grpc_durable_store::{GrpcClientAuth, grpc_client_tls_from_env};

/// Default deadline of one control-plane call (`WORKER_GRPC_REQUEST_TIMEOUT_SECS`).
pub const DEFAULT_GRPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

impl GrpcClient {
    /// Connect to the control plane gRPC server, each call bounded by
    /// `WORKER_GRPC_REQUEST_TIMEOUT_SECS` (default [`DEFAULT_GRPC_REQUEST_TIMEOUT`]).
    pub async fn connect(addr: &str) -> Result<Self> {
        let timeout = crate::core::config::env_duration_secs(
            "WORKER_GRPC_REQUEST_TIMEOUT_SECS",
            DEFAULT_GRPC_REQUEST_TIMEOUT,
        );
        Self::connect_with_request_timeout(addr, timeout).await
    }

    /// Connect with an explicit deadline for each call.
    pub async fn connect_with_request_timeout(addr: &str, timeout: Duration) -> Result<Self> {
        let tls = grpc_client_tls_from_env();
        let addrs = if tls.is_none() {
            crate::grpc_balance::resolve(addr).await
        } else {
            BTreeSet::new()
        };
        Self::connect_with_resolved(addr, timeout, tls, addrs).await
    }

    async fn connect_with_resolved(
        addr: &str,
        timeout: Duration,
        tls: Option<ClientTlsConfig>,
        addrs: BTreeSet<SocketAddr>,
    ) -> Result<Self> {
        // THREAT[TM-DURABLE-002]: Apply configured TLS before attaching the bearer.
        // Match the durable client: the plaintext DNS balancer constructs HTTP
        // endpoints. TLS must retain the original hostname for SNI and certificate
        // verification, and must never fall back to that balancer on failure.
        if tls.is_none() && addrs.len() > 1 {
            let channel =
                crate::grpc_balance::balanced_channel(addr.to_string(), addrs, move |e| {
                    e.timeout(timeout)
                });
            return Ok(Self::from_channel(channel));
        }
        let scheme = if tls.is_some() { "https" } else { "http" };
        let mut endpoint = tonic::transport::Endpoint::from_shared(format!("{scheme}://{addr}"))
            .map_err(|e| AgentLoopError::store(format!("Invalid gRPC endpoint: {}", e)))?
            .timeout(timeout);
        if let Some(tls) = tls {
            // The request deadline does not cover a TLS handshake. Bound both
            // connection establishment and the handshake before returning a client.
            endpoint = endpoint
                .connect_timeout(timeout)
                .tls_config(tls.timeout(timeout))
                .map_err(|e| {
                    AgentLoopError::store(format!("Invalid gRPC client TLS configuration: {e}"))
                })?;
        }
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| AgentLoopError::store(format!("gRPC connection failed: {e}")))?;
        Ok(Self::from_channel(channel))
    }

    /// Create from an existing channel (its deadline, if any, is the channel's).
    pub fn from_channel(channel: Channel) -> Self {
        // THREAT[TM-DURABLE-002]: gRPC unauthenticated access
        // Mitigation: Attach bearer token from WORKER_GRPC_AUTH_TOKEN env
        let auth = GrpcClientAuth::from_env();
        let token = std::env::var("WORKER_GRPC_AUTH_TOKEN").ok();
        let blobs = super::blob_fetch::BlobFetcher::new(channel.clone(), token.as_deref());
        let client = WorkerServiceClient::with_interceptor(channel, auth)
            .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE)
            .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE);
        Self {
            inner: SharedClient::new(client),
            checkpoints: CheckpointAvailability::new(),
            blobs,
            history: super::history_cache::HistoryCache::from_env(),
        }
    }
}

#[cfg(test)]
mod tests;
