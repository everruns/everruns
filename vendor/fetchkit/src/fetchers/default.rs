// Decisions:
// - Ignore ambient proxy env by default. Shared agent runtimes should not inherit network routing.
// - Cap decompressed textual bodies. LLM-oriented fetches do not need unbounded response growth.
//! Default HTTP fetcher
//!
//! Handles general HTTP/HTTPS URLs with HTML conversion support.
//! This is the fallback fetcher that handles any URL not matched by
//! specialized fetchers.

use crate::client::FetchOptions;
use crate::convert::{
    extract_headings, extract_metadata, filter_excessive_newlines, html_to_markdown, html_to_text,
    is_html, is_markdown_content_type, is_plain_text_content_type, strip_boilerplate,
};
use crate::error::FetchError;
use crate::fetchers::Fetcher;
use crate::file_saver::FileSaver;
use crate::transport::{BodyStream, TransportMethod, TransportRequest, TransportResponse};
use crate::types::{FetchRequest, FetchResponse, HttpMethod};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_DISPOSITION, LOCATION, USER_AGENT};
use std::time::Duration;
use tracing::{debug, error, warn};
use url::Url;

/// Look up a header value (case-insensitive) from a transport response's header list.
pub(crate) fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Convert a reqwest `HeaderMap` into the transport's `(name, value)` header list.
/// Headers whose value is not valid UTF-8 are dropped (fetchkit only sets UTF-8 headers).
fn headers_to_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

/// Resolve the policy-pinned socket addresses for a URL (TM-SSRF-001/005).
pub(crate) fn pinned_addrs_for_url(
    url: &Url,
    options: &FetchOptions,
) -> Result<Vec<std::net::SocketAddr>, FetchError> {
    let Some(host) = url.host_str() else {
        return Ok(Vec::new());
    };
    let port = url.port_or_known_default().unwrap_or(80);
    options
        .dns_policy
        .pinned_addrs(host, port)
        .map_err(|_| FetchError::BlockedUrl)
}

/// Binary content type prefixes
const BINARY_PREFIXES: &[&str] = &[
    "image/",
    "audio/",
    "video/",
    "application/octet-stream",
    "application/pdf",
    "application/zip",
    "application/gzip",
    "application/x-tar",
    "application/x-rar",
    "application/x-7z",
    "application/vnd.ms-",
    "application/vnd.openxmlformats",
    "font/",
];

// THREAT[TM-DOS-002]: First-byte timeout prevents slowloris / slow-start attacks
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(1);

// THREAT[TM-DOS-002]: Body timeout caps total request duration
pub(crate) const BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// Truncation message appended when body is cut short (timeout or size limit)
pub(crate) const TRUNCATION_MESSAGE: &str = "\n\n[..content truncated...]";

// THREAT[TM-SSRF-010]: Maximum redirects to follow with IP validation at each hop
const MAX_REDIRECTS: usize = 10;

// THREAT[TM-DOS-001]: Default max body size (10 MB) to prevent memory exhaustion
// THREAT[TM-DOS-003]: Also protects against compressed content bombs (gzip bombs)
pub(crate) const DEFAULT_MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// Default HTTP fetcher
///
/// Handles all HTTP/HTTPS URLs with:
/// - GET and HEAD methods
/// - HTML to markdown/text conversion
/// - Binary content detection
/// - Timeout handling with partial content
pub struct DefaultFetcher;

impl DefaultFetcher {
    /// Create a new default fetcher
    pub fn new() -> Self {
        Self
    }
}

impl Default for DefaultFetcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Build headers for HTTP requests
pub(crate) fn build_headers(
    options: &FetchOptions,
    accept: &str,
    request: &FetchRequest,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT)),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_str(accept).unwrap_or_else(|_| HeaderValue::from_static("*/*")),
    );

    // Conditional request headers
    if let Some(ref etag) = request.if_none_match {
        if let Ok(v) = HeaderValue::from_str(etag) {
            headers.insert(reqwest::header::IF_NONE_MATCH, v);
        }
    }
    if let Some(ref date) = request.if_modified_since {
        if let Ok(v) = HeaderValue::from_str(date) {
            headers.insert(reqwest::header::IF_MODIFIED_SINCE, v);
        }
    }

    headers
}

/// Apply bot-auth signature headers when the feature is enabled and configured.
#[cfg(feature = "bot-auth")]
pub(crate) fn apply_bot_auth_if_enabled(
    mut headers: HeaderMap,
    options: &FetchOptions,
    url: &Url,
) -> HeaderMap {
    if let Some(ref bot_auth) = options.bot_auth {
        if let Some(authority) = url.host_str() {
            match bot_auth.sign_request(authority) {
                Ok(auth_headers) => {
                    if let Ok(v) = HeaderValue::from_str(&auth_headers.signature) {
                        headers.insert("signature", v);
                    }
                    if let Ok(v) = HeaderValue::from_str(&auth_headers.signature_input) {
                        headers.insert("signature-input", v);
                    }
                    if let Some(ref fqdn) = auth_headers.signature_agent {
                        if let Ok(v) = HeaderValue::from_str(fqdn) {
                            headers.insert("signature-agent", v);
                        }
                    }
                }
                Err(e) => {
                    warn!("Bot-auth signing failed: {e}");
                }
            }
        }
    }
    headers
}

#[cfg(not(feature = "bot-auth"))]
pub(crate) fn apply_bot_auth_if_enabled(
    headers: HeaderMap,
    _options: &FetchOptions,
    _url: &Url,
) -> HeaderMap {
    headers
}

/// Extract common response metadata from headers
struct ResponseMeta {
    content_type: Option<String>,
    last_modified: Option<String>,
    etag: Option<String>,
    content_length: Option<u64>,
    filename: Option<String>,
}

fn extract_response_meta(headers: &[(String, String)], url: &str) -> ResponseMeta {
    ResponseMeta {
        content_type: header_value(headers, "content-type").map(|s| s.to_string()),
        last_modified: header_value(headers, "last-modified").map(|s| s.to_string()),
        etag: header_value(headers, "etag").map(|s| s.to_string()),
        content_length: header_value(headers, "content-length").and_then(|s| s.parse().ok()),
        filename: extract_filename(headers, url),
    }
}

#[async_trait]
impl Fetcher for DefaultFetcher {
    fn name(&self) -> &'static str {
        "default"
    }

    fn matches(&self, _url: &Url) -> bool {
        // Default fetcher matches all URLs
        true
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        if request.url.is_empty() {
            return Err(FetchError::MissingUrl);
        }

        let method = request.effective_method();
        let wants_markdown = options.enable_markdown && request.wants_markdown();
        let wants_text = options.enable_text && request.wants_text();
        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);

        let accept = if wants_markdown {
            "text/html, text/markdown, text/plain, */*;q=0.8"
        } else if wants_text {
            "text/html, text/plain, */*;q=0.8"
        } else {
            "*/*"
        };

        let headers = build_headers(options, accept, request);
        let parsed_url = url::Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let reqwest_method = match method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Head => reqwest::Method::HEAD,
        };

        // THREAT[TM-SSRF-010]: Follow redirects manually so every hop is re-validated.
        let (response, redirect_chain) = send_request_following_redirects(
            parsed_url,
            reqwest_method,
            headers,
            options,
            FIRST_BYTE_TIMEOUT,
        )
        .await?;

        let status_code = response.status;
        let final_url = response.url.to_string();
        let meta = extract_response_meta(&response.headers, &final_url);

        // Handle 304 Not Modified (conditional request response)
        if status_code == 304 {
            return Ok(FetchResponse {
                url: final_url,
                status_code,
                content_type: meta.content_type,
                last_modified: meta.last_modified,
                etag: meta.etag,
                ..Default::default()
            });
        }

        // Handle HEAD request
        if method == HttpMethod::Head {
            return Ok(FetchResponse {
                url: final_url,
                status_code,
                content_type: meta.content_type,
                size: meta.content_length,
                last_modified: meta.last_modified,
                etag: meta.etag,
                filename: meta.filename,
                method: Some("HEAD".to_string()),
                redirect_chain,
                ..Default::default()
            });
        }

        // Check for binary content
        if let Some(ref ct) = meta.content_type {
            if is_binary_content_type(ct) {
                return Ok(FetchResponse {
                    url: final_url,
                    status_code,
                    content_type: meta.content_type,
                    size: meta.content_length,
                    last_modified: meta.last_modified,
                    etag: meta.etag,
                    filename: meta.filename,
                    redirect_chain,
                    error: Some(
                        "Binary content is not supported. Only textual content (HTML, text, JSON, etc.) can be fetched."
                            .to_string(),
                    ),
                    ..Default::default()
                });
            }
        }

        // THREAT[TM-DOS-001]: Read body with timeout and size limit
        // THREAT[TM-DOS-003]: Size limit also protects against compressed content bombs
        let (body, truncated) =
            read_body_with_timeout(response, BODY_TIMEOUT, max_body_size).await?;
        let size = body.len() as u64;

        // Convert to string
        let content = String::from_utf8_lossy(&body).to_string();

        // Detect paywall before content is moved by conversion
        let is_paywall = detect_paywall(&content);

        // Determine format and convert if needed
        // THREAT[TM-DOS-006]: Conversion input is bounded by max_body_size
        let is_html_content = is_html(&meta.content_type, &content);
        let wants_main = request.wants_main_content();

        // Extract structured metadata from HTML content (before boilerplate stripping)
        let page_metadata = if is_html_content {
            let mut pm = extract_metadata(&content);
            pm.headings = extract_headings(&content);
            if pm.is_empty() {
                None
            } else {
                Some(pm)
            }
        } else {
            None
        };

        let (format, final_content) =
            if is_markdown_content_type(&meta.content_type) && wants_markdown {
                // Server already returned markdown — skip conversion
                debug!("Content-type is markdown; skipping HTML conversion");
                ("markdown".to_string(), content)
            } else if is_plain_text_content_type(&meta.content_type) && wants_text {
                // Server already returned plain text — skip conversion
                debug!("Content-type is plain text; skipping HTML conversion");
                ("text".to_string(), content)
            } else if is_html_content {
                // Strip boilerplate before conversion if content_focus is "main"
                let html = if wants_main {
                    strip_boilerplate(&content)
                } else {
                    content
                };
                if wants_markdown {
                    ("markdown".to_string(), html_to_markdown(&html))
                } else if wants_text {
                    ("text".to_string(), html_to_text(&html))
                } else {
                    ("raw".to_string(), html)
                }
            } else {
                ("raw".to_string(), content)
            };

        // Apply newline filtering
        let mut final_content = filter_excessive_newlines(&final_content);

        // Add truncation messages
        if truncated {
            final_content.push_str(TRUNCATION_MESSAGE);
        }

        // Compute quality signals
        let word_count = count_words(&final_content);

        Ok(FetchResponse {
            url: final_url,
            status_code,
            content_type: meta.content_type,
            size: Some(size),
            last_modified: meta.last_modified,
            etag: meta.etag,
            filename: meta.filename,
            format: Some(format),
            content: Some(final_content),
            truncated: if truncated { Some(true) } else { None },
            metadata: page_metadata,
            word_count: Some(word_count),
            redirect_chain,
            is_paywall: if is_paywall { Some(true) } else { None },
            ..Default::default()
        })
    }

    /// Fetch and save to file — binary-aware override.
    ///
    /// Unlike `fetch()`, this does NOT reject binary content. Downloads raw bytes
    /// and saves them through the provided [`FileSaver`].
    async fn fetch_to_file(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
        saver: &dyn FileSaver,
    ) -> Result<FetchResponse, FetchError> {
        let save_path = match &request.save_to_file {
            Some(path) => path.clone(),
            None => return self.fetch(request, options).await,
        };

        if request.url.is_empty() {
            return Err(FetchError::MissingUrl);
        }

        let method = request.effective_method();
        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);

        let headers = build_headers(options, "*/*", request);
        let parsed_url = url::Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let reqwest_method = match method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Head => reqwest::Method::HEAD,
        };

        // THREAT[TM-SSRF-010]: Follow redirects manually with IP validation at each hop
        let (response, redirect_chain) = send_request_following_redirects(
            parsed_url,
            reqwest_method,
            headers,
            options,
            FIRST_BYTE_TIMEOUT,
        )
        .await?;

        let status_code = response.status;
        let final_url = response.url.to_string();
        let meta = extract_response_meta(&response.headers, &final_url);

        // HEAD request — return metadata only
        if method == HttpMethod::Head {
            return Ok(FetchResponse {
                url: final_url,
                status_code,
                content_type: meta.content_type,
                size: meta.content_length,
                last_modified: meta.last_modified,
                etag: meta.etag,
                filename: meta.filename,
                method: Some("HEAD".to_string()),
                redirect_chain,
                ..Default::default()
            });
        }

        // Read raw body (no binary rejection for file saves)
        let (body, truncated) =
            read_body_with_timeout(response, BODY_TIMEOUT, max_body_size).await?;
        let size = body.len() as u64;

        // Save through the FileSaver
        let save_result = saver
            .save(&save_path, &body)
            .await
            .map_err(|e| FetchError::SaveError(e.to_string()))?;

        Ok(FetchResponse {
            url: final_url,
            status_code,
            content_type: meta.content_type,
            size: Some(size),
            last_modified: meta.last_modified,
            etag: meta.etag,
            filename: meta.filename,
            truncated: if truncated { Some(true) } else { None },
            saved_path: Some(save_result.path),
            bytes_written: Some(save_result.bytes_written),
            redirect_chain,
            // No inline content when saving to file
            ..Default::default()
        })
    }
}

/// Returns `(response, redirect_chain)` where redirect_chain lists intermediate URLs.
///
/// Performs manual, per-hop-validated redirect following (TM-SSRF-010). Each hop is
/// resolved via DNS policy (producing pinned addrs) and executed through the
/// configured [`HttpTransport`]; only the socket send is delegated to the transport.
pub(crate) async fn send_request_following_redirects(
    initial_url: Url,
    method: reqwest::Method,
    headers: HeaderMap,
    options: &FetchOptions,
    timeout: Duration,
) -> Result<(TransportResponse, Vec<String>), FetchError> {
    let transport = options.transport();
    let transport_method = if method == reqwest::Method::HEAD {
        TransportMethod::Head
    } else {
        TransportMethod::Get
    };
    let mut current_url = initial_url;
    let mut redirect_chain = Vec::new();

    for redirect_count in 0..=MAX_REDIRECTS {
        options.validate_url(&current_url)?;

        // THREAT[TM-AUTH]: re-sign bot-auth headers per hop so each authority is covered.
        let request_headers = apply_bot_auth_if_enabled(headers.clone(), options, &current_url);
        // THREAT[TM-SSRF-001]/[TM-SSRF-005]: resolve-then-check produces the pinned addrs
        // the transport must connect to.
        let pinned_addrs = pinned_addrs_for_url(&current_url, options)?;

        let req = TransportRequest {
            method: transport_method,
            url: current_url.clone(),
            headers: headers_to_pairs(&request_headers),
            timeout: Some(timeout),
            pinned_addrs,
            respect_proxy_env: options.respect_proxy_env,
        };
        let response = transport.execute(req).await?;

        let Some(next_url) = redirect_target(&current_url, &response, options)? else {
            return Ok((response, redirect_chain));
        };

        if redirect_count == MAX_REDIRECTS {
            return Err(FetchError::RequestError("too many redirects".to_string()));
        }

        debug!(
            from = %current_url,
            to = %next_url,
            hop = redirect_count + 1,
            "Following redirect with IP validation"
        );

        redirect_chain.push(current_url.to_string());
        current_url = next_url;
    }

    unreachable!("redirect loop must return before exhausting iterations");
}

/// Execute a single (non-redirect-following) request through the configured transport.
///
/// Shared by specialized fetchers that issue simple API GETs to hardcoded hosts.
/// DNS policy resolution against `pin_host`/`pin_port` produces the pinned addrs the
/// transport must connect to (TM-SSRF-001/005). Bot-auth headers are applied for the
/// request URL's authority. The transport never follows redirects.
///
/// `pin_host`/`pin_port` are the hardcoded API host/port (so DNS pinning matches the
/// host the URL will actually connect to). They normally equal the URL's host/port.
pub(crate) async fn transport_request(
    url: Url,
    method: reqwest::Method,
    headers: HeaderMap,
    options: &FetchOptions,
    timeout: Duration,
    pin_host: &str,
    pin_port: u16,
) -> Result<TransportResponse, FetchError> {
    let transport = options.transport();
    let transport_method = if method == reqwest::Method::HEAD {
        TransportMethod::Head
    } else {
        TransportMethod::Get
    };
    options.validate_url(&url)?;

    // THREAT[TM-AUTH]: sign for the target authority before handing to the transport.
    let request_headers = apply_bot_auth_if_enabled(headers, options, &url);
    // THREAT[TM-SSRF-001]/[TM-SSRF-005]: resolve-then-check the API host; pin the result.
    let pinned_addrs = options
        .dns_policy
        .pinned_addrs(pin_host, pin_port)
        .map_err(|_| FetchError::BlockedUrl)?;

    let req = TransportRequest {
        method: transport_method,
        url,
        headers: headers_to_pairs(&request_headers),
        timeout: Some(timeout),
        pinned_addrs,
        respect_proxy_env: options.respect_proxy_env,
    };
    Ok(transport.execute(req).await?)
}

/// Read an entire transport response body as bytes, applying the body timeout and
/// `max_body_size` cap. Returns the bytes (truncation flag discarded — callers that
/// need it should use [`read_body_with_timeout`] directly).
pub(crate) async fn read_full_body(
    response: TransportResponse,
    options: &FetchOptions,
) -> Result<Bytes, FetchError> {
    let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);
    let (body, _truncated) = read_body_with_timeout(response, BODY_TIMEOUT, max_body_size).await?;
    Ok(body)
}

fn redirect_target(
    base_url: &Url,
    response: &TransportResponse,
    options: &FetchOptions,
) -> Result<Option<Url>, FetchError> {
    // 304 Not Modified is in the 3xx range but is not a redirect
    let status = response.status;
    let is_redirection = (300..400).contains(&status);
    if !is_redirection || status == 304 {
        return Ok(None);
    }

    let location = header_value(&response.headers, LOCATION.as_str()).ok_or_else(|| {
        FetchError::RequestError("redirect response missing Location header".to_string())
    })?;

    let next_url = base_url.join(location).map_err(|_| {
        FetchError::RequestError("redirect Location is not a valid URL".to_string())
    })?;

    // THREAT[TM-INPUT-001]: Validate scheme at each redirect hop
    if next_url.scheme() != "http" && next_url.scheme() != "https" {
        return Err(FetchError::InvalidUrlScheme);
    }

    options.validate_redirect_target(base_url, &next_url)?;

    Ok(Some(next_url))
}

/// Check if content type indicates binary content
fn is_binary_content_type(content_type: &str) -> bool {
    let ct_lower = content_type.to_lowercase();
    BINARY_PREFIXES
        .iter()
        .any(|prefix| ct_lower.starts_with(prefix))
}

/// Extract filename from Content-Disposition header or URL
fn extract_filename(headers: &[(String, String)], url: &str) -> Option<String> {
    // Try Content-Disposition header first
    if let Some(value) = header_value(headers, CONTENT_DISPOSITION.as_str()) {
        if let Some(filename) = parse_content_disposition_filename(value) {
            return Some(filename);
        }
    }

    // Fallback to URL path
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(mut segments) = parsed.path_segments() {
            if let Some(last) = segments.next_back() {
                if last.contains('.') && !last.is_empty() {
                    return Some(last.to_string());
                }
            }
        }
    }

    None
}

/// Parse filename from Content-Disposition header value
fn parse_content_disposition_filename(value: &str) -> Option<String> {
    let patterns = ["filename=\"", "filename="];
    for pattern in patterns {
        if let Some(start) = value.find(pattern) {
            let rest = &value[start + pattern.len()..];
            if pattern.ends_with('"') {
                // Quoted
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            } else {
                // Unquoted - take until space or semicolon
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == ';')
                    .unwrap_or(rest.len());
                let filename = rest[..end].trim_matches('"');
                if !filename.is_empty() {
                    return Some(filename.to_string());
                }
            }
        }
    }
    None
}

/// Read response body with timeout and size limit, returning partial content if either is hit.
///
/// Returns `(body_bytes, truncated)`. `truncated` is true if the body was cut short
/// due to timeout or exceeding `max_size`.
// THREAT[TM-DOS-001]: Configurable max body size prevents unbounded memory usage
// THREAT[TM-DOS-003]: Decompressed size is checked, catching gzip/brotli bombs
pub(crate) async fn read_body_with_timeout(
    response: TransportResponse,
    timeout: Duration,
    max_size: usize,
) -> Result<(Bytes, bool), FetchError> {
    read_body_stream_with_timeout(response.body, timeout, max_size).await
}

/// Read a boxed transport body stream with timeout and size limit.
///
/// Splitting this out from [`read_body_with_timeout`] lets the transport's streaming
/// body drive the same caps without depending on `reqwest::Response`.
pub(crate) async fn read_body_stream_with_timeout(
    mut stream: BodyStream,
    timeout: Duration,
    max_size: usize,
) -> Result<(Bytes, bool), FetchError> {
    let mut body = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let chunk_future = stream.next();
        let timeout_future = tokio::time::sleep_until(deadline);

        tokio::select! {
            chunk = chunk_future => {
                match chunk {
                    Some(Ok(bytes)) => {
                        let remaining = max_size.saturating_sub(body.len());
                        if remaining == 0 {
                            warn!("Body size limit reached ({}), truncating", max_size);
                            return Ok((Bytes::from(body), true));
                        }
                        if bytes.len() > remaining {
                            body.extend_from_slice(&bytes[..remaining]);
                            warn!("Body size limit reached ({}), truncating", max_size);
                            return Ok((Bytes::from(body), true));
                        }
                        body.extend_from_slice(&bytes);
                    }
                    Some(Err(e)) => {
                        error!("Error reading body chunk: {}", e);
                        if body.is_empty() {
                            return Err(e.into());
                        }
                        return Ok((Bytes::from(body), true));
                    }
                    None => {
                        // Stream complete
                        return Ok((Bytes::from(body), false));
                    }
                }
            }
            _ = timeout_future => {
                warn!("Body timeout reached, returning partial content");
                return Ok((Bytes::from(body), true));
            }
        }
    }
}

/// Count words in text content.
fn count_words(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

/// Common paywall indicators in raw HTML content.
const PAYWALL_INDICATORS: &[&str] = &[
    "paywall",
    "subscribe to read",
    "subscribe to continue",
    "subscription required",
    "premium content",
    "members only",
    "sign in to read",
    "log in to read",
    "create a free account",
    "already a subscriber",
    "unlock this article",
    "get unlimited access",
    "start your free trial",
];

/// Heuristic paywall detection from raw HTML.
fn detect_paywall(html: &str) -> bool {
    let lower = html.to_lowercase();
    PAYWALL_INDICATORS
        .iter()
        .any(|indicator| lower.contains(indicator))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::DnsPolicy;
    use crate::types::FetchRequest;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_is_binary_content_type() {
        assert!(is_binary_content_type("image/png"));
        assert!(is_binary_content_type("image/jpeg"));
        assert!(is_binary_content_type("audio/mp3"));
        assert!(is_binary_content_type("video/mp4"));
        assert!(is_binary_content_type("application/pdf"));
        assert!(is_binary_content_type("application/octet-stream"));
        assert!(is_binary_content_type("application/zip"));
        assert!(is_binary_content_type("application/vnd.ms-excel"));
        assert!(is_binary_content_type(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        ));
        assert!(is_binary_content_type("font/woff2"));

        assert!(!is_binary_content_type("text/html"));
        assert!(!is_binary_content_type("text/plain"));
        assert!(!is_binary_content_type("application/json"));
        assert!(!is_binary_content_type("application/javascript"));
    }

    #[test]
    fn test_parse_content_disposition_filename() {
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=\"file.pdf\""),
            Some("file.pdf".to_string())
        );
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=file.pdf"),
            Some("file.pdf".to_string())
        );
        assert_eq!(
            parse_content_disposition_filename("inline; filename=\"report.xlsx\"; size=1234"),
            Some("report.xlsx".to_string())
        );
        assert_eq!(parse_content_disposition_filename("inline"), None);
    }

    #[test]
    fn test_extract_filename_from_url() {
        let headers: Vec<(String, String)> = Vec::new();
        assert_eq!(
            extract_filename(&headers, "https://example.com/path/to/file.pdf"),
            Some("file.pdf".to_string())
        );
        assert_eq!(
            extract_filename(&headers, "https://example.com/path/to/document"),
            None
        );
        assert_eq!(extract_filename(&headers, "https://example.com/"), None);
    }

    #[test]
    fn test_default_fetcher_matches_all() {
        let fetcher = DefaultFetcher::new();
        let url = Url::parse("https://example.com").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://github.com/owner/repo").unwrap();
        assert!(fetcher.matches(&url));
    }

    #[tokio::test]
    async fn test_manual_redirect_following() {
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("redirected")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&destination)
            .await;

        let origin = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/final", destination.uri())),
            )
            .mount(&origin)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            enable_markdown: true,
            enable_text: true,
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/start", origin.uri())).as_markdown();
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.content.as_deref(), Some("redirected"));
    }

    /// Build a synthetic redirect [`TransportResponse`] for redirect_target tests.
    fn redirect_response(status: u16, location: &str) -> TransportResponse {
        TransportResponse {
            status,
            url: Url::parse("https://origin.example/start").unwrap(),
            headers: vec![("location".to_string(), location.to_string())],
            body: Box::pin(futures::stream::empty()),
        }
    }

    #[test]
    fn test_redirect_target_handles_relative_location() {
        let base_url = Url::parse("https://origin.example/start").unwrap();
        let response = redirect_response(302, "/final");

        let redirect = redirect_target(&base_url, &response, &FetchOptions::default()).unwrap();
        assert_eq!(
            redirect.unwrap(),
            Url::parse("https://origin.example/final").unwrap()
        );
    }

    #[test]
    fn test_redirect_target_rejects_non_http_location() {
        let base_url = Url::parse("https://origin.example/start").unwrap();
        let response = redirect_response(302, "file:///etc/passwd");

        let redirect = redirect_target(&base_url, &response, &FetchOptions::default());
        assert!(matches!(redirect, Err(FetchError::InvalidUrlScheme)));
    }

    #[tokio::test]
    async fn test_skip_conversion_for_markdown_content_type() {
        let server = MockServer::start().await;
        let md_body = "# Already Markdown\n\nThis is **already** formatted.";
        Mock::given(method("GET"))
            .and(path("/doc"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(md_body, "text/markdown; charset=utf-8"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            enable_markdown: true,
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/doc", server.uri())).as_markdown();
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.format.as_deref(), Some("markdown"));
        // Content should be passed through without HTML conversion mangling
        assert!(response
            .content
            .as_deref()
            .unwrap()
            .contains("# Already Markdown"));
        assert!(response.content.as_deref().unwrap().contains("**already**"));
    }

    #[tokio::test]
    async fn test_skip_conversion_for_plain_text_content_type() {
        let server = MockServer::start().await;
        let text_body = "Just plain text\nwith newlines.";
        Mock::given(method("GET"))
            .and(path("/plain"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(text_body)
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            enable_text: true,
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/plain", server.uri())).as_text();
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.format.as_deref(), Some("text"));
        assert!(response
            .content
            .as_deref()
            .unwrap()
            .contains("Just plain text"));
    }

    #[tokio::test]
    async fn test_markdown_content_type_without_markdown_request_returns_raw() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/doc"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("# Title", "text/markdown; charset=utf-8"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            enable_markdown: true,
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        // Request without .as_markdown() — wants_markdown is false
        let request = FetchRequest::new(format!("{}/doc", server.uri()));
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.format.as_deref(), Some("raw"));
        assert!(response.content.as_deref().unwrap().contains("# Title"));
    }

    #[tokio::test]
    async fn test_plain_text_content_type_without_text_request_returns_raw() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/plain"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("hello world", "text/plain"))
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        // Request without .as_text() — wants_text is false
        let request = FetchRequest::new(format!("{}/plain", server.uri()));
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.format.as_deref(), Some("raw"));
    }

    #[cfg(feature = "bot-auth")]
    #[tokio::test]
    async fn test_bot_auth_headers_sent() {
        use crate::bot_auth::BotAuthConfig;
        use wiremock::matchers::header_exists;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/authed"))
            .and(header_exists("signature"))
            .and(header_exists("signature-input"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("ok")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            enable_markdown: true,
            dns_policy: DnsPolicy::allow_all(),
            bot_auth: Some(BotAuthConfig::from_seed([10u8; 32])),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/authed", server.uri())).as_markdown();
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.content.as_deref(), Some("ok"));
    }

    #[cfg(feature = "bot-auth")]
    #[tokio::test]
    async fn test_bot_auth_signature_agent_header_sent() {
        use crate::bot_auth::BotAuthConfig;
        use wiremock::matchers::{header, header_exists};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/agent"))
            .and(header_exists("signature"))
            .and(header_exists("signature-input"))
            .and(header("signature-agent", "bot.example.com"))
            .respond_with(ResponseTemplate::new(200).set_body_string("agent ok"))
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            bot_auth: Some(BotAuthConfig::from_seed([11u8; 32]).with_agent_fqdn("bot.example.com")),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/agent", server.uri()));
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.status_code, 200);
    }

    #[tokio::test]
    async fn test_etag_returned_in_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("content")
                    .insert_header("content-type", "text/plain")
                    .insert_header("etag", "\"abc123\""),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/page", server.uri()));
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.etag.as_deref(), Some("\"abc123\""));
    }

    #[tokio::test]
    async fn test_conditional_fetch_304_not_modified() {
        use wiremock::matchers::header;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .and(header("if-none-match", "\"abc123\""))
            .respond_with(ResponseTemplate::new(304).insert_header("etag", "\"abc123\""))
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request =
            FetchRequest::new(format!("{}/page", server.uri())).if_none_match("\"abc123\"");
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.status_code, 304);
        assert_eq!(response.etag.as_deref(), Some("\"abc123\""));
        assert!(response.content.is_none());
        assert!(response.format.is_none());
    }

    #[tokio::test]
    async fn test_conditional_fetch_if_modified_since() {
        use wiremock::matchers::header_exists;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .and(header_exists("if-modified-since"))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/page", server.uri()))
            .if_modified_since("Wed, 21 Oct 2015 07:28:00 GMT");
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.status_code, 304);
        assert!(response.content.is_none());
    }

    #[test]
    fn test_count_words() {
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("  one  two  three  "), 3);
        assert_eq!(count_words("word"), 1);
    }

    #[test]
    fn test_detect_paywall() {
        assert!(detect_paywall("<div class=\"paywall\">Subscribe</div>"));
        assert!(detect_paywall("<p>Subscribe to read the full article</p>"));
        assert!(detect_paywall("<span>Already a subscriber? Log in</span>"));
        assert!(detect_paywall("<div>Unlock this article</div>"));
        assert!(!detect_paywall("<p>This is a normal article</p>"));
        assert!(!detect_paywall("<h1>Hello World</h1><p>Free content</p>"));
    }

    #[tokio::test]
    async fn test_word_count_in_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/article"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("Hello world this is a test")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/article", server.uri()));
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.word_count, Some(6));
    }

    #[tokio::test]
    async fn test_redirect_chain_tracked() {
        let destination = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("arrived")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&destination)
            .await;

        let origin = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/final", destination.uri())),
            )
            .mount(&origin)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/start", origin.uri()));
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.redirect_chain.len(), 1);
        assert!(response.redirect_chain[0].contains("/start"));
    }

    #[tokio::test]
    async fn test_no_redirect_chain_for_direct_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/direct"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("direct")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/direct", server.uri()));
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert!(response.redirect_chain.is_empty());
    }

    #[tokio::test]
    async fn test_paywall_detection() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/paywalled"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body><div class='paywall'>Subscribe to read the full article</div><p>Preview...</p></body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            enable_markdown: true,
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/paywalled", server.uri())).as_markdown();
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert_eq!(response.is_paywall, Some(true));
    }

    #[tokio::test]
    async fn test_no_paywall_for_normal_content() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/free"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body><p>This is free content</p></body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&server)
            .await;

        let fetcher = DefaultFetcher::new();
        let options = FetchOptions {
            enable_markdown: true,
            dns_policy: DnsPolicy::allow_all(),
            ..Default::default()
        };
        let request = FetchRequest::new(format!("{}/free", server.uri())).as_markdown();
        let response = fetcher.fetch(&request, &options).await.unwrap();

        assert!(response.is_paywall.is_none());
    }
}
