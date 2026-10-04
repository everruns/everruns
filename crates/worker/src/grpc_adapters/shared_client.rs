//! The worker's handle on its control-plane gRPC client.
//!
//! Why: the client used to sit behind one `tokio::sync::Mutex` held for each
//! whole RPC, so every call from every activity on a worker ran one at a time.
//! A claim poll or task heartbeat stalled another session's setup reads, and
//! streamed deltas were dropped whenever anything else was in flight. A tonic
//! client is a cheap handle over one multiplexed HTTP/2 channel, meant to be
//! cloned per call, so each call now takes its own clone.
//!
//! Decision: ephemeral (delta) emits keep their backpressure, at most one in
//! flight with extras dropped, through a permit of their own instead of the
//! lock every other RPC used to share.

use std::sync::Arc;

use everruns_internal_protocol::WorkerServiceClient;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

use crate::grpc_durable_store::GrpcClientAuth;

pub(crate) type WorkerClient = WorkerServiceClient<InterceptedService<Channel, GrpcClientAuth>>;

#[derive(Clone)]
pub struct SharedClient {
    client: WorkerClient,
    ephemeral: Arc<Semaphore>,
}

impl SharedClient {
    pub(crate) fn new(client: WorkerClient) -> Self {
        Self {
            client,
            ephemeral: Arc::new(Semaphore::new(1)),
        }
    }

    /// A client for one call. Calls run concurrently over the shared channel.
    pub(crate) fn client(&self) -> WorkerClient {
        self.client.clone()
    }

    /// A client for an ephemeral emit, or `None` while one is still in flight.
    pub(crate) fn try_ephemeral(&self) -> Option<(WorkerClient, OwnedSemaphorePermit)> {
        let permit = self.ephemeral.clone().try_acquire_owned().ok()?;
        Some((self.client.clone(), permit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> SharedClient {
        let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
        SharedClient::new(WorkerServiceClient::with_interceptor(
            channel,
            GrpcClientAuth::from_env(),
        ))
    }

    #[tokio::test]
    async fn calls_do_not_wait_on_each_other() {
        let shared = shared();
        // Two clients at once: under the old lock the second could not exist
        // until the first call finished.
        let _first = shared.client();
        let _second = shared.client();
        // An in-flight call does not hold back an ephemeral emit either.
        assert!(shared.try_ephemeral().is_some());
    }

    #[tokio::test]
    async fn one_ephemeral_emit_in_flight_at_a_time() {
        let shared = shared();
        let held = shared.try_ephemeral().expect("first emit");
        assert!(shared.try_ephemeral().is_none(), "second emit is dropped");
        drop(held);
        assert!(
            shared.try_ephemeral().is_some(),
            "slot frees when the emit ends"
        );
    }
}
