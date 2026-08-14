// TM-AUTH-001: Per-IP rate limiting for authentication endpoints.
// Decision: Dual backend — in-memory (governor) for single-instance/dev, Valkey for distributed.
// Decision: When VALKEY_URL is set, use Valkey sliding-window counter (Lua script, atomic).
//   When not set, fall back to governor in-memory (per-instance, same as before).
// Decision: Different limits for login (strict), register (strict), refresh (relaxed).
// Decision: Keyed by client IP from socket peer; forwarded headers are trusted
//   only when the peer is a trusted proxy (loopback/private ranges).
// Decision: On Valkey errors, fail closed (deny request) and log — auth hardening over availability.

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
};
use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DashMapStateStore};
use std::net::SocketAddr;
use std::{net::IpAddr, num::NonZeroU32, sync::Arc};

use crate::valkey::{RateLimitCheckError, ValkeyClient};
use uuid::Uuid;

type KeyedLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;
// String key covers both i64 org_ids and Uuid user_ids in OrgRateLimiter.
type StringKeyedLimiter = RateLimiter<String, DashMapStateStore<String>, DefaultClock>;

/// Rate limiter for auth endpoints, shared via Arc.
///
/// Supports two backends:
/// - **In-memory** (governor): per-instance, used when `VALKEY_URL` is not set.
/// - **Valkey**: distributed sliding-window counter, used when `VALKEY_URL` is set.
#[derive(Clone)]
pub struct AuthRateLimiter {
    backend: RateLimitBackend,
}

#[derive(Clone)]
enum RateLimitBackend {
    /// Per-instance rate limiting (governor crate, DashMap state)
    InMemory {
        login: Arc<KeyedLimiter>,
        register: Arc<KeyedLimiter>,
        refresh: Arc<KeyedLimiter>,
        account_login: Arc<StringKeyedLimiter>,
        email_send_minute: Arc<StringKeyedLimiter>,
        email_send_daily: Arc<StringKeyedLimiter>,
    },
    /// Distributed rate limiting (Valkey sliding-window counter)
    Valkey(ValkeyClient),
}

impl Default for AuthRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate limits per endpoint (requests per minute)
const LOGIN_LIMIT: u32 = 10;
const REGISTER_LIMIT: u32 = 5;
const REFRESH_LIMIT: u32 = 30;
const WINDOW_SECS: u64 = 60;

/// Per-account login attempts across ALL source IPs (TM-AUTH-001): the per-IP
/// limiter alone leaves one account open to distributed credential stuffing.
/// 20 attempts / 15 min is far above any human retry pattern.
const ACCOUNT_LOGIN_LIMIT: u32 = 20;
const ACCOUNT_LOGIN_WINDOW_SECS: u64 = 900;

/// Per-address outbound account email (verification / password reset):
/// 1 per minute with a small daily budget — email-bombing protection on top
/// of the per-IP register limiter. Callers stay enumeration-safe by returning
/// the generic success response without sending when over budget.
const EMAIL_SEND_MINUTE_LIMIT: u32 = 1;
const EMAIL_SEND_DAILY_LIMIT: u32 = 8;
const DAY_SECS: u64 = 86_400;

fn string_keyed_limiter(limit: u32, window_secs: u64) -> Arc<StringKeyedLimiter> {
    // Token bucket shaped to "limit per window": burst of `limit`, refilling
    // one permit per window/limit seconds.
    let period = std::time::Duration::from_secs((window_secs / u64::from(limit.max(1))).max(1));
    let quota = Quota::with_period(period)
        .expect("non-zero period")
        .allow_burst(NonZeroU32::new(limit.max(1)).unwrap());
    Arc::new(RateLimiter::keyed(quota))
}

impl AuthRateLimiter {
    /// Create an in-memory rate limiter (per-instance, no external deps).
    pub fn new() -> Self {
        Self {
            backend: RateLimitBackend::InMemory {
                login: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(LOGIN_LIMIT).unwrap(),
                ))),
                register: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(REGISTER_LIMIT).unwrap(),
                ))),
                refresh: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(REFRESH_LIMIT).unwrap(),
                ))),
                account_login: string_keyed_limiter(ACCOUNT_LOGIN_LIMIT, ACCOUNT_LOGIN_WINDOW_SECS),
                email_send_minute: string_keyed_limiter(EMAIL_SEND_MINUTE_LIMIT, WINDOW_SECS),
                email_send_daily: string_keyed_limiter(EMAIL_SEND_DAILY_LIMIT, DAY_SECS),
            },
        }
    }

    /// Create a distributed rate limiter backed by Valkey.
    pub fn with_valkey(client: ValkeyClient) -> Self {
        Self {
            backend: RateLimitBackend::Valkey(client),
        }
    }

    /// Check login rate limit. Returns Err(429) if exceeded.
    pub async fn check_login(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check("login", LOGIN_LIMIT, ip).await
    }

    /// Check register rate limit. Returns Err(429) if exceeded.
    pub async fn check_register(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check("register", REGISTER_LIMIT, ip).await
    }

    /// Check refresh rate limit. Returns Err(429) if exceeded.
    pub async fn check_refresh(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        self.check("refresh", REFRESH_LIMIT, ip).await
    }

    /// Per-account login throttle across all IPs. `account_key` should be the
    /// submitted email, trimmed and lowercased (works for unknown emails too —
    /// no DB read required). Fail-closed on backend errors like the other
    /// auth limiters.
    pub async fn check_account_login(&self, account_key: &str) -> Result<(), RateLimitError> {
        match &self.backend {
            RateLimitBackend::InMemory { account_login, .. } => {
                match account_login.check_key(&account_key.to_string()) {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        tracing::warn!("Per-account login rate limit exceeded (in-memory)");
                        Err(RateLimitError)
                    }
                }
            }
            RateLimitBackend::Valkey(client) => {
                let key = format!("rl:auth:account_login:{account_key}");
                match client
                    .check_rate_limit(&key, ACCOUNT_LOGIN_LIMIT, ACCOUNT_LOGIN_WINDOW_SECS)
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(RateLimitCheckError::Exceeded) => {
                        tracing::warn!("Per-account login rate limit exceeded (valkey)");
                        Err(RateLimitError)
                    }
                    Err(RateLimitCheckError::BackendError) => {
                        tracing::warn!(
                            "Per-account login limit Valkey error — denying (fail-closed)"
                        );
                        Err(RateLimitError)
                    }
                }
            }
        }
    }

    /// Per-address budget for verification / password-reset emails
    /// (1/minute + a small daily cap). `email_key` should be trimmed and
    /// lowercased. Callers return the normal enumeration-safe success
    /// response without sending when this is exceeded.
    pub async fn check_account_email_send(&self, email_key: &str) -> Result<(), RateLimitError> {
        match &self.backend {
            RateLimitBackend::InMemory {
                email_send_minute,
                email_send_daily,
                ..
            } => {
                let key = email_key.to_string();
                if email_send_minute.check_key(&key).is_err()
                    || email_send_daily.check_key(&key).is_err()
                {
                    tracing::warn!("Per-address email-send budget exceeded (in-memory)");
                    return Err(RateLimitError);
                }
                Ok(())
            }
            RateLimitBackend::Valkey(client) => {
                let minute_key = format!("rl:auth:email_send_min:{email_key}");
                let daily_key = format!("rl:auth:email_send_day:{email_key}");
                for (key, limit, window) in [
                    (&minute_key, EMAIL_SEND_MINUTE_LIMIT, WINDOW_SECS),
                    (&daily_key, EMAIL_SEND_DAILY_LIMIT, DAY_SECS),
                ] {
                    match client.check_rate_limit(key, limit, window).await {
                        Ok(_) => {}
                        Err(RateLimitCheckError::Exceeded) => {
                            tracing::warn!("Per-address email-send budget exceeded (valkey)");
                            return Err(RateLimitError);
                        }
                        Err(RateLimitCheckError::BackendError) => {
                            tracing::warn!(
                                "Email-send budget Valkey error — denying (fail-closed)"
                            );
                            return Err(RateLimitError);
                        }
                    }
                }
                Ok(())
            }
        }
    }

    async fn check(&self, endpoint: &str, limit: u32, ip: IpAddr) -> Result<(), RateLimitError> {
        match &self.backend {
            RateLimitBackend::InMemory {
                login,
                register,
                refresh,
                ..
            } => {
                let limiter = match endpoint {
                    "login" => login,
                    "register" => register,
                    "refresh" => refresh,
                    _ => return Ok(()),
                };
                match limiter.check_key(&ip) {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        tracing::warn!(ip = %ip, endpoint, "Rate limit exceeded (in-memory)");
                        Err(RateLimitError)
                    }
                }
            }
            RateLimitBackend::Valkey(client) => {
                let key = format!("rl:auth:{endpoint}:{ip}");
                match client.check_rate_limit(&key, limit, WINDOW_SECS).await {
                    Ok(_remaining) => Ok(()),
                    // Auth: fail-closed on both exceeded and backend error.
                    Err(RateLimitCheckError::Exceeded) => {
                        tracing::warn!(ip = %ip, endpoint, "Auth rate limit exceeded (valkey)");
                        Err(RateLimitError)
                    }
                    Err(RateLimitCheckError::BackendError) => {
                        tracing::warn!(ip = %ip, endpoint, "Auth rate limit Valkey error — denying (fail-closed)");
                        Err(RateLimitError)
                    }
                }
            }
        }
    }
}

/// Small error type to avoid large Result<(), Response> on the stack.
#[derive(Debug)]
pub struct RateLimitError;

impl From<RateLimitError> for Response {
    fn from(_: RateLimitError) -> Self {
        let body =
            crate::api::common::ErrorResponse::new("Too many requests. Please try again later.")
                .with_code("rate_limited")
                .with_retry_after(60)
                .with_action(
                    crate::api::common::AllowedAction::new("retry-later")
                        .with_hint("Wait retry_after_seconds before retrying."),
                );
        let (status, json) = body.into_response(StatusCode::TOO_MANY_REQUESTS);
        (status, [("retry-after", "60")], json).into_response()
    }
}

/// Number of trusted reverse-proxy hops directly in front of the server.
///
/// Each trusted proxy *appends* the peer IP it observed to the RIGHT of
/// `X-Forwarded-For`, so the real client IP is the entry `hops`-from-the-right.
/// Everything further left is client-supplied and MUST NOT be trusted. Default
/// is 1 (a single reverse proxy — the documented deployment topology, e.g.
/// Caddy). Operators who stack additional proxies (a CDN/LB in front of the
/// reverse proxy) set `TRUSTED_PROXY_HOPS` to that count.
fn trusted_proxy_hops() -> usize {
    static HOPS: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
        everruns_core::config::env_or("TRUSTED_PROXY_HOPS", 1usize).max(1)
    });
    *HOPS
}

/// Extract the real client IP from `X-Forwarded-For`, honoring `trusted_hops`
/// trusted reverse-proxy hops.
///
/// TM-AUTH-001 / TM-DOS: a standard reverse proxy *appends* the peer IP it saw
/// to the right of any client-supplied `X-Forwarded-For`. Taking the leftmost
/// entry therefore returns a fully attacker-controlled value, letting one client
/// mint unlimited distinct rate-limit keys. The real client IP is the entry
/// `hops`-from-the-right; entries further left are never used. Returns `None`
/// when the header is absent, empty, unparseable, or shorter than the configured
/// hop count (a misconfiguration / spoofing attempt — the caller then falls back
/// to the trusted peer IP).
fn extract_forwarded_ip_from_headers(headers: &HeaderMap, trusted_hops: usize) -> Option<IpAddr> {
    let forwarded = headers.get("x-forwarded-for")?.to_str().ok()?;
    let entries: Vec<&str> = forwarded
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let hops = trusted_hops.max(1);
    // The rightmost `hops` entries were appended by trusted proxies; the client
    // IP is the entry immediately to their left.
    let idx = entries.len().checked_sub(hops)?;
    entries.get(idx)?.parse::<IpAddr>().ok()
}

fn extract_real_ip_from_headers(headers: &HeaderMap) -> Option<IpAddr> {
    if let Some(real_ip) = headers.get("x-real-ip")
        && let Ok(val) = real_ip.to_str()
        && let Ok(ip) = val.trim().parse::<IpAddr>()
    {
        return Some(ip);
    }

    None
}

fn is_trusted_proxy_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => ipv4.is_loopback() || ipv4.is_private() || ipv4.is_link_local(),
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback() || ipv6.is_unique_local() || ipv6.is_unicast_link_local()
        }
    }
}

/// Extract client IP from request.
///
/// Security: Forwarding headers are accepted only when the immediate peer is a
/// trusted proxy (loopback/private ranges). Direct clients cannot spoof
/// X-Forwarded-For / X-Real-IP to influence rate-limit keys.
pub fn extract_client_ip(req: &Request<Body>) -> IpAddr {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    extract_client_ip_from_parts(peer, req.headers())
}

/// Extract client IP from a peer address and header map.
///
/// Same trusted-proxy contract as `extract_client_ip`, but callable from
/// handlers that have already consumed the request body.
pub fn extract_client_ip_from_parts(peer: Option<SocketAddr>, headers: &HeaderMap) -> IpAddr {
    if let Some(peer) = peer {
        let peer_ip = peer.ip();

        if is_trusted_proxy_ip(peer_ip) {
            if let Some(ip) = extract_forwarded_ip_from_headers(headers, trusted_proxy_hops()) {
                return ip;
            }
            if let Some(ip) = extract_real_ip_from_headers(headers) {
                return ip;
            }
        }

        return peer_ip;
    }

    // Fallback: no peer addr attached.
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

// ============================================================================
// Generic API rate limiting middleware
// ============================================================================

// TM-DOS: Global per-IP rate limiter for all API endpoints.
// Decision: Dual backend mirrors AuthRateLimiter — in-memory when VALKEY_URL is
//   not set, Valkey sliding-window when set.
// Decision: Fail-open on Valkey errors — a brief window of unrestricted traffic
//   is preferable to a complete API outage. (Auth endpoints are fail-closed because
//   auth hardening > availability; general API traffic inverts that tradeoff.)

/// Global per-IP rate limiter for all API endpoints.
///
/// Applied as Axum middleware. Dual backend: in-memory (governor) or Valkey
/// distributed sliding-window when `VALKEY_URL` is set.
/// Configurable via `RATE_LIMIT_API_REQUESTS_PER_MINUTE` env var (default: 1200).
#[derive(Clone)]
pub struct ApiRateLimiter {
    backend: ApiBackend,
}

#[derive(Clone)]
enum ApiBackend {
    InMemory(Arc<KeyedLimiter>),
    Valkey { client: ValkeyClient, limit: u32 },
}

impl ApiRateLimiter {
    fn api_rpm() -> u32 {
        let rpm: u32 = everruns_core::config::env_or("RATE_LIMIT_API_REQUESTS_PER_MINUTE", 1200);
        rpm.max(1)
    }

    pub fn from_env() -> Self {
        Self {
            backend: ApiBackend::InMemory(Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(Self::api_rpm()).unwrap(),
            )))),
        }
    }

    /// Create with a Valkey backend for distributed rate limiting.
    pub fn from_env_with_valkey(client: Option<ValkeyClient>) -> Self {
        match client {
            Some(c) => Self {
                backend: ApiBackend::Valkey {
                    client: c,
                    limit: Self::api_rpm(),
                },
            },
            None => Self::from_env(),
        }
    }

    /// Check if rate limiting is disabled (set to 0 via env var).
    pub fn is_disabled() -> bool {
        std::env::var("RATE_LIMIT_API_REQUESTS_PER_MINUTE")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            == Some(0)
    }

    pub async fn check(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        match &self.backend {
            ApiBackend::InMemory(limiter) => match limiter.check_key(&ip) {
                Ok(_) => Ok(()),
                Err(_) => Err(RateLimitError),
            },
            ApiBackend::Valkey { client, limit } => {
                let key = format!("rl:api:global:{ip}");
                match client.check_rate_limit(&key, *limit, WINDOW_SECS).await {
                    Ok(_) => Ok(()),
                    Err(RateLimitCheckError::Exceeded) => {
                        tracing::debug!(ip = %ip, "API rate limit exceeded (valkey)");
                        Err(RateLimitError)
                    }
                    // Fail-open: Valkey backend error → allow the request.
                    Err(RateLimitCheckError::BackendError) => Ok(()),
                }
            }
        }
    }
}

/// Axum middleware function for global API rate limiting.
pub async fn api_rate_limit_middleware(
    limiter: ApiRateLimiter,
    req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let ip = extract_client_ip(&req);
    if limiter.check(ip).await.is_err() {
        tracing::debug!(ip = %ip, "API rate limit exceeded");
        return RateLimitError.into();
    }
    next.run(req).await
}

// ============================================================================
// Per-org / per-user rate limiting for expensive operations
// ============================================================================

// TM-DOS: Per-identity rate limiting for expensive mutations.
// Decision: Keyed by org_id for session creation, by user UUID
//   for schedule creation and org creation.
// Decision: Fail-open on Valkey errors — same rationale as ApiRateLimiter.
//   The DB-level resource caps (max_orgs_per_user, max_sessions_per_org) still
//   bound total resource consumption; the rate limiter adds a per-minute velocity
//   cap on top.
// Decision: Configurable via env vars; defaults are generous enough to not
//   affect normal usage but stop trivial abuse.

const ORG_WINDOW_SECS: u64 = 60;
const ORG_HOUR_SECS: u64 = 3600;

/// Default per-minute limits (configurable via env vars).
const DEFAULT_SESSION_CREATE_RPM: u32 = 60;
const DEFAULT_SCHEDULE_CREATE_RPM: u32 = 20;
const DEFAULT_ORG_CREATE_RPH: u32 = 10;
/// Default per-org outbound tool calls per minute (TM-TOOL-009).
/// Generous enough not to affect normal agentic workloads; stops flood abuse.
const DEFAULT_ORG_TOOL_CALLS_RPM: u32 = 1000;

/// Per-org and per-user rate limiter for expensive operations.
///
/// Operations:
/// - `session_create`: keyed by org_id, 60/min default
/// - `schedule_create`: keyed by user_id, 20/min default
/// - `org_create`: keyed by user_id, 10/hr default
/// - `tool_calls`: keyed by org public UUID, 1000/min default (TM-TOOL-009)
#[derive(Clone)]
pub struct OrgRateLimiter {
    backend: OrgBackend,
}

#[derive(Clone)]
enum OrgBackend {
    InMemory {
        session_create: Arc<StringKeyedLimiter>,
        schedule_create: Arc<StringKeyedLimiter>,
        org_create: Arc<StringKeyedLimiter>,
        tool_calls: Arc<StringKeyedLimiter>,
    },
    Valkey {
        client: ValkeyClient,
        session_create_limit: u32,
        schedule_create_limit: u32,
        org_create_limit: u32,
        tool_calls_limit: u32,
    },
}

impl Default for OrgRateLimiter {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Operation discriminant for `OrgRateLimiter::check_keyed`.
/// Using an enum eliminates the `_ => Ok(())` silent bypass of unknown op strings.
#[derive(Debug, Clone, Copy)]
enum OrgOp {
    Session,
    Schedule,
    Org,
    ToolCall,
}

impl OrgRateLimiter {
    #[cfg(test)]
    pub(crate) fn for_test_with_session_rpm(session_rpm: u32) -> Self {
        Self {
            backend: OrgBackend::InMemory {
                session_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(session_rpm).unwrap(),
                ))),
                schedule_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(100).unwrap(),
                ))),
                org_create: Arc::new(RateLimiter::keyed(Quota::per_hour(
                    NonZeroU32::new(100).unwrap(),
                ))),
                tool_calls: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(100).unwrap(),
                ))),
            },
        }
    }

    pub fn from_env() -> Self {
        Self::from_env_with_valkey(None)
    }

    pub fn from_env_with_valkey(client: Option<ValkeyClient>) -> Self {
        let session_rpm: u32 = everruns_core::config::env_or(
            "RATE_LIMIT_ORG_SESSION_CREATE_PER_MINUTE",
            DEFAULT_SESSION_CREATE_RPM,
        );
        let schedule_rpm: u32 = everruns_core::config::env_or(
            "RATE_LIMIT_ORG_SCHEDULE_CREATE_PER_MINUTE",
            DEFAULT_SCHEDULE_CREATE_RPM,
        );
        let org_rph: u32 = everruns_core::config::env_or(
            "RATE_LIMIT_USER_ORG_CREATE_PER_HOUR",
            DEFAULT_ORG_CREATE_RPH,
        );
        let tool_rpm: u32 = everruns_core::config::env_or(
            "RATE_LIMIT_ORG_TOOL_CALLS_PER_MINUTE",
            DEFAULT_ORG_TOOL_CALLS_RPM,
        );

        match client {
            Some(c) => Self {
                backend: OrgBackend::Valkey {
                    client: c,
                    session_create_limit: session_rpm.max(1),
                    schedule_create_limit: schedule_rpm.max(1),
                    org_create_limit: org_rph.max(1),
                    tool_calls_limit: tool_rpm.max(1),
                },
            },
            None => Self {
                backend: OrgBackend::InMemory {
                    session_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                        NonZeroU32::new(session_rpm.max(1)).unwrap(),
                    ))),
                    schedule_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                        NonZeroU32::new(schedule_rpm.max(1)).unwrap(),
                    ))),
                    org_create: Arc::new(RateLimiter::keyed(Quota::per_hour(
                        NonZeroU32::new(org_rph.max(1)).unwrap(),
                    ))),
                    tool_calls: Arc::new(RateLimiter::keyed(Quota::per_minute(
                        NonZeroU32::new(tool_rpm.max(1)).unwrap(),
                    ))),
                },
            },
        }
    }

    /// Check per-org session creation rate. Returns Err if exceeded.
    pub async fn check_session_create(&self, org_id: i64) -> Result<(), RateLimitError> {
        self.check_keyed(OrgOp::Session, org_id.to_string(), ORG_WINDOW_SECS)
            .await
    }

    /// Check per-user schedule creation rate (keyed by user UUID). Returns Err if exceeded.
    pub async fn check_schedule_create(&self, user_id: Uuid) -> Result<(), RateLimitError> {
        self.check_keyed(OrgOp::Schedule, user_id.to_string(), ORG_WINDOW_SECS)
            .await
    }

    /// Check per-user org creation rate (keyed by user UUID). Returns Err if exceeded.
    pub async fn check_org_create(&self, user_id: Uuid) -> Result<(), RateLimitError> {
        self.check_keyed(OrgOp::Org, user_id.to_string(), ORG_HOUR_SECS)
            .await
    }

    /// Check per-org outbound tool call rate (keyed by the org's typed-ID string, e.g. `org_<uuid>`).
    /// Returns Err if the org has exceeded RATE_LIMIT_ORG_TOOL_CALLS_PER_MINUTE.
    pub async fn check_outbound_tool_call(&self, org_key: &str) -> Result<(), RateLimitError> {
        self.check_keyed(OrgOp::ToolCall, org_key.to_string(), ORG_WINDOW_SECS)
            .await
    }

    async fn check_keyed(
        &self,
        op: OrgOp,
        id: String,
        window_secs: u64,
    ) -> Result<(), RateLimitError> {
        match &self.backend {
            OrgBackend::InMemory {
                session_create,
                schedule_create,
                org_create,
                tool_calls,
            } => {
                let limiter = match op {
                    OrgOp::Session => session_create,
                    OrgOp::Schedule => schedule_create,
                    OrgOp::Org => org_create,
                    OrgOp::ToolCall => tool_calls,
                };
                match limiter.check_key(&id) {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        tracing::warn!(op = ?op, %id, "Org/user rate limit exceeded (in-memory)");
                        Err(RateLimitError)
                    }
                }
            }
            OrgBackend::Valkey {
                client,
                session_create_limit,
                schedule_create_limit,
                org_create_limit,
                tool_calls_limit,
            } => {
                let (key, limit) = match op {
                    OrgOp::Session => {
                        (format!("rl:org:session_create:{id}"), *session_create_limit)
                    }
                    OrgOp::Schedule => (
                        format!("rl:user:schedule_create:{id}"),
                        *schedule_create_limit,
                    ),
                    OrgOp::Org => (format!("rl:user:org_create:{id}"), *org_create_limit),
                    OrgOp::ToolCall => (format!("rl:org:tool_calls:{id}"), *tool_calls_limit),
                };
                match client.check_rate_limit(&key, limit, window_secs).await {
                    Ok(_) => Ok(()),
                    Err(RateLimitCheckError::Exceeded) => {
                        tracing::warn!(op = ?op, %id, "Org/user rate limit exceeded (valkey)");
                        Err(RateLimitError)
                    }
                    // Fail-open: Valkey backend error → allow the request.
                    // DB-level resource caps still apply; rate limiter only adds velocity control.
                    Err(RateLimitCheckError::BackendError) => Ok(()),
                }
            }
        }
    }
}

/// `everruns-core` trait implementation: lets `OrgRateLimiter` be injected into
/// `ActAtom` as the per-org outbound tool-call gate (TM-TOOL-009).
///
/// Keyed by `OrgId::to_string()` (typed-ID form, e.g. `org_<uuid>`) — the
/// same format used by `check_outbound_tool_call`. The internal i64 is not
/// available in `ActAtom`, so `OrgId` is the natural key.
#[async_trait::async_trait]
impl everruns_core::tool_execution::OutboundToolRateLimiter for OrgRateLimiter {
    async fn check_org(&self, org_id: &everruns_core::typed_id::OrgId) -> bool {
        // Fail-open: rate limit exceeded → false; backend error → true (already
        // handled inside check_keyed via Valkey BackendError path).
        self.check_outbound_tool_call(&org_id.to_string())
            .await
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn test_login_rate_limit_allows_initial_requests() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        assert!(limiter.check_login(ip).await.is_ok());
    }

    #[tokio::test]
    async fn test_login_rate_limit_blocks_after_burst() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for _ in 0..10 {
            let _ = limiter.check_login(ip).await;
        }
        assert!(limiter.check_login(ip).await.is_err());
    }

    #[tokio::test]
    async fn test_register_rate_limit_stricter_than_login() {
        let limiter = AuthRateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        for _ in 0..5 {
            let _ = limiter.check_register(ip).await;
        }
        assert!(
            limiter.check_register(ip).await.is_err(),
            "Register should be blocked after 5 requests"
        );
    }

    #[tokio::test]
    async fn account_login_limit_blocks_across_ips() {
        let limiter = AuthRateLimiter::new();
        // The per-account limiter is IP-independent: exhaust it for one
        // account and it stays blocked regardless of source, while another
        // account is unaffected.
        for _ in 0..ACCOUNT_LOGIN_LIMIT {
            let _ = limiter.check_account_login("victim@example.com").await;
        }
        assert!(
            limiter
                .check_account_login("victim@example.com")
                .await
                .is_err()
        );
        assert!(
            limiter
                .check_account_login("other@example.com")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn email_send_budget_one_per_minute_per_address() {
        let limiter = AuthRateLimiter::new();
        assert!(
            limiter
                .check_account_email_send("a@example.com")
                .await
                .is_ok()
        );
        assert!(
            limiter
                .check_account_email_send("a@example.com")
                .await
                .is_err(),
            "second send within a minute must be over budget"
        );
        assert!(
            limiter
                .check_account_email_send("b@example.com")
                .await
                .is_ok(),
            "budget is per address"
        );
    }

    #[tokio::test]
    async fn test_different_ips_have_separate_limits() {
        let limiter = AuthRateLimiter::new();
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        for _ in 0..5 {
            let _ = limiter.check_register(ip1).await;
        }
        assert!(limiter.check_register(ip1).await.is_err());
        assert!(limiter.check_register(ip2).await.is_ok());
    }

    #[test]
    fn test_extract_ip_from_x_forwarded_for_from_trusted_proxy() {
        // With one trusted proxy hop (default), the client IP is the RIGHTMOST
        // X-Forwarded-For entry — the one the trusted proxy appended. The left
        // entry is client-supplied and must not be used.
        let mut req = Request::builder()
            .header("x-forwarded-for", "203.0.113.50, 70.41.3.18")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                443,
            ))));
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(70, 41, 3, 18)));
    }

    // TM-AUTH-001 / TM-DOS regression: a spoofed leftmost X-Forwarded-For entry
    // must not change the derived client IP, so an attacker rotating the left
    // value cannot mint distinct rate-limit keys.
    #[test]
    fn test_spoofed_leftmost_xff_does_not_change_client_ip() {
        let client_ip = |xff: &str| {
            let mut req = Request::builder()
                .header("x-forwarded-for", xff)
                .body(Body::empty())
                .unwrap();
            req.extensions_mut()
                .insert(ConnectInfo(std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    443,
                ))));
            extract_client_ip(&req)
        };
        // The real (rightmost, proxy-appended) IP is identical; only the spoofed
        // leftmost value differs. Both must resolve to the same real client IP.
        let real = IpAddr::V4(Ipv4Addr::new(70, 41, 3, 18));
        assert_eq!(client_ip("1.1.1.1, 70.41.3.18"), real);
        assert_eq!(client_ip("2.2.2.2, 70.41.3.18"), real);
        assert_eq!(client_ip("9.9.9.9, 8.8.8.8, 70.41.3.18"), real);
    }

    #[test]
    fn test_extract_forwarded_ip_selects_hops_from_right() {
        use axum::http::HeaderValue;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("1.1.1.1, 2.2.2.2, 3.3.3.3"),
        );
        // One hop: rightmost entry.
        assert_eq!(
            extract_forwarded_ip_from_headers(&headers, 1),
            Some(IpAddr::V4(Ipv4Addr::new(3, 3, 3, 3)))
        );
        // Two hops: second from the right.
        assert_eq!(
            extract_forwarded_ip_from_headers(&headers, 2),
            Some(IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2)))
        );
        // Chain shorter than the hop count is not trusted.
        assert_eq!(extract_forwarded_ip_from_headers(&headers, 5), None);
    }

    #[test]
    fn test_extract_ip_from_x_real_ip_from_trusted_proxy() {
        let mut req = Request::builder()
            .header("x-real-ip", "198.51.100.25")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                443,
            ))));
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 25)));
    }

    #[test]
    fn test_extract_ip_fallback_to_loopback() {
        let req = Request::builder().body(Body::empty()).unwrap();
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn test_extract_ip_ignores_forwarded_for_from_untrusted_peer() {
        let mut req = Request::builder()
            .header("x-forwarded-for", "203.0.113.50")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [198, 51, 100, 10],
                443,
            ))));
        let ip = extract_client_ip(&req);
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10)));
    }

    // ============================================
    // API rate limiter tests
    // ============================================

    #[tokio::test]
    async fn test_api_rate_limiter_allows_initial_requests() {
        let limiter = ApiRateLimiter::from_env();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
        assert!(limiter.check(ip).await.is_ok());
    }

    #[tokio::test]
    async fn test_api_rate_limiter_blocks_after_burst() {
        let limiter = ApiRateLimiter {
            backend: ApiBackend::InMemory(Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(5).unwrap(),
            )))),
        };
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 101));
        for _ in 0..5 {
            let _ = limiter.check(ip).await;
        }
        assert!(
            limiter.check(ip).await.is_err(),
            "Should block after exceeding limit"
        );
    }

    #[tokio::test]
    async fn test_api_rate_limiter_separate_ips() {
        let limiter = ApiRateLimiter {
            backend: ApiBackend::InMemory(Arc::new(RateLimiter::keyed(Quota::per_minute(
                NonZeroU32::new(2).unwrap(),
            )))),
        };
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 10));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 11));
        for _ in 0..2 {
            let _ = limiter.check(ip1).await;
        }
        assert!(limiter.check(ip1).await.is_err());
        assert!(limiter.check(ip2).await.is_ok());
    }

    // ============================================
    // OrgRateLimiter tests
    // ============================================

    fn make_test_limiter_rpm(session_rpm: u32, schedule_rpm: u32, org_rph: u32) -> OrgRateLimiter {
        OrgRateLimiter {
            backend: OrgBackend::InMemory {
                session_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(session_rpm).unwrap(),
                ))),
                schedule_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(schedule_rpm).unwrap(),
                ))),
                org_create: Arc::new(RateLimiter::keyed(Quota::per_hour(
                    NonZeroU32::new(org_rph).unwrap(),
                ))),
                tool_calls: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(1000).unwrap(),
                ))),
            },
        }
    }

    #[tokio::test]
    async fn org_rate_limiter_session_create_allows_initial() {
        let limiter = make_test_limiter_rpm(60, 20, 10);
        assert!(limiter.check_session_create(1).await.is_ok());
    }

    #[tokio::test]
    async fn org_rate_limiter_session_create_blocks_after_burst() {
        let limiter = make_test_limiter_rpm(3, 3, 3);
        for _ in 0..3 {
            let _ = limiter.check_session_create(42).await;
        }
        assert!(limiter.check_session_create(42).await.is_err());
        // Different org_id is unaffected
        assert!(limiter.check_session_create(99).await.is_ok());
    }

    #[tokio::test]
    async fn org_rate_limiter_org_create_blocks_after_burst() {
        let user1 = Uuid::new_v4();
        let user2 = Uuid::new_v4();
        let limiter = make_test_limiter_rpm(3, 3, 2);
        for _ in 0..2 {
            let _ = limiter.check_org_create(user1).await;
        }
        assert!(limiter.check_org_create(user1).await.is_err());
        // Different user_id is unaffected
        assert!(limiter.check_org_create(user2).await.is_ok());
    }

    #[tokio::test]
    async fn org_rate_limiter_tool_calls_allows_initial() {
        use everruns_core::typed_id::OrgId;
        let limiter = make_test_limiter_rpm(60, 20, 10);
        let org_id = OrgId::new();
        assert!(
            limiter
                .check_outbound_tool_call(&org_id.to_string())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn org_rate_limiter_tool_calls_blocks_after_burst() {
        use everruns_core::typed_id::OrgId;
        let limiter = OrgRateLimiter {
            backend: OrgBackend::InMemory {
                session_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(100).unwrap(),
                ))),
                schedule_create: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(100).unwrap(),
                ))),
                org_create: Arc::new(RateLimiter::keyed(Quota::per_hour(
                    NonZeroU32::new(100).unwrap(),
                ))),
                tool_calls: Arc::new(RateLimiter::keyed(Quota::per_minute(
                    NonZeroU32::new(2).unwrap(),
                ))),
            },
        };
        let org1 = OrgId::new();
        let org2 = OrgId::new();
        for _ in 0..2 {
            let _ = limiter.check_outbound_tool_call(&org1.to_string()).await;
        }
        assert!(
            limiter
                .check_outbound_tool_call(&org1.to_string())
                .await
                .is_err()
        );
        assert!(
            limiter
                .check_outbound_tool_call(&org2.to_string())
                .await
                .is_ok()
        );
    }
}
