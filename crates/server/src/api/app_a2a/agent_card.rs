// The A2A Agent Card: unauthenticated discovery for an A2A endpoint.
//
// Split out of `app_a2a.rs` because none of it touches the JSON-RPC request
// path: the card is built from the endpoint's stored config and the request's
// own URI, and its only contract with the rest of the channel is the security
// scheme it advertises for the auth policy that channel actually enforces.
// See `knowledge/integrations/a2a-channel.md`.

use axum::{
    Json,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};

use super::{
    A2A_AGENT_VERSION, A2A_PROTOCOL_BINDING_JSONRPC, A2A_PROTOCOL_VERSION, AppA2aState,
    endpoint_app_id, internal_error, not_found,
};
use crate::api::a2a_signing::A2A_SIGNATURE_HEADER;
use crate::api::common::ErrorResponse;

/// GET /v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json
#[utoipa::path(
    get,
    path = "/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json",
    params(
        ("app_id" = String, Path, description = "App ID"),
        ("channel_id" = String, Path, description = "A2A channel ID")
    ),
    responses(
        (status = 200, description = "Agent Card JSON"),
        (status = 404, description = "App or channel not found / unpublished / disabled", body = ErrorResponse),
    ),
    tag = "apps"
)]
pub async fn agent_card_legacy(
    State(state): State<AppA2aState>,
    OriginalUri(original_uri): OriginalUri,
    Path((app_id, channel_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    agent_card(state, original_uri, app_id, channel_id, headers).await
}

#[utoipa::path(
    description = "Get the public Agent Card for a published A2A endpoint.",
    get,
    path = "/v1/e/{channel_id}/a2a/.well-known/agent-card.json",
    params(("channel_id" = String, Path, description = "A2A endpoint channel ID")),
    responses(
        (status = 200, description = "Agent Card JSON"),
        (status = 404, description = "Endpoint not found, app not published, or channel disabled", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn agent_card_endpoint(
    State(state): State<AppA2aState>,
    OriginalUri(original_uri): OriginalUri,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = endpoint_app_id(&state, &channel_id).await?;
    agent_card(state, original_uri, app_id, channel_id, headers).await
}

async fn agent_card(
    state: AppA2aState,
    original_uri: axum::http::Uri,
    app_id: String,
    channel_id: String,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let (app, channel) = crate::api::app_ingress::resolve_endpoint(
        &state.db,
        state.encryption.as_ref(),
        &channel_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;
    if !app.matches_legacy_app_id(&app_id) {
        return Err(not_found());
    }
    if channel.channel_type != everruns_platform::ChannelType::A2a {
        return Err(not_found());
    }
    // The Agent Card is only served for a live endpoint: it advertises the
    // invocation URL and security scheme, so publishing it for a draft or
    // suspended endpoint would leak a surface that refuses traffic.
    if crate::api::app_ingress::endpoint_liveness(&app, &channel).is_err() {
        return Err(not_found());
    }
    let config = channel.a2a_config().ok_or_else(not_found)?;

    // Build the absolute endpoint URL from the actual request URI and inbound
    // Host header. Test and proxy deployments can mount API routes under a
    // prefix such as `/api`; deriving from the original URI preserves it.
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("https");
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok());
    let endpoint_path = original_uri
        .path()
        .strip_suffix("/.well-known/agent-card.json")
        .unwrap_or_else(|| original_uri.path());
    let endpoint = match host {
        Some(host) => format!("{scheme}://{host}{endpoint_path}"),
        None => endpoint_path.to_string(),
    };

    let name = config
        .agent_card_name
        .clone()
        .unwrap_or_else(|| app.name.clone());
    let description = config
        .agent_card_description
        .clone()
        .or_else(|| app.description.clone())
        .unwrap_or_default();

    let (security_schemes, security) = a2a_security_for_config(&config, channel.auth.as_deref());
    let card = json!({
        "name": name,
        "description": description,
        "version": A2A_AGENT_VERSION,
        "supportedInterfaces": [
            {
                "url": endpoint,
                "protocolBinding": A2A_PROTOCOL_BINDING_JSONRPC,
                "protocolVersion": A2A_PROTOCOL_VERSION,
            }
        ],
        "capabilities": {
            // Streaming is only supported on session_per_invocation channels.
            // Shared-session channels reject message/stream because events
            // cannot be safely correlated across concurrent callers.
            "streaming": config.session_mode == everruns_platform::app::SessionBinding::Ephemeral,
            "pushNotifications": false,
            "stateTransitionHistory": false,
        },
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [
            {
                "id": "default",
                "name": app.name,
                "description": description,
                "tags": ["everruns", "a2a"],
            }
        ],
        "securitySchemes": security_schemes,
        "securityRequirements": security,
    });
    Ok(Json(card))
}

fn a2a_security_for_config(
    config: &everruns_platform::A2aChannelConfig,
    auth: Option<&everruns_platform::AppEndpointAuthConfig>,
) -> (Value, Value) {
    let (mut schemes, mut requirements) = base_a2a_security(auth);
    // THREAT[TM-A2A-010]: When the channel opts into HMAC signing, advertise
    // a vendor `everrunsHmacSignature` scheme alongside whichever primary
    // scheme is in use so the calling A2A client knows it must sign on top
    // of authentication.
    if config
        .signing_secret
        .as_deref()
        .is_some_and(|s| !s.is_empty())
    {
        if let Value::Object(map) = &mut schemes {
            map.insert(
                "everrunsHmacSignature".to_string(),
                json!({
                    "apiKeySecurityScheme": {
                        "location": "header",
                        "name": A2A_SIGNATURE_HEADER,
                        "description": "HMAC-SHA256 over v0:{timestamp}:{channel_scope}:{body}; pair with X-Everruns-A2A-Timestamp",
                    }
                }),
            );
        }
        if let Value::Array(arr) = &mut requirements {
            if let Some(Value::Object(first)) = arr.first_mut() {
                first.insert("everrunsHmacSignature".to_string(), json!([]));
            } else {
                arr.push(json!({ "everrunsHmacSignature": [] }));
            }
        }
    }
    (schemes, requirements)
}

fn base_a2a_security(auth: Option<&everruns_platform::AppEndpointAuthConfig>) -> (Value, Value) {
    let Some(auth) = auth else {
        return (
            json!({ "apiKey": { "httpAuthSecurityScheme": { "scheme": "bearer" } } }),
            json!([{ "apiKey": [] }]),
        );
    };
    match (&auth.mode, auth.provider.as_ref()) {
        (everruns_platform::AppEndpointAuthMode::HttpBasic, _) => (
            json!({ "httpBasic": { "httpAuthSecurityScheme": { "scheme": "basic" } } }),
            json!([{ "httpBasic": [] }]),
        ),
        (
            everruns_platform::AppEndpointAuthMode::GoogleOidc,
            Some(everruns_platform::AppEndpointAuthProviderConfig::GoogleOidc { .. }),
        ) => (
            json!({
                "googleOidc": {
                    "openIdConnectSecurityScheme": {
                        "openIdConnectUrl": "https://accounts.google.com/.well-known/openid-configuration"
                    }
                }
            }),
            json!([{ "googleOidc": auth.requirements.scopes.clone() }]),
        ),
        (
            everruns_platform::AppEndpointAuthMode::Oidc,
            Some(everruns_platform::AppEndpointAuthProviderConfig::Oidc { issuer, .. }),
        ) => {
            let discovery = format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            );
            (
                json!({
                    "oidc": {
                        "openIdConnectSecurityScheme": {
                            "openIdConnectUrl": discovery
                        }
                    }
                }),
                json!([{ "oidc": auth.requirements.scopes.clone() }]),
            )
        }
        // The linked A2A schema models OAuth2 as concrete OpenAPI flows. An
        // introspection-only channel has no token URL to publish, so advertise
        // generic bearer auth rather than fabricating an unusable OAuth flow.
        (everruns_platform::AppEndpointAuthMode::OAuth2Introspection, _) => (
            json!({ "oauth2Bearer": { "httpAuthSecurityScheme": { "scheme": "bearer" } } }),
            json!([{ "oauth2Bearer": auth.requirements.scopes.clone() }]),
        ),
        (everruns_platform::AppEndpointAuthMode::Mtls, _) => (
            json!({ "mtls": { "mtlsSecurityScheme": {} } }),
            json!([{ "mtls": [] }]),
        ),
        (everruns_platform::AppEndpointAuthMode::Anonymous, _) => (json!({}), json!([])),
        _ => (
            json!({ "apiKey": { "httpAuthSecurityScheme": { "scheme": "bearer" } } }),
            json!([{ "apiKey": [] }]),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth2_introspection_security_advertises_bearer_auth() {
        let config = everruns_platform::A2aChannelConfig {
            api_key_hash: "hash".to_string(),
            api_key_prefix: "evra2a_abcd...".to_string(),
            session_mode: everruns_platform::app::SessionBinding::Endpoint,
            message: "{{a2a.text}}".to_string(),
            agent_card_name: None,
            agent_card_description: None,
            rate_limit_per_minute: None,
            auth: Some(everruns_platform::AppEndpointAuthConfig {
                mode: everruns_platform::AppEndpointAuthMode::OAuth2Introspection,
                provider: Some(
                    everruns_platform::AppEndpointAuthProviderConfig::OAuth2Introspection {
                        introspection_url: "https://auth.example.test/introspect".to_string(),
                        client_id: None,
                        client_secret: None,
                        client_secret_configured: false,
                    },
                ),
                requirements: everruns_platform::AppEndpointAuthRequirements {
                    audiences: vec![],
                    scopes: vec!["app:invoke".to_string()],
                    claims: serde_json::Map::new(),
                    subjects: vec![],
                    groups: vec![],
                    domains: vec![],
                },
            }),
            signing_secret: None,
        };

        let (schemes, requirements) = a2a_security_for_config(&config, config.auth.as_ref());

        assert_eq!(
            schemes["oauth2Bearer"]["httpAuthSecurityScheme"]["scheme"],
            "bearer"
        );
        assert_eq!(requirements, json!([{ "oauth2Bearer": ["app:invoke"] }]));
        serde_json::from_value::<std::collections::HashMap<String, a2a::SecurityScheme>>(schemes)
            .expect("securitySchemes should parse as linked A2A security schemes");
    }
}
