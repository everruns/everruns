// Injected capabilities served by the control plane.
//
// Design: knowledge/execution/injected-capabilities.md
//
// The server keeps hosted capabilities (Knowledge, Memory, Slack actions,
// Poppy, platform commands) and serves them over MCP to whichever data plane
// runs the session: `POST {api}/injected/{name}/mcp`. Each request carries a
// session token the data plane signed; the server checks it against the data
// planes it trusts and acts for that session in that organization.
//
// Decisions:
// - Mounted beside `/mcp`, outside the user-auth API stack: the caller is a
//   data plane vouching for a session, never a person.
// - Off until trusted keys are configured (`INJECTED_CAPABILITIES_JWKS`,
//   `INJECTED_CAPABILITIES_ISSUERS`): without them every request is refused
//   with 404, so nothing is reachable by default.
// - The audience a token must carry is this server's own public URL for the
//   capability, built from the auth base URL, never from request headers.
// - Every failure to authenticate is the same 401, so the endpoint does not
//   tell a caller which part of its token was wrong.
// - Side by side with today's paths: serving a capability here does not take
//   it off v1 workers or cells. Converting each one is a separate step.

pub mod knowledge_base;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use everruns_contracts::runtime::capabilities::injected::{
    InjectedCapabilityDefinition, injected_capability_id,
};
use everruns_core::mcp::injected_server::{
    InjectedCapabilityService, VerifiedSession, handle_injected_request,
};
use everruns_core::mcp::session_token::SessionTokenVerifier;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Largest request body the endpoint reads.
const MAX_BODY_BYTES: usize = 1024 * 1024;

pub const JWKS_ENV: &str = "INJECTED_CAPABILITIES_JWKS";
pub const ISSUERS_ENV: &str = "INJECTED_CAPABILITIES_ISSUERS";

/// The capabilities this server serves, and whom it trusts to call them.
pub struct InjectedCapabilities {
    base_url: String,
    verifier: Option<SessionTokenVerifier>,
    services: BTreeMap<String, Arc<dyn InjectedCapabilityService>>,
}

impl InjectedCapabilities {
    /// `base_url` is the public API base URL, API prefix included.
    pub fn new(base_url: &str, verifier: Option<SessionTokenVerifier>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            verifier: verifier.filter(|v| !v.is_empty()),
            services: BTreeMap::new(),
        }
    }

    /// The public URL of one capability's MCP endpoint, which is also the
    /// audience its session tokens carry.
    pub fn url_for(base_url: &str, name: &str) -> String {
        format!("{}/injected/{name}/mcp", base_url.trim_end_matches('/'))
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn register(mut self, service: Arc<dyn InjectedCapabilityService>) -> Self {
        self.services
            .insert(service.definition().name().to_string(), service);
        self
    }

    /// What a controller registers with each data plane.
    pub fn definitions(&self) -> Vec<InjectedCapabilityDefinition> {
        self.services
            .values()
            .map(|service| service.definition().clone())
            .collect()
    }

    pub fn is_enabled(&self) -> bool {
        self.verifier.is_some()
    }

    /// Trusted data plane keys from the environment. `None` when unset.
    pub fn verifier_from_env() -> anyhow::Result<Option<SessionTokenVerifier>> {
        let Ok(jwks) = std::env::var(JWKS_ENV) else {
            return Ok(None);
        };
        if jwks.trim().is_empty() {
            return Ok(None);
        }
        let issuers: Vec<String> = std::env::var(ISSUERS_ENV)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        anyhow::ensure!(
            !issuers.is_empty(),
            "{ISSUERS_ENV} must name the issuers whose keys {JWKS_ENV} holds"
        );
        let jwks: Value = serde_json::from_str(&jwks)
            .map_err(|e| anyhow::anyhow!("{JWKS_ENV} is not JSON: {e}"))?;
        Ok(Some(SessionTokenVerifier::from_jwks(&jwks, issuers)?))
    }
}

/// The capabilities this deployment serves, trusting the data plane keys in
/// the environment. Serves nothing (404) when none are configured; a broken
/// configuration is logged and treated the same, so it never stops startup.
pub fn from_env(db: Arc<crate::storage::StorageBackend>, base_url: &str) -> InjectedCapabilities {
    let verifier = match InjectedCapabilities::verifier_from_env() {
        Ok(verifier) => verifier,
        Err(error) => {
            tracing::error!(error = %error, "injected capabilities disabled: invalid trusted keys");
            None
        }
    };
    let knowledge = knowledge_base::KnowledgeBaseService::new(
        base_url,
        Arc::new(knowledge_base::StorageSessionCapabilityConfig::new(
            db.clone(),
        )),
        Arc::new(crate::knowledge_store::StorageBackendKnowledgeStore::new(
            db,
        )),
    );
    InjectedCapabilities::new(base_url, verifier).register(Arc::new(knowledge))
}

/// `POST {api_prefix}/injected/{name}/mcp`.
pub fn routes(state: Arc<InjectedCapabilities>, api_prefix: &str) -> Router {
    let path = format!("{}/injected/{{name}}/mcp", api_prefix.trim_end_matches('/'));
    Router::new().route(&path, post(serve)).with_state(state)
}

fn unauthorized() -> Response {
    let mut response = (StatusCode::UNAUTHORIZED, "invalid session token").into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer error=\"invalid_token\""),
    );
    response
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

async fn serve(
    State(state): State<Arc<InjectedCapabilities>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (Some(verifier), Some(service)) = (&state.verifier, state.services.get(&name)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(token) = bearer(&headers) else {
        return unauthorized();
    };
    let audience = InjectedCapabilities::url_for(&state.base_url, &name);
    let claims = match verifier.verify(token, &audience, &injected_capability_id(&name)) {
        Ok(claims) => claims,
        Err(error) => {
            tracing::debug!(capability = %name, error = %error, "injected capability token refused");
            return unauthorized();
        }
    };
    let Some(session) = VerifiedSession::from_claims(claims) else {
        return unauthorized();
    };
    if body.len() > MAX_BODY_BYTES {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": { "code": -32700, "message": "parse error" }
                })),
            )
                .into_response();
        }
    };
    match handle_injected_request(service.as_ref(), &session, message).await {
        Some(reply) => Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

#[cfg(test)]
mod tests;
