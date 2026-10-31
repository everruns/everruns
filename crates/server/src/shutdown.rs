//! Graceful server shutdown: a rolling deploy must not cut running work.
//!
//! Decision: on SIGTERM (or Ctrl+C) the server drains in three steps instead
//! of dying mid-request. Before this, the HTTP loop ran "until the process
//! exits"; as PID 1 in a container with no handler the signal did nothing, and
//! Kubernetes killed the pod 30 s later with every SSE stream and worker call
//! cut at once (prod load test, 2026-10-29).
//!
//! 1. **Linger** (`SERVER_SHUTDOWN_DELAY_SECS`, default 5): keep serving as
//!    usual. Kubernetes stops routing new connections to a terminating pod,
//!    but proxies hold keep-alive connections to it and endpoint changes take
//!    a moment to reach them.
//! 2. **Drain** ([`ServerShutdown::draining`] fires): stop accepting, end SSE
//!    streams with a `disconnecting` event (reason `shutdown`) so clients
//!    resume with `since_id` on another replica, and let in-flight requests
//!    finish, up to `SERVER_SHUTDOWN_GRACE_SECS` (default 20).
//! 3. **Stop the worker link** ([`ServerShutdown::stop_grpc`] fires): the
//!    gRPC listener stops last, so workers' calls keep landing here while
//!    they re-resolve the replica list, then get a GOAWAY instead of a reset.
//!
//! Keep linger + grace + [`GRPC_STOP_TIMEOUT`] under the orchestrator's kill
//! timeout (`terminationGracePeriodSeconds`).
//!
//! The signal is process-wide: one server per process. Tests that never send
//! a signal never see it fire.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// Default linger before draining starts.
pub const DEFAULT_SHUTDOWN_DELAY: Duration = Duration::from_secs(5);
/// Default time in-flight HTTP requests get to finish once draining.
pub const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(20);
/// Most the gRPC listener gets to finish in-flight worker calls.
pub const GRPC_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Process-wide shutdown phases. See the module docs.
#[derive(Debug, Default)]
pub struct ServerShutdown {
    draining: CancellationToken,
    stop_grpc: CancellationToken,
    grpc_running: AtomicBool,
    grpc_stopped: CancellationToken,
}

impl ServerShutdown {
    /// Fires when the server stops accepting and SSE streams should close.
    pub fn draining(&self) -> &CancellationToken {
        &self.draining
    }

    /// Fires when the gRPC listener should stop.
    pub fn stop_grpc(&self) -> &CancellationToken {
        &self.stop_grpc
    }

    /// The gRPC listener calls this as it starts serving, so shutdown knows
    /// to wait for it (DEV_MODE runs without one).
    pub fn grpc_started(&self) {
        self.grpc_running.store(true, Ordering::SeqCst);
    }

    /// The gRPC listener calls this once it has stopped.
    pub fn grpc_stopped(&self) {
        self.grpc_stopped.cancel();
    }

    /// Stop the gRPC listener and wait for it, at most [`GRPC_STOP_TIMEOUT`].
    pub async fn stop_grpc_and_wait(&self) {
        self.stop_grpc.cancel();
        if !self.grpc_running.load(Ordering::SeqCst) {
            return;
        }
        if tokio::time::timeout(GRPC_STOP_TIMEOUT, self.grpc_stopped.cancelled())
            .await
            .is_err()
        {
            tracing::warn!("gRPC listener did not stop in time; exiting anyway");
        }
    }
}

/// The process-wide shutdown state.
pub fn server_shutdown() -> &'static ServerShutdown {
    static SHUTDOWN: OnceLock<ServerShutdown> = OnceLock::new();
    SHUTDOWN.get_or_init(ServerShutdown::default)
}

/// Linger and grace read from the environment (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownTiming {
    pub delay: Duration,
    pub grace: Duration,
}

impl ShutdownTiming {
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var("SERVER_SHUTDOWN_DELAY_SECS").ok().as_deref(),
            std::env::var("SERVER_SHUTDOWN_GRACE_SECS").ok().as_deref(),
        )
    }

    fn from_values(delay: Option<&str>, grace: Option<&str>) -> Self {
        let secs = |value: Option<&str>, default: Duration| {
            value
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(default)
        };
        Self {
            delay: secs(delay, DEFAULT_SHUTDOWN_DELAY),
            grace: secs(grace, DEFAULT_SHUTDOWN_GRACE),
        }
    }
}

/// Wait for SIGTERM or Ctrl+C, linger for `delay`, then start draining.
pub async fn drain_on_signal(delay: Duration) {
    wait_for_signal().await;
    tracing::info!(
        delay_secs = delay.as_secs_f64(),
        "Shutdown signal received; draining after the linger"
    );
    tokio::time::sleep(delay).await;
    server_shutdown().draining().cancel();
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_defaults_and_overrides() {
        assert_eq!(
            ShutdownTiming::from_values(None, None),
            ShutdownTiming {
                delay: DEFAULT_SHUTDOWN_DELAY,
                grace: DEFAULT_SHUTDOWN_GRACE,
            }
        );
        assert_eq!(
            ShutdownTiming::from_values(Some("0"), Some(" 40 ")),
            ShutdownTiming {
                delay: Duration::ZERO,
                grace: Duration::from_secs(40),
            }
        );
        // Garbage falls back to the default rather than to zero.
        assert_eq!(
            ShutdownTiming::from_values(Some("soon"), None).delay,
            DEFAULT_SHUTDOWN_DELAY
        );
    }

    #[tokio::test]
    async fn stop_grpc_waits_for_the_listener() {
        let shutdown = ServerShutdown::default();
        shutdown.grpc_started();
        let stop = shutdown.stop_grpc().clone();
        let listener = async {
            stop.cancelled().await;
            shutdown.grpc_stopped();
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(listener, shutdown.stop_grpc_and_wait());
        })
        .await
        .expect("returns as soon as the listener stops");
    }

    #[tokio::test]
    async fn stop_grpc_without_a_listener_returns_at_once() {
        let shutdown = ServerShutdown::default();
        tokio::time::timeout(Duration::from_millis(100), shutdown.stop_grpc_and_wait())
            .await
            .expect("nothing to wait for");
    }
}
