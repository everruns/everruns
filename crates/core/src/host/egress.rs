//! Reqwest-backed implementation of the neutral Everruns egress contract.

use super::egress_proxy::ProxyRoute;
use super::org_egress_allowlist::{
    ORG_EGRESS_ALLOWLIST_CACHE_TTL, OrgAllowlistCache, OrgAllowlistSource,
};
use crate::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService, EgressSigning,
    EgressStreamResponse, SystemAllowlist,
};
use async_trait::async_trait;
use everruns_contracts::runtime::org_egress_allowlist::OrgEgressAllowlist;
use everruns_contracts::runtime::{
    EgressAccess, EgressPolicyDenial, EgressPolicyGrant, SystemEgressPolicy,
};
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
    /// Optional domain reputation consulted before an open read.
    reputation: Option<Arc<super::egress_reputation::DomainReputation>>,
    /// Where org allowlist extensions come from. Consulted only for a request
    /// the system policy would refuse as not allowlisted.
    org_allowlist: OrgAllowlistSource,
    /// Where tenant requests leave from: directly, or through the tenant
    /// egress proxy (`EVERRUNS_EGRESS_PROXY_URL`).
    proxy: ProxyRoute,
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
            reputation: None,
            org_allowlist: OrgAllowlistSource::None,
            proxy: ProxyRoute::Direct,
            dns_resolver: None,
        }
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            system_policy: None,
            open_reads: Arc::new(OpenReadMeter::from_env()),
            reputation: None,
            org_allowlist: OrgAllowlistSource::None,
            proxy: ProxyRoute::Direct,
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
    /// Org allowlist extensions come from the process-wide resolver the host
    /// installs (`install_runtime_org_egress_allowlist`), when there is one.
    pub fn from_env() -> Self {
        let mut service = Self::new()
            .with_system_policy(SystemEgressPolicy::from_env())
            .with_reputation(super::egress_reputation::DomainReputation::from_env().map(Arc::new));
        service.org_allowlist = OrgAllowlistSource::Runtime;
        service.proxy = ProxyRoute::from_env();
        service
    }

    /// Send tenant requests through `proxy` (or directly with `None`).
    pub fn with_tenant_proxy(mut self, proxy: Option<super::TenantEgressProxy>) -> Self {
        self.proxy = proxy.map_or(ProxyRoute::Direct, ProxyRoute::Proxy);
        self
    }

    /// Attach (or clear) the domain reputation check for open reads.
    pub fn with_reputation(
        mut self,
        reputation: Option<Arc<super::egress_reputation::DomainReputation>>,
    ) -> Self {
        self.reputation = reputation;
        self
    }

    /// Resolve org allowlist extensions through `resolver` (cached for
    /// [`ORG_EGRESS_ALLOWLIST_CACHE_TTL`]) instead of the process-wide one.
    pub fn with_org_allowlist(mut self, resolver: Arc<dyn OrgEgressAllowlist>) -> Self {
        self.org_allowlist = OrgAllowlistSource::Explicit(Arc::new(OrgAllowlistCache::new(
            resolver,
            ORG_EGRESS_ALLOWLIST_CACHE_TTL,
        )));
        self
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

    /// Decide the system policy for one request. The org's grant is
    /// consulted only when the policy alone would not allow the request
    /// outright: an open grant lifts everything but the deny list, and an
    /// allowlist extension can admit a request refused as not allowlisted.
    async fn decide_policy(&self, request: &EgressRequest) -> PolicyDecision {
        let Some(policy) = &self.system_policy else {
            return PolicyDecision::NoPolicy;
        };
        let access = request_access(request);
        let initial = match policy.check(&request.url, access, None) {
            Ok(EgressPolicyGrant::Allowlisted) => return PolicyDecision::Allowlisted,
            // The deny list holds for every org, granted or not.
            Err(EgressPolicyDenial::Denylisted) => {
                return PolicyDecision::Denied(EgressPolicyDenial::Denylisted);
            }
            Ok(EgressPolicyGrant::OpenRead) => PolicyDecision::OpenRead,
            Err(denial) => PolicyDecision::Denied(denial),
        };
        let org = request
            .scope
            .as_ref()
            .and_then(|scope| scope.org_id.as_ref());
        let Some(grant) = (match (org, self.org_allowlist.resolve()) {
            (Some(org), Some(cache)) => cache.grant(org).await,
            _ => None,
        }) else {
            return initial;
        };
        if grant.open {
            return PolicyDecision::OpenOrg;
        }
        if initial == PolicyDecision::Denied(EgressPolicyDenial::NotAllowlisted)
            && let Some(extension) = &grant.extension
            && policy.check(&request.url, access, Some(extension))
                == Ok(EgressPolicyGrant::Allowlisted)
        {
            return PolicyDecision::AllowlistedOrg;
        }
        initial
    }

    fn validate_request(
        &self,
        request: &EgressRequest,
        decision: PolicyDecision,
    ) -> EgressResult<()> {
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
        // carry data out must match the curated allowlist (or the org's
        // granted extension). Host-owned services should not use
        // `EgressService`.
        match decision {
            PolicyDecision::NoPolicy
            | PolicyDecision::Allowlisted
            | PolicyDecision::AllowlistedOrg
            | PolicyDecision::OpenOrg => {}
            PolicyDecision::OpenRead => {
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
            PolicyDecision::Denied(_) => {
                return Err(EgressError::NetworkAccessDenied {
                    url: request.url.clone(),
                });
            }
        }
        Ok(())
    }

    async fn prepare_request(
        &self,
        mut request: EgressRequest,
        decision: PolicyDecision,
    ) -> EgressResult<EgressRequest> {
        self.validate_request(&request, decision)?;
        if request.signing == EgressSigning::Required {
            return Err(EgressError::SigningUnavailable);
        }
        // The reputation lookup and the DNS-pinning resolution are independent
        // network round trips, so a first read of an unknown host pays for one,
        // not both. The reputation verdict still decides first.
        let pinning = async {
            if !request.dns_pinning_required {
                return None;
            }
            Some(match &self.dns_resolver {
                Some(resolve) => {
                    let resolve = Arc::clone(resolve);
                    validate_url_with_resolver(&request.url, move |host, port| {
                        let resolve = Arc::clone(&resolve);
                        async move { resolve(host, port).await }
                    })
                    .await
                }
                None => validate_url_dns_pinned(&request.url).await,
            })
        };
        let (reputation, pinned) = tokio::join!(self.check_reputation(&request, decision), pinning);
        reputation?;
        if let Some(validated) = pinned {
            let (validated_url, resolved_addrs) =
                validated.map_err(|error| EgressError::NetworkAccessDenied {
                    url: format!("{} ({error})", request.url),
                })?;
            let pin_host = validated_url.host_str().unwrap_or("").to_string();
            request = request.pinned_addrs(pin_host, resolved_addrs);
        }
        Ok(request)
    }

    /// Open reads and open-org requests reach hosts nobody curated, so ask the
    /// reputation source about the host first. Allowlisted and unpoliced
    /// traffic skips it.
    async fn check_reputation(
        &self,
        request: &EgressRequest,
        decision: PolicyDecision,
    ) -> EgressResult<()> {
        let Some(reputation) = &self.reputation else {
            return Ok(());
        };
        if !matches!(decision, PolicyDecision::OpenRead | PolicyDecision::OpenOrg) {
            return Ok(());
        }
        let Some(host) = reqwest::Url::parse(&request.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
        else {
            return Ok(());
        };
        if reputation.is_flagged(&host).await {
            return Err(EgressError::NetworkAccessDenied {
                url: format!("{} (domain flagged by reputation check)", request.url),
            });
        }
        Ok(())
    }

    fn build_request(&self, request: EgressRequest) -> EgressResult<reqwest::RequestBuilder> {
        let proxy = self.proxy.for_kind(&request.kind)?.cloned();
        let EgressRequest {
            method,
            url,
            headers,
            body,
            timeout_ms,
            pinned_addrs,
            scope,
            ..
        } = request;

        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|error| EgressError::invalid(format!("invalid HTTP method: {error}")))?;

        // Tenant traffic through the proxy: a per-request client carrying the
        // request's signed scope token. The proxy connects to the target, so
        // it re-validates the resolved address instead of the pins.
        let mut builder = if let Some(proxy) = proxy {
            reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(DEFAULT_REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .proxy(proxy.reqwest_proxy(scope.as_ref())?)
                .build()
                .map_err(|e| EgressError::invalid(format!("proxied client build failed: {e}")))?
                .request(method, &url)
        // When pre-resolved addresses are provided, build a per-request client
        // pinned to those IPs.  This closes the TOCTOU window between
        // validate_url_dns_pinned and the actual TCP connect (TM-TOOL-018).
        } else if let Some((ref host, ref addrs)) = pinned_addrs {
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
        let decision = self.decide_policy(&request).await;
        let audit = EgressAudit::start(&request, decision);
        let result = self.send_unaudited(request, decision).await;
        audit.finish(
            result
                .as_ref()
                .map(|response| (response.status, Some(response.body.len()))),
        );
        result
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let decision = self.decide_policy(&request).await;
        let audit = EgressAudit::start(&request, decision);
        let result = self.send_stream_unaudited(request, decision).await;
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
    async fn send_unaudited(
        &self,
        request: EgressRequest,
        decision: PolicyDecision,
    ) -> EgressResult<EgressResponse> {
        let request = self.prepare_request(request, decision).await?;
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
        decision: PolicyDecision,
    ) -> EgressResult<EgressStreamResponse> {
        let request = self.prepare_request(request, decision).await?;
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

/// How the system policy treated one request, as enforced and audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyDecision {
    /// No system policy is active.
    NoPolicy,
    /// The curated allowlist matched.
    Allowlisted,
    /// Only the org's granted allowlist extension matched.
    AllowlistedOrg,
    /// A read of a host outside the allowlist (`curated-writes`), metered.
    OpenRead,
    /// The org holds an open grant: any host the deny list allows.
    OpenOrg,
    Denied(EgressPolicyDenial),
}

impl PolicyDecision {
    fn label(self) -> String {
        match self {
            Self::NoPolicy => "none".to_string(),
            Self::Allowlisted => "allowlisted".to_string(),
            Self::AllowlistedOrg => "allowlisted_org".to_string(),
            Self::OpenRead => "open_read".to_string(),
            Self::OpenOrg => "open_org".to_string(),
            Self::Denied(denial) => format!("denied:{}", denial.reason()),
        }
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
    /// `allowlisted_org` (passed only through the org's granted extension),
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
    fn start(request: &EgressRequest, decision: PolicyDecision) -> Self {
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
            policy: decision.label(),
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
#[path = "egress_tests.rs"]
mod tests;
