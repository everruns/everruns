// The A2A Agent Card: unauthenticated discovery for an A2A endpoint.
//
// Split out of `channel_a2a.rs` because none of it touches the JSON-RPC request
// path: the card is built from the endpoint's stored config and the request's
// own URI, and its only contract with the rest of the channel is the security
// scheme it advertises for the auth policy that channel actually enforces.
//
// Design Decision: one card serves A2A 1.0 and 0.3 clients, the union shape
// a2a-go's `a2acompat/a2av0` producer publishes. 1.0 clients read
// `supportedInterfaces` (one JSONRPC interface per protocol version, same URL),
// `securityRequirements` and the wrapped `securitySchemes`; 0.3 clients read
// the top-level `url` / `protocolVersion` / `preferredTransport`, `security`,
// and the flat OpenAPI `type` fields of the same schemes. Each side ignores the
// other's fields.
// See `knowledge/integrations/a2a-channel.md`.

use axum::{
    Json,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::{Value, json};

use super::{
    A2A_AGENT_VERSION, A2A_PROTOCOL_BINDING_JSONRPC, ChannelA2aState, channel_app_id,
    internal_error, not_found,
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
    State(state): State<ChannelA2aState>,
    OriginalUri(original_uri): OriginalUri,
    Path((app_id, channel_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    agent_card(state, original_uri, app_id, channel_id, headers).await
}

#[utoipa::path(
    description = "Get the public Agent Card for a published A2A endpoint.",
    get,
    path = "/v1/channels/{channel_id}/a2a/.well-known/agent-card.json",
    params(("channel_id" = String, Path, description = "A2A endpoint channel ID")),
    responses(
        (status = 200, description = "Agent Card JSON"),
        (status = 404, description = "Endpoint not found, app not published, or channel disabled", body = ErrorResponse)
    ),
    tag = "apps"
)]
pub async fn agent_card_channel(
    State(state): State<ChannelA2aState>,
    OriginalUri(original_uri): OriginalUri,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = channel_app_id(&state, &channel_id).await?;
    agent_card(state, original_uri, app_id, channel_id, headers).await
}

async fn agent_card(
    state: ChannelA2aState,
    original_uri: axum::http::Uri,
    app_id: String,
    channel_id: String,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<ErrorResponse>)> {
    let (app, channel) = crate::api::channel_ingress::resolve_channel(
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
    if channel.channel_type != crate::records::ChannelType::A2a {
        return Err(not_found());
    }
    // The Agent Card is only served for a live endpoint: it advertises the
    // invocation URL and security scheme, so publishing it for a draft or
    // suspended endpoint would leak a surface that refuses traffic.
    if crate::api::channel_ingress::channel_liveness(&app, &channel).is_err() {
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

    let (security_schemes, requirements) =
        a2a_security_for_config(&config, channel.auth.as_deref());
    // Streaming is only supported on session_per_invocation channels.
    // Shared-session channels reject it because events cannot be safely
    // correlated across concurrent callers.
    let streaming = config.session_mode == crate::records::agent_channel::SessionBinding::Ephemeral;
    let interfaces: Vec<Value> = super::wire::SUPPORTED_VERSIONS
        .iter()
        .map(|version| {
            json!({
                "url": endpoint,
                "protocolBinding": A2A_PROTOCOL_BINDING_JSONRPC,
                "protocolVersion": version,
            })
        })
        .collect();
    let card = json!({
        "name": name,
        "description": description,
        "version": A2A_AGENT_VERSION,
        "supportedInterfaces": interfaces,
        "capabilities": {
            "streaming": streaming,
            "pushNotifications": true,
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
        "securityRequirements": v1_security_requirements(&requirements),
        // A2A 0.3 fields, for clients that predate `supportedInterfaces`.
        "url": endpoint,
        "protocolVersion": "0.3.0",
        "preferredTransport": A2A_PROTOCOL_BINDING_JSONRPC,
        "security": requirements,
    });
    Ok(Json(card))
}

/// 0.3 / OpenAPI requirements (`[{"scheme": ["scope"]}]`) in the 1.0 shape
/// (`[{"schemes": {"scheme": ["scope"]}}]`).
///
/// Design Decision: scopes stay a bare array rather than ProtoJSON's
/// `{"list": [...]}` wrapper. Every 1.0 parser we tested accepts the array
/// (a2a-python, the a2a-lf Rust SDK, a2a-go >= 2.6), while a2a-go 2.5 (the
/// official `a2a` CLI) rejects the wrapper and the Rust SDK rejects the empty
/// `{}` form outright.
fn v1_security_requirements(requirements: &Value) -> Value {
    let Some(requirements) = requirements.as_array() else {
        return json!([]);
    };
    Value::Array(
        requirements
            .iter()
            .filter_map(Value::as_object)
            .map(|requirement| json!({ "schemes": requirement }))
            .collect(),
    )
}

/// One security scheme in both shapes: the 1.0 wrapper object and the 0.3
/// flat OpenAPI fields.
fn union_scheme(v1_wrapper: &str, v1: Value, v0_3: Value) -> Value {
    let mut scheme = v0_3;
    if let Some(obj) = scheme.as_object_mut() {
        obj.insert(v1_wrapper.to_string(), v1);
    }
    scheme
}

fn http_scheme(scheme: &str) -> Value {
    union_scheme(
        "httpAuthSecurityScheme",
        json!({ "scheme": scheme }),
        json!({ "type": "http", "scheme": scheme }),
    )
}

fn oidc_scheme(url: &str) -> Value {
    union_scheme(
        "openIdConnectSecurityScheme",
        json!({ "openIdConnectUrl": url }),
        json!({ "type": "openIdConnect", "openIdConnectUrl": url }),
    )
}

const HMAC_DESCRIPTION: &str =
    "HMAC-SHA256 over v0:{timestamp}:{channel_scope}:{body}; pair with X-Everruns-A2A-Timestamp";

/// The channel's security schemes and its requirements in the 0.3 / OpenAPI
/// shape; [`v1_security_requirements`] derives the 1.0 shape from them.
fn a2a_security_for_config(
    config: &crate::records::A2aChannelConfig,
    auth: Option<&crate::records::ChannelAuthConfig>,
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
                union_scheme(
                    "apiKeySecurityScheme",
                    json!({
                        "location": "header",
                        "name": A2A_SIGNATURE_HEADER,
                        "description": HMAC_DESCRIPTION,
                    }),
                    json!({
                        "type": "apiKey",
                        "in": "header",
                        "name": A2A_SIGNATURE_HEADER,
                        "description": HMAC_DESCRIPTION,
                    }),
                ),
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

fn base_a2a_security(auth: Option<&crate::records::ChannelAuthConfig>) -> (Value, Value) {
    let Some(auth) = auth else {
        return (
            json!({ "apiKey": http_scheme("bearer") }),
            json!([{ "apiKey": [] }]),
        );
    };
    match (&auth.mode, auth.provider.as_ref()) {
        (crate::records::ChannelAuthMode::HttpBasic, _) => (
            json!({ "httpBasic": http_scheme("basic") }),
            json!([{ "httpBasic": [] }]),
        ),
        (
            crate::records::ChannelAuthMode::GoogleOidc,
            Some(crate::records::ChannelAuthProviderConfig::GoogleOidc { .. }),
        ) => (
            json!({
                "googleOidc": oidc_scheme("https://accounts.google.com/.well-known/openid-configuration")
            }),
            json!([{ "googleOidc": auth.requirements.scopes.clone() }]),
        ),
        (
            crate::records::ChannelAuthMode::Oidc,
            Some(crate::records::ChannelAuthProviderConfig::Oidc { issuer, .. }),
        ) => {
            let discovery = format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            );
            (
                json!({ "oidc": oidc_scheme(&discovery) }),
                json!([{ "oidc": auth.requirements.scopes.clone() }]),
            )
        }
        // The linked A2A schema models OAuth2 as concrete OpenAPI flows. An
        // introspection-only channel has no token URL to publish, so advertise
        // generic bearer auth rather than fabricating an unusable OAuth flow.
        (crate::records::ChannelAuthMode::OAuth2Introspection, _) => (
            json!({ "oauth2Bearer": http_scheme("bearer") }),
            json!([{ "oauth2Bearer": auth.requirements.scopes.clone() }]),
        ),
        (crate::records::ChannelAuthMode::Mtls, _) => (
            json!({
                "mtls": union_scheme("mtlsSecurityScheme", json!({}), json!({ "type": "mutualTLS" }))
            }),
            json!([{ "mtls": [] }]),
        ),
        (crate::records::ChannelAuthMode::Anonymous, _) => (json!({}), json!([])),
        _ => (
            json!({ "apiKey": http_scheme("bearer") }),
            json!([{ "apiKey": [] }]),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth2_introspection_security_advertises_bearer_auth() {
        let config = crate::records::A2aChannelConfig {
            api_key_hash: "hash".to_string(),
            api_key_prefix: "evra2a_abcd...".to_string(),
            session_mode: crate::records::agent_channel::SessionBinding::Shared,
            message: "{{a2a.text}}".to_string(),
            agent_card_name: None,
            agent_card_description: None,
            rate_limit_per_minute: None,
            auth: Some(crate::records::ChannelAuthConfig {
                mode: crate::records::ChannelAuthMode::OAuth2Introspection,
                provider: Some(
                    crate::records::ChannelAuthProviderConfig::OAuth2Introspection {
                        introspection_url: "https://auth.example.test/introspect".to_string(),
                        client_id: None,
                        client_secret: None,
                        client_secret_configured: false,
                    },
                ),
                requirements: crate::records::ChannelAuthRequirements {
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

    #[test]
    fn security_requirements_render_in_both_versions() {
        let requirements = json!([{ "apiKey": [] }, { "oidc": ["read", "write"] }]);
        assert_eq!(
            v1_security_requirements(&requirements),
            json!([
                { "schemes": { "apiKey": [] } },
                { "schemes": { "oidc": ["read", "write"] } }
            ])
        );
    }

    #[test]
    fn union_schemes_parse_as_1_0_schemes_and_keep_0_3_fields() {
        let schemes = json!({
            "apiKey": http_scheme("bearer"),
            "oidc": oidc_scheme("https://issuer.test/.well-known/openid-configuration"),
        });
        assert_eq!(schemes["apiKey"]["type"], "http");
        assert_eq!(schemes["apiKey"]["scheme"], "bearer");
        assert_eq!(schemes["oidc"]["type"], "openIdConnect");
        serde_json::from_value::<std::collections::HashMap<String, a2a::SecurityScheme>>(schemes)
            .expect("union schemes should parse as A2A 1.0 security schemes");
    }
}
