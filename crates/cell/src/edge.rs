//! The edge router (`app edge`): one public URL for the Everruns API while
//! some orgs run on a data plane (design.md, step 8).
//!
//! Decisions:
//! - The proxy in front (Caddy locally) sends `/v1/sessions` and below to
//!   the edge, and everything else straight to the control plane: only
//!   session calls can belong to a data plane.
//! - A session's record (get, update, delete, archive, pin, cost, budgets,
//!   resolved model, participants, slash commands, run requests, and the
//!   list and create calls) always goes to the control plane,
//!   which keeps the record of every session, a data plane's too. The rest
//!   of a session's calls go to the data plane when the session is there.
//! - The edge finds the caller's org token itself, so the console and the
//!   CLI need no token code: an explicit one (the `X-Everruns-Org-Token`
//!   header, the `everruns_org_token` cookie, or a bearer org token), or by
//!   exchanging the caller's own credential at the control plane's
//!   `POST /v1/auth/token`. Exchanged tokens are cached by a hash of the
//!   credential and refreshed before they expire; "this org is not on a
//!   data plane" is cached for a minute. While the control plane is down a
//!   cached token keeps working until it expires.
//! - A cell never sees the caller's own credential: the edge replaces it
//!   with the org token, and the gateway checks that token as it would from
//!   any client (signature, data plane, the session's org).
//! - A session the data plane does not know (made before its org moved) is
//!   the control plane's; that answer is cached for half a minute.
//! - The CLI's command transport (`POST /v1/commands/{name}`) is routed the
//!   same way for a session's running calls (`edge_commands.rs`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use everruns_contracts::typed_id::SessionId;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tower::ServiceExt as _;

use serde_json::Value;

use crate::edge_commands;
use crate::gateway::Gateway;
use crate::org_token::OrgTokens;

/// A session's calls that stay with the control plane: its record, and
/// what the record answers (the model it resolves to, its participants,
/// the agent's slash commands, run requests). The console reads these for
/// every chat; without the resolved model its composer stays disabled.
const RECORD: &[&str] = &[
    "archive",
    "pin",
    "cost",
    "budgets",
    "budget-check",
    "resolved-model",
    "participants",
    "commands",
    "run-requests",
];
/// How long "not on a data plane" is believed.
const NOT_V2_FOR: Duration = Duration::from_secs(60);
/// How long "the data plane does not know this session" is believed.
const UNKNOWN_FOR: Duration = Duration::from_secs(30);
/// Exchanged tokens kept at most; past it, expired ones are dropped.
const CACHE: usize = 10_000;

pub struct Edge {
    gateway: Arc<Gateway>,
    app: Router,
    control_plane: String,
    /// The path prefix the API is served under (`/api` behind the local
    /// proxy, empty in production), kept on calls to the control plane and
    /// dropped on calls to the data plane.
    prefix: String,
    http: reqwest::Client,
    tokens: Arc<OrgTokens>,
    exchanged: Mutex<HashMap<[u8; 32], Exchanged>>,
    unknown: Mutex<HashMap<SessionId, Instant>>,
    /// Requests by route, for the bench.
    pub routed: crate::prom::Counters,
}

#[derive(Clone)]
enum Exchanged {
    Token {
        token: String,
        /// Unix seconds.
        expires: i64,
        /// When to fetch a fresh one.
        refresh: i64,
    },
    NotV2 {
        until: Instant,
    },
}

/// Where a request goes.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    ControlPlane,
    /// A session's data call: the data plane if the session is there.
    Session(SessionId),
}

fn route(path: &str) -> Route {
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match parts.as_slice() {
        ["v1", "sessions", id, first, ..] if !RECORD.contains(first) => {
            match SessionId::parse(id) {
                Ok(id) => Route::Session(id),
                Err(_) => Route::ControlPlane,
            }
        }
        _ => Route::ControlPlane,
    }
}

impl Edge {
    /// An edge over `gateway` (its topology names the cells), passing
    /// control plane calls to `control_plane` and checking org tokens with
    /// `tokens`.
    pub fn new(gateway: Arc<Gateway>, control_plane: &str, tokens: Arc<OrgTokens>) -> Arc<Self> {
        Self::with_prefix(gateway, control_plane, tokens, "")
    }

    /// [`Edge::new`] for an API served under `prefix` (such as `/api`).
    pub fn with_prefix(
        gateway: Arc<Gateway>,
        control_plane: &str,
        tokens: Arc<OrgTokens>,
        prefix: &str,
    ) -> Arc<Self> {
        gateway.require_org_tokens(tokens.clone());
        let app = crate::gateway::router(gateway.clone());
        Arc::new(Self {
            gateway,
            app,
            control_plane: control_plane.trim_end_matches('/').to_owned(),
            prefix: prefix.trim_end_matches('/').to_owned(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            tokens,
            exchanged: Mutex::default(),
            unknown: Mutex::default(),
            routed: Default::default(),
        })
    }

    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .route("/metrics", axum::routing::get(metrics))
            .fallback(handle)
            .with_state(self.clone())
    }

    /// The caller's org token from its own credential, or None when the
    /// caller's org is not on a data plane (or the caller is not signed in:
    /// the control plane says so in its own words).
    async fn exchanged_token(&self, headers: &HeaderMap) -> Option<String> {
        let key = credential_key(headers);
        let now = chrono::Utc::now().timestamp();
        let cached = self.exchanged.lock().get(&key).cloned();
        match &cached {
            Some(Exchanged::NotV2 { until }) if Instant::now() < *until => return None,
            Some(Exchanged::Token { token, refresh, .. }) if now < *refresh => {
                return Some(token.clone());
            }
            _ => {}
        }
        match self.exchange(headers).await {
            Ok(fresh) => {
                let token = match &fresh {
                    Exchanged::Token { token, .. } => Some(token.clone()),
                    Exchanged::NotV2 { .. } => None,
                };
                let mut exchanged = self.exchanged.lock();
                if exchanged.len() >= CACHE {
                    exchanged.retain(|_, e| match e {
                        Exchanged::Token { expires, .. } => *expires > now,
                        Exchanged::NotV2 { until } => Instant::now() < *until,
                    });
                }
                exchanged.insert(key, fresh);
                token
            }
            // The control plane did not answer: a token that has not
            // expired yet still works.
            Err(()) => match cached {
                Some(Exchanged::Token { token, expires, .. }) if now < expires => Some(token),
                _ => None,
            },
        }
    }

    /// Exchange the caller's credential. `Err` when the control plane did
    /// not answer; a refusal is `Ok(NotV2)` only for "not on a data plane".
    async fn exchange(&self, headers: &HeaderMap) -> Result<Exchanged, ()> {
        let mut request = self
            .http
            .post(format!(
                "{}{}/v1/auth/token",
                self.control_plane, self.prefix
            ))
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .timeout(Duration::from_secs(5));
        for name in [header::AUTHORIZATION, header::COOKIE] {
            for value in headers.get_all(&name) {
                request = request.header(&name, value);
            }
        }
        if let Some(org) = headers.get("x-org-id") {
            request = request.header("x-org-id", org);
        }
        let reply = request.send().await.map_err(|_| ())?;
        match reply.status() {
            StatusCode::OK => {
                let body: serde_json::Value = reply.json().await.map_err(|_| ())?;
                let token = body["token"].as_str().ok_or(())?.to_owned();
                let expires = body["expires_at_unix"].as_i64().ok_or(())?;
                let now = chrono::Utc::now().timestamp();
                // Fetch again a minute before expiry, or halfway through a
                // short life.
                let refresh = expires - ((expires - now) / 2).min(60);
                self.routed.inc(&[("exchange", "token")]);
                Ok(Exchanged::Token {
                    token,
                    expires,
                    refresh,
                })
            }
            StatusCode::CONFLICT => {
                self.routed.inc(&[("exchange", "not_v2")]);
                Ok(Exchanged::NotV2 {
                    until: Instant::now() + NOT_V2_FOR,
                })
            }
            // Not signed in, or no such org: not cached, and the control
            // plane answers the call itself.
            status if status.is_client_error() => Ok(Exchanged::NotV2 {
                until: Instant::now(),
            }),
            _ => Err(()),
        }
    }

    /// Whether the data plane holds `session`.
    async fn on_data_plane(&self, session: SessionId) -> bool {
        if self
            .unknown
            .lock()
            .get(&session)
            .is_some_and(|at| at.elapsed() < UNKNOWN_FOR)
        {
            return false;
        }
        match self.gateway.locate(session).await {
            Ok(_) => true,
            Err(_) => {
                let mut unknown = self.unknown.lock();
                if unknown.len() >= CACHE {
                    unknown.retain(|_, at| at.elapsed() < UNKNOWN_FOR);
                }
                unknown.insert(session, Instant::now());
                false
            }
        }
    }

    /// Pass `request` to the control plane as it came.
    async fn to_control_plane(&self, request: Request) -> Response {
        self.routed.inc(&[("route", "control_plane")]);
        let (parts, body) = request.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or(parts.uri.path(), |p| p.as_str());
        let mut upstream = self
            .http
            .request(
                parts.method.clone(),
                format!("{}{path}", self.control_plane),
            )
            .body(reqwest::Body::wrap_stream(body.into_data_stream()));
        for (name, value) in &parts.headers {
            if !hop_by_hop(name) && name != header::HOST {
                upstream = upstream.header(name, value);
            }
        }
        if !parts.headers.contains_key("x-forwarded-host")
            && let Some(host) = parts.headers.get(header::HOST)
        {
            upstream = upstream.header("x-forwarded-host", host);
        }
        match upstream.send().await {
            Ok(reply) => {
                let status = reply.status();
                let mut headers = HeaderMap::new();
                for (name, value) in reply.headers() {
                    if !hop_by_hop(name) {
                        headers.append(name.clone(), value.clone());
                    }
                }
                let mut response = Response::new(Body::from_stream(reply.bytes_stream()));
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                response
            }
            Err(error) => (
                StatusCode::BAD_GATEWAY,
                format!("the control plane did not answer: {error}"),
            )
                .into_response(),
        }
    }

    /// Serve `request` from the data plane, as the org `token` names.
    async fn to_data_plane(&self, mut request: Request, token: &str) -> Response {
        self.routed.inc(&[("route", "data_plane")]);
        let headers = request.headers_mut();
        headers.remove(header::COOKIE);
        headers.remove(crate::org_token::HEADER);
        let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) else {
            return (StatusCode::BAD_GATEWAY, "unusable org token").into_response();
        };
        headers.insert(header::AUTHORIZATION, value);
        if !self.prefix.is_empty() {
            let unprefixed = request
                .uri()
                .path_and_query()
                .map(|p| p.as_str())
                .and_then(|p| p.strip_prefix(self.prefix.as_str()))
                .and_then(|p| p.parse::<axum::http::Uri>().ok());
            if let Some(uri) = unprefixed {
                *request.uri_mut() = uri;
            }
        }
        match self.app.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }
}

async fn handle(State(edge): State<Arc<Edge>>, request: Request) -> Response {
    let path = request.uri().path();
    let path = path.strip_prefix(edge.prefix.as_str()).unwrap_or(path);
    if let Some(name) = path.strip_prefix("/v1/commands/")
        && request.method() == Method::POST
        && edge_commands::is_session_command(name)
    {
        let name = name.to_owned();
        return edge.command(name, request).await;
    }
    let Route::Session(session) = route(path) else {
        return edge.to_control_plane(request).await;
    };
    let token = match edge.token_for(request.headers()).await {
        Ok(Some(token)) => token,
        Ok(None) => return edge.to_control_plane(request).await,
        Err(refused) => return refused,
    };
    if !edge.on_data_plane(session).await {
        return edge.to_control_plane(request).await;
    }
    edge.to_data_plane(request, &token).await
}

impl Edge {
    /// The org token for a caller's call: the one it sent, checked, or its
    /// credential's exchanged one; None when its org is not on a data plane.
    async fn token_for(&self, headers: &HeaderMap) -> Result<Option<String>, Response> {
        match explicit(headers) {
            // An org token the caller sent is checked here: an expired or
            // forged one is refused, never swapped for another credential's.
            Some(token) => match self.tokens.verify(token).await {
                Ok(_) => Ok(Some(token.to_owned())),
                Err(error) => Err(error.into_response()),
            },
            None => Ok(self.exchanged_token(headers).await),
        }
    }

    /// `POST /v1/commands/{name}` for a session command (`edge_commands.rs`):
    /// the data plane's call in the command's envelope when the session is
    /// there, the control plane's answer otherwise.
    async fn command(&self, name: String, request: Request) -> Response {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, edge_commands::MAX_BODY).await else {
            return (StatusCode::PAYLOAD_TOO_LARGE, "command body too large").into_response();
        };
        let envelope: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let key = parts
            .headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok());
        let target = match edge_commands::translate(&name, &envelope, key) {
            None => None,
            Some(call) => match self.token_for(&parts.headers).await {
                Err(refused) => return refused,
                Ok(None) => None,
                Ok(Some(token)) => self
                    .on_data_plane(call.session)
                    .await
                    .then_some((call, token)),
            },
        };
        let Some((call, token)) = target else {
            return self
                .to_control_plane(Request::from_parts(parts, Body::from(bytes)))
                .await;
        };
        let mut forwarded = Request::new(Body::from(
            call.body.map(|body| body.to_string()).unwrap_or_default(),
        ));
        *forwarded.method_mut() = call.method;
        let Ok(uri) = call.path_and_query.parse() else {
            return (StatusCode::BAD_REQUEST, "unusable command params").into_response();
        };
        *forwarded.uri_mut() = uri;
        for (name, value) in &parts.headers {
            if !hop_by_hop(name)
                && name != header::HOST
                && name != header::CONTENT_LENGTH
                && name != header::CONTENT_TYPE
            {
                forwarded.headers_mut().append(name.clone(), value.clone());
            }
        }
        forwarded.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let (answer, body) = self.to_data_plane(forwarded, &token).await.into_parts();
        if !answer.status.is_success() {
            return Response::from_parts(answer, body);
        }
        let Ok(bytes) = axum::body::to_bytes(body, edge_commands::MAX_BODY).await else {
            return (
                StatusCode::BAD_GATEWAY,
                "the data plane's answer was too large",
            )
                .into_response();
        };
        let output = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        axum::Json(edge_commands::answer(&name, &envelope, output)).into_response()
    }
}

async fn metrics(State(edge): State<Arc<Edge>>) -> String {
    let mut w = crate::prom::Writer::new(&[]);
    w.counters(
        "everruns_edge_requests_total",
        "Requests by where the edge sent them, and token exchanges by answer.",
        &edge.routed,
    );
    w.out
}

/// An org token the caller sent itself.
fn explicit(headers: &HeaderMap) -> Option<&str> {
    let token = crate::org_token::bearer(headers)?;
    // An API key or a console session token is not an org token: only an
    // EdDSA JWT can be.
    let header = jsonwebtoken::decode_header(token).ok()?;
    (header.alg == jsonwebtoken::Algorithm::EdDSA).then_some(token)
}

/// What identifies a caller's credential, hashed: the token exchange
/// depends on nothing else.
fn credential_key(headers: &HeaderMap) -> [u8; 32] {
    let mut hash = Sha256::new();
    for name in [header::AUTHORIZATION, header::COOKIE] {
        for value in headers.get_all(&name) {
            hash.update(value.as_bytes());
            hash.update([0]);
        }
        hash.update([1]);
    }
    if let Some(org) = headers.get("x-org-id") {
        hash.update(org.as_bytes());
    }
    hash.finalize().into()
}

fn hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_stay_with_the_control_plane_and_data_calls_route_to_the_session() {
        let id = SessionId::new();
        assert_eq!(route("/v1/sessions"), Route::ControlPlane);
        assert_eq!(route(&format!("/v1/sessions/{id}")), Route::ControlPlane);
        assert_eq!(
            route(&format!("/v1/sessions/{id}/pin")),
            Route::ControlPlane
        );
        assert_eq!(
            route(&format!("/v1/sessions/{id}/archive")),
            Route::ControlPlane
        );
        assert_eq!(
            route(&format!("/v1/sessions/{id}/resolved-model")),
            Route::ControlPlane
        );
        assert_eq!(
            route(&format!("/v1/sessions/{id}/participants")),
            Route::ControlPlane
        );
        assert_eq!(route(&format!("/v1/sessions/{id}/sse")), Route::Session(id));
        assert_eq!(
            route(&format!("/v1/sessions/{id}/messages")),
            Route::Session(id)
        );
        // A stage 2 call goes to the data plane, which answers 501 for it.
        assert_eq!(
            route(&format!("/v1/sessions/{id}/fs/a.md")),
            Route::Session(id)
        );
        assert_eq!(route("/v1/sessions/not-an-id/sse"), Route::ControlPlane);
        assert_eq!(route("/v1/agents"), Route::ControlPlane);
    }

    #[test]
    fn a_credential_is_keyed_by_what_identifies_it() {
        let mut a = HeaderMap::new();
        a.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer one"),
        );
        let mut b = a.clone();
        assert_eq!(credential_key(&a), credential_key(&b));
        b.insert("x-org-id", HeaderValue::from_static("org_2"));
        assert_ne!(credential_key(&a), credential_key(&b));
        let mut c = HeaderMap::new();
        c.insert(header::COOKIE, HeaderValue::from_static("Bearer one"));
        assert_ne!(credential_key(&a), credential_key(&c));
    }

    #[test]
    fn an_api_key_is_not_taken_for_an_org_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer evr_abc"),
        );
        assert_eq!(explicit(&headers), None);
    }
}
