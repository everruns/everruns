// Shared Chat Driver Helpers
//
// Common utilities extracted from individual LLM driver implementations
// (Anthropic, Gemini, OpenAI) to eliminate duplication.
//
// See knowledge/foundations/llm-drivers.md for driver requirements.

use crate::driver_registry::DiscoveredModel;
use crate::error::{AgentLoopError, Result};
use crate::url_validation::is_blocked_ip;
use reqwest::StatusCode;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde::de::DeserializeOwned;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Placeholder text for audio content in providers that don't support audio input.
pub const AUDIO_CONTENT_PLACEHOLDER: &str = "[Audio content not supported]";

// ============================================================================
// Shared HTTP clients (EVE-635)
// ============================================================================

/// Connect/TLS handshake timeout applied to every provider HTTP client.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-read inactivity timeout for streaming chat clients. This bounds a
/// silently stalled connection (no bytes for this long) without capping the
/// total time a long, actively-streaming response may take — an overall
/// `.timeout()` would do the latter and kill legitimate long streams. Set well
/// above the agent loop's own stall timeout (EVE-531, ~120s) so it only acts as
/// a transport-level backstop.
const HTTP_STREAM_READ_TIMEOUT: Duration = Duration::from_secs(300);
/// Overall request timeout for non-streaming reads (embeddings, vector store)
/// so a hung response body cannot block a request indefinitely.
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Idle pooled-connection lifetime, so reused HTTP keep-alive/HTTP-2
/// connections are eventually recycled.
const HTTP_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Shorter idle lifetime for the streaming client. A provider/edge often closes
/// idle keep-alive sockets after ~10-30s; reusing one the peer already closed is
/// a dominant source of the mid-stream `error decoding response body` flake. The
/// streaming reconnect layer (`stream_reconnect`) recovers from those, but
/// recycling sooner avoids most of them outright — matching the official OpenAI
/// SDK transport (httpx), whose default `keepalive_expiry` is 5s. Kept a little
/// above 5s so genuinely rapid successive turns still reuse a warm connection.
const HTTP_STREAM_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(15);
/// DNS resolution is capped so a stuck resolver cannot stall a provider call
/// past the outbound HTTP timeout. Mirrors `url_validation`'s lookup cap.
const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// SSRF-guarding DNS resolver for the shared provider HTTP clients (EVE-623).
///
/// Provider `base_url`s are org-configurable and validated only at create time,
/// so a hostname that passed validation can later DNS-rebind to a private or
/// cloud-metadata address at request time. These streaming/request drivers hold
/// a `reqwest::Client` directly rather than routing each call through
/// [`crate::EgressService`], so we enforce the same DNS-pinning contract
/// (TM-API-013, TM-TOOL-018) inside the client itself: every resolved address is
/// checked against [`is_blocked_ip`] and the connection is refused if any
/// resolved IP is private/internal. Combined with redirects disabled, this keeps
/// these clients from reaching `169.254.169.254`/loopback/RFC1918 regardless of
/// what the configured URL or a 3xx `Location` later points at.
struct SsrfGuardResolver;

/// Boxed error type expected by reqwest's [`Resolving`] future. reqwest's own
/// `BoxError` alias is crate-private, so we spell it out here.
type DnsBoxError = Box<dyn std::error::Error + Send + Sync>;

impl Resolve for SsrfGuardResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            // hyper strips the port before resolving; resolve with port 0 and let
            // reqwest apply the URL's actual port. We only inspect the IPs here.
            let lookup = tokio::time::timeout(
                DNS_LOOKUP_TIMEOUT,
                tokio::net::lookup_host(format!("{host}:0")),
            )
            .await
            .map_err(|_| -> DnsBoxError {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "DNS lookup timed out",
                ))
            })?
            .map_err(|e| -> DnsBoxError { Box::new(e) })?;

            let addrs: Vec<std::net::SocketAddr> = lookup.collect();
            for addr in &addrs {
                if is_blocked_ip(addr.ip()) {
                    tracing::warn!(
                        host = %host,
                        resolved_ip = %addr.ip(),
                        "Provider HTTP client blocked: hostname resolves to private/internal address"
                    );
                    return Err(Box::new(std::io::Error::other(format!(
                        "host {host} resolves to blocked address {} (private/internal)",
                        addr.ip()
                    ))) as DnsBoxError);
                }
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Apply the shared SSRF-hardening to a provider HTTP client builder (EVE-623):
/// disable redirect following (a 3xx `Location` must never be auto-fetched, since
/// it can point at an internal address) and install the [`SsrfGuardResolver`].
fn harden_builder(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(SsrfGuardResolver))
}

/// Process-wide HTTP client shared by all streaming chat drivers.
///
/// `reqwest::Client` is internally reference-counted and built to be cloned and
/// shared; it carries no per-request credentials (auth headers are attached per
/// request), so one pool is safe to reuse across providers, credentials, and
/// reasoning steps. Sharing it is what lets TCP/TLS handshakes and HTTP/2
/// connections be reused across agent turns even though the driver structs are
/// rebuilt on every step (EVE-635).
pub fn shared_streaming_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            crate::install_default_crypto_provider();
            harden_builder(
                reqwest::Client::builder()
                    .connect_timeout(HTTP_CONNECT_TIMEOUT)
                    .read_timeout(HTTP_STREAM_READ_TIMEOUT)
                    .pool_idle_timeout(HTTP_STREAM_POOL_IDLE_TIMEOUT),
            )
            .build()
            // Fall back to a minimal but still SSRF-hardened client rather than
            // a bare `Client::new()`, so a builder failure can never silently
            // drop the redirect/DNS-pinning guard (EVE-623).
            .unwrap_or_else(|_| {
                harden_builder(reqwest::Client::builder())
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            })
        })
        .clone()
}

/// Process-wide HTTP client shared by non-streaming request/response drivers
/// (embeddings, vector store). Uses an overall request timeout because these
/// reads are bounded and must not hang forever.
pub fn shared_request_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            crate::install_default_crypto_provider();
            harden_builder(
                reqwest::Client::builder()
                    .connect_timeout(HTTP_CONNECT_TIMEOUT)
                    .timeout(HTTP_REQUEST_TIMEOUT)
                    .pool_idle_timeout(HTTP_POOL_IDLE_TIMEOUT),
            )
            .build()
            // Fall back to a minimal but still SSRF-hardened client rather than
            // a bare `Client::new()`, so a builder failure can never silently
            // drop the redirect/DNS-pinning guard (EVE-623).
            .unwrap_or_else(|_| {
                harden_builder(reqwest::Client::builder())
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            })
        })
        .clone()
}

// ============================================================================
// Data URL Parsing
// ============================================================================

/// Parsed data URL components (e.g., `data:image/jpeg;base64,/9j/4AAQ...`).
#[derive(Debug, Clone)]
pub struct ParsedDataUrl {
    /// MIME type (e.g., "image/jpeg", "image/png")
    pub media_type: String,
    /// Base64-encoded data (without the `data:...;base64,` prefix)
    pub data: String,
}

/// Parse a data URL into its media type and data components.
///
/// Handles formats like `data:<media_type>;base64,<data>` and `data:<media_type>,<data>`.
/// The `;base64` suffix is stripped from the media type if present, but its presence
/// is not enforced — callers should assume data may be base64-encoded.
///
/// Returns `None` if the URL doesn't start with `data:` or has no comma separator.
/// Unlike the previous per-driver implementations, this does NOT silently
/// fall back to `image/jpeg` on parse failure — callers handle fallback.
pub fn parse_data_url(url: &str) -> Option<ParsedDataUrl> {
    if !url.starts_with("data:") {
        return None;
    }

    let parts: Vec<&str> = url.splitn(2, ',').collect();
    if parts.len() != 2 {
        return None;
    }

    let media_type = parts[0]
        .trim_start_matches("data:")
        .trim_end_matches(";base64")
        .to_string();
    let data = parts[1].to_string();

    Some(ParsedDataUrl { media_type, data })
}

// ============================================================================
// Error Detection Helpers
// ============================================================================

/// Check if an HTTP error indicates the request payload is too large.
///
/// Detects common patterns across LLM providers:
/// - HTTP 413 Payload Too Large
/// - HTTP 4xx with context length / token limit errors
/// - Generic "too long" / "exceeds maximum" patterns (with token/context qualifiers)
///
/// Provider-specific patterns (must be lowercase) can be checked via `extra_patterns`.
pub fn is_request_too_large(status: StatusCode, error_text: &str, extra_patterns: &[&str]) -> bool {
    let error_lower = error_text.to_lowercase();

    // HTTP 413 Payload Too Large (universal)
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return true;
    }

    // Only check text patterns for client errors
    if status.is_client_error() {
        // Generic patterns that apply across providers
        if error_lower.contains("input is too long") || error_lower.contains("maximum context") {
            return true;
        }

        // Require a token/context qualifier with "exceeds the maximum" to avoid false positives
        if error_lower.contains("exceeds the maximum")
            && (error_lower.contains("token") || error_lower.contains("context"))
        {
            return true;
        }

        // Provider-specific patterns (already lowercase, no allocation needed)
        for pattern in extra_patterns {
            if error_lower.contains(pattern) {
                return true;
            }
        }
    }

    false
}

/// Anthropic-specific "request too large" error patterns (passed to `is_request_too_large`).
pub const ANTHROPIC_TOO_LARGE_PATTERNS: &[&str] = &[
    "prompt is too long",
    "request size exceeded",
    "context length",
    "too many tokens",
];

/// Gemini-specific "request too large" error patterns (passed to `is_request_too_large`).
pub const GEMINI_TOO_LARGE_PATTERNS: &[&str] = &[
    "request payload size exceeds",
    "content too large",
    "token limit exceeded",
];

/// Check if an HTTP error indicates the model was not found.
///
/// Only matches on 404 status. Uses provider-specific patterns (must be lowercase)
/// to avoid false positives on generic 404s (e.g., "Endpoint not found").
pub fn is_model_not_found(status: StatusCode, error_text: &str, patterns: &[&str]) -> bool {
    if status != StatusCode::NOT_FOUND {
        return false;
    }

    let error_lower = error_text.to_lowercase();

    // Provider-specific patterns (already lowercase, no allocation needed)
    for pattern in patterns {
        if error_lower.contains(pattern) {
            return true;
        }
    }

    false
}

/// Anthropic-specific model-not-found patterns.
/// Matches `not_found_error` (Anthropic's error type) or `model` + `not found` together.
pub const ANTHROPIC_NOT_FOUND_PATTERNS: &[&str] = &["not_found_error"];

/// Gemini-specific model-not-found patterns.
/// Gemini returns 404 with `"NOT_FOUND"` status or `"model"` in the message.
pub const GEMINI_NOT_FOUND_PATTERNS: &[&str] = &["not_found", "model"];

// ============================================================================
// Model Discovery (/models endpoint)
// ============================================================================

/// Fetch and map a provider's `/models` catalog into [`DiscoveredModel`]s.
///
/// Extracts the skeleton shared by the OpenAI-compatible `/models` discovery
/// implementations (Fireworks, OpenRouter, MAI/Foundry):
/// 1. send the (already authenticated) request,
/// 2. on a non-success status, drain the body to allow connection reuse and
///    return [`models_api_status_error`] — unless the status is in
///    `none_on_statuses`, in which case discovery is treated as unsupported and
///    `Ok(None)` is returned,
/// 3. deserialize the body into the provider-specific response type `T`,
/// 4. apply the provider's `map` to produce the discovered models.
///
/// Error message prefixes are passed in so each provider keeps its exact
/// user-facing wording.
///
/// Note: callers resolve the request through their runtime
/// `ProviderEndpoint` first, so this stays agnostic to the auth scheme.
pub async fn fetch_models<T, F>(
    request: reqwest::RequestBuilder,
    fetch_err_prefix: &str,
    parse_err_prefix: &str,
    none_on_statuses: &[StatusCode],
    map: F,
) -> Result<Option<Vec<DiscoveredModel>>>
where
    T: DeserializeOwned,
    F: FnOnce(T) -> Vec<DiscoveredModel>,
{
    let response = request
        .send()
        .await
        .map_err(|e| AgentLoopError::llm(format!("{fetch_err_prefix}: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let _ = response.bytes().await; // drain body to allow connection reuse
        if none_on_statuses.contains(&status) {
            return Ok(None);
        }
        return Err(crate::openai_protocol::models_api_status_error(status));
    }

    let parsed: T = response
        .json()
        .await
        .map_err(|e| AgentLoopError::llm(format!("{parse_err_prefix}: {e}")))?;

    Ok(Some(map(parsed)))
}

// ============================================================================
// Thinking Budget Constants
// ============================================================================

/// Thinking token budgets for Anthropic's extended thinking feature.
/// Maps reasoning effort levels to token budgets.
pub mod thinking_budget {
    use crate::model::ReasoningEffort;

    /// Anthropic's documented floor for a budget-based thinking request.
    pub const MINIMAL: u32 = 1024;
    pub const LOW: u32 = 1024;
    pub const MEDIUM: u32 = 4096;
    pub const HIGH: u32 = 16384;
    pub const XHIGH: u32 = 32768;

    /// Map a reasoning effort to a thinking budget.
    ///
    /// `None` means "do not send a budget": either the caller asked for no
    /// reasoning, in which case the request must omit thinking entirely.
    /// `Minimal` previously fell through here and silently produced *no*
    /// thinking rather than the smallest budget, so a user selecting the lowest
    /// non-zero effort got the same behavior as selecting none.
    pub fn from_effort(effort: ReasoningEffort) -> Option<u32> {
        match effort {
            ReasoningEffort::None => None,
            ReasoningEffort::Minimal => Some(MINIMAL),
            ReasoningEffort::Low => Some(LOW),
            ReasoningEffort::Medium => Some(MEDIUM),
            ReasoningEffort::High => Some(HIGH),
            // `Max` (OpenAI-only, above `Xhigh`) collapses onto the same top
            // budget: Anthropic's own profiles never offer it as a distinct
            // choice.
            ReasoningEffort::Xhigh | ReasoningEffort::Max => Some(XHIGH),
        }
    }
}

// ============================================================================
// Per-request headers
// ============================================================================

/// Header names a caller may never set on an outbound provider request: they
/// describe the connection, not the call, and letting a config value override
/// them corrupts the request (`host` also repoints an SSRF-guarded connection
/// at another virtual host).
const PROTECTED_REQUEST_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
];

/// Merge caller-supplied per-request headers over the headers a driver has
/// already resolved (its own protocol headers plus the provider's configured
/// and auth headers).
///
/// Matching is case-insensitive and an extra header *replaces* the existing
/// value in place rather than appending a second copy, so
/// `LlmCallConfig::extra_headers` is an override channel, not an append-only
/// one. Connection-level headers ([`PROTECTED_REQUEST_HEADERS`]) and entries
/// with an empty name are dropped with a warning.
pub fn merge_request_headers(
    base: Vec<(String, String)>,
    extra: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = base;
    for (name, value) in extra {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if PROTECTED_REQUEST_HEADERS
            .iter()
            .any(|protected| name.eq_ignore_ascii_case(protected))
        {
            tracing::warn!(
                header = %name,
                "Ignoring connection-level header in extra_headers"
            );
            continue;
        }
        // Preserve the first slot's order/name, but replace every occurrence
        // of the overridden name so a stale duplicate cannot also be sent.
        let mut replaced = false;
        merged.retain_mut(|(existing, existing_value)| {
            if !existing.eq_ignore_ascii_case(name) {
                return true;
            }
            if replaced {
                return false;
            }
            *existing_value = value.clone();
            replaced = true;
            true
        });
        if !replaced {
            merged.push((name.to_string(), value.clone()));
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn data_urls_preserve_media_and_complete_payload() {
        for (url, media, data) in [
            ("data:image/png;base64,iVBOR", "image/png", "iVBOR"),
            ("data:image/jpeg;base64,/9j/4AAQ", "image/jpeg", "/9j/4AAQ"),
            ("data:text/plain,one,two", "text/plain", "one,two"),
            ("data:image/svg+xml;base64,PD4=", "image/svg+xml", "PD4="),
            (
                "data:text/plain;charset=utf-8;base64,",
                "text/plain;charset=utf-8",
                "",
            ),
        ] {
            let parsed = parse_data_url(url).unwrap();
            assert_eq!(
                (parsed.media_type.as_str(), parsed.data.as_str()),
                (media, data)
            );
        }
        for invalid in [
            "https://example.com/image.png",
            "data:image/jpeg;base64",
            "",
            "image/png,data",
        ] {
            assert!(parse_data_url(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn payload_classification_requires_status_and_token_context_or_provider_pattern() {
        for (message, patterns, matches) in [
            ("INPUT IS TOO LONG", &[][..], true),
            ("maximum context reached", &[][..], true),
            ("request exceeds the maximum token count", &[][..], true),
            ("context exceeds the maximum", &[][..], true),
            ("rate exceeds the maximum", &[][..], false),
            ("authentication failed", &[][..], false),
            (
                "prompt is too long: 100000 tokens",
                ANTHROPIC_TOO_LARGE_PATTERNS,
                true,
            ),
            ("request size exceeded", ANTHROPIC_TOO_LARGE_PATTERNS, true),
            (
                "context length exceeded",
                ANTHROPIC_TOO_LARGE_PATTERNS,
                true,
            ),
            ("TOO MANY TOKENS", ANTHROPIC_TOO_LARGE_PATTERNS, true),
            (
                "request payload size exceeds limit",
                GEMINI_TOO_LARGE_PATTERNS,
                true,
            ),
            ("content too large", GEMINI_TOO_LARGE_PATTERNS, true),
            ("token limit exceeded", GEMINI_TOO_LARGE_PATTERNS, true),
            ("prompt is too long", GEMINI_TOO_LARGE_PATTERNS, false),
        ] {
            for (status, expected) in [
                (StatusCode::BAD_REQUEST, matches),
                (StatusCode::PAYLOAD_TOO_LARGE, true),
                (StatusCode::INTERNAL_SERVER_ERROR, false),
                (StatusCode::OK, false),
            ] {
                assert_eq!(
                    is_request_too_large(status, message, patterns),
                    expected,
                    "{status}: {message}"
                );
            }
        }
        assert!(is_request_too_large(StatusCode::PAYLOAD_TOO_LARGE, "", &[]));
    }

    #[test]
    fn missing_model_classification_requires_404_and_provider_evidence() {
        for (message, patterns, matches) in [
            (
                r#"{"error":{"type":"not_found_error"}}"#,
                ANTHROPIC_NOT_FOUND_PATTERNS,
                true,
            ),
            ("Endpoint not found", ANTHROPIC_NOT_FOUND_PATTERNS, false),
            ("NOT_FOUND", GEMINI_NOT_FOUND_PATTERNS, true),
            ("MODEL foo", GEMINI_NOT_FOUND_PATTERNS, true),
            ("missing endpoint", GEMINI_NOT_FOUND_PATTERNS, false),
            ("model not found", &[][..], false),
        ] {
            for (status, expected) in [
                (StatusCode::NOT_FOUND, matches),
                (StatusCode::BAD_REQUEST, false),
                (StatusCode::INTERNAL_SERVER_ERROR, false),
                (StatusCode::OK, false),
            ] {
                assert_eq!(
                    is_model_not_found(status, message, patterns),
                    expected,
                    "{status}: {message}"
                );
            }
        }
    }

    #[tokio::test]
    async fn resolver_rejects_private_addresses_and_returns_exact_public_address() {
        for host in [
            "127.0.0.1",
            "169.254.169.254",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
        ] {
            let error = match SsrfGuardResolver
                .resolve(Name::from_str(host).unwrap())
                .await
            {
                Ok(_) => panic!("private address {host} accepted"),
                Err(error) => error,
            };
            assert_eq!(
                error.to_string(),
                format!("host {host} resolves to blocked address {host} (private/internal)")
            );
        }
        let addresses: Vec<_> = SsrfGuardResolver
            .resolve(Name::from_str("1.1.1.1").unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addresses, ["1.1.1.1:0".parse().unwrap()]);
    }

    #[tokio::test]
    async fn both_shared_http_clients_refuse_to_follow_redirects() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&destination)
            .await;
        let source = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/source"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", destination.uri()))
            .expect(2)
            .mount(&source)
            .await;
        for client in [shared_streaming_http_client(), shared_request_http_client()] {
            let response = client
                .get(format!("{}/source", source.uri()))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FOUND);
            assert_eq!(response.headers()["location"], destination.uri());
        }
        destination.verify().await;
    }

    #[test]
    fn thinking_efforts_have_literal_budgets_including_max_alias() {
        use crate::model::ReasoningEffort::*;
        for (effort, expected) in [
            (None, Option::None),
            (Minimal, Some(1024)),
            (Low, Some(1024)),
            (Medium, Some(4096)),
            (High, Some(16384)),
            (Xhigh, Some(32768)),
            (Max, Some(32768)),
        ] {
            assert_eq!(thinking_budget::from_effort(effort), expected, "{effort:?}");
        }
    }

    #[test]
    fn caller_overrides_replace_all_duplicates_preserving_order_and_other_headers() {
        assert_eq!(
            merge_request_headers(
                vec![
                    ("X-Token".into(), "old-a".into()),
                    ("anthropic-version".into(), "2023-06-01".into()),
                    ("x-token".into(), "old-b".into())
                ],
                &[
                    (" x-TOKEN ".into(), "new".into()),
                    ("Anthropic-Version".into(), "2024-01-01".into()),
                    ("x-trace".into(), "first".into()),
                    ("X-TRACE".into(), "last".into())
                ]
            ),
            [
                ("X-Token".into(), "new".into()),
                ("anthropic-version".into(), "2024-01-01".into()),
                ("x-trace".into(), "last".into())
            ]
        );
    }

    #[test]
    fn caller_cannot_override_connection_headers_or_insert_blank_names() {
        let base = vec![
            ("content-type".into(), "application/json".into()),
            ("host".into(), "configured.example".into()),
        ];
        let extra = [
            " Host ",
            "CONTENT-LENGTH",
            "transfer-Encoding",
            "connection",
            "UPGRADE",
            "",
            " \t",
        ]
        .map(|name| (name.into(), "untrusted".into()));
        assert_eq!(merge_request_headers(base.clone(), &extra), base);
    }
}
