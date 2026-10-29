// HTTP serve loop (startup phase 8).
//
// Decision: Uses hyper_util auto builder instead of axum::serve to configure
//   HTTP/2 flow control. Default 65KB per-stream window exhausts under high SSE
//   concurrency (50+ streams over single HTTP/2 connection) -> streams block ->
//   timeout -> cascade failure. We set 2MB stream windows, 16MB connection
//   windows, and enable adaptive flow control.
//
// Tunable via env vars for production sizing:
//   HTTP2_STREAM_WINDOW_SIZE    per-stream window (default: 2MB)
//   HTTP2_CONNECTION_WINDOW_SIZE connection window (default: 16MB)
//   HTTP2_MAX_CONCURRENT_STREAMS max streams per connection (default: 256)
//
// Decision: a request body the handler never reads is drained in the
//   background (bounded by DRAIN_LIMIT_BYTES / DRAIN_TIMEOUT) instead of
//   dropped. Dropping it makes hyper close the HTTP/1 connection with unread
//   bytes in the socket, the kernel answers the proxy's next body write with a
//   reset, and Caddy turns our 404/401/405 into a 502. GitHub webhooks posted
//   to a stale URL did exactly that on 2026-10-13 and tripped the prod 5xx
//   alert. Draining lets hyper finish the request and keep the connection.
//
// Decision: SIGTERM drains instead of killing (crate::shutdown has the
//   phases). Each connection is watched by hyper's GracefulShutdown, so once
//   draining starts HTTP/2 gets a GOAWAY and HTTP/1 closes after its current
//   response; SSE streams end themselves on the same signal.

use anyhow::{Context, Result};
use axum::Router;

/// Accept connections on `bind_addr` and serve `app` until SIGTERM or Ctrl+C,
/// then drain (see `crate::shutdown`).
pub(super) async fn serve(bind_addr: &str, app: Router) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("Failed to bind to address")?;
    tracing::info!("HTTP server listening on {}", bind_addr);
    let timing = crate::shutdown::ShutdownTiming::from_env();
    tokio::spawn(crate::shutdown::drain_on_signal(timing.delay));
    let shutdown = crate::shutdown::server_shutdown();
    serve_on(listener, app, shutdown.draining().clone(), timing.grace).await?;
    shutdown.stop_grpc_and_wait().await;
    tracing::info!("Server stopped");
    Ok(())
}

/// Serve `app` on an already-bound listener until `draining` fires, then give
/// open connections `grace` to finish.
async fn serve_on(
    listener: tokio::net::TcpListener,
    app: Router,
    draining: tokio_util::sync::CancellationToken,
    grace: std::time::Duration,
) -> Result<()> {
    let h2_config = Http2FlowConfig::from_env();
    h2_config.log();

    use hyper_util::rt::{TokioExecutor, TokioIo};
    use hyper_util::server::conn::auto::Builder as AutoBuilder;

    let mut builder = AutoBuilder::new(TokioExecutor::new());
    builder
        .http2()
        .initial_stream_window_size(h2_config.stream_window)
        .initial_connection_window_size(h2_config.connection_window)
        .adaptive_window(true)
        .max_concurrent_streams(h2_config.max_concurrent_streams)
        .keep_alive_interval(Some(std::time::Duration::from_secs(20)))
        .keep_alive_timeout(std::time::Duration::from_secs(20));

    let graceful = hyper_util::server::graceful::GracefulShutdown::new();

    loop {
        let (tcp, addr) = tokio::select! {
            accepted = listener.accept() => accepted.context("Failed to accept connection")?,
            _ = draining.cancelled() => break,
        };
        let _ = tcp.set_nodelay(true);

        let app = app.clone();
        let builder = builder.clone();
        let watcher = graceful.watcher();

        tokio::spawn(async move {
            let socket = TokioIo::new(tcp);
            let service =
                hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let app = app.clone();
                    let addr = addr;
                    async move {
                        let mut req = req.map(|body| axum::body::Body::new(DrainOnDrop::new(body)));
                        req.extensions_mut()
                            .insert(axum::extract::ConnectInfo(addr));
                        let mut app = app;
                        tower::Service::call(&mut app, req).await
                    }
                });

            let connection = builder.serve_connection_with_upgrades(socket, service);
            if let Err(err) = watcher.watch(connection).await {
                // Don't log normal connection closures (client disconnect, reset)
                let msg = err.to_string();
                if !msg.contains("connection closed")
                    && !msg.contains("reset by peer")
                    && !msg.contains("broken pipe")
                {
                    tracing::debug!(error = %err, "HTTP connection error");
                }
            }
        });
    }

    drop(listener);
    let open = graceful.count();
    tracing::info!(
        open_connections = open,
        grace_secs = grace.as_secs_f64(),
        "Draining HTTP connections"
    );
    if tokio::time::timeout(grace, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("HTTP drain grace elapsed; closing remaining connections");
    }
    Ok(())
}

// =========================================================================
// Unread request bodies
// =========================================================================

/// Most a dropped body is read to keep its connection alive. Past this, closing
/// the connection is cheaper than reading the rest.
const DRAIN_LIMIT_BYTES: usize = 1024 * 1024;
const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Request body that, when dropped before its end, drains the rest on a
/// background task. See the decision at the top of this file.
struct DrainOnDrop {
    inner: Option<hyper::body::Incoming>,
}

impl DrainOnDrop {
    fn new(inner: hyper::body::Incoming) -> Self {
        Self { inner: Some(inner) }
    }
}

impl hyper::body::Body for DrainOnDrop {
    type Data = hyper::body::Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let Some(inner) = self.inner.as_mut() else {
            return std::task::Poll::Ready(None);
        };
        let polled = std::pin::Pin::new(inner).poll_frame(cx);
        if matches!(polled, std::task::Poll::Ready(None | Some(Err(_)))) {
            // Finished or failed: nothing left to drain.
            self.inner = None;
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.as_ref().is_none_or(|b| b.is_end_stream())
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner
            .as_ref()
            .map_or_else(|| hyper::body::SizeHint::with_exact(0), |b| b.size_hint())
    }
}

impl Drop for DrainOnDrop {
    fn drop(&mut self) {
        use hyper::body::Body as _;
        let Some(body) = self.inner.take() else {
            return;
        };
        if body.is_end_stream() {
            return;
        }
        if body.size_hint().lower() > DRAIN_LIMIT_BYTES as u64 {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            use http_body_util::BodyExt;
            let mut body = body;
            let mut read = 0usize;
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, async {
                while let Some(Ok(frame)) = body.frame().await {
                    if let Some(data) = frame.data_ref() {
                        read += data.len();
                        if read > DRAIN_LIMIT_BYTES {
                            break;
                        }
                    }
                }
            })
            .await;
        });
    }
}

// =========================================================================
// HTTP/2 Flow Control Configuration
// =========================================================================

/// HTTP/2 flow control settings tuned for high-concurrency SSE streaming.
///
/// Default HTTP/2 per-stream window is 65KB. With 50+ concurrent SSE streams
/// over a single connection, windows exhaust when server produces events faster
/// than clients read. This causes streams to block, timeout, and cascade.
///
/// Defaults: 2MB stream window, 16MB connection window, 256 max streams.
/// Override via environment variables for production sizing.
pub(super) struct Http2FlowConfig {
    pub(super) stream_window: u32,
    pub(super) connection_window: u32,
    pub(super) max_concurrent_streams: u32,
}

impl Http2FlowConfig {
    const DEFAULT_STREAM_WINDOW: u32 = 2 * 1024 * 1024; // 2 MB
    const DEFAULT_CONNECTION_WINDOW: u32 = 16 * 1024 * 1024; // 16 MB
    const DEFAULT_MAX_CONCURRENT_STREAMS: u32 = 256;

    fn from_env() -> Self {
        Self::from_values(
            std::env::var("HTTP2_STREAM_WINDOW_SIZE").ok().as_deref(),
            std::env::var("HTTP2_CONNECTION_WINDOW_SIZE")
                .ok()
                .as_deref(),
            std::env::var("HTTP2_MAX_CONCURRENT_STREAMS")
                .ok()
                .as_deref(),
        )
    }

    pub(super) fn from_values(
        stream_window: Option<&str>,
        connection_window: Option<&str>,
        max_concurrent_streams: Option<&str>,
    ) -> Self {
        Self {
            stream_window: stream_window
                .and_then(|v| v.parse().ok())
                .unwrap_or(Self::DEFAULT_STREAM_WINDOW),
            connection_window: connection_window
                .and_then(|v| v.parse().ok())
                .unwrap_or(Self::DEFAULT_CONNECTION_WINDOW),
            max_concurrent_streams: max_concurrent_streams
                .and_then(|v| v.parse().ok())
                .unwrap_or(Self::DEFAULT_MAX_CONCURRENT_STREAMS),
        }
    }

    fn log(&self) {
        tracing::info!(
            h2_stream_window_kb = self.stream_window / 1024,
            h2_conn_window_kb = self.connection_window / 1024,
            h2_max_streams = self.max_concurrent_streams,
            "HTTP/2 flow control configured"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A proxy streams a POST body after the headers; the handler answers
    /// before reading it. The proxy must still get the answer, not a reset.
    async fn post_unread_body(path: &str) -> String {
        let app = Router::new().route("/known", post(|| async { "ok" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve_on(
            listener,
            app,
            tokio_util::sync::CancellationToken::new(),
            std::time::Duration::from_secs(1),
        ));

        let body_len = 256 * 1024;
        let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let head = format!("POST {path} HTTP/1.1\r\nhost: x\r\ncontent-length: {body_len}\r\n\r\n");
        tcp.write_all(head.as_bytes()).await.unwrap();
        // Let the server route and answer before the body arrives, as it does
        // when Caddy relays a body the client is still uploading.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let chunk = vec![b'x'; 16 * 1024];
        for _ in 0..body_len / chunk.len() {
            tcp.write_all(&chunk)
                .await
                .expect("body write must not be reset");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let mut buf = vec![0u8; 512];
        let n = tcp
            .read(&mut buf)
            .await
            .expect("response must not be reset");
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    #[tokio::test]
    async fn unmatched_route_with_unread_body_answers_404_without_reset() {
        let response = post_unread_body("/missing").await;
        assert!(response.starts_with("HTTP/1.1 404"), "{response}");
    }

    #[tokio::test]
    async fn handler_ignoring_body_answers_without_reset() {
        let response = post_unread_body("/known").await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }

    /// Draining lets a request already in flight finish, then stops accepting
    /// and returns.
    #[tokio::test]
    async fn draining_finishes_in_flight_requests_then_stops() {
        use axum::routing::get;
        let app = Router::new().route(
            "/slow",
            get(|| async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                "done"
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let draining = tokio_util::sync::CancellationToken::new();
        let server = tokio::spawn(serve_on(
            listener,
            app,
            draining.clone(),
            std::time::Duration::from_secs(5),
        ));

        let mut tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        tcp.write_all(b"GET /slow HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        draining.cancel();

        let mut response = String::new();
        tcp.read_to_string(&mut response)
            .await
            .expect("in-flight request must finish");
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("done"), "{response}");

        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .expect("serve returns once drained")
            .unwrap()
            .unwrap();
        assert!(
            tokio::net::TcpStream::connect(addr).await.is_err(),
            "no new connections after draining"
        );
    }
}
