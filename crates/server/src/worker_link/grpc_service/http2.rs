//! HTTP/2 settings of the worker gRPC listener.
//!
//! Decision: the listener sets its own connection window and stream cap
//! instead of tonic's defaults (1 MB connection window, no stream cap). Since
//! h2 0.4.16 a connection holds a budget for small DATA frames (under 256
//! bytes) that it has received but the handlers have not read yet, sized at
//! half the connection window. When the budget runs out, h2 ends the whole
//! connection with GOAWAY `ENHANCE_YOUR_CALM` ("too_many_data_frames"), and
//! every call on it fails at once. Almost every worker call is a small unary
//! request, one small frame each, and a worker running hundreds of turns keeps
//! hundreds of them in flight on one connection. With the default 512 KB
//! budget, falling a moment behind with about 3,500 of them unread was enough:
//! the 500-agent load test on 2026-11-06 lost the connection dozens of times in
//! two minutes, and about 600 turns failed and were retried.
//!
//! A 16 MB connection window gives an 8 MB budget. The stream cap bounds what
//! one unread frame per open stream can cost (`MAX_CONCURRENT_STREAMS` × 256
//! bytes = 1 MB), so a burst of small calls can no longer exhaust it; calls
//! past the cap wait in the worker's client until a stream frees up. The
//! stream window matches the public HTTP listener (`Http2FlowConfig`).

/// Connection-level receive window: the small-frame budget is half of it.
pub(crate) const CONNECTION_WINDOW: u32 = 16 * 1024 * 1024;
/// Per-stream receive window.
pub(crate) const STREAM_WINDOW: u32 = 2 * 1024 * 1024;
/// Streams one worker connection may keep open at once.
pub(crate) const MAX_CONCURRENT_STREAMS: u32 = 4096;

/// A gRPC server builder with the worker listener's HTTP/2 settings.
pub fn grpc_server_builder() -> tonic::transport::Server {
    tonic::transport::Server::builder()
        .initial_connection_window_size(CONNECTION_WINDOW)
        .initial_stream_window_size(STREAM_WINDOW)
        .max_concurrent_streams(MAX_CONCURRENT_STREAMS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_internal_protocol::WorkerServiceClient;
    use everruns_internal_protocol::proto::CheckOutboundToolRateLimitRequest;
    use std::time::Duration;

    /// Small frames under h2's 256-byte threshold cost `256 - len` each.
    const SMALL_FRAME_THRESHOLD: u32 = 256;

    #[test]
    fn open_streams_cannot_exhaust_the_small_frame_budget() {
        let budget = CONNECTION_WINDOW / 2;
        assert!(MAX_CONCURRENT_STREAMS * SMALL_FRAME_THRESHOLD <= budget / 4);
    }

    /// Serve an empty router on a single-threaded runtime of its own, so the
    /// test can stall the server the way a busy replica stalls.
    fn serve(builder: tonic::transport::Server) -> (String, tokio::runtime::Handle) {
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        std_listener.set_nonblocking(true).expect("nonblocking");
        let address = format!("http://{}", std_listener.local_addr().expect("address"));
        let (handle_tx, handle_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            handle_tx.send(runtime.handle().clone()).expect("handle");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(std_listener).expect("listener");
                let mut builder = builder;
                let _ = builder
                    .add_routes(tonic::service::Routes::default())
                    .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener))
                    .await;
            });
        });
        (address, handle_rx.recv().expect("server handle"))
    }

    /// Every call of a burst gets the server's answer (here `UNIMPLEMENTED`,
    /// the empty router's), none a dropped connection.
    async fn burst_while_the_server_stalls(builder: tonic::transport::Server) -> Vec<tonic::Code> {
        let (address, server) = serve(builder);
        let channel = tonic::transport::Endpoint::from_shared(address)
            .expect("endpoint")
            .connect()
            .await
            .expect("connect");
        let client = WorkerServiceClient::new(channel);
        let call = |mut client: WorkerServiceClient<tonic::transport::Channel>| async move {
            let request = CheckOutboundToolRateLimitRequest {
                org_key: "org_00000000000000000000000000000014".into(),
            };
            match client.check_outbound_tool_rate_limit(request).await {
                Ok(_) => tonic::Code::Ok,
                Err(status) => status.code(),
            }
        };
        // One call first, so the client knows the server's settings.
        call(client.clone()).await;

        server.spawn(async { std::thread::sleep(Duration::from_millis(1500)) });
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..8_000 {
            calls.spawn(call(client.clone()));
        }
        calls.join_all().await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_burst_of_small_calls_keeps_the_connection() {
        let codes = burst_while_the_server_stalls(grpc_server_builder()).await;
        let failed: Vec<_> = codes
            .iter()
            .filter(|code| **code != tonic::Code::Unimplemented)
            .collect();
        assert!(
            failed.is_empty(),
            "{} of {} calls lost the connection: {:?}",
            failed.len(),
            codes.len(),
            failed.first()
        );
    }
}
