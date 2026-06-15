//! HTTP client for FetchKit
//!
//! This module provides the main entry points for fetching URLs.
//! The actual fetch logic is implemented by fetchers in the [`fetchers`](crate::fetchers) module.

#[cfg(feature = "bot-auth")]
use crate::bot_auth::BotAuthConfig;
use crate::dns::DnsPolicy;
use crate::error::FetchError;
use crate::fetchers::FetcherRegistry;
use crate::transport::{HttpTransport, ReqwestTransport};
use crate::types::{FetchRequest, FetchResponse};
use std::sync::Arc;
use url::Url;

/// Fetch options that can be configured via tool builder
#[derive(Clone, Default)]
pub struct FetchOptions {
    /// Custom User-Agent
    pub user_agent: Option<String>,
    /// Allow list of URL prefixes
    pub allow_prefixes: Vec<String>,
    /// Block list of URL prefixes
    pub block_prefixes: Vec<String>,
    /// Enable as_markdown option
    pub enable_markdown: bool,
    /// Enable as_text option
    pub enable_text: bool,
    /// DNS resolution policy for SSRF prevention
    pub dns_policy: DnsPolicy,
    /// Maximum response body size in bytes (default: 10 MB)
    /// Protects against TM-DOS-001 (unbounded body) and TM-DOS-003 (gzip bombs)
    pub max_body_size: Option<usize>,
    /// Enable save_to_file parameter in requests
    pub enable_save_to_file: bool,
    /// Whether to respect HTTP_PROXY/HTTPS_PROXY/NO_PROXY from the environment
    pub respect_proxy_env: bool,
    /// Restrict outbound requests to these ports. Empty means any port.
    pub allowed_ports: Vec<u16>,
    /// Block exact hosts and suffix rules. Leading '.' means suffix match.
    pub blocked_hosts: Vec<String>,
    /// Restrict redirects to the original host only.
    pub same_host_redirects_only: bool,
    /// Optional caller-owned URL policy evaluated before DNS resolution on every hop.
    pub url_policy: Option<Arc<dyn Fn(&Url) -> bool + Send + Sync>>,
    /// Web Bot Authentication config (draft-meunier-web-bot-auth-architecture).
    /// When set, outgoing requests are signed with Ed25519.
    #[cfg(feature = "bot-auth")]
    pub bot_auth: Option<BotAuthConfig>,
    /// Pluggable HTTP transport. When `None`, the default [`ReqwestTransport`] is used.
    ///
    /// A host application can supply its own transport to route fetchkit's outbound
    /// HTTP through a dedicated egress boundary. fetchkit still performs all URL
    /// validation, DNS policy resolution, redirect following, and content handling;
    /// only the socket-level send is delegated.
    pub transport: Option<Arc<dyn HttpTransport>>,
}

// Manual Debug: `transport` is a trait object that does not implement Debug.
impl std::fmt::Debug for FetchOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("FetchOptions");
        d.field("user_agent", &self.user_agent)
            .field("allow_prefixes", &self.allow_prefixes)
            .field("block_prefixes", &self.block_prefixes)
            .field("enable_markdown", &self.enable_markdown)
            .field("enable_text", &self.enable_text)
            .field("dns_policy", &self.dns_policy)
            .field("max_body_size", &self.max_body_size)
            .field("enable_save_to_file", &self.enable_save_to_file)
            .field("respect_proxy_env", &self.respect_proxy_env)
            .field("allowed_ports", &self.allowed_ports)
            .field("blocked_hosts", &self.blocked_hosts)
            .field("same_host_redirects_only", &self.same_host_redirects_only)
            .field("url_policy", &self.url_policy.as_ref().map(|_| "<custom>"));
        #[cfg(feature = "bot-auth")]
        d.field("bot_auth", &self.bot_auth);
        d.field(
            "transport",
            &self
                .transport
                .as_ref()
                .map(|_| "<custom>")
                .unwrap_or("None"),
        );
        d.finish()
    }
}

impl FetchOptions {
    /// Resolve the active transport, falling back to [`ReqwestTransport`] when none
    /// was supplied.
    pub(crate) fn transport(&self) -> Arc<dyn HttpTransport> {
        match &self.transport {
            Some(t) => Arc::clone(t),
            None => Arc::new(ReqwestTransport::new()),
        }
    }

    pub(crate) fn validate_url(&self, url: &Url) -> Result<(), FetchError> {
        self.validate_host(url)?;
        self.validate_port(url)?;
        if let Some(policy) = &self.url_policy {
            if !policy(url) {
                return Err(FetchError::BlockedUrl);
            }
        }
        Ok(())
    }

    pub(crate) fn validate_redirect_target(
        &self,
        current_url: &Url,
        next_url: &Url,
    ) -> Result<(), FetchError> {
        self.validate_url(next_url)?;

        if self.same_host_redirects_only
            && normalized_host(current_url) != normalized_host(next_url)
        {
            return Err(FetchError::BlockedUrl);
        }

        Ok(())
    }

    fn validate_host(&self, url: &Url) -> Result<(), FetchError> {
        let Some(host) = normalized_host(url) else {
            return Ok(());
        };

        if self
            .blocked_hosts
            .iter()
            .any(|rule| host_matches_rule(&host, rule))
        {
            return Err(FetchError::BlockedUrl);
        }

        Ok(())
    }

    fn validate_port(&self, url: &Url) -> Result<(), FetchError> {
        if self.allowed_ports.is_empty() {
            return Ok(());
        }

        let port = url.port_or_known_default().ok_or(FetchError::BlockedUrl)?;
        if self.allowed_ports.contains(&port) {
            Ok(())
        } else {
            Err(FetchError::BlockedUrl)
        }
    }
}

fn normalized_host(url: &Url) -> Option<String> {
    url.host_str()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
}

fn host_matches_rule(host: &str, rule: &str) -> bool {
    let normalized_rule = rule.trim_end_matches('.').to_ascii_lowercase();

    if let Some(suffix) = normalized_rule.strip_prefix('.') {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == normalized_rule
    }
}

/// Fetch a URL and return the response
///
/// Uses the default fetcher registry with all built-in fetchers.
/// Markdown and text conversions are enabled by default.
/// For custom options, use [`fetch_with_options`].
///
/// # Examples
///
/// ```no_run
/// use fetchkit::{FetchRequest, fetch};
///
/// # async fn example() -> Result<(), fetchkit::FetchError> {
/// let response = fetch(FetchRequest::new("https://example.com")).await?;
/// println!("Status: {}", response.status_code);
/// # Ok(())
/// # }
/// ```
pub async fn fetch(req: FetchRequest) -> Result<FetchResponse, FetchError> {
    let options = FetchOptions {
        enable_markdown: true,
        enable_text: true,
        ..Default::default()
    };
    fetch_with_options(req, options).await
}

/// Fetch a URL with custom options
///
/// Uses the default fetcher registry with all built-in fetchers.
/// For custom fetcher configuration, use [`FetcherRegistry`] directly.
pub async fn fetch_with_options(
    req: FetchRequest,
    options: FetchOptions,
) -> Result<FetchResponse, FetchError> {
    // Validate URL early
    if req.url.is_empty() {
        return Err(FetchError::MissingUrl);
    }

    // Use registry with default fetchers
    let registry = FetcherRegistry::with_defaults();
    registry.fetch(req, options).await
}

/// Default concurrency limit for batch fetching.
const DEFAULT_BATCH_CONCURRENCY: usize = 5;
/// Maximum allowed concurrency for batch fetching to prevent resource exhaustion.
const MAX_BATCH_CONCURRENCY: usize = 20;

/// Fetch multiple URLs concurrently.
///
/// Each URL is processed independently — one failure doesn't affect others.
/// Results are returned in the same order as the input requests.
/// Concurrency is limited to `concurrency` (default: 5).
///
/// # Examples
///
/// ```no_run
/// use fetchkit::{FetchRequest, batch_fetch};
///
/// # async fn example() -> Result<(), fetchkit::FetchError> {
/// let requests = vec![
///     FetchRequest::new("https://example.com").as_markdown(),
///     FetchRequest::new("https://example.org").as_markdown(),
/// ];
/// let results = batch_fetch(requests, None).await;
/// for result in &results {
///     match result {
///         Ok(resp) => println!("{}: {}", resp.url, resp.status_code),
///         Err(e) => println!("Error: {}", e),
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub async fn batch_fetch(
    requests: Vec<FetchRequest>,
    concurrency: Option<usize>,
) -> Vec<Result<FetchResponse, FetchError>> {
    let options = FetchOptions {
        enable_markdown: true,
        enable_text: true,
        ..Default::default()
    };
    batch_fetch_with_options(requests, options, concurrency).await
}

/// Fetch multiple URLs concurrently with custom options.
///
/// Each URL is processed independently with the shared options.
/// Returns results in the same order as input requests.
pub async fn batch_fetch_with_options(
    requests: Vec<FetchRequest>,
    options: FetchOptions,
    concurrency: Option<usize>,
) -> Vec<Result<FetchResponse, FetchError>> {
    use futures::stream::{self, StreamExt};
    use std::sync::Arc;

    let concurrency = concurrency
        .unwrap_or(DEFAULT_BATCH_CONCURRENCY)
        .clamp(1, MAX_BATCH_CONCURRENCY);
    let num_requests = requests.len();
    let options = Arc::new(options);

    let mut indexed_results: Vec<(usize, Result<FetchResponse, FetchError>)> =
        stream::iter(requests.into_iter().enumerate())
            .map(|(idx, req)| {
                let options = Arc::clone(&options);
                async move {
                    let registry = FetcherRegistry::with_defaults();
                    let result = registry.fetch(req, (*options).clone()).await;
                    (idx, result)
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

    // Sort by original index to preserve request order
    indexed_results.sort_by_key(|(idx, _)| *idx);

    // Pre-fill with errors, then replace with actual results
    let mut results: Vec<Result<FetchResponse, FetchError>> = (0..num_requests)
        .map(|_| Err(FetchError::MissingUrl))
        .collect();
    for (idx, result) in indexed_results {
        results[idx] = result;
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_empty_url() {
        let req = FetchRequest::new("");
        let result = fetch(req).await;
        assert!(matches!(result, Err(FetchError::MissingUrl)));
    }

    #[tokio::test]
    async fn test_fetch_invalid_scheme() {
        let req = FetchRequest::new("ftp://example.com");
        let result = fetch(req).await;
        assert!(matches!(result, Err(FetchError::InvalidUrlScheme)));
    }

    #[tokio::test]
    async fn test_fetch_options_default() {
        let options = FetchOptions::default();
        assert!(options.user_agent.is_none());
        assert!(options.allow_prefixes.is_empty());
        assert!(options.block_prefixes.is_empty());
        assert!(!options.enable_markdown);
        assert!(!options.enable_text);
        // Safe by default: private IPs blocked
        assert!(options.dns_policy.block_private);
        assert!(options.max_body_size.is_none());
        assert!(!options.enable_save_to_file);
        assert!(!options.respect_proxy_env);
        assert!(options.allowed_ports.is_empty());
        assert!(options.blocked_hosts.is_empty());
        assert!(!options.same_host_redirects_only);
    }

    #[test]
    fn test_validate_url_blocks_configured_host_and_port() {
        let options = FetchOptions {
            allowed_ports: vec![443],
            blocked_hosts: vec!["localhost".to_string(), ".internal".to_string()],
            ..Default::default()
        };

        assert!(matches!(
            options.validate_url(&Url::parse("https://api.internal").unwrap()),
            Err(FetchError::BlockedUrl)
        ));
        assert!(matches!(
            options.validate_url(&Url::parse("https://example.com:8443").unwrap()),
            Err(FetchError::BlockedUrl)
        ));
        assert!(options
            .validate_url(&Url::parse("https://example.com").unwrap())
            .is_ok());
    }

    #[test]
    fn test_validate_redirect_target_blocks_cross_host_when_enabled() {
        let options = FetchOptions {
            same_host_redirects_only: true,
            ..Default::default()
        };

        let current = Url::parse("https://example.com/start").unwrap();
        let next = Url::parse("https://www.example.com/end").unwrap();

        assert!(matches!(
            options.validate_redirect_target(&current, &next),
            Err(FetchError::BlockedUrl)
        ));
    }

    #[tokio::test]
    async fn test_batch_fetch_multiple_urls() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("Page 1")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/page2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("Page 2")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;

        let requests = vec![
            FetchRequest::new(format!("{}/page1", server.uri())),
            FetchRequest::new(format!("{}/page2", server.uri())),
        ];

        let options = FetchOptions {
            enable_markdown: true,
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let results = batch_fetch_with_options(requests, options, None).await;

        assert_eq!(results.len(), 2);
        assert!(results[0]
            .as_ref()
            .unwrap()
            .content
            .as_deref()
            .unwrap()
            .contains("Page 1"));
        assert!(results[1]
            .as_ref()
            .unwrap()
            .content
            .as_deref()
            .unwrap()
            .contains("Page 2"));
    }

    #[tokio::test]
    async fn test_batch_fetch_partial_failure() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ok"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("OK")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;

        let requests = vec![
            FetchRequest::new(format!("{}/ok", server.uri())),
            FetchRequest::new(""), // Will fail with MissingUrl
        ];

        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let results = batch_fetch_with_options(requests, options, None).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
    }

    #[tokio::test]
    async fn test_batch_fetch_respects_concurrency_limit() {
        // Just verify it works with concurrency=1
        let requests = vec![
            FetchRequest::new(""), // Will fail
            FetchRequest::new(""), // Will fail
        ];

        let results = batch_fetch(requests, Some(1)).await;
        assert_eq!(results.len(), 2);
        assert!(results[0].is_err());
        assert!(results[1].is_err());
    }

    #[tokio::test]
    async fn test_batch_fetch_caps_caller_concurrency() {
        // Caller-provided concurrency above the cap should still work safely.
        let requests = vec![
            FetchRequest::new(""), // Will fail
            FetchRequest::new(""), // Will fail
        ];

        let results = batch_fetch(requests, Some(usize::MAX)).await;
        assert_eq!(results.len(), 2);
        assert!(results[0].is_err());
        assert!(results[1].is_err());
    }

    #[tokio::test]
    async fn test_batch_fetch_empty_input() {
        let results = batch_fetch(vec![], None).await;
        assert!(results.is_empty());
    }
}
