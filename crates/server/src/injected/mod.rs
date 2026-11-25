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
// - Off until a data plane is trusted (the linked one, or
//   `INJECTED_CAPABILITIES_JWKS` with `INJECTED_CAPABILITIES_ISSUERS`):
//   without one every request is refused with 404, so nothing is reachable
//   by default.
// - The audience a token must carry is this server's own URL for the
//   capability, built from configuration (`INJECTED_CAPABILITIES_BASE_URL`,
//   else the auth base URL), never from request headers. Data planes often
//   reach the control plane on an internal address, so the URL it hands
//   them can be that one.
// - The data plane this control plane links to (`EVERRUNS_DP_KEY`) is
//   trusted without configuration: its executors sign with a key derived
//   from that key (`dataplane_seal::session_token_pkcs8`), so the control
//   plane derives the same public key instead of fetching or being given
//   a JWKS. `INJECTED_CAPABILITIES_JWKS` adds other data planes (agentd).
// - Every failure to authenticate is the same 401, so the endpoint does not
//   tell a caller which part of its token was wrong.
// - Side by side with today's paths: serving a capability here does not take
//   it off v1 workers or shards. Converting each one is a separate step.

pub mod hosted;
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
/// The base URL data planes reach this server at (API prefix included),
/// when not the public one: a cluster service or `127.0.0.1`. Data planes
/// trust it as their control plane (`AGENTD_CONTROL_PLANE`).
pub const BASE_URL_ENV: &str = "INJECTED_CAPABILITIES_BASE_URL";

/// The capabilities this server serves, and whom it trusts to call them.
pub struct InjectedCapabilities {
    base_url: String,
    verifier: Option<SessionTokenVerifier>,
    services: BTreeMap<String, Arc<dyn InjectedCapabilityService>>,
    /// The handler hosted services run their tools' calls with
    /// (`data_plane::calls`).
    calls: hosted::Calls,
}

impl InjectedCapabilities {
    /// `base_url` is the public API base URL, API prefix included.
    pub fn new(base_url: &str, verifier: Option<SessionTokenVerifier>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            verifier: verifier.filter(|v| !v.is_empty()),
            services: BTreeMap::new(),
            calls: hosted::Calls::default(),
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

    /// Where hosted services' calls go; bound once the API state exists.
    pub fn calls(&self) -> hosted::Calls {
        self.calls.clone()
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

/// The verifier for the session tokens of the data plane whose key is
/// `dp_key` (the link's `EVERRUNS_DP_KEY`).
pub fn linked_data_plane_verifier(
    dp_key: &everruns_contracts::dataplane_seal::Key,
) -> anyhow::Result<SessionTokenVerifier> {
    use everruns_contracts::dataplane_seal::{SESSION_TOKEN_ISSUER, session_token_pkcs8};
    let signer = everruns_core::mcp::session_token::SessionTokenSigner::from_pkcs8(
        SESSION_TOKEN_ISSUER,
        &session_token_pkcs8(dp_key),
    )?;
    SessionTokenVerifier::from_jwks(&signer.jwks(), vec![SESSION_TOKEN_ISSUER.to_string()])
}

/// Trusted data planes: the linked one, then any configured in the
/// environment. `None` when there are none.
fn trusted_from_env() -> Option<SessionTokenVerifier> {
    let mut trusted = None::<SessionTokenVerifier>;
    let linked = crate::domains::data_plane::Link::from_env()
        .ok()
        .and_then(|link| link.dp_key);
    if let Some(dp_key) = linked {
        match linked_data_plane_verifier(&dp_key) {
            Ok(verifier) => trusted = Some(verifier),
            Err(error) => {
                tracing::error!(error = %error, "injected capabilities: linked data plane key")
            }
        }
    }
    match InjectedCapabilities::verifier_from_env() {
        Ok(Some(configured)) => match trusted.as_mut() {
            Some(trusted) => trusted.extend(configured),
            None => trusted = Some(configured),
        },
        Ok(None) => {}
        Err(error) => {
            tracing::error!(error = %error, "injected capabilities: invalid trusted keys ignored");
        }
    }
    trusted
}

/// The capabilities this deployment serves, trusting the linked data plane
/// and the data plane keys in the environment. Serves nothing (404) when
/// there are none; a broken configuration is logged and left out, so it
/// never stops startup.
pub fn from_env(db: Arc<crate::storage::StorageBackend>, base_url: &str) -> InjectedCapabilities {
    let configured = std::env::var(BASE_URL_ENV)
        .ok()
        .filter(|url| !url.trim().is_empty());
    let base_url = configured.as_deref().unwrap_or(base_url);
    let verifier = trusted_from_env();
    let configs: Arc<dyn knowledge_base::SessionCapabilityConfig> = Arc::new(
        knowledge_base::StorageSessionCapabilityConfig::new(db.clone()),
    );
    let knowledge = knowledge_base::KnowledgeBaseService::new(
        base_url,
        configs.clone(),
        Arc::new(crate::knowledge_store::StorageBackendKnowledgeStore::new(
            db.clone(),
        )),
    );
    let capabilities = InjectedCapabilities::new(base_url, verifier);
    let calls = capabilities.calls();
    capabilities
        .register(Arc::new(knowledge))
        .register(Arc::new(hosted::poppy::service(
            base_url,
            configs.clone(),
            calls.clone(),
        )))
        .register(Arc::new(hosted::plugins::service(
            base_url,
            configs.clone(),
            calls.clone(),
        )))
        .register(Arc::new(hosted::slack::service(
            base_url,
            configs,
            calls.clone(),
            db,
        )))
        .register(Arc::new(hosted::memory::service(base_url, calls.clone())))
        .register(Arc::new(hosted::platform::service(base_url, calls.clone())))
        .register(Arc::new(hosted::person_mcp::service(
            base_url,
            calls.clone(),
        )))
        .register(Arc::new(hosted::credentials::service(base_url, calls)))
}

/// What this control plane hands its data planes: the definitions it
/// serves, once a data plane is trusted to call them; none before.
static SERVED: std::sync::RwLock<Vec<InjectedCapabilityDefinition>> =
    std::sync::RwLock::new(Vec::new());

/// Hand `capabilities` to data planes from now on (startup).
pub fn serve_to_data_planes(capabilities: &InjectedCapabilities) {
    let definitions = if capabilities.is_enabled() {
        capabilities.definitions()
    } else {
        Vec::new()
    };
    *SERVED.write().unwrap_or_else(|p| p.into_inner()) = definitions;
}

/// The definitions data planes get ([`serve_to_data_planes`]).
pub fn served() -> Vec<InjectedCapabilityDefinition> {
    SERVED.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// A capability ref's id, in either spelling a package uses.
fn ref_id(entry: &Value) -> Option<&str> {
    entry
        .as_str()
        .or_else(|| entry.get("ref").and_then(Value::as_str))
}

/// Swap the hosted capabilities of an agent package a data plane will run
/// for the injected capabilities this control plane serves.
///
/// Decisions:
/// - An injected capability named `N` takes the place of the hosted
///   capability with id `N`, in its position: the data plane then calls
///   this server for it instead of running it.
/// - Every `injected:` ref gets the served definition as its config,
///   overwriting whatever an author wrote: the server URL and the
///   session-token auth mode only ever come from here. One this control
///   plane does not serve is dropped, since nothing could answer it.
/// - A hosted capability's native part stays: Slack's model view rewrites
///   every model call's history, which a server's tool cannot do, so it is
///   added beside `injected:slack` as `slack_model_view`.
/// - `always` capabilities are added to every package. Packages carry no
///   session labels, so one with `match` labels is never added here.
pub fn attach(package: &mut Value, served: &[InjectedCapabilityDefinition]) {
    use everruns_contracts::runtime::capabilities::injected::{
        InjectedAttach, parse_injected_capability_id,
    };
    const FIELD: &str = "capabilities";
    let Some(object) = package.as_object_mut() else {
        return;
    };
    let entries = object
        .get(FIELD)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // The native part a hosted capability keeps on the data plane when its
    // tools are served here.
    const NATIVE_PARTS: &[(&str, &str)] = &[(
        everruns_capabilities::capabilities::SLACK_CAPABILITY_ID,
        everruns_capabilities::capabilities::SLACK_MODEL_VIEW_CAPABILITY_ID,
    )];
    let definition_of = |name: &str| served.iter().find(|d| d.name() == name);
    let mut seen = std::collections::BTreeSet::new();
    let mut attached = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(id) = ref_id(&entry) else {
            attached.push(entry);
            continue;
        };
        let name = parse_injected_capability_id(id).unwrap_or(id).to_string();
        let injected = parse_injected_capability_id(id).is_some();
        match definition_of(&name) {
            Some(definition) => {
                if seen.insert(name.clone()) {
                    attached.push(serde_json::json!({
                        "ref": definition.id(),
                        "config": definition.to_declarative(),
                    }));
                    for (_, part) in NATIVE_PARTS.iter().filter(|(hosted, _)| *hosted == name) {
                        if seen.insert(part.to_string()) {
                            attached.push(serde_json::json!({ "ref": part }));
                        }
                    }
                }
            }
            None if injected => {}
            None => {
                if seen.insert(name) {
                    attached.push(entry);
                }
            }
        }
    }
    for definition in served {
        if definition.attach == InjectedAttach::Always
            && definition.match_labels.is_empty()
            && seen.insert(definition.name().to_string())
        {
            attached.push(serde_json::json!({
                "ref": definition.id(),
                "config": definition.to_declarative(),
            }));
        }
    }
    if attached.is_empty() && !object.contains_key(FIELD) {
        return;
    }
    object.insert(FIELD.to_string(), Value::Array(attached));
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
