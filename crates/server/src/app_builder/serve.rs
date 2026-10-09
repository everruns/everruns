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

use anyhow::{Context, Result};
use axum::Router;

/// Accept connections on `bind_addr` and serve `app` until the process exits.
pub(super) async fn serve(bind_addr: &str, app: Router) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("Failed to bind to address")?;
    tracing::info!("HTTP server listening on {}", bind_addr);

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

    loop {
        let (tcp, addr) = listener
            .accept()
            .await
            .context("Failed to accept connection")?;
        let _ = tcp.set_nodelay(true);

        let app = app.clone();
        let builder = builder.clone();

        tokio::spawn(async move {
            let socket = TokioIo::new(tcp);
            let service =
                hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let app = app.clone();
                    let addr = addr;
                    async move {
                        let mut req = req.map(axum::body::Body::new);
                        req.extensions_mut()
                            .insert(axum::extract::ConnectInfo(addr));
                        let mut app = app;
                        tower::Service::call(&mut app, req).await
                    }
                });

            if let Err(err) = builder
                .serve_connection_with_upgrades(socket, service)
                .await
            {
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
