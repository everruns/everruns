//! Operator-configured forward proxies are infrastructure, not provider endpoints.
//! Validate the destination before forwarding; the proxy must enforce public-only
//! DNS resolution at connect time, as with the host's tenant egress proxy. Direct
//! requests retain the shared DNS-pinning resolver. Never exempt proxy hostnames
//! in that resolver: the same hostname could be supplied as a provider endpoint.

use super::{HttpError, HttpRequest, HttpResponse, HttpTransport};
use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

type ProxyKey = (http::Uri, Option<http::HeaderValue>);

struct ProxyTransport {
    matcher: Matcher,
    clients: Mutex<HashMap<ProxyKey, reqwest::Client>>,
}

pub(super) async fn execute(request: HttpRequest) -> Result<HttpResponse, HttpError> {
    static TRANSPORT: OnceLock<ProxyTransport> = OnceLock::new();
    let transport = TRANSPORT.get_or_init(|| ProxyTransport {
        matcher: Matcher::from_env(),
        clients: Mutex::new(HashMap::new()),
    });
    let Some(proxy) = transport.matcher.intercept(request.uri()) else {
        return HttpTransport::execute(
            &crate::driver_helpers::shared_streaming_http_client(),
            request,
        )
        .await;
    };

    // THREAT[TM-API-013]: credentials never reach the proxy for a blocked
    // destination, including IP literals which bypass a DNS resolver entirely.
    // Pins cannot control the proxy's connection: its operator owns that check.
    crate::url_validation::validate_safe_url(&request.uri().to_string())
        .map_err(|_| HttpError::new("provider destination blocked by network policy", false))?;
    let client = transport.client(proxy)?;
    HttpTransport::execute(&client, request).await
}

impl ProxyTransport {
    fn client(&self, proxy: Intercept) -> Result<reqwest::Client, HttpError> {
        if !matches!(proxy.uri().scheme_str(), Some("http" | "https")) {
            return Err(HttpError::new(
                "provider proxy must use HTTP or HTTPS",
                false,
            ));
        }
        let key = (proxy.uri().clone(), proxy.basic_auth().cloned());
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| HttpError::new("provider proxy client cache unavailable", false))?;
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
        let mut configured = reqwest::Proxy::all(proxy.uri().to_string())
            .map_err(|_| HttpError::new("invalid provider proxy configuration", false))?;
        if let Some(auth) = proxy.basic_auth() {
            configured = configured.custom_http_auth(auth.clone());
        }
        // Explicit proxy selection disables reqwest's automatic proxy selection.
        // NO_PROXY was already applied by the matcher. A failed proxy request has
        // no direct fallback, and build errors must not select an unhardened client.
        let client = crate::driver_helpers::proxy_streaming_http_client_builder()
            .proxy(configured)
            .build()
            .map_err(|_| HttpError::new("provider proxy client initialization failed", false))?;
        clients.insert(key, client.clone());
        Ok(client)
    }
}
