//! Reqwest-backed implementation of the neutral Everruns egress contract.

use crate::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService, EgressSigning,
    EgressStreamResponse, SystemAllowlist,
};
use async_trait::async_trait;
use everruns_contracts::runtime::{EgressAccess, EgressPolicyGrant, SystemEgressPolicy};
use everruns_contracts::url_validation::{validate_url_dns_pinned, validate_url_with_resolver};
use futures::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

type DnsResolveFuture = Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>>;
type DnsResolver = Arc<dyn Fn(String, u16) -> DnsResolveFuture + Send + Sync>;

#[derive(Clone)]
pub struct DirectEgressService {
    client: reqwest::Client,
    /// Optional host-wide egress policy (allowlist, deny list, mode) applied
    /// to every outbound request, independently of the per-request
    /// `network_access`. `None` means no global enforcement. See
    /// `crate::system_allowlist`.
    system_policy: Option<Arc<SystemEgressPolicy>>,
    /// Per-org meter for open reads (`curated-writes`).
    open_reads: Arc<OpenReadMeter>,
    /// Optional DNS override for `dns_pinning_required` requests. Production
    /// leaves this unset and uses the system resolver; tests install a
    /// controlled resolver to prove private answers are denied before connect.
    dns_resolver: Option<DnsResolver>,
}

impl std::fmt::Debug for DirectEgressService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectEgressService")
            .finish_non_exhaustive()
    }
}

impl Default for DirectEgressService {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectEgressService {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(DEFAULT_REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build direct egress HTTP client"),
            system_policy: None,
            open_reads: Arc::new(OpenReadMeter::from_env()),
            dns_resolver: None,
        }
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            system_policy: None,
            open_reads: Arc::new(OpenReadMeter::from_env()),
            dns_resolver: None,
        }
    }

    /// Install a controlled DNS resolver for `dns_pinning_required` requests.
    ///
    /// Intended for tests that must prove a private, link-local, or loopback
    /// answer is denied before any TCP connect (EVE-1154). Production call
    /// sites leave the default system resolver in place.
    pub fn with_dns_resolver<F, Fut>(mut self, resolve: F) -> Self
    where
        F: Fn(String, u16) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::io::Result<Vec<SocketAddr>>> + Send + 'static,
    {
        self.dns_resolver = Some(Arc::new(move |host, port| Box::pin(resolve(host, port))));
        self
    }

    /// Construct the default direct transport for tenant/agent runtime egress.
    ///
    /// This honors `EVERRUNS_EGRESS_POLICY` and is the right default
    /// for capabilities, MCP, integrations, and runtime HTTP surfaces.
    pub fn for_runtime_traffic_from_env() -> Self {
        Self::from_env()
    }

    /// Construct with the global system allowlist resolved from the environment.
    ///
    /// Enforcement is active only when `EVERRUNS_EGRESS_POLICY` (or the legacy
    /// `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED`) selects a curated mode; otherwise
    /// this behaves exactly like [`DirectEgressService::new`].
    /// Prefer [`DirectEgressService::for_runtime_traffic_from_env`] for new
    /// runtime/agent call sites. Host-owned services should use direct provider
    /// clients instead of `EgressService`.
    pub fn from_env() -> Self {
        Self::new().with_system_policy(SystemEgressPolicy::from_env())
    }

    /// Attach (or clear) the host-wide system egress policy.
    pub fn with_system_policy(mut self, system_policy: Option<Arc<SystemEgressPolicy>>) -> Self {
        self.system_policy = system_policy;
        self
    }

    /// Attach (or clear) an allowlist-only policy: every request must match.
    pub fn with_system_allowlist(self, system_allowlist: Option<Arc<SystemAllowlist>>) -> Self {
        self.with_system_policy(
            system_allowlist.map(|list| Arc::new(SystemEgressPolicy::allowlist_only(list))),
        )
    }

    /// Replace the open-read meter's per-org limit (requests per minute).
    pub fn with_open_read_limit(mut self, per_minute: u32) -> Self {
        self.open_reads = Arc::new(OpenReadMeter::new(per_minute));
        self
    }

    fn validate_request(&self, request: &EgressRequest) -> EgressResult<()> {
        if request.method.trim().is_empty() {
            return Err(EgressError::invalid("method is required"));
        }
        let parsed = reqwest::Url::parse(&request.url)
            .map_err(|error| EgressError::invalid(format!("invalid URL: {error}")))?;
        match parsed.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(EgressError::invalid(format!(
                    "URL must use http or https, got '{scheme}'"
                )));
            }
        }
        if let Some(acl) = &request.network_access
            && !acl.is_url_allowed(&request.url)
        {
            return Err(EgressError::NetworkAccessDenied {
                url: request.url.clone(),
            });
        }
        // Host-wide policy: when enabled, every request routed through this
        // runtime egress boundary passes the deny list, and requests that can
        // carry data out must match the curated allowlist. Host-owned
        // services should not use `EgressService`.
        if let Some(policy) = &self.system_policy {
            match policy.check(&request.url, request_access(request), None) {
                Ok(EgressPolicyGrant::Allowlisted) => {}
                Ok(EgressPolicyGrant::OpenRead) => {
                    let org = request
                        .scope
                        .as_ref()
                        .and_then(|scope| scope.org_id.as_ref())
                        .map(ToString::to_string);
                    if !self.open_reads.admit(org.as_deref()) {
                        return Err(EgressError::NetworkAccessDenied {
                            url: format!(
                                "{} (open-read rate limit reached for this organization; \
                                 retry in a minute)",
                                request.url
                            ),
                        });
                    }
                }
                Err(_) => {
                    return Err(EgressError::NetworkAccessDenied {
                        url: request.url.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    async fn prepare_request(&self, mut request: EgressRequest) -> EgressResult<EgressRequest> {
        self.validate_request(&request)?;
        if request.signing == EgressSigning::Required {
            return Err(EgressError::SigningUnavailable);
        }
        if request.dns_pinning_required {
            let validated = match &self.dns_resolver {
                Some(resolve) => {
                    let resolve = Arc::clone(resolve);
                    validate_url_with_resolver(&request.url, move |host, port| {
                        let resolve = Arc::clone(&resolve);
                        async move { resolve(host, port).await }
                    })
                    .await
                }
                None => validate_url_dns_pinned(&request.url).await,
            };
            let (validated_url, resolved_addrs) =
                validated.map_err(|error| EgressError::NetworkAccessDenied {
                    url: format!("{} ({error})", request.url),
                })?;
            let pin_host = validated_url.host_str().unwrap_or("").to_string();
            request = request.pinned_addrs(pin_host, resolved_addrs);
        }
        Ok(request)
    }

    fn build_request(&self, request: EgressRequest) -> EgressResult<reqwest::RequestBuilder> {
        let EgressRequest {
            method,
            url,
            headers,
            body,
            timeout_ms,
            pinned_addrs,
            ..
        } = request;

        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|error| EgressError::invalid(format!("invalid HTTP method: {error}")))?;

        // When pre-resolved addresses are provided, build a per-request client
        // pinned to those IPs.  This closes the TOCTOU window between
        // validate_url_dns_pinned and the actual TCP connect (TM-TOOL-018).
        let mut builder = if let Some((ref host, ref addrs)) = pinned_addrs {
            reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, addrs)
                .build()
                .map_err(|e| EgressError::invalid(format!("pinned client build failed: {e}")))?
                .request(method, &url)
        } else {
            self.client.request(method, &url)
        };

        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        if let Some(timeout_ms) = timeout_ms {
            builder = builder.timeout(Duration::from_millis(timeout_ms));
        }
        if !body.is_empty() {
            builder = builder.body(body);
        }
        Ok(builder)
    }
}

#[async_trait]
impl EgressService for DirectEgressService {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let audit = EgressAudit::start(&request, self.system_policy.as_deref());
        let result = self.send_unaudited(request).await;
        audit.finish(
            result
                .as_ref()
                .map(|response| (response.status, Some(response.body.len()))),
        );
        result
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let audit = EgressAudit::start(&request, self.system_policy.as_deref());
        let result = self.send_stream_unaudited(request).await;
        // Streamed bodies are not buffered here, so the response size is
        // unknown; the status and request side are still recorded.
        audit.finish(result.as_ref().map(|response| (response.status, None)));
        result
    }

    fn name(&self) -> &'static str {
        "DirectEgressService"
    }
}

impl DirectEgressService {
    async fn send_unaudited(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let request = self.prepare_request(request).await?;
        let response = self
            .build_request(request)?
            .send()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?
            .to_vec();

        Ok(EgressResponse {
            status,
            headers,
            body,
        })
    }

    async fn send_stream_unaudited(
        &self,
        request: EgressRequest,
    ) -> EgressResult<EgressStreamResponse> {
        let request = self.prepare_request(request).await?;
        let response = self
            .build_request(request)?
            .send()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        let body = response.bytes_stream().map(|chunk| {
            chunk
                .map(|bytes| bytes.to_vec())
                .map_err(|error| EgressError::Transport(error.to_string()))
        });

        Ok(EgressStreamResponse {
            status,
            headers,
            body: Box::pin(body),
        })
    }
}

/// Whether a request can carry data out. MCP and integration traffic is
/// treated as a write whatever its method: those clients exchange tenant data
/// with the endpoint by design.
fn request_access(request: &EgressRequest) -> EgressAccess {
    match request.kind {
        crate::EgressRequestKind::Mcp | crate::EgressRequestKind::Integration => {
            EgressAccess::Write
        }
        _ => EgressAccess::classify(&request.method, !request.body.is_empty()),
    }
}

/// Environment variable overriding the per-org open-read limit (requests per
/// minute, per process). `0` disables open reads entirely.
pub const OPEN_READS_PER_MINUTE_ENV: &str = "EVERRUNS_EGRESS_OPEN_READS_PER_MINUTE";
const DEFAULT_OPEN_READS_PER_MINUTE: u32 = 120;

/// Fixed-window per-org counter for open reads. Per process, so a tenant
/// spread across workers gets a multiple of the limit; it bounds bulk abuse,
/// not precise quotas. Requests with no org share one bucket.
struct OpenReadMeter {
    per_minute: u32,
    windows: Mutex<HashMap<String, (u64, u32)>>,
}

impl OpenReadMeter {
    fn new(per_minute: u32) -> Self {
        Self {
            per_minute,
            windows: Mutex::new(HashMap::new()),
        }
    }

    fn from_env() -> Self {
        let per_minute = std::env::var(OPEN_READS_PER_MINUTE_ENV)
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(DEFAULT_OPEN_READS_PER_MINUTE);
        Self::new(per_minute)
    }

    fn admit(&self, org: Option<&str>) -> bool {
        let minute = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() / 60)
            .unwrap_or(0);
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Drop stale windows so the map tracks only orgs active this minute.
        windows.retain(|_, (window, _)| *window == minute);
        let (_, count) = windows
            .entry(org.unwrap_or("").to_string())
            .or_insert((minute, 0));
        if *count >= self.per_minute {
            return false;
        }
        *count += 1;
        true
    }
}

/// Target of the outbound audit log. One `info` event per request that
/// reaches this boundary, allowed or denied, so abuse can be traced to an org
/// and session (TM-AGENT-018 residual). The query string is never logged, only
/// its length: queries routinely carry tokens.
pub const EGRESS_AUDIT_TARGET: &str = "everruns::egress::audit";

struct EgressAudit {
    started: std::time::Instant,
    kind: String,
    method: String,
    host: String,
    path: String,
    query_len: usize,
    request_bytes: usize,
    org_id: Option<String>,
    session_id: Option<String>,
    /// How the system policy classified the request: `allowlisted`,
    /// `open_read`, `denied:<reason>`, or `none` when no policy is active.
    policy: String,
}

/// Decision label, status, response size, and error text for one outcome. A
/// policy denial logs no error text: it repeats the URL, query included.
type AuditOutcome = (&'static str, Option<u16>, Option<usize>, Option<String>);

fn audit_outcome(outcome: Result<(u16, Option<usize>), &EgressError>) -> AuditOutcome {
    match outcome {
        Ok((status, bytes)) => ("allowed", Some(status), bytes, None),
        Err(EgressError::NetworkAccessDenied { .. }) => ("denied", None, None, None),
        Err(error) => ("failed", None, None, Some(error.to_string())),
    }
}

impl EgressAudit {
    fn start(request: &EgressRequest, policy: Option<&SystemEgressPolicy>) -> Self {
        let parsed = reqwest::Url::parse(&request.url).ok();
        let scope = request.scope.as_ref();
        Self {
            started: std::time::Instant::now(),
            kind: match &request.kind {
                crate::EgressRequestKind::Provider => "provider".to_string(),
                crate::EgressRequestKind::Capability => "capability".to_string(),
                crate::EgressRequestKind::Integration => "integration".to_string(),
                crate::EgressRequestKind::SystemEmail => "system_email".to_string(),
                crate::EgressRequestKind::UtilityLlm => "utility_llm".to_string(),
                crate::EgressRequestKind::Mcp => "mcp".to_string(),
                crate::EgressRequestKind::Other(label) => format!("other:{label}"),
            },
            method: request.method.to_ascii_uppercase(),
            host: parsed
                .as_ref()
                .and_then(|url| url.host_str().map(str::to_string))
                .unwrap_or_default(),
            path: parsed
                .as_ref()
                .map(|url| url.path().to_string())
                .unwrap_or_default(),
            query_len: parsed
                .as_ref()
                .and_then(|url| url.query().map(str::len))
                .unwrap_or(0),
            request_bytes: request.body.len(),
            org_id: scope.and_then(|scope| scope.org_id.as_ref().map(ToString::to_string)),
            session_id: scope.and_then(|scope| scope.session_id.map(|id| id.to_string())),
            policy: match policy.map(|p| p.check(&request.url, request_access(request), None)) {
                None => "none".to_string(),
                Some(Ok(EgressPolicyGrant::Allowlisted)) => "allowlisted".to_string(),
                Some(Ok(EgressPolicyGrant::OpenRead)) => "open_read".to_string(),
                Some(Err(denial)) => format!("denied:{}", denial.reason()),
            },
        }
    }

    fn finish(self, outcome: Result<(u16, Option<usize>), &EgressError>) {
        let duration_ms = self.started.elapsed().as_millis() as u64;
        let (decision, status, response_bytes, error) = audit_outcome(outcome);
        tracing::info!(
            target: EGRESS_AUDIT_TARGET,
            decision,
            kind = %self.kind,
            policy = %self.policy,
            method = %self.method,
            host = %self.host,
            path = %self.path,
            query_len = self.query_len,
            request_bytes = self.request_bytes,
            status,
            response_bytes,
            duration_ms,
            org_id = self.org_id.as_deref(),
            session_id = self.session_id.as_deref(),
            error = error.as_deref(),
            "egress"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EgressRequestKind;
    use crate::network_access::NetworkAccessList;
    use futures::StreamExt;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn direct_service_sends_json_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/test"))
            .and(header("Authorization", "Bearer test"))
            .and(body_json(json!({"ok": true})))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({
                        "id": "response_123"
                    }))
                    .insert_header("X-Request-Id", "request-42"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let response = DirectEgressService::new()
            .send(
                EgressRequest::new(
                    "POST",
                    format!("{}/v1/test", server.uri()),
                    EgressRequestKind::Capability,
                )
                .header("Authorization", "Bearer test")
                .header("Content-Type", "application/json")
                .body(serde_json::to_vec(&json!({"ok": true})).unwrap()),
            )
            .await
            .unwrap();

        assert_eq!(response.status, 201);
        assert_eq!(
            response.headers.get("x-request-id").map(String::as_str),
            Some("request-42")
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["id"],
            "response_123"
        );
    }

    #[tokio::test]
    async fn direct_service_enforces_network_access() {
        let error = DirectEgressService::new()
            .send(
                EgressRequest::new(
                    "GET",
                    "https://blocked.example.com/path",
                    EgressRequestKind::Capability,
                )
                .network_access(Some(NetworkAccessList::allow_only(["allowed.example.com"]))),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }

    #[tokio::test]
    async fn system_allowlist_blocks_unlisted_hosts() {
        use crate::SystemAllowlist;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/blocked"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let allowlist = SystemAllowlist::from_toml(
            r#"
            [groups.test]
            allowed = ["allowed.example.com"]
            "#,
        )
        .unwrap();

        let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
        let error = service
            .send(EgressRequest::new(
                "GET",
                format!("{}/blocked", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }

    #[tokio::test]
    async fn system_allowlist_cannot_be_overridden_by_request_acl() {
        use crate::SystemAllowlist;
        // The system allowlist is a hard ceiling: even a request whose own
        // network_access explicitly permits the host must still be denied when
        // the system allowlist does not list it. Sessions can narrow, never widen.
        let allowlist = SystemAllowlist::from_toml(
            r#"
            [groups.test]
            allowed = ["allowed.example.com"]
            "#,
        )
        .unwrap();

        let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
        let error = service
            .send(
                EgressRequest::new(
                    "GET",
                    "https://blocked.example.com/path",
                    EgressRequestKind::Capability,
                )
                // A maximally-permissive per-request ACL that allows the host.
                .network_access(Some(NetworkAccessList::allow_only(["blocked.example.com"]))),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }

    #[tokio::test]
    async fn system_allowlist_permits_listed_hosts() {
        use crate::SystemAllowlist;
        let server = MockServer::start().await;
        let host = reqwest::Url::parse(&server.uri())
            .unwrap()
            .host_str()
            .unwrap()
            .to_string();
        Mock::given(method("GET"))
            .and(path("/ok"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let allowlist =
            SystemAllowlist::from_toml(&format!("[groups.test]\nallowed = [\"{host}\"]\n"))
                .unwrap();
        let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
        let response = service
            .send(EgressRequest::new(
                "GET",
                format!("{}/ok", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn dns_pinning_blocks_loopback_after_request_policy_passes() {
        let error = DirectEgressService::new()
            .send(
                EgressRequest::new(
                    "GET",
                    "http://127.0.0.1/latest/meta-data",
                    EgressRequestKind::Capability,
                )
                .network_access(Some(NetworkAccessList::allow_only(["127.0.0.1"])))
                .require_dns_pinning(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
        assert!(error.to_string().contains("private/internal address"));
    }

    /// EVE-1154: bashkit can hand the egress boundary an unpinned hostname
    /// after its DNS precheck fails open. With `require_dns_pinning`, a
    /// subsequent private/link-local/loopback answer must be refused before
    /// any TCP connect — proved here with a controlled resolver aimed at a
    /// live mock server that must see zero requests.
    #[tokio::test]
    async fn require_dns_pinning_denies_private_answers_before_connect() {
        use std::net::{IpAddr, SocketAddr};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secret"))
            .respond_with(ResponseTemplate::new(200).set_body_string("leaked"))
            .expect(0)
            .mount(&server)
            .await;

        let listen = reqwest::Url::parse(&server.uri()).unwrap();
        let port = listen.port().unwrap();
        for blocked in ["127.0.0.1", "169.254.169.254", "10.0.0.1"] {
            let blocked_ip: IpAddr = blocked.parse().unwrap();
            let service =
                DirectEgressService::new().with_dns_resolver(move |_host, resolved_port| {
                    let addr = SocketAddr::new(blocked_ip, resolved_port);
                    async move { Ok(vec![addr]) }
                });
            let error = service
                .send(
                    EgressRequest::new(
                        "GET",
                        format!("http://rebind.example:{port}/secret"),
                        EgressRequestKind::Capability,
                    )
                    .network_access(Some(NetworkAccessList::allow_only(["rebind.example"])))
                    // Same flag BashkitEgressTransport sets when pins are empty.
                    .require_dns_pinning(),
                )
                .await
                .unwrap_err();
            assert!(
                matches!(error, EgressError::NetworkAccessDenied { .. }),
                "blocked answer {blocked} must deny: {error}"
            );
            assert!(
                error.to_string().contains(blocked)
                    || error.to_string().contains("blocked address")
                    || error.to_string().contains("private"),
                "denial must mention the blocked answer {blocked}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn dns_pinning_does_not_run_before_system_allowlist_denial() {
        use crate::SystemAllowlist;
        let allowlist = SystemAllowlist::from_toml(
            r#"
            [groups.test]
            allowed = ["allowed.example.com"]
            "#,
        )
        .unwrap();

        let blocked_url = "https://blocked.invalid/path";
        let error = DirectEgressService::new()
            .with_system_allowlist(Some(Arc::new(allowlist)))
            .send(
                EgressRequest::new("GET", blocked_url, EgressRequestKind::Capability)
                    .network_access(Some(NetworkAccessList::allow_only(["blocked.invalid"])))
                    .require_dns_pinning(),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(error, EgressError::NetworkAccessDenied { ref url } if url == blocked_url)
        );
    }

    #[tokio::test]
    async fn required_signing_fails_when_no_signer_is_configured() {
        let error = DirectEgressService::new()
            .send(
                EgressRequest::new("GET", "https://example.com", EgressRequestKind::Capability)
                    .signing(EgressSigning::Required),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::SigningUnavailable));
    }

    #[tokio::test]
    async fn direct_service_does_not_follow_redirects() {
        let redirect_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secret"))
            .respond_with(ResponseTemplate::new(200).set_body_string("secret"))
            .expect(0)
            .mount(&redirect_server)
            .await;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/secret", redirect_server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let response = DirectEgressService::new()
            .send(EgressRequest::new(
                "GET",
                format!("{}/start", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap();

        assert_eq!(response.status, 302);
    }

    #[tokio::test]
    async fn direct_service_streams_response_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw("data: one\n\ndata: two\n\n", "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mut response = DirectEgressService::new()
            .send_stream(EgressRequest::new(
                "GET",
                format!("{}/stream", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("text/event-stream")
        );
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            body.extend(chunk.unwrap());
        }
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "data: one\n\ndata: two\n\n"
        );
    }
    #[tokio::test]
    async fn merged_network_policy_denies_escaped_urls_before_transport() {
        use crate::network_access::merge_network_access;
        let service = DirectEgressService::new();
        for (parent, child, target) in [
            (
                "*.example.com",
                "https://outside.invalid/path.example.com",
                "https://outside.invalid/path.example.com",
            ),
            (
                "https://api.example.com/v1/",
                "https://api.example.com/v1/../admin",
                "https://api.example.com/admin",
            ),
            (
                "https://",
                "https://outside.invalid/data",
                "https://outside.invalid/data",
            ),
        ] {
            let parent = NetworkAccessList::allow_only([parent]);
            let child = NetworkAccessList::allow_only([child]);
            let policy = merge_network_access(Some(&parent), Some(&child));
            for streaming in [false, true] {
                let request = EgressRequest::new("GET", target, EgressRequestKind::Capability)
                    .network_access(policy.clone());
                let error = if streaming {
                    match service.send_stream(request).await {
                        Err(error) => error,
                        Ok(_) => panic!("streaming request escaped policy: {target}"),
                    }
                } else {
                    service.send(request).await.unwrap_err()
                };
                assert!(matches!(error, EgressError::NetworkAccessDenied { url } if url == target));
            }
        }
    }

    #[test]
    fn audit_record_carries_scope_and_hides_query() {
        let session_id = everruns_contracts::typed_id::SessionId::new();
        let request = EgressRequest::new(
            "post",
            "https://hooks.example.com/hook?token=sekret",
            EgressRequestKind::Other("plugin".into()),
        )
        .scope(crate::EgressScope {
            org_id: Some(everruns_contracts::typed_id::DEFAULT_ORG_ID),
            session_id: Some(session_id),
        })
        .body(b"12345".to_vec());
        let audit = EgressAudit::start(&request, None);
        assert_eq!(audit.kind, "other:plugin");
        assert_eq!(audit.method, "POST");
        assert_eq!(audit.host, "hooks.example.com");
        assert_eq!(audit.path, "/hook");
        assert_eq!(audit.query_len, "token=sekret".len());
        assert_eq!(audit.request_bytes, 5);
        assert_eq!(
            audit.org_id,
            Some(everruns_contracts::typed_id::DEFAULT_ORG_ID.to_string())
        );
        assert_eq!(audit.session_id, Some(session_id.to_string()));
        let debug = format!(
            "{} {} {} {:?} {:?}",
            audit.kind, audit.host, audit.path, audit.org_id, audit.session_id
        );
        assert!(
            !debug.contains("sekret"),
            "query values must not be recorded"
        );
    }

    #[tokio::test]
    async fn scoped_service_overwrites_caller_scope() {
        #[derive(Default)]
        struct Recorder(std::sync::Mutex<Vec<Option<crate::EgressScope>>>);
        #[async_trait]
        impl EgressService for Recorder {
            async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
                self.0.lock().unwrap().push(request.scope);
                Err(EgressError::Transport("recorded".into()))
            }
            async fn send_stream(
                &self,
                request: EgressRequest,
            ) -> EgressResult<EgressStreamResponse> {
                self.0.lock().unwrap().push(request.scope);
                Err(EgressError::Transport("recorded".into()))
            }
        }
        let recorder = Arc::new(Recorder::default());
        let host_scope = crate::EgressScope {
            org_id: Some(everruns_contracts::typed_id::DEFAULT_ORG_ID),
            session_id: Some(everruns_contracts::typed_id::SessionId::new()),
        };
        let scoped = crate::ScopedEgressService::new(recorder.clone(), host_scope.clone());
        let forged = EgressRequest::new("GET", "https://a.test/", EgressRequestKind::Capability)
            .scope(crate::EgressScope::default());
        let _ = scoped.send(forged.clone()).await;
        let _ = scoped.send_stream(forged).await;
        assert_eq!(
            *recorder.0.lock().unwrap(),
            vec![Some(host_scope.clone()), Some(host_scope)]
        );
    }

    #[test]
    fn audit_outcome_labels() {
        assert_eq!(
            audit_outcome(Ok((200, Some(3)))),
            ("allowed", Some(200), Some(3), None)
        );
        assert_eq!(
            audit_outcome(Err(&EgressError::NetworkAccessDenied {
                url: "https://x.test/?k=v".into()
            })),
            ("denied", None, None, None)
        );
        assert_eq!(
            audit_outcome(Err(&EgressError::Transport("reset".into()))),
            (
                "failed",
                None,
                None,
                Some("Outbound transport error: reset".into())
            )
        );
    }

    fn curated_writes_service(open_reads_per_minute: u32) -> DirectEgressService {
        use everruns_contracts::runtime::EgressPolicyMode;
        let allowlist = crate::SystemAllowlist::from_toml(
            "[groups.test]\nallowed = [\"allowed.example.com\"]\n",
        )
        .unwrap();
        DirectEgressService::new()
            .with_system_policy(Some(Arc::new(SystemEgressPolicy::new(
                EgressPolicyMode::CuratedWrites,
                Arc::new(allowlist),
                vec!["*.webhook.site".to_string()],
            ))))
            .with_open_read_limit(open_reads_per_minute)
    }

    fn scoped_request(method: &str, url: &str, org: &str) -> EgressRequest {
        EgressRequest::new(method, url, EgressRequestKind::Capability).scope(crate::EgressScope {
            org_id: Some(org.parse().unwrap()),
            session_id: None,
        })
    }

    #[tokio::test]
    async fn curated_writes_lets_reads_through_and_gates_writes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let service = curated_writes_service(10);
        // Wiremock listens on an IP literal, which open reads refuse; that
        // refusal proves the read path ran (a write would say not allowlisted).
        let read = service
            .send(EgressRequest::new(
                "GET",
                format!("{}/page", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap_err();
        assert!(matches!(read, EgressError::NetworkAccessDenied { .. }));

        let policy = service.system_policy.clone().unwrap();
        let read = EgressRequest::new(
            "GET",
            "https://blog.example.net/p",
            EgressRequestKind::Capability,
        );
        assert_eq!(
            policy.check(&read.url, request_access(&read), None),
            Ok(EgressPolicyGrant::OpenRead)
        );
        for request in [
            EgressRequest::new(
                "POST",
                "https://blog.example.net/p",
                EgressRequestKind::Capability,
            ),
            EgressRequest::new(
                "GET",
                "https://blog.example.net/p",
                EgressRequestKind::Capability,
            )
            .body(b"x".to_vec()),
            EgressRequest::new("GET", "https://blog.example.net/p", EgressRequestKind::Mcp),
            EgressRequest::new(
                "GET",
                "https://blog.example.net/p",
                EgressRequestKind::Integration,
            ),
        ] {
            let error = service.send(request).await.unwrap_err();
            assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
        }
        let denied = service
            .send(EgressRequest::new(
                "GET",
                "https://webhook.site/abc",
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap_err();
        assert!(matches!(denied, EgressError::NetworkAccessDenied { .. }));
    }

    #[test]
    fn open_reads_are_metered_per_org() {
        let service = curated_writes_service(2);
        for org in [
            "org_00000000000000000000000000000001",
            "org_00000000000000000000000000000002",
        ] {
            for _ in 0..2 {
                service
                    .validate_request(&scoped_request("GET", "https://blog.example.net/", org))
                    .unwrap();
            }
            let error = service
                .validate_request(&scoped_request("GET", "https://blog.example.net/", org))
                .unwrap_err();
            assert!(
                matches!(&error, EgressError::NetworkAccessDenied { url } if url.contains("rate limit")),
                "{error}"
            );
            // Allowlisted traffic is never metered.
            service
                .validate_request(&scoped_request("GET", "https://allowed.example.com/", org))
                .unwrap();
        }
    }

    #[test]
    fn audit_record_names_the_policy_outcome() {
        let service = curated_writes_service(10);
        let policy = service.system_policy.as_deref();
        for (method, url, expected) in [
            ("GET", "https://blog.example.net/", "open_read"),
            ("POST", "https://allowed.example.com/", "allowlisted"),
            (
                "POST",
                "https://blog.example.net/",
                "denied:not_allowlisted",
            ),
            ("GET", "https://webhook.site/x", "denied:denylisted"),
        ] {
            let request = EgressRequest::new(method, url, EgressRequestKind::Capability);
            assert_eq!(
                EgressAudit::start(&request, policy).policy,
                expected,
                "{method} {url}"
            );
        }
        let request = EgressRequest::new("GET", "https://x.test/", EgressRequestKind::Capability);
        assert_eq!(EgressAudit::start(&request, None).policy, "none");
    }
}
