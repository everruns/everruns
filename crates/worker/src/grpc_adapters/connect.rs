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

use std::time::Duration;

use everruns_contracts::error::{AgentLoopError, Result};
use everruns_internal_protocol::{MAX_GRPC_MESSAGE_SIZE, WorkerServiceClient};
use tonic::transport::Channel;

use super::checkpoint_availability::CheckpointAvailability;
use super::{GrpcClient, SharedClient};
use crate::grpc_durable_store::GrpcClientAuth;

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
        let endpoint = format!("http://{}", addr);
        let channel = tonic::transport::Endpoint::from_shared(endpoint)
            .map_err(|e| AgentLoopError::store(format!("Invalid gRPC endpoint: {}", e)))?
            .timeout(timeout)
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
mod tests {
    use super::*;
    use everruns_contracts::typed_id::SessionId;
    use std::time::Instant;

    /// The incident shape: a control plane that takes the connection and
    /// never answers. The call errors at the deadline instead of hanging.
    #[tokio::test]
    async fn a_call_the_control_plane_never_answers_errors_at_the_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        // Accept and hold every connection without reading or writing.
        let server = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let client =
            GrpcClient::connect_with_request_timeout(&addr.to_string(), Duration::from_millis(300))
                .await
                .expect("the TCP connection is accepted");
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            client.set_session_status(1, SessionId::new(), "active"),
        )
        .await
        .expect("the call ends at its deadline, not never");
        assert!(result.is_err(), "a call nobody answered cannot succeed");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "ended long after the deadline: {:?}",
            started.elapsed()
        );
        server.abort();
    }
}
