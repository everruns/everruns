//! Tenant egress proxy (`everruns-egress-proxy`).
//!
//! When `EVERRUNS_EGRESS_PROXY_URL` is set on server and worker, tenant
//! HTTP(S) traffic (request kinds `capability`, `integration`, `mcp`,
//! `other:*`) leaves through this proxy, so it exits from an address that can
//! be drained and replaced without touching the deployment's own calls (LLM
//! providers, system email). Each request carries a short-lived token minted
//! by the egress service (`everruns_core::host::sign_proxy_token`); the proxy
//! verifies it, re-runs the SSRF floor against the address it actually
//! connects to, blocks SMTP ports, enforces per-org caps, and counts bytes
//! per org and session.
//!
//! Sandboxes whose provider can tunnel their traffic (E2B's SOCKS5 "bring
//! your own proxy") use a second listener, SOCKS5 on `EGRESS_PROXY_SOCKS_ADDR`,
//! authenticated with a per-sandbox credential that lives as long as the
//! sandbox (`everruns_core::host::sign_sandbox_proxy_credential`). Past
//! authentication it runs exactly the HTTP listener's checks. The control
//! plane turns it on for sandboxes with `EVERRUNS_EGRESS_PROXY_SOCKS_ADDRESS`,
//! the public `host:port` providers reach it at, over TLS by default.
//!
//! Two extension points let a control plane own policy without forking the
//! proxy: [`limits::LimitsSource`] (per-org caps, suspension) and
//! [`usage::UsageSink`] (where counted usage goes). The binary speaks the HTTP
//! form of both when `EGRESS_PROXY_CONTROL_URL` is set; without it every org
//! gets the static `EGRESS_PROXY_ORG_*` caps (off unless set) and usage goes
//! to the log.
//!
//! Decision: a module and binary in this crate, not a crate of its own. It
//! ships in the server image and keeps the workspace small.
//!
//! Environment (all optional except the secret):
//!
//! | Variable | Meaning | Default |
//! |---|---|---|
//! | `EVERRUNS_EGRESS_PROXY_SECRET` | shared with server and worker, 32+ bytes | required |
//! | `EGRESS_PROXY_ADDR` | proxy listener | `0.0.0.0:4750` |
//! | `EGRESS_PROXY_HEALTH_ADDR` | probe listener (`/healthz`) | `0.0.0.0:4751` |
//! | `EGRESS_PROXY_SOCKS_ADDR` | SOCKS5 listener for sandbox providers | off |
//! | `EGRESS_PROXY_CONTROL_URL` | control endpoint for limits and usage | none |
//! | `EGRESS_PROXY_ORG_RPM` | requests per minute per org | no cap |
//! | `EGRESS_PROXY_ORG_HOST_RPM` | requests per minute per org to one host | no cap |
//! | `EGRESS_PROXY_ORG_CONCURRENCY` | open connections per org | no cap |
//! | `EGRESS_PROXY_ORG_BYTES_PER_DAY` | bytes per org per UTC day | no cap |
//! | `EGRESS_PROXY_GLOBAL_PER_HOST_RPM` | requests per minute to one host, all orgs | no cap |
//! | `EGRESS_PROXY_REPLICAS` | replicas sharing the caps | `1` |
//! | `EGRESS_PROXY_BIND_ADDR` | source address for outbound connections | none |
//! | `EGRESS_PROXY_INSTANCE` | instance label in usage batches | hostname |
//!
//! With a control endpoint the `EGRESS_PROXY_ORG_*` caps are the fallback for
//! when it cannot be asked.

pub mod address;
pub mod limiter;
pub mod limits;
pub mod proxy;
pub mod socks;
pub mod usage;

#[cfg(test)]
mod tests;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use everruns_core::host::MIN_PROXY_SECRET_LEN;

use limiter::Limiter;
use limits::{HttpLimitsSource, LimitsCache, OrgLimits};
use proxy::{Proxy, ProxyOptions, serve_health, unix_now};
use usage::{HttpUsageSink, LogSink, UsageAggregator, UsageSink};

/// Environment variable naming the control endpoint.
pub const CONTROL_URL_ENV: &str = "EGRESS_PROXY_CONTROL_URL";

const FLUSH_INTERVAL: Duration = Duration::from_secs(60);
const LIMITS_TTL: Duration = Duration::from_secs(60);

/// Everything the proxy binary needs; [`ProxyConfig::from_env`] reads it.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub secret: String,
    pub listen: SocketAddr,
    pub health: SocketAddr,
    /// SOCKS5 listener for sandbox providers; off when `None`.
    pub socks: Option<SocketAddr>,
    /// Base URL of the control endpoint, if any.
    pub control_url: Option<String>,
    /// Caps for every org without a control endpoint, and the fallback when
    /// it cannot be asked.
    pub org_limits: OrgLimits,
    pub global_per_host_rpm: u64,
    pub replicas: u64,
    pub bind_addr: Option<IpAddr>,
    /// Label for this process; a random suffix is added at start.
    pub instance_label: String,
}

impl ProxyConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        // Decision: refuse to start without a strong secret. A proxy that
        // accepts any token would be an open relay on the tenant address.
        let secret =
            std::env::var(everruns_core::host::EGRESS_PROXY_SECRET_ENV).unwrap_or_default();
        if secret.len() < MIN_PROXY_SECRET_LEN {
            anyhow::bail!(
                "EVERRUNS_EGRESS_PROXY_SECRET must be set to at least {MIN_PROXY_SECRET_LEN} bytes"
            );
        }
        let bind_addr = match env("EGRESS_PROXY_BIND_ADDR") {
            Some(value) => Some(
                value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("EGRESS_PROXY_BIND_ADDR is not an IP address"))?,
            ),
            None => None,
        };
        Ok(Self {
            secret,
            listen: parse_env("EGRESS_PROXY_ADDR", "0.0.0.0:4750".parse()?)?,
            health: parse_env("EGRESS_PROXY_HEALTH_ADDR", "0.0.0.0:4751".parse()?)?,
            socks: match env("EGRESS_PROXY_SOCKS_ADDR") {
                Some(value) => Some(value.parse().map_err(|_| {
                    anyhow::anyhow!("EGRESS_PROXY_SOCKS_ADDR is not an address: {value:?}")
                })?),
                None => None,
            },
            control_url: env(CONTROL_URL_ENV),
            org_limits: OrgLimits {
                requests_per_minute: parse_env("EGRESS_PROXY_ORG_RPM", u64::MAX)?,
                per_host_requests_per_minute: parse_env("EGRESS_PROXY_ORG_HOST_RPM", u64::MAX)?,
                max_concurrent: parse_env("EGRESS_PROXY_ORG_CONCURRENCY", u64::MAX)?,
                bytes_per_day: parse_env("EGRESS_PROXY_ORG_BYTES_PER_DAY", u64::MAX)?,
                ..OrgLimits::unlimited()
            },
            global_per_host_rpm: parse_env("EGRESS_PROXY_GLOBAL_PER_HOST_RPM", u64::MAX)?,
            replicas: parse_env("EGRESS_PROXY_REPLICAS", 1)?,
            bind_addr,
            instance_label: env("EGRESS_PROXY_INSTANCE")
                .or_else(|| env("HOSTNAME"))
                .unwrap_or_else(|| "egress-proxy".to_string()),
        })
    }
}

/// A non-empty, trimmed environment value.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Parse an environment value, or `default` when unset.
pub fn parse_env<T: std::str::FromStr>(name: &str, default: T) -> anyhow::Result<T> {
    match env(name) {
        Some(value) => value
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} is not valid: {value:?}")),
        None => Ok(default),
    }
}

/// Run the proxy until SIGTERM or Ctrl-C.
///
/// Decision: on shutdown it stops accepting, flushes all counted usage
/// (including the current minute) and exits. Open tunnels are cut; their
/// bytes up to that point are what the counters hold, and clients reconnect
/// to a healthy replica.
pub async fn run(config: ProxyConfig) -> anyhow::Result<()> {
    // Unique per process start, so a restart never reuses a usage key.
    let label = config
        .instance_label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(40)
        .collect::<String>();
    let instance = format!(
        "{label}.{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    );

    let (limits, sink): (LimitsCache, Box<dyn UsageSink>) = match &config.control_url {
        Some(url) => (
            LimitsCache::new(
                Box::new(HttpLimitsSource::new(url, &config.secret)?),
                LIMITS_TTL,
                config.org_limits.clone(),
            ),
            Box::new(HttpUsageSink::new(url, &config.secret)?),
        ),
        None => (
            LimitsCache::fixed(config.org_limits.clone()),
            Box::new(LogSink),
        ),
    };

    let mut options = ProxyOptions::new(config.secret.clone().into_bytes());
    options.bind_addr = config.bind_addr;
    let usage = Arc::new(UsageAggregator::new(instance.clone()));
    let proxy = Arc::new(Proxy {
        options,
        limits,
        limiter: Limiter::new(config.replicas, config.global_per_host_rpm),
        usage: usage.clone(),
    });

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let health_listener = tokio::net::TcpListener::bind(config.health).await?;
    let socks_listener = match config.socks {
        Some(addr) => Some(tokio::net::TcpListener::bind(addr).await?),
        None => None,
    };
    tracing::info!(
        listen = %config.listen,
        health = %config.health,
        socks = ?config.socks,
        instance,
        replicas = config.replicas,
        bind_addr = ?config.bind_addr,
        control = config.control_url.is_some(),
        "egress proxy listening"
    );

    let sink: Arc<dyn UsageSink> = Arc::from(sink);
    let flusher = {
        let usage = usage.clone();
        let sink = sink.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(FLUSH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let now = unix_now();
                let current_minute = now - now.rem_euclid(60);
                let report = usage.flush(sink.as_ref(), Some(current_minute)).await;
                if report.items > 0 || report.requeued > 0 {
                    tracing::debug!(
                        items = report.items,
                        batches = report.batches,
                        requeued = report.requeued,
                        "egress usage flushed"
                    );
                }
            }
        })
    };

    let socks = async {
        match socks_listener {
            Some(listener) => proxy.clone().serve_socks(listener).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        result = proxy.clone().serve(listener) => result?,
        result = socks => result?,
        result = serve_health(health_listener) => result?,
        _ = shutdown_signal() => tracing::info!("egress proxy shutting down"),
    }

    flusher.abort();
    // Final flush: everything, including the current minute. Retry briefly;
    // after that the counts are lost with the process, and the log says so.
    for attempt in 0..3 {
        let report = usage.flush(sink.as_ref(), None).await;
        if report.requeued == 0 {
            tracing::info!(items = report.items, "egress usage flushed on shutdown");
            return Ok(());
        }
        tracing::warn!(attempt, requeued = report.requeued, "shutdown flush failed");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    tracing::error!(
        pending = usage.pending_len(),
        "egress usage could not be flushed on shutdown and is lost"
    );
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(term) => term,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
