// Personal Agent Protocol ("Poppy", draft 0.1,
// https://personalagentprotocol.org) channel: a company's front door for
// personal agents, served at `/v1/channels/{channel_id}/poppy`.
//
// The company points its own `/.well-known/poppy.json` here (a redirect); we
// serve the discovery document, the OAuth server that starts Sessions, and
// the conversation API over the channel's agent.
//
// Design Decisions:
// - One channel is one Poppy Company. Its OAuth issuer is the channel's base
//   URL, so two companies hosted here never share an issuer, User IDs or
//   tokens. RFC 8414 metadata is served both where RFC 8414 puts it for an
//   issuer with a path (`/.well-known/oauth-authorization-server{path}`, a
//   root route) and appended to the issuer, which many clients try first.
// - Personal agents need no registration (spec 4.1): their `client_id` is the
//   HTTPS URL of their client metadata document, fetched through the same
//   SSRF-safe pinned client as channel JWKS. The channel may allow or block
//   agents by `client_id`.
// - Session Tokens are ES256 JWTs signed with the channel's own key (shared
//   with PACT's key table, keyed by channel), bound to the personal agent's
//   DPoP key (`cnf.jkt`). They carry only what the personal agent already
//   knows (its `client_id`, its User ID, the Session id), so they are signed,
//   not encrypted (spec 4.2 "Token format"). Every request also checks the
//   Session row, so an ended Session stops its tokens at once.
// - Only DPoP tokens are issued: no MCP API is listed yet, and spec 4.3
//   allows Bearer Session Tokens only at MCP APIs.
// - Routing before authentication: an unknown, disabled or non-Poppy channel
//   is the same `404` on every route, with or without a valid token.
// See `knowledge/integrations/poppy-channel.md`.

use std::sync::Arc;

use axum::{
    Extension, Router,
    extract::ConnectInfo,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};

use crate::api::channel_auth::ChannelAuthVerifier;
use crate::api::channel_ingress::{IngressChannel, IngressContext, channel_liveness};
use crate::api::channel_rate_limit::ChannelRateLimiter;
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::channels::a2a::signing::A2aReplayStore;
use crate::domains::agent_channels::record::poppy::PoppyChannelConfig;
use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::{EncryptionService, StorageBackend};

mod client;
mod conversations;
mod discovery;
mod dpop;
mod events;
mod tokens;

/// Where a channel's Poppy routes start.
const BASE: &str = "/v1/channels/{channel_id}/poppy";

#[derive(Clone)]
pub struct PoppyState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_delivery: EventDelivery,
    pub rate_limiter: ChannelRateLimiter,
    /// Fetches personal agents' client metadata documents and keys.
    pub verifier: ChannelAuthVerifier,
    /// Single use of assertion and DPoP proof `jti`s.
    pub replay_store: A2aReplayStore,
}

impl PoppyState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        runner: Arc<dyn everruns_core::host::TurnBackend>,
        notifications_enabled: bool,
        event_delivery: EventDelivery,
        rate_limiter: ChannelRateLimiter,
        replay_store: A2aReplayStore,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db.clone(),
                runner,
                notifications_enabled,
                event_delivery.clone(),
            )),
            db,
            encryption,
            event_delivery,
            rate_limiter,
            verifier: ChannelAuthVerifier::new(),
            replay_store,
        }
    }
}

/// Routes under the API prefix.
pub fn routes(state: PoppyState) -> Router {
    Router::new()
        .route(&format!("{BASE}/poppy.json"), get(discovery::poppy_json))
        .route(
            &format!("{BASE}/.well-known/oauth-authorization-server"),
            get(discovery::metadata),
        )
        .route(&format!("{BASE}/oauth/token"), post(tokens::token))
        .route(&format!("{BASE}/oauth/revoke"), post(tokens::revoke))
        .route(&format!("{BASE}/conversations"), post(conversations::start))
        .route(
            &format!("{BASE}/conversations/{{conversation_id}}/messages"),
            post(conversations::send),
        )
        .route(
            &format!("{BASE}/conversations/{{conversation_id}}/events"),
            get(conversations::events),
        )
        .route(
            &format!("{BASE}/conversations/{{conversation_id}}/handoff"),
            post(conversations::handoff),
        )
        .route(
            &format!("{BASE}/conversations/{{conversation_id}}/close"),
            post(conversations::close),
        )
        .with_state(state)
}

/// Routes at the server root, outside the API prefix: RFC 8414 metadata for
/// an issuer with a path.
pub fn well_known_routes(state: PoppyState) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server/{*issuer_path}",
            get(discovery::root_metadata),
        )
        .with_state(state)
}

pub(super) type Peer = Option<Extension<ConnectInfo<std::net::SocketAddr>>>;

/// A live Poppy channel.
pub(super) struct Channel {
    pub context: IngressContext,
    pub channel: IngressChannel,
    pub config: PoppyChannelConfig,
}

/// The live Poppy channel `channel_id`, or the finished `404`.
pub(super) async fn channel(state: &PoppyState, channel_id: &str) -> Result<Channel, Response> {
    let resolved = crate::api::channel_ingress::resolve_channel(
        &state.db,
        state.encryption.as_ref(),
        channel_id,
    )
    .await
    .map_err(internal)?;
    let Some((context, channel)) = resolved else {
        return Err(not_found());
    };
    if channel_liveness(&context, &channel).is_err() {
        return Err(not_found());
    }
    let Some(config) = channel.poppy_config() else {
        return Err(not_found());
    };
    Ok(Channel {
        context,
        channel,
        config,
    })
}

/// The URLs of one channel, derived from the request's own origin and path,
/// so they match what the personal agent called (DPoP `htu`, assertion `aud`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Urls {
    /// The OAuth issuer and the root of every other URL.
    pub issuer: String,
}

impl Urls {
    /// From any request under the channel's Poppy routes.
    pub fn from_request(headers: &HeaderMap, path: &str, channel_id: &str) -> Self {
        let marker = format!("/v1/channels/{channel_id}/poppy");
        let base = match path.find(&marker) {
            Some(at) => &path[..at + marker.len()],
            None => path,
        };
        Self {
            issuer: crate::channels::a2a::agent_card::absolute_url(headers, base),
        }
    }

    pub fn at(&self, path: &str) -> String {
        format!("{}/{path}", self.issuer)
    }
    pub fn token(&self) -> String {
        self.at("oauth/token")
    }
    pub fn revocation(&self) -> String {
        self.at("oauth/revoke")
    }
    pub fn conversations(&self) -> String {
        self.at("conversations")
    }
}

/// The request's own URL without query, for DPoP `htu` (spec 4.3).
pub(super) fn request_url(headers: &HeaderMap, path: &str) -> String {
    crate::channels::a2a::agent_card::absolute_url(headers, path)
}

/// THREAT[TM-POPPY-005]: the channel's optional per-IP cap.
pub(super) async fn rate_limit(
    state: &PoppyState,
    channel: &Channel,
    headers: &HeaderMap,
    peer: Peer,
) -> Result<(), Response> {
    let Some(limit) = channel
        .config
        .rate_limit_per_minute
        .filter(|limit| *limit > 0)
    else {
        return Ok(());
    };
    let client_ip = extract_client_ip_from_parts(peer.map(|Extension(ci)| ci.0), headers);
    let scope = format!(
        "{}:{}",
        channel.context.public_id, channel.channel.public_id
    );
    match state.rate_limiter.check(&scope, client_ip, limit).await {
        Ok(_) => Ok(()),
        Err(_) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            [
                (header::RETRY_AFTER, HeaderValue::from_static("60")),
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ],
            json!({ "error": "rate_limited", "error_description": "Too many requests" })
                .to_string(),
        )
            .into_response()),
    }
}

/// An error with a JSON `{error, error_description}` body (spec 4.2, 7.13).
pub(super) fn error(status: StatusCode, code: &str, description: &str) -> Response {
    json_response(
        status,
        &json!({ "error": code, "error_description": description }),
    )
}

pub(super) fn json_response(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body.to_string(),
    )
        .into_response()
}

pub(super) fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not_found", "Not found")
}

pub(super) fn internal(err: anyhow::Error) -> Response {
    tracing::error!(error = %err, "Poppy request failed");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server_error",
        "Internal error",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_keep_the_api_prefix_and_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("app.example"));
        let urls = Urls::from_request(
            &headers,
            "/api/v1/channels/ch_1/poppy/conversations/cnv_1/events",
            "ch_1",
        );
        assert_eq!(
            urls.issuer,
            "https://app.example/api/v1/channels/ch_1/poppy"
        );
        assert_eq!(
            urls.token(),
            "https://app.example/api/v1/channels/ch_1/poppy/oauth/token"
        );
    }
}
