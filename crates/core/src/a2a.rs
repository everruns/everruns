//! Opt-in outbound A2A protocol client. No product records or delegation state.
//!
//! Hosts validate their configured URLs, then inject the runtime network policy.
//! Discovery is checked before HTTP and every discovered interface is checked
//! before constructing a transport.

pub use a2a_wire::*;

use crate::network_access::NetworkAccessList;
use a2a_client::A2AClientFactory;
use a2a_client::agent_card::AgentCardResolver;
use a2a_client::jsonrpc::JsonRpcTransportFactory;
use a2a_client::middleware::CallInterceptor;
use a2a_client::rest::RestTransportFactory;
use a2a_client::transport::ServiceParams;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

mod network;
use network::hardened_a2a_http_client;
pub use network::{DnsResolveFuture, DnsResolver};

/// Outbound protocol client; delegation persistence remains a host concern.
pub type Client = a2a_client::A2AClient<Box<dyn a2a_client::Transport>>;

/// Reject a discovery URL before performing any outbound request.
pub fn enforce_discovery_policy(
    base_url: &str,
    acl: Option<&NetworkAccessList>,
) -> std::result::Result<(), String> {
    if let Some(acl) = acl
        && !acl.is_url_allowed(base_url)
    {
        return Err(format!(
            "A2A base URL blocked by network access policy: {base_url}"
        ));
    }
    Ok(())
}

/// Reject every disallowed endpoint in a discovered or inline AgentCard.
pub fn enforce_interface_policy(
    card: &AgentCard,
    acl: Option<&NetworkAccessList>,
) -> std::result::Result<(), String> {
    if let Some(acl) = acl {
        for iface in &card.supported_interfaces {
            if !acl.is_url_allowed(&iface.url) {
                return Err(format!(
                    "A2A interface URL blocked by network access policy: {}",
                    iface.url
                ));
            }
        }
    }
    Ok(())
}

/// Discover an AgentCard with no redirects and public-address DNS pinning.
///
/// The host must restrict `allow_local` to its development grade. A supplied
/// resolver controls DNS lookup; its answers still pass the same public-IP checks.
pub async fn discover_agent_card(
    base_url: &str,
    acl: Option<&NetworkAccessList>,
    allow_local: bool,
    resolver: Option<&DnsResolver>,
) -> std::result::Result<AgentCard, String> {
    enforce_discovery_policy(base_url, acl)?;
    let http = hardened_a2a_http_client(&[base_url], allow_local, resolver).await?;
    AgentCardResolver::new(Some(http))
        .resolve(base_url)
        .await
        .map_err(|error| format!("Failed to resolve A2A AgentCard: {error}"))
}

/// Construct pinned, no-redirect JSON-RPC and REST transports for a validated card.
///
/// Every interface is checked against `acl`. The host must restrict `allow_local`
/// to its development grade; configured headers and binding order are preserved.
pub async fn client_for_card(
    card: &AgentCard,
    preferred_binding: Option<&str>,
    headers: &BTreeMap<String, String>,
    acl: Option<&NetworkAccessList>,
    allow_local: bool,
    resolver: Option<&DnsResolver>,
) -> std::result::Result<Client, String> {
    enforce_interface_policy(card, acl)?;
    let interface_urls: Vec<&str> = card
        .supported_interfaces
        .iter()
        .map(|iface| iface.url.as_str())
        .collect();
    let http = hardened_a2a_http_client(&interface_urls, allow_local, resolver).await?;
    let mut builder = A2AClientFactory::builder()
        .no_defaults()
        .register(Arc::new(JsonRpcTransportFactory::new(Some(http.clone()))))
        .register(Arc::new(RestTransportFactory::new(Some(http))));
    if let Some(binding) = preferred_binding {
        builder = builder.preferred_bindings(vec![binding.to_owned()]);
    }
    let headers = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    if !headers.is_empty() {
        builder = builder.with_interceptor(Arc::new(StaticHeaderInterceptor { headers }));
    }
    builder
        .build()
        .create_from_card(card)
        .await
        .map_err(|error| format!("Failed to create A2A client: {error}"))
}

#[derive(Clone)]
struct StaticHeaderInterceptor {
    headers: Vec<(String, String)>,
}

#[async_trait]
impl CallInterceptor for StaticHeaderInterceptor {
    async fn before(
        &self,
        _method: &str,
        params: &mut ServiceParams,
    ) -> std::result::Result<(), A2AError> {
        for (name, value) in &self.headers {
            params.entry(name.clone()).or_default().push(value.clone());
        }
        Ok(())
    }
}
