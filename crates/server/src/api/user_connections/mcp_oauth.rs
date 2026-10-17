use std::collections::BTreeMap;

use axum::http::StatusCode;
use everruns_core::EgressService;
use everruns_core::mcp::validate_oauth_resource;
use serde::{Deserialize, Serialize};

use super::{AppState, mcp_oauth_redirect_uri, parse_and_validate_url, resource_origin};
use crate::api::common::{sanitized_bad_gateway, sanitized_internal_error};
use crate::domains::mcp_servers::{McpServerOAuthSettings, McpServerSettings};
use crate::kernel_imports::contracts::url_validation::validate_safe_url;
use crate::oauth_client::{egress_oauth_discovery_json, egress_oauth_json};

#[derive(Debug, Deserialize)]
pub(super) struct OAuthProtectedResourceMetadata {
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OAuthServerMetadata {
    issuer: Option<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(super) struct OAuthClientRegistration {
    pub client_id: String,
    client_secret: Option<String>,
}

pub(super) async fn ensure_mcp_oauth_registration(
    state: &AppState,
    row: &crate::storage::McpServerRow,
    settings: McpServerSettings,
    provider: &str,
) -> Result<
    (
        OAuthServerMetadata,
        OAuthClientRegistration,
        McpServerSettings,
    ),
    (StatusCode, String),
> {
    let egress = state.mcp_service.egress_service();
    ensure_mcp_oauth_registration_via(state, egress.as_ref(), row, settings, provider).await
}

/// [`ensure_mcp_oauth_registration`] over an explicit egress, so the catalog
/// connection check can see which host a failed request went to.
pub(super) async fn ensure_mcp_oauth_registration_via(
    state: &AppState,
    egress: &dyn EgressService,
    row: &crate::storage::McpServerRow,
    mut settings: McpServerSettings,
    provider: &str,
) -> Result<
    (
        OAuthServerMetadata,
        OAuthClientRegistration,
        McpServerSettings,
    ),
    (StatusCode, String),
> {
    let oauth = settings
        .oauth
        .get_or_insert_with(McpServerOAuthSettings::default);
    let needs_server_discovery =
        oauth.authorization_endpoint.is_none() || oauth.token_endpoint.is_none();
    // `None` also when the server publishes no protected-resource metadata.
    let resource_metadata = if needs_server_discovery {
        discover_resource_metadata(egress, &row.url).await?
    } else {
        None
    };
    if oauth.resource.is_none()
        && let Some(resource) = resource_metadata
            .as_ref()
            .and_then(|metadata| metadata.resource.clone())
    {
        oauth.resource = Some(resource);
    }
    if let Some(resource) = oauth.resource.as_deref() {
        // THREAT[TM-MCP-008]: validate discovered and preconfigured audiences
        // before persisting them or involving the authorization server.
        validate_oauth_resource(resource, &row.url).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("OAuth protected resource blocked: {e}"),
            )
        })?;
    }
    let metadata = if !needs_server_discovery {
        OAuthServerMetadata {
            issuer: oauth.issuer.clone(),
            authorization_endpoint: oauth.authorization_endpoint.clone().unwrap_or_default(),
            token_endpoint: oauth.token_endpoint.clone().unwrap_or_default(),
            registration_endpoint: oauth.registration_endpoint.clone(),
            scopes_supported: oauth.scopes_supported.clone(),
        }
    } else {
        let issuer = authorization_server_issuer(resource_metadata.as_ref(), &row.url)?;
        validate_safe_url(&issuer).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("OAuth issuer blocked: {e}"),
            )
        })?;
        let metadata = discover_oauth_server_metadata(egress, &issuer).await?;
        oauth.issuer = metadata.issuer.clone().or(Some(issuer));
        oauth.authorization_endpoint = Some(metadata.authorization_endpoint.clone());
        oauth.token_endpoint = Some(metadata.token_endpoint.clone());
        oauth.registration_endpoint = metadata.registration_endpoint.clone();
        oauth.scopes_supported = if !metadata.scopes_supported.is_empty() {
            metadata.scopes_supported.clone()
        } else {
            resource_metadata
                .as_ref()
                .map(|resource| resource.scopes_supported.clone())
                .unwrap_or_default()
        };
        metadata
    };

    let registration = if let Some(client_id) = oauth.client_id.clone() {
        OAuthClientRegistration {
            client_id,
            client_secret: oauth
                .client_secret_encrypted
                .as_deref()
                .map(|value| state.mcp_service.decrypt_string_from_b64(value))
                .transpose()
                .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
        }
    } else {
        let registration_endpoint = metadata.registration_endpoint.as_deref().ok_or((
            StatusCode::BAD_REQUEST,
            "OAuth server metadata missing registration_endpoint; configure client_id manually"
                .to_string(),
        ))?;
        let registration = register_oauth_client(
            egress,
            registration_endpoint,
            &mcp_oauth_redirect_uri(&state.auth_config, provider),
        )
        .await?;
        oauth.client_id = Some(registration.client_id.clone());
        oauth.client_secret_encrypted = registration
            .client_secret
            .as_deref()
            .map(|value| state.mcp_service.encrypt_string_to_b64(value))
            .transpose()
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        registration
    };

    Ok((metadata, registration, settings))
}

/// Protected-resource metadata for the MCP server (RFC 9728), or `None` when
/// it publishes none.
///
/// Candidates, in the MCP authorization spec's order: the path-inserted
/// document for the endpoint (`/.well-known/oauth-protected-resource/mcp`
/// for `/mcp`), then the origin's root document. A 404 moves to the next
/// candidate; any other failure stops discovery. The query is left out: it
/// can carry a key. Everruns sends no unauthenticated MCP request before
/// sign-in, so there is no `WWW-Authenticate: resource_metadata` hint to
/// start from.
pub(super) async fn discover_resource_metadata(
    egress: &dyn EgressService,
    server_url: &str,
) -> Result<Option<OAuthProtectedResourceMetadata>, (StatusCode, String)> {
    let resource_url = parse_and_validate_url(server_url)?;
    let origin = resource_origin(&resource_url)?;
    let path = resource_url.path().trim_end_matches('/');
    let mut candidates = Vec::with_capacity(2);
    if !path.is_empty() {
        candidates.push(format!(
            "{origin}/.well-known/oauth-protected-resource{path}"
        ));
    }
    candidates.push(format!("{origin}/.well-known/oauth-protected-resource"));
    for candidate in candidates {
        if let Some(metadata) = egress_oauth_discovery_json(egress, &candidate).await? {
            return Ok(Some(metadata));
        }
    }
    Ok(None)
}

/// The authorization server to discover: the first one the protected-resource
/// metadata names, otherwise (no metadata, or none listed) the MCP server's
/// own origin, which is where legacy MCP servers host their OAuth.
pub(super) fn authorization_server_issuer(
    resource_metadata: Option<&OAuthProtectedResourceMetadata>,
    server_url: &str,
) -> Result<String, (StatusCode, String)> {
    match resource_metadata.and_then(|metadata| metadata.authorization_servers.first()) {
        Some(issuer) => Ok(issuer.clone()),
        None => resource_origin(&parse_and_validate_url(server_url)?),
    }
}

/// Well-known metadata locations for `issuer`, in the MCP authorization
/// spec's order: RFC 8414 inserts the segment before an issuer path, OpenID
/// Connect Discovery appends it after.
fn authorization_server_metadata_urls(
    issuer: &reqwest::Url,
) -> Result<Vec<String>, (StatusCode, String)> {
    let origin = resource_origin(issuer)?;
    let path = issuer.path().trim_end_matches('/');
    Ok(if path.is_empty() {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ]
    } else {
        vec![
            format!("{origin}/.well-known/oauth-authorization-server{path}"),
            format!("{origin}/.well-known/openid-configuration{path}"),
            format!("{origin}{path}/.well-known/openid-configuration"),
        ]
    })
}

/// Authorization-server metadata for `issuer` (RFC 8414 / OpenID Connect
/// Discovery). A 404 moves to the next well-known location; any other
/// failure stops discovery.
pub(super) async fn discover_oauth_server_metadata(
    egress: &dyn EgressService,
    issuer: &str,
) -> Result<OAuthServerMetadata, (StatusCode, String)> {
    let issuer = parse_and_validate_url(issuer)?;
    let mut discovered = None;
    for candidate in authorization_server_metadata_urls(&issuer)? {
        discovered = egress_oauth_discovery_json::<OAuthServerMetadata>(egress, &candidate).await?;
        if discovered.is_some() {
            break;
        }
    }
    let metadata = discovered.ok_or((
        StatusCode::BAD_GATEWAY,
        "OAuth authorization server publishes no metadata".to_string(),
    ))?;
    if let Some(discovered_issuer) = metadata.issuer.as_deref() {
        let discovered_issuer = parse_and_validate_url(discovered_issuer)?;
        if discovered_issuer.as_str().trim_end_matches('/') != issuer.as_str().trim_end_matches('/')
        {
            return Err((
                StatusCode::BAD_GATEWAY,
                "OAuth authorization server returned mismatched issuer metadata".to_string(),
            ));
        }
    }
    validate_safe_url(&metadata.authorization_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Authorization endpoint blocked: {e}"),
        )
    })?;
    validate_safe_url(&metadata.token_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Token endpoint blocked: {e}"),
        )
    })?;
    if let Some(registration_endpoint) = &metadata.registration_endpoint {
        validate_safe_url(registration_endpoint).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Registration endpoint blocked: {e}"),
            )
        })?;
    }
    Ok(metadata)
}

async fn register_oauth_client(
    egress: &dyn EgressService,
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<OAuthClientRegistration, (StatusCode, String)> {
    validate_safe_url(registration_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Registration endpoint blocked: {e}"),
        )
    })?;
    let body = serde_json::to_vec(&serde_json::json!({
        "client_name": "Everruns MCP",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "client_secret_post"
    }))
    .map_err(|e| sanitized_bad_gateway("OAuth registration body", &e))?;
    egress_oauth_json(
        egress,
        "POST",
        registration_endpoint,
        &[("Content-Type", "application/json".to_string())],
        body,
    )
    .await
}

const RESERVED_OAUTH_AUTHORIZATION_PARAMS: &[&str] = &[
    "client_id",
    "code_challenge",
    "code_challenge_method",
    "redirect_uri",
    "resource",
    "response_type",
    "scope",
    "state",
];

pub(super) fn validate_authorization_params(
    params: &BTreeMap<String, String>,
) -> Result<(), (StatusCode, String)> {
    for key in params.keys() {
        if RESERVED_OAUTH_AUTHORIZATION_PARAMS.contains(&key.as_str()) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("OAuth authorization parameter '{key}' is reserved"),
            ));
        }
    }
    Ok(())
}

pub(super) fn oauth_refusal_message(error: &str, description: Option<&str>) -> String {
    fn sanitized(value: &str) -> String {
        value
            .chars()
            .filter(|character| !character.is_control())
            .take(256)
            .collect()
    }

    let error = sanitized(error);
    match description.map(sanitized).filter(|value| !value.is_empty()) {
        Some(description) => {
            format!("OAuth authorization was refused: {error} ({description})")
        }
        None => format!("OAuth authorization was refused: {error}"),
    }
}
