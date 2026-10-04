//! Controlled DNS fixture for the canonical protocol client.
use everruns_core::a2a::DnsResolver;
use std::{future::Future, net::SocketAddr, sync::Arc};

pub(super) fn controlled_dns_resolver<F, Fut>(resolve: F) -> DnsResolver
where
    F: Fn(String, u16) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = std::io::Result<Vec<SocketAddr>>> + Send + 'static,
{
    Arc::new(move |host, port| Box::pin(resolve(host, port)))
}
