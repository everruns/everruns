//! Egress-backed fetch path for the `web_fetch` capability.
//!
//! Implements `specs/egress.md` migration step 3: outbound web_fetch HTTP goes
//! through the host `EgressService` instead of fetchkit's internal clients.
//! fetchkit remains the request/response adapter — it still owns the tool
//! schema, description, and HTML→markdown/text conversion — but transport,
//! the per-session `NetworkAccessList`, and the deployment-wide system
//! allowlist are enforced at the egress boundary (`specs/system-allowlist.md`).
//!
//! Behavior differences vs the legacy fetchkit direct path:
//! - Specialized fetchers (GitHub, Wikipedia, arXiv, ...) do not apply; every
//!   URL is fetched generically. They own private HTTP clients inside fetchkit
//!   that cannot route through egress. Restoring them requires upstream
//!   transport injection in fetchkit.
//! - SSRF protection uses `crate::url_validation::validate_url_dns_pinned`
//!   (resolve-then-check with DNS pinning, TM-TOOL-018) instead of fetchkit's
//!   `DnsPolicy`, matching the MCP and Lua egress call sites.
//! - Request signing is requested as `EgressSigning::PlatformDefault`; the
//!   fetchkit-local bot-auth Ed25519 signer applies only to the legacy direct
//!   path (`specs/fetchkit.md`).
//!
//! Redirects are followed manually (TM-SSRF-010): every hop is re-validated
//! against the network access list and SSRF rules, and each hop is a separate
//! egress request, so the system allowlist applies to redirect targets too.

use crate::egress::{
    EgressError, EgressRequest, EgressRequestKind, EgressService, EgressSigning,
    EgressStreamResponse,
};
use crate::network_access::NetworkAccessList;
use fetchkit::file_saver::FileSaver;
use fetchkit::{
    DEFAULT_USER_AGENT, FetchError, FetchRequest, FetchResponse, HttpMethod, extract_headings,
    extract_metadata, html_to_markdown, html_to_text, strip_boilerplate,
};
use futures::StreamExt;
use std::collections::BTreeMap;
use std::time::Duration;
use url::Url;

// Limits mirror fetchkit's DefaultFetcher so tool behavior stays consistent
// across the egress and legacy paths.
const MAX_REDIRECTS: usize = 10;
const BODY_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_BODY_SIZE: usize = 10 * 1024 * 1024;
const TRUNCATION_MESSAGE: &str = "\n\n[..content truncated...]";

/// Binary content type prefixes (mirrors fetchkit's DefaultFetcher).
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

/// Fetch a URL through the host egress boundary.
///
/// `network_access` is the merged harness/agent/session list; it is checked
/// per redirect hop here (for clear errors) and passed on the `EgressRequest`
/// so the egress service remains the final enforcement point. When `saver` is
/// set and the request has `save_to_file`, raw bytes (including binary) are
/// written through it instead of being returned inline.
pub(crate) async fn fetch_via_egress(
    request: &FetchRequest,
    egress: &dyn EgressService,
    network_access: Option<&NetworkAccessList>,
    saver: Option<&dyn FileSaver>,
) -> Result<FetchResponse, FetchError> {
    fetch_via_egress_with_limit(
        request,
        egress,
        network_access,
        saver,
        DEFAULT_MAX_BODY_SIZE,
    )
    .await
}

async fn fetch_via_egress_with_limit(
    request: &FetchRequest,
    egress: &dyn EgressService,
    network_access: Option<&NetworkAccessList>,
    saver: Option<&dyn FileSaver>,
    max_body_size: usize,
) -> Result<FetchResponse, FetchError> {
    if request.url.is_empty() {
        return Err(FetchError::MissingUrl);
    }
    let method = request.effective_method();
    let saving = saver.is_some() && request.save_to_file.is_some();

    let accept = if saving {
        "*/*"
    } else if request.wants_markdown() {
        "text/html, text/markdown, text/plain, */*;q=0.8"
    } else if request.wants_text() {
        "text/html, text/plain, */*;q=0.8"
    } else {
        "*/*"
    };

    let (response, final_url, redirect_chain) = send_following_redirects(
        &request.url,
        method,
        accept,
        request,
        egress,
        network_access,
    )
    .await?;

    let status_code = response.status;
    let meta = ResponseMeta::from_headers(&response.headers, &final_url);

    // 304 Not Modified (conditional request response) — no body to process.
    if status_code == 304 {
        return Ok(FetchResponse {
            url: final_url,
            status_code,
            content_type: meta.content_type,
            last_modified: meta.last_modified,
            etag: meta.etag,
            redirect_chain,
            ..Default::default()
        });
    }

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

    if saving {
        // File saves accept binary content; raw bytes go through the saver.
        let (body, truncated) = read_body_capped(response, max_body_size).await?;
        let size = body.len() as u64;
        let save_path = request.save_to_file.as_deref().unwrap_or_default();
        let result = saver
            .expect("saving implies saver")
            .save(save_path, &body)
            .await
            .map_err(|e| FetchError::SaveError(e.to_string()))?;
        return Ok(FetchResponse {
            url: final_url,
            status_code,
            content_type: meta.content_type,
            size: Some(size),
            last_modified: meta.last_modified,
            etag: meta.etag,
            filename: meta.filename,
            truncated: if truncated { Some(true) } else { None },
            saved_path: Some(result.path),
            bytes_written: Some(result.bytes_written),
            redirect_chain,
            ..Default::default()
        });
    }

    // Inline responses reject binary content (metadata only).
    if let Some(ref ct) = meta.content_type
        && is_binary_content_type(ct)
    {
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

    let (body, truncated) = read_body_capped(response, max_body_size).await?;
    let size = body.len() as u64;
    let content = String::from_utf8_lossy(&body).to_string();
    let is_paywall = detect_paywall(&content);
    let is_html_content = is_html(&meta.content_type, &content);

    // Structured page metadata is extracted before boilerplate stripping.
    let page_metadata = if is_html_content {
        let mut pm = extract_metadata(&content);
        pm.headings = extract_headings(&content);
        if pm.is_empty() { None } else { Some(pm) }
    } else {
        None
    };

    let wants_markdown = request.wants_markdown();
    let wants_text = request.wants_text();
    let (format, final_content) = if is_markdown_content_type(&meta.content_type) && wants_markdown
    {
        ("markdown".to_string(), content)
    } else if is_plain_text_content_type(&meta.content_type) && wants_text {
        ("text".to_string(), content)
    } else if is_html_content {
        let html = if request.wants_main_content() {
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

    let mut final_content = filter_excessive_newlines(&final_content);
    if truncated {
        final_content.push_str(TRUNCATION_MESSAGE);
    }
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

/// Send one egress request per redirect hop, re-validating every hop.
///
/// Returns `(response, final_url, redirect_chain)` where `redirect_chain`
/// lists intermediate URLs (empty when no redirects occurred).
async fn send_following_redirects(
    initial_url: &str,
    method: HttpMethod,
    accept: &str,
    request: &FetchRequest,
    egress: &dyn EgressService,
    network_access: Option<&NetworkAccessList>,
) -> Result<(EgressStreamResponse, String, Vec<String>), FetchError> {
    let mut current_url = initial_url.to_string();
    let mut redirect_chain: Vec<String> = Vec::new();

    for _hop in 0..=MAX_REDIRECTS {
        // Per-hop network access check for a clear error; the egress service
        // re-checks via `EgressRequest.network_access` as the final gate.
        if let Some(acl) = network_access
            && !acl.is_url_allowed(&current_url)
        {
            return Err(FetchError::FetcherError(format!(
                "URL blocked by network access policy: {current_url}"
            )));
        }

        // THREAT[TM-API-008]/[TM-TOOL-018]: SSRF resolve-then-check with DNS
        // pinning; the pinned addresses are handed to the egress transport.
        let (validated_url, resolved_addrs) =
            crate::url_validation::validate_url_dns_pinned(&current_url)
                .await
                .map_err(map_url_validation_error)?;
        let pin_host = validated_url.host_str().unwrap_or("").to_string();

        let mut egress_request = EgressRequest::new(
            method.to_string(),
            &current_url,
            EgressRequestKind::Capability,
        )
        .header("user-agent", DEFAULT_USER_AGENT)
        .header("accept", accept)
        .signing(EgressSigning::PlatformDefault)
        .network_access(network_access.cloned())
        .pinned_addrs(pin_host, resolved_addrs)
        .timeout_ms(REQUEST_TIMEOUT_MS);
        if let Some(ref etag) = request.if_none_match {
            egress_request = egress_request.header("if-none-match", etag.clone());
        }
        if let Some(ref date) = request.if_modified_since {
            egress_request = egress_request.header("if-modified-since", date.clone());
        }

        let response = egress
            .send_stream(egress_request)
            .await
            .map_err(map_egress_error)?;

        // 304 is in the 3xx range but is not a redirect.
        if !(300..400).contains(&response.status) || response.status == 304 {
            return Ok((response, current_url, redirect_chain));
        }

        let location = response
            .headers
            .get("location")
            .ok_or_else(|| {
                FetchError::RequestError("redirect response missing Location header".to_string())
            })?
            .clone();
        let base = Url::parse(&current_url)
            .map_err(|_| FetchError::RequestError("redirect base URL invalid".to_string()))?;
        let next_url = base.join(&location).map_err(|_| {
            FetchError::RequestError("redirect Location is not a valid URL".to_string())
        })?;
        // THREAT[TM-INPUT-001]: Validate scheme at each redirect hop.
        if next_url.scheme() != "http" && next_url.scheme() != "https" {
            return Err(FetchError::InvalidUrlScheme);
        }

        redirect_chain.push(current_url);
        current_url = next_url.to_string();
    }

    Err(FetchError::RequestError(format!(
        "too many redirects (more than {MAX_REDIRECTS})"
    )))
}

/// Map URL validation failures to fetch errors, keeping parity with the
/// legacy path: malformed URLs and bad schemes surface as "Invalid URL…"
/// rather than a generic policy block, which is reserved for SSRF denials.
fn map_url_validation_error(error: crate::url_validation::UrlValidationError) -> FetchError {
    use crate::url_validation::UrlValidationError;
    match error {
        UrlValidationError::InvalidUrl(_) | UrlValidationError::DisallowedScheme(_) => {
            FetchError::InvalidUrlScheme
        }
        UrlValidationError::MissingHostname | UrlValidationError::BlockedHost(_) => {
            FetchError::BlockedUrl
        }
    }
}

/// Map egress failures to fetch errors.
///
/// The caller checks the session network access list itself before sending,
/// so a `NetworkAccessDenied` from the egress boundary means the deployment's
/// system allowlist denied the URL (`specs/system-allowlist.md`).
fn map_egress_error(error: EgressError) -> FetchError {
    match error {
        EgressError::NetworkAccessDenied { url } => FetchError::FetcherError(format!(
            "Endpoint blocked by system policy: {url} is not on the deployment's \
             system allowlist of permitted public resources."
        )),
        EgressError::InvalidRequest(message) => FetchError::RequestError(message),
        EgressError::SigningUnavailable => {
            FetchError::RequestError("outbound request signing unavailable".to_string())
        }
        EgressError::Transport(message) => FetchError::RequestError(message),
    }
}

/// Read the streaming body with a deadline and size cap, returning partial
/// content with `truncated = true` when either limit is hit (TM-DOS-001/003).
async fn read_body_capped(
    response: EgressStreamResponse,
    max_size: usize,
) -> Result<(Vec<u8>, bool), FetchError> {
    let mut body: Vec<u8> = Vec::new();
    let mut stream = response.body;
    let deadline = tokio::time::Instant::now() + BODY_TIMEOUT;

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        let remaining = max_size.saturating_sub(body.len());
                        if remaining == 0 {
                            return Ok((body, true));
                        }
                        if bytes.len() > remaining {
                            body.extend_from_slice(&bytes[..remaining]);
                            return Ok((body, true));
                        }
                        body.extend_from_slice(&bytes);
                    }
                    Some(Err(e)) => {
                        if body.is_empty() {
                            return Err(FetchError::RequestError(e.to_string()));
                        }
                        return Ok((body, true));
                    }
                    None => return Ok((body, false)),
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Ok((body, true));
            }
        }
    }
}

/// Common response metadata pulled from egress headers (lowercase names).
struct ResponseMeta {
    content_type: Option<String>,
    last_modified: Option<String>,
    etag: Option<String>,
    content_length: Option<u64>,
    filename: Option<String>,
}

impl ResponseMeta {
    fn from_headers(headers: &BTreeMap<String, String>, url: &str) -> Self {
        Self {
            content_type: headers.get("content-type").cloned(),
            last_modified: headers.get("last-modified").cloned(),
            etag: headers.get("etag").cloned(),
            content_length: headers.get("content-length").and_then(|s| s.parse().ok()),
            filename: extract_filename(headers, url),
        }
    }
}

// ----------------------------------------------------------------------------
// Helpers mirrored from fetchkit's private internals (DefaultFetcher/convert),
// kept in sync so egress-path output matches the legacy direct path.
// ----------------------------------------------------------------------------

fn is_binary_content_type(content_type: &str) -> bool {
    let ct_lower = content_type.to_lowercase();
    BINARY_PREFIXES
        .iter()
        .any(|prefix| ct_lower.starts_with(prefix))
}

fn is_markdown_content_type(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .and_then(|ct| ct.split(';').next())
        .map(|media_type| media_type.trim().eq_ignore_ascii_case("text/markdown"))
        .unwrap_or(false)
}

fn is_plain_text_content_type(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .and_then(|ct| ct.split(';').next())
        .map(|media_type| media_type.trim().eq_ignore_ascii_case("text/plain"))
        .unwrap_or(false)
}

fn is_html(content_type: &Option<String>, body: &str) -> bool {
    if let Some(ct) = content_type {
        let ct_lower = ct.to_lowercase();
        if ct_lower.contains("text/html") || ct_lower.contains("application/xhtml") {
            return true;
        }
    }
    let trimmed = body.trim_start();
    trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html")
}

fn filter_excessive_newlines(s: &str) -> String {
    let mut result = String::new();
    let mut newline_count = 0;
    for c in s.chars() {
        if c == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(c);
            }
        } else {
            newline_count = 0;
            result.push(c);
        }
    }
    result
}

fn count_words(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

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

fn detect_paywall(html: &str) -> bool {
    let lower = html.to_lowercase();
    PAYWALL_INDICATORS
        .iter()
        .any(|indicator| lower.contains(indicator))
}

fn extract_filename(headers: &BTreeMap<String, String>, url: &str) -> Option<String> {
    if let Some(disposition) = headers.get("content-disposition")
        && let Some(filename) = parse_content_disposition_filename(disposition)
    {
        return Some(filename);
    }
    if let Ok(parsed) = Url::parse(url)
        && let Some(mut segments) = parsed.path_segments()
        && let Some(last) = segments.next_back()
        && last.contains('.')
        && !last.is_empty()
    {
        return Some(last.to_string());
    }
    None
}

fn parse_content_disposition_filename(value: &str) -> Option<String> {
    let patterns = ["filename=\"", "filename="];
    for pattern in patterns {
        if let Some(start) = value.find(pattern) {
            let rest = &value[start + pattern.len()..];
            if pattern.ends_with('"') {
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            } else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::{EgressResponse, EgressResult};
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Programmable egress mock: pops queued responses in order and records
    /// every request it receives. Test URLs use public IP literals so
    /// `validate_url_dns_pinned` passes without DNS resolution.
    struct MockEgress {
        responses: Mutex<Vec<EgressResult<EgressResponse>>>,
        requests: Mutex<Vec<EgressRequest>>,
    }

    impl MockEgress {
        fn with_responses(responses: Vec<EgressResult<EgressResponse>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn ok(status: u16, headers: &[(&str, &str)], body: &str) -> EgressResult<EgressResponse> {
            Ok(EgressResponse {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: body.as_bytes().to_vec(),
            })
        }

        fn requested_urls(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.url.clone())
                .collect()
        }
    }

    #[async_trait]
    impl EgressService for MockEgress {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            self.requests.lock().unwrap().push(request);
            let mut responses = self.responses.lock().unwrap();
            assert!(!responses.is_empty(), "MockEgress ran out of responses");
            responses.remove(0)
        }

        async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
            })
        }
    }

    fn allow_all() -> Option<NetworkAccessList> {
        None
    }

    #[tokio::test]
    async fn fetches_html_and_converts_to_markdown() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "text/html; charset=utf-8")],
            "<html><head><title>T</title></head><body><h1>Title</h1><p>Body</p></body></html>",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/page").as_markdown();

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.format.as_deref(), Some("markdown"));
        assert!(response.content.unwrap().contains("# Title"));
        assert!(response.word_count.unwrap() > 0);
    }

    #[tokio::test]
    async fn html_without_conversion_request_returns_raw() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "text/html")],
            "<html><body><p>Raw</p></body></html>",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/page");

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        assert_eq!(response.format.as_deref(), Some("raw"));
        assert!(response.content.unwrap().contains("<p>Raw</p>"));
    }

    #[tokio::test]
    async fn follows_redirects_re_sending_each_hop_through_egress() {
        let egress = MockEgress::with_responses(vec![
            MockEgress::ok(302, &[("location", "http://93.184.216.35/final")], ""),
            MockEgress::ok(200, &[("content-type", "text/plain")], "done"),
        ]);
        let request = FetchRequest::new("http://93.184.216.34/start");

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.url, "http://93.184.216.35/final");
        assert_eq!(response.redirect_chain, vec!["http://93.184.216.34/start"]);
        assert_eq!(
            egress.requested_urls(),
            vec!["http://93.184.216.34/start", "http://93.184.216.35/final"]
        );
    }

    #[tokio::test]
    async fn relative_redirect_location_is_resolved_against_base() {
        let egress = MockEgress::with_responses(vec![
            MockEgress::ok(301, &[("location", "/moved/here")], ""),
            MockEgress::ok(200, &[("content-type", "text/plain")], "moved"),
        ]);
        let request = FetchRequest::new("http://93.184.216.34/start");

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        assert_eq!(response.url, "http://93.184.216.34/moved/here");
        assert_eq!(
            egress.requested_urls(),
            vec![
                "http://93.184.216.34/start",
                "http://93.184.216.34/moved/here"
            ]
        );
    }

    #[tokio::test]
    async fn redirect_to_non_http_scheme_is_rejected() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            302,
            &[("location", "ftp://93.184.216.34/file")],
            "",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/start");

        let error = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap_err();

        assert!(
            matches!(error, FetchError::InvalidUrlScheme),
            "got: {error}"
        );
    }

    #[tokio::test]
    async fn redirect_loop_stops_after_max_redirects() {
        // Every hop redirects back to itself; the loop must terminate.
        let hop = || MockEgress::ok(302, &[("location", "http://93.184.216.34/loop")], "");
        let egress = MockEgress::with_responses((0..=MAX_REDIRECTS + 1).map(|_| hop()).collect());
        let request = FetchRequest::new("http://93.184.216.34/loop");

        let error = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("too many redirects"),
            "got: {error}"
        );
        assert_eq!(egress.requested_urls().len(), MAX_REDIRECTS + 1);
    }

    #[tokio::test]
    async fn network_access_list_is_attached_to_every_egress_request() {
        // The egress boundary is the final enforcement point, so the merged
        // list must ride on each hop's EgressRequest, not just the first.
        let egress = MockEgress::with_responses(vec![
            MockEgress::ok(302, &[("location", "http://93.184.216.35/final")], ""),
            MockEgress::ok(200, &[("content-type", "text/plain")], "done"),
        ]);
        let acl = NetworkAccessList::allow_only(["93.184.216.34", "93.184.216.35"]);
        let request = FetchRequest::new("http://93.184.216.34/start");

        fetch_via_egress(&request, &egress, Some(&acl), None)
            .await
            .unwrap();

        let requests = egress.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for sent in requests.iter() {
            assert_eq!(sent.network_access.as_ref(), Some(&acl));
        }
    }

    #[tokio::test]
    async fn redirect_to_host_outside_access_list_is_blocked() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            302,
            &[("location", "http://93.184.216.35/elsewhere")],
            "",
        )]);
        let acl = NetworkAccessList::allow_only(["93.184.216.34"]);
        let request = FetchRequest::new("http://93.184.216.34/start");

        let error = fetch_via_egress(&request, &egress, Some(&acl), None)
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("blocked by network access policy"),
            "got: {error}"
        );
        // Only the first hop reached egress.
        assert_eq!(egress.requested_urls().len(), 1);
    }

    #[tokio::test]
    async fn redirect_to_private_address_is_blocked_by_ssrf_validation() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            302,
            &[("location", "http://169.254.169.254/latest/meta-data/")],
            "",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/start");

        let error = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap_err();

        assert!(matches!(error, FetchError::BlockedUrl), "got: {error}");
    }

    #[tokio::test]
    async fn invalid_scheme_and_malformed_urls_report_invalid_url() {
        let egress = MockEgress::with_responses(vec![]);

        for url in ["ftp://93.184.216.34/file", "not a url"] {
            let request = FetchRequest::new(url);
            let error = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
                .await
                .unwrap_err();
            assert!(
                matches!(error, FetchError::InvalidUrlScheme),
                "{url}: got {error}"
            );
        }
    }

    #[tokio::test]
    async fn egress_denial_maps_to_system_policy_error() {
        let egress = MockEgress::with_responses(vec![Err(EgressError::NetworkAccessDenied {
            url: "http://93.184.216.34/x".to_string(),
        })]);
        let request = FetchRequest::new("http://93.184.216.34/x");

        let error = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("blocked by system policy"),
            "got: {error}"
        );
    }

    #[tokio::test]
    async fn head_request_returns_metadata_without_content() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "text/html"), ("content-length", "1234")],
            "",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/page").method(HttpMethod::Head);

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        assert_eq!(response.method.as_deref(), Some("HEAD"));
        assert_eq!(response.size, Some(1234));
        assert!(response.content.is_none());
    }

    #[tokio::test]
    async fn binary_content_returns_metadata_with_error() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "image/png")],
            "\u{1}\u{2}",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/image.png");

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        assert!(response.error.unwrap().contains("Binary content"));
        assert!(response.content.is_none());
        assert_eq!(response.filename.as_deref(), Some("image.png"));
    }

    #[tokio::test]
    async fn save_to_file_accepts_binary_and_writes_through_saver() {
        use fetchkit::file_saver::{FileSaveError, SaveResult};

        struct RecordingSaver {
            saved: Mutex<Option<(String, Vec<u8>)>>,
        }

        #[async_trait]
        impl FileSaver for RecordingSaver {
            async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError> {
                *self.saved.lock().unwrap() = Some((path.to_string(), bytes.to_vec()));
                Ok(SaveResult {
                    path: path.to_string(),
                    bytes_written: bytes.len() as u64,
                })
            }
        }

        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "application/pdf")],
            "%PDF-fake",
        )]);
        let saver = RecordingSaver {
            saved: Mutex::new(None),
        };
        let request = FetchRequest::new("http://93.184.216.34/doc.pdf").save_to_file("doc.pdf");

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), Some(&saver))
            .await
            .unwrap();

        assert_eq!(response.saved_path.as_deref(), Some("doc.pdf"));
        assert_eq!(response.bytes_written, Some(9));
        assert!(response.content.is_none());
        let saved = saver.saved.lock().unwrap().clone().unwrap();
        assert_eq!(saved.1, b"%PDF-fake");
    }

    #[tokio::test]
    async fn body_is_truncated_at_size_limit() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "text/plain")],
            "0123456789",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/big");

        let response =
            fetch_via_egress_with_limit(&request, &egress, allow_all().as_ref(), None, 4)
                .await
                .unwrap();

        assert_eq!(response.truncated, Some(true));
        let content = response.content.unwrap();
        assert!(content.starts_with("0123"));
        assert!(content.contains("truncated"));
    }

    #[tokio::test]
    async fn not_modified_304_passes_through() {
        let egress =
            MockEgress::with_responses(vec![MockEgress::ok(304, &[("etag", "\"abc\"")], "")]);
        let request = FetchRequest::new("http://93.184.216.34/page").if_none_match("\"abc\"");

        let response = fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        assert_eq!(response.status_code, 304);
        assert_eq!(response.etag.as_deref(), Some("\"abc\""));
        assert!(response.content.is_none());
    }

    #[tokio::test]
    async fn conditional_headers_are_forwarded() {
        let egress = MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "text/plain")],
            "ok",
        )]);
        let request = FetchRequest::new("http://93.184.216.34/page")
            .if_none_match("\"etag1\"")
            .if_modified_since("Wed, 21 Oct 2015 07:28:00 GMT");

        fetch_via_egress(&request, &egress, allow_all().as_ref(), None)
            .await
            .unwrap();

        let requests = egress.requests.lock().unwrap();
        assert_eq!(
            requests[0].headers.get("if-none-match").map(String::as_str),
            Some("\"etag1\"")
        );
        assert_eq!(
            requests[0]
                .headers
                .get("if-modified-since")
                .map(String::as_str),
            Some("Wed, 21 Oct 2015 07:28:00 GMT")
        );
        assert_eq!(requests[0].kind, EgressRequestKind::Capability);
        assert_eq!(requests[0].signing, EgressSigning::PlatformDefault);
    }
}
