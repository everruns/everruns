//! Spreading a worker's control-plane calls over every server replica.
//!
//! Decision: a Kubernetes ClusterIP Service balances per connection, and a
//! gRPC channel is one long-lived HTTP/2 connection, so with several server
//! replicas each worker's whole call stream lands on whichever replica its
//! connection happened to pick. The 2026-10-28 load test on two replicas showed
//! one at 1.5 cores and the other at 0.2, while browser traffic split evenly.
//!
//! When the control-plane host resolves to several addresses (a headless
//! Service lists every ready replica), the channel keeps one connection per
//! address and sends each call to the less busy of two (tower's
//! power-of-two-choices balancer). It resolves the host again every
//! [`RESOLVE_INTERVAL`], so replicas added or removed by a deploy or a scale-up
//! are picked up. A host with one address keeps today's single connection, and
//! so does TLS: certificate checks need the host name, not a replica address.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::time::Duration;

use tonic::transport::channel::Change;
use tonic::transport::{Channel, Endpoint};

/// How often the host is resolved again for added or removed replicas. Kept
/// well inside a draining server's linger plus grace (`crate::shutdown` in
/// everruns-server, about 25 s), so workers move off a replica before its
/// gRPC listener stops.
pub const RESOLVE_INTERVAL: Duration = Duration::from_secs(10);

/// Every address `host:port` resolves to, sorted, or none when it does not
/// resolve.
pub async fn resolve(address: &str) -> BTreeSet<SocketAddr> {
    match tokio::net::lookup_host(address).await {
        Ok(addrs) => addrs.collect(),
        Err(e) => {
            tracing::debug!(address, error = %e, "control-plane address did not resolve");
            BTreeSet::new()
        }
    }
}

/// A channel balanced over `addrs`, kept current by resolving `address` again
/// every [`RESOLVE_INTERVAL`]. `configure` applies per-connection settings
/// (such as a call deadline) to each replica's endpoint.
pub fn balanced_channel(
    address: String,
    addrs: BTreeSet<SocketAddr>,
    configure: impl Fn(Endpoint) -> Endpoint + Send + Sync + 'static,
) -> Channel {
    let (channel, tx) = Channel::balance_channel::<SocketAddr>(addrs.len().max(16));
    tracing::info!(
        address = %address,
        replicas = addrs.len(),
        "Balancing control-plane calls over every server replica"
    );
    tokio::spawn(async move {
        // A socket address always forms a valid URI; skipping is only a guard.
        let endpoint = |addr: SocketAddr| {
            Endpoint::from_shared(format!("http://{addr}"))
                .ok()
                .map(&configure)
        };
        let mut current = BTreeSet::new();
        let mut next = addrs;
        loop {
            let (added, removed) = diff(&current, &next);
            for (addr, endpoint) in added.into_iter().filter_map(|a| Some((a, endpoint(a)?))) {
                if tx.send(Change::Insert(addr, endpoint)).await.is_err() {
                    return; // the channel was dropped
                }
            }
            for addr in removed {
                if tx.send(Change::Remove(addr)).await.is_err() {
                    return;
                }
            }
            if current != next {
                tracing::info!(address = %address, replicas = next.len(), "Control-plane replicas changed");
            }
            current = next;
            tokio::time::sleep(RESOLVE_INTERVAL).await;
            if tx.is_closed() {
                return;
            }
            next = resolve(&address).await;
            // A failed or empty lookup keeps the known replicas: dropping every
            // connection over one DNS hiccup would fail calls that could succeed.
            if next.is_empty() {
                next = current.clone();
            }
        }
    });
    channel
}

/// Addresses to add and to remove to turn `current` into `next`.
fn diff(
    current: &BTreeSet<SocketAddr>,
    next: &BTreeSet<SocketAddr>,
) -> (Vec<SocketAddr>, Vec<SocketAddr>) {
    (
        next.difference(current).copied().collect(),
        current.difference(next).copied().collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tonic::codegen::Service;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn diff_adds_new_replicas_and_removes_gone_ones() {
        let current = BTreeSet::from([addr("10.0.0.1:9001"), addr("10.0.0.2:9001")]);
        let next = BTreeSet::from([addr("10.0.0.2:9001"), addr("10.0.0.3:9001")]);
        let (added, removed) = diff(&current, &next);
        assert_eq!(added, vec![addr("10.0.0.3:9001")]);
        assert_eq!(removed, vec![addr("10.0.0.1:9001")]);
        assert_eq!(diff(&next, &next), (vec![], vec![]));
    }

    #[tokio::test]
    async fn resolve_lists_every_address_and_none_for_an_unknown_host() {
        assert_eq!(
            resolve("127.0.0.1:9001").await,
            BTreeSet::from([addr("127.0.0.1:9001")])
        );
        assert!(resolve("no-such-host.invalid:9001").await.is_empty());
    }

    /// A listener that counts the connections it accepts and holds them open
    /// without answering, so every call on it stays in flight.
    async fn counting_listener() -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                held.push(socket);
            }
        });
        (addr, accepted)
    }

    /// The load-test shape: calls from one worker reach every replica rather
    /// than all riding the one connection a Service happened to route.
    #[tokio::test]
    async fn calls_reach_every_replica() {
        let (a, a_accepted) = counting_listener().await;
        let (b, b_accepted) = counting_listener().await;
        let channel = balanced_channel(
            "unused-in-this-test:9001".into(),
            BTreeSet::from([a, b]),
            |e| e.timeout(Duration::from_secs(5)),
        );
        for _ in 0..8 {
            let mut channel = channel.clone();
            tokio::spawn(async move {
                let request = tonic::codegen::http::Request::builder()
                    .uri("http://replica/everruns.Probe/Call")
                    .body(tonic::body::Body::empty())
                    .unwrap();
                if std::future::poll_fn(|cx| channel.poll_ready(cx))
                    .await
                    .is_ok()
                {
                    let _ = channel.call(request).await;
                }
            });
        }
        let reached_both = tokio::time::timeout(Duration::from_secs(5), async {
            while a_accepted.load(Ordering::SeqCst) == 0 || b_accepted.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(
            reached_both.is_ok(),
            "connections: a={} b={}",
            a_accepted.load(Ordering::SeqCst),
            b_accepted.load(Ordering::SeqCst)
        );
    }
}
