//! ARD registry REST client and protocol types.
//!
//! Implements the consumer side of the [Agentic Resource Discovery](https://agenticresourcediscovery.org/spec/)
//! protocol: `POST /search` (and `/explore`) against a configured registry, plus
//! the catalog-entry envelope, URN parsing, and `trustManifest` verification.
//!
//! All registry-returned text (descriptions, URNs, display names) is untrusted
//! external data. Callers must SSRF-validate any resolved `url` before fetching
//! and must run the trust gate before attaching.

use std::collections::HashMap;
use std::sync::Arc;

use everruns_core::network_access::NetworkAccessList;
use everruns_core::{EgressRequest, EgressRequestKind, EgressService};
use serde::{Deserialize, Serialize};

/// IANA media type for an MCP server catalog entry.
pub const MEDIA_TYPE_MCP_SERVER: &str = "application/mcp-server+json";
/// IANA media type for an A2A agent-card catalog entry.
pub const MEDIA_TYPE_A2A_AGENT_CARD: &str = "application/a2a-agent-card+json";

/// Federation mode requested on a search (`none` | `referrals` | `auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Federation {
    #[default]
    None,
    Referrals,
    Auto,
}

/// `POST /search` request body.
#[derive(Debug, Clone, Serialize)]
pub struct SearchRequest {
    pub query: SearchQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub federation: Option<Federation>,
    #[serde(rename = "pageSize", skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
}

/// Search query: required `text`, optional structured `filter`.
#[derive(Debug, Clone, Serialize)]
pub struct SearchQuery {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
}

/// `POST /search` response body.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchResponse {
    #[serde(default)]
    pub results: Vec<CatalogEntry>,
    #[serde(default)]
    pub referrals: Vec<serde_json::Value>,
    #[serde(rename = "pageToken", default)]
    pub page_token: Option<String>,
}

/// A catalog entry as returned in search results or a manifest. The
/// value-or-reference envelope requires exactly one of `url` / `data`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub identifier: String,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(rename = "type")]
    pub media_type: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub source: Option<String>,
    /// Remote reference to the artifact document. Mutually exclusive with `data`.
    #[serde(default)]
    pub url: Option<String>,
    /// Embedded artifact document. Mutually exclusive with `url`.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(rename = "trustManifest", default)]
    pub trust_manifest: Option<TrustManifest>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl CatalogEntry {
    pub fn display_name(&self) -> String {
        self.display_name
            .clone()
            .unwrap_or_else(|| self.identifier.clone())
    }

    /// Enforce the value-or-reference envelope: exactly one of `url` / `data`.
    pub fn validate_envelope(&self) -> Result<(), String> {
        match (self.url.as_ref(), self.data.as_ref()) {
            (Some(_), Some(_)) => Err(format!(
                "entry {} has both url and data; exactly one is allowed",
                self.identifier
            )),
            (None, None) => Err(format!(
                "entry {} has neither url nor data",
                self.identifier
            )),
            _ => Ok(()),
        }
    }
}

/// Identity/compliance metadata attached to a catalog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustManifest {
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(rename = "identityType", default)]
    pub identity_type: Option<String>,
    #[serde(default)]
    pub attestations: Vec<Attestation>,
}

/// A single attestation entry within a `trustManifest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    #[serde(rename = "type")]
    pub attestation_type: String,
    #[serde(default)]
    pub uri: Option<String>,
}

// ============================================================================
// URN parsing
// ============================================================================

/// A parsed `urn:ai:<publisher>:<namespace...>:<name>` identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArdUrn {
    /// The `<publisher>` FQDN — the trust anchor for the entry.
    pub publisher: String,
    /// Hierarchical namespace + terminal name segments after the publisher.
    pub segments: Vec<String>,
}

/// Parse an ARD URN and extract its publisher domain. Returns `Err` for
/// malformed URNs (wrong scheme, wrong NID, missing publisher).
pub fn parse_urn(urn: &str) -> Result<ArdUrn, String> {
    let parts: Vec<&str> = urn.split(':').collect();
    // urn : ai : <publisher> : <segment...>
    if parts.len() < 4 {
        return Err(format!("URN {urn} is too short"));
    }
    if !parts[0].eq_ignore_ascii_case("urn") {
        return Err(format!("URN {urn} must start with 'urn:'"));
    }
    if !parts[1].eq_ignore_ascii_case("ai") {
        return Err(format!("URN {urn} must use the 'ai' namespace identifier"));
    }
    let publisher = parts[2].trim().to_ascii_lowercase();
    if publisher.is_empty() || !publisher.contains('.') {
        return Err(format!(
            "URN {urn} has an invalid publisher domain '{publisher}'"
        ));
    }
    Ok(ArdUrn {
        publisher,
        segments: parts[3..].iter().map(|s| s.to_string()).collect(),
    })
}

// ============================================================================
// Trust verification
// ============================================================================

/// Outcome of trust verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustOutcome {
    Ok,
    Rejected(String),
}

/// Extract the host/domain from a trust-manifest identity URI such as
/// `spiffe://acme.com/travel/concierge` or `did:web:acme.com`.
pub fn identity_domain(identity: &str) -> Option<String> {
    if let Some(rest) = identity.strip_prefix("did:web:") {
        // did:web:example.com[:path...] — host is the first colon segment.
        let host = rest.split([':', '/']).next().unwrap_or(rest);
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }
    // scheme://host/path — take authority host.
    if let Some((_scheme, rest)) = identity.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        let host = host.split('@').next_back().unwrap_or(host);
        let host = host.split(':').next().unwrap_or(host);
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }
    None
}

/// Whether `identity_host` matches the URN `publisher` (exact or subdomain).
fn domain_matches(publisher: &str, identity_host: &str) -> bool {
    identity_host == publisher || identity_host.ends_with(&format!(".{publisher}"))
}

/// Verify an entry's `trustManifest` against the URN publisher domain and the
/// configured `require_trust` attestation list.
///
/// Rules:
/// - When `require_trust` is empty the gate still verifies domain ↔ URN binding
///   **if** a manifest is present, but a missing manifest is permitted.
/// - When `require_trust` is non-empty a manifest is mandatory, its identity
///   domain MUST match the URN publisher, and every required attestation
///   (case-insensitive substring match against attestation `type`) MUST appear.
pub fn verify_trust(
    urn: &ArdUrn,
    manifest: Option<&TrustManifest>,
    require_trust: &[String],
) -> TrustOutcome {
    let Some(manifest) = manifest else {
        if require_trust.is_empty() {
            return TrustOutcome::Ok;
        }
        return TrustOutcome::Rejected(format!(
            "entry has no trustManifest but require_trust={require_trust:?}"
        ));
    };

    // Domain ↔ URN binding.
    match manifest.identity.as_deref().and_then(identity_domain) {
        Some(host) if domain_matches(&urn.publisher, &host) => {}
        Some(host) => {
            return TrustOutcome::Rejected(format!(
                "trustManifest identity domain '{host}' does not match URN publisher '{}'",
                urn.publisher
            ));
        }
        None => {
            if require_trust.is_empty() {
                // No identity to verify and nothing required — allow.
                return TrustOutcome::Ok;
            }
            return TrustOutcome::Rejected(
                "trustManifest has no resolvable identity domain".to_string(),
            );
        }
    }

    // Required attestations.
    for required in require_trust {
        let needle = required.to_ascii_lowercase();
        let present = manifest
            .attestations
            .iter()
            .any(|a| a.attestation_type.to_ascii_lowercase().contains(&needle));
        if !present {
            return TrustOutcome::Rejected(format!(
                "trustManifest is missing required attestation '{required}'"
            ));
        }
    }

    TrustOutcome::Ok
}

// ============================================================================
// Registry client
// ============================================================================

/// Thin HTTP client for one ARD registry base URL.
pub struct ArdRegistryClient {
    base_url: String,
    auth_token: Option<String>,
    http: reqwest::Client,
}

impl ArdRegistryClient {
    pub fn new(base_url: impl Into<String>, auth_token: Option<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            auth_token,
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build ARD registry HTTP client"),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        let mut req = self
            .http
            .request(method, url)
            .header("Accept", "application/json");
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        req
    }

    /// `POST /search` — semantic discovery against this registry.
    pub async fn search(&self, body: &SearchRequest) -> Result<SearchResponse, String> {
        let resp = self
            .request(reqwest::Method::POST, "search")
            .json(body)
            .send()
            .await
            .map_err(|e| format!("ARD registry search request failed: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(format!(
                "ARD registry search returned HTTP {status}: {}",
                truncate(&detail, 500)
            ));
        }
        resp.json::<SearchResponse>()
            .await
            .map_err(|e| format!("failed to parse ARD search response: {e}"))
    }

    /// `POST /search` through the platform egress boundary.
    pub async fn search_with_egress(
        &self,
        body: &SearchRequest,
        egress: Arc<dyn EgressService>,
        network_access: Option<NetworkAccessList>,
        dns_pinning_required: bool,
    ) -> Result<SearchResponse, String> {
        let url = format!("{}/search", self.base_url);
        let request_body = serde_json::to_vec(body)
            .map_err(|e| format!("failed to encode ARD search request: {e}"))?;
        let mut request = EgressRequest::new("POST", url, EgressRequestKind::Integration)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(request_body)
            .network_access(network_access);
        if dns_pinning_required {
            request = request.require_dns_pinning();
        }
        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let resp = egress
            .send(request)
            .await
            .map_err(|e| format!("ARD registry search request failed: {e}"))?;
        if !(200..300).contains(&resp.status) {
            let detail = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "ARD registry search returned HTTP {}: {}",
                resp.status,
                truncate(&detail, 500)
            ));
        }
        serde_json::from_slice::<SearchResponse>(&resp.body)
            .map_err(|e| format!("failed to parse ARD search response: {e}"))
    }

    /// Fetch an artifact document referenced by an entry `url`. The URL MUST be
    /// SSRF-validated by the caller before invoking this.
    pub async fn fetch_artifact(
        &self,
        url: &str,
        resolved_addrs: &[std::net::SocketAddr],
    ) -> Result<serde_json::Value, String> {
        // Pin the connection to the validated IPs to close the DNS-rebinding
        // TOCTOU window (mirrors the scoped-MCP path).
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
        if let (Ok(parsed), false) = (url::Url::parse(url), resolved_addrs.is_empty())
            && let Some(host) = parsed.host_str()
        {
            builder = builder.resolve_to_addrs(host, resolved_addrs);
        }
        let client = builder
            .build()
            .map_err(|e| format!("failed to build artifact client: {e}"))?;
        let mut req = client.get(url).header("Accept", "application/json");
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("artifact fetch failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("artifact fetch returned HTTP {}", resp.status()));
        }
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| format!("failed to parse artifact document: {e}"))
    }

    /// Fetch an artifact document through the platform egress boundary.
    pub async fn fetch_artifact_with_egress(
        &self,
        url: &str,
        resolved_addrs: &[std::net::SocketAddr],
        egress: Arc<dyn EgressService>,
        network_access: Option<NetworkAccessList>,
    ) -> Result<serde_json::Value, String> {
        let mut request = EgressRequest::new("GET", url, EgressRequestKind::Integration)
            .header("Accept", "application/json")
            .network_access(network_access);
        if let Ok(parsed) = url::Url::parse(url)
            && let Some(host) = parsed.host_str()
        {
            request = request.pinned_addrs(host, resolved_addrs.to_vec());
        }
        if let Some(token) = &self.auth_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        let resp = egress
            .send(request)
            .await
            .map_err(|e| format!("artifact fetch failed: {e}"))?;
        if !(200..300).contains(&resp.status) {
            let detail = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "artifact fetch returned HTTP {}: {}",
                resp.status,
                truncate(&detail, 500)
            ));
        }
        serde_json::from_slice::<serde_json::Value>(&resp.body)
            .map_err(|e| format!("failed to parse artifact document: {e}"))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Slice on a char boundary at or below `max` so a non-ASCII registry
        // error body (where `max` may fall mid-codepoint) cannot panic.
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

// ============================================================================
// Artifact → attachment mapping helpers
// ============================================================================

/// Extract MCP connection details (endpoint URL + headers) from an
/// `application/mcp-server+json` artifact document. Accepts either a single
/// server object (`{ "url": ... }`) or a `.mcp.json`-style
/// `{ "mcpServers": { name: { "url": ... } } }` map (first remote entry wins).
pub fn mcp_endpoint_from_artifact(
    artifact: &serde_json::Value,
) -> Result<(String, HashMap<String, String>), String> {
    // .mcp.json style map.
    if let Some(map) = artifact.get("mcpServers").and_then(|v| v.as_object()) {
        if let Some((_, server)) = map.iter().next() {
            return mcp_endpoint_from_server_obj(server);
        }
        return Err("mcp-server artifact has empty mcpServers map".to_string());
    }
    mcp_endpoint_from_server_obj(artifact)
}

fn mcp_endpoint_from_server_obj(
    server: &serde_json::Value,
) -> Result<(String, HashMap<String, String>), String> {
    let url = server
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "mcp-server artifact is missing a 'url'".to_string())?
        .to_string();
    let headers = server
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Ok((url, headers))
}

/// Derive an A2A base URL or inline agent card from an
/// `application/a2a-agent-card+json` artifact document. Returns
/// `(base_url, agent_card)` where at least one is `Some`.
pub fn a2a_target_from_artifact(
    artifact: &serde_json::Value,
) -> Result<(Option<String>, Option<serde_json::Value>), String> {
    // A full agent card: prefer the card's own URL / supported interfaces.
    if let Some(url) = artifact.get("url").and_then(|v| v.as_str()) {
        return Ok((Some(url.to_string()), Some(artifact.clone())));
    }
    if let Some(iface_url) = artifact
        .get("supportedInterfaces")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|i| i.get("url"))
        .and_then(|v| v.as_str())
    {
        return Ok((Some(iface_url.to_string()), Some(artifact.clone())));
    }
    if let Some(base) = artifact.get("base_url").and_then(|v| v.as_str()) {
        return Ok((Some(base.to_string()), None));
    }
    Err("a2a-agent-card artifact has no url/supportedInterfaces/base_url".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_handles_non_ascii_without_panic() {
        let s = "é".repeat(400);
        // Odd byte limits split a two-byte character, including the first one.
        for (limit, expected) in [
            (0, "…".to_string()),
            (1, "…".to_string()),
            (500, format!("{}…", "é".repeat(250))),
            (501, format!("{}…", "é".repeat(250))),
            (800, s.clone()),
            (801, s.clone()),
        ] {
            assert_eq!(truncate(&s, limit), expected, "byte limit {limit}");
        }
    }

    #[test]
    fn parses_valid_urn() {
        let urn = parse_urn("urn:ai:acme.com:agent:assistant").unwrap();
        assert_eq!(urn.publisher, "acme.com");
        assert_eq!(urn.segments, vec!["agent", "assistant"]);
    }

    #[test]
    fn rejects_bad_urn_scheme_and_nid() {
        assert!(parse_urn("uri:ai:acme.com:x").is_err());
        assert!(parse_urn("urn:xx:acme.com:x").is_err());
        assert!(parse_urn("urn:ai:nodot:x").is_err());
        assert!(parse_urn("urn:ai").is_err());
    }

    #[test]
    fn extracts_identity_domain() {
        assert_eq!(
            identity_domain("spiffe://acme.com/travel/concierge").as_deref(),
            Some("acme.com")
        );
        assert_eq!(
            identity_domain("did:web:example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(identity_domain("not-a-uri"), None);
    }

    #[test]
    fn envelope_rejects_both_and_neither() {
        let mut e = CatalogEntry {
            identifier: "urn:ai:x.com:a:b".into(),
            display_name: None,
            media_type: MEDIA_TYPE_MCP_SERVER.into(),
            description: None,
            score: None,
            source: None,
            url: Some("https://x.com".into()),
            data: Some(serde_json::json!({})),
            trust_manifest: None,
            tags: vec![],
        };
        assert!(e.validate_envelope().is_err());
        e.url = None;
        e.data = None;
        assert!(e.validate_envelope().is_err());
        e.url = Some("https://x.com".into());
        assert!(e.validate_envelope().is_ok());
        e.url = None;
        e.data = Some(serde_json::json!({}));
        assert!(e.validate_envelope().is_ok());
    }

    fn urn(p: &str) -> ArdUrn {
        ArdUrn {
            publisher: p.into(),
            segments: vec!["a".into(), "b".into()],
        }
    }

    fn manifest(identity: &str, attestations: &[&str]) -> TrustManifest {
        TrustManifest {
            identity: Some(identity.into()),
            identity_type: None,
            attestations: attestations
                .iter()
                .map(|t| Attestation {
                    attestation_type: t.to_string(),
                    uri: None,
                })
                .collect(),
        }
    }

    #[test]
    fn trust_passes_with_matching_domain_and_attestation() {
        let m = manifest("spiffe://acme.com/x", &["SOC2-Type2"]);
        assert_eq!(
            verify_trust(&urn("acme.com"), Some(&m), &["soc2".into()]),
            TrustOutcome::Ok
        );
    }

    #[test]
    fn trust_rejects_domain_mismatch() {
        for identity in [
            "spiffe://evil.com/x",
            "spiffe://evilacme.com/x",
            "spiffe://acme.com.evil.com/x",
        ] {
            let m = manifest(identity, &["SOC2-Type2"]);
            for required in [vec![], vec!["soc2".into()]] {
                assert!(
                    matches!(
                        verify_trust(&urn("acme.com"), Some(&m), &required),
                        TrustOutcome::Rejected(_)
                    ),
                    "accepted {identity} with required attestations {required:?}"
                );
            }
        }
    }

    #[test]
    fn trust_rejects_missing_attestation() {
        let m = manifest("spiffe://acme.com/x", &["ISO27001"]);
        assert!(matches!(
            verify_trust(&urn("acme.com"), Some(&m), &["soc2".into()]),
            TrustOutcome::Rejected(_)
        ));
    }

    #[test]
    fn trust_rejects_missing_manifest_when_required() {
        assert!(matches!(
            verify_trust(&urn("acme.com"), None, &["soc2".into()]),
            TrustOutcome::Rejected(_)
        ));
    }

    #[test]
    fn trust_allows_missing_manifest_when_not_required() {
        assert_eq!(verify_trust(&urn("acme.com"), None, &[]), TrustOutcome::Ok);
    }

    #[test]
    fn trust_allows_subdomain_identity() {
        let m = manifest("spiffe://travel.acme.com/x", &[]);
        assert_eq!(
            verify_trust(&urn("acme.com"), Some(&m), &[]),
            TrustOutcome::Ok
        );
    }

    #[test]
    fn maps_mcp_artifact_single_and_map() {
        let single = serde_json::json!({"url": "https://mcp.x.com", "headers": {"A": "b"}});
        let (url, headers) = mcp_endpoint_from_artifact(&single).unwrap();
        assert_eq!(url, "https://mcp.x.com");
        assert_eq!(headers.get("A").map(String::as_str), Some("b"));

        let map = serde_json::json!({"mcpServers": {"docs": {"url": "https://mcp.y.com"}}});
        let (url, _) = mcp_endpoint_from_artifact(&map).unwrap();
        assert_eq!(url, "https://mcp.y.com");
    }

    #[test]
    fn maps_a2a_artifact() {
        let card = serde_json::json!({"name": "C", "url": "https://agent.x.com"});
        let (base, inline) = a2a_target_from_artifact(&card).unwrap();
        assert_eq!(base.as_deref(), Some("https://agent.x.com"));
        assert_eq!(inline, Some(card));
    }
}
