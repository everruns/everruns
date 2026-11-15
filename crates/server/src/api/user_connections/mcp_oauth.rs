use axum::http::StatusCode;
use everruns_core::EgressService;
use everruns_core::mcp::validate_oauth_resource;

use super::{AppState, mcp_oauth_redirect_uri};
use crate::api::common::sanitized_internal_error;
use crate::domains::mcp_servers::{McpServerOAuthSettings, McpServerSettings};
use crate::kernel_imports::contracts::url_validation::validate_safe_url;
use crate::oauth_client::org_scoped_egress;

use crate::oauth_client::{ServerOAuthEgress, oauth_error};
use everruns_core::mcp::oauth::{
    AuthorizationServerMetadata as OAuthServerMetadata, OAuthClient,
    ProtectedResourceMetadata as OAuthProtectedResourceMetadata,
    RegisteredClient as OAuthClientRegistration,
};

pub(super) async fn ensure_mcp_oauth_registration(
    state: &AppState,
    org_id: i64,
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
    let egress = org_scoped_egress(state.mcp_service.egress_service(), org_id);
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
    let resource = oauth
        .resource
        .clone()
        .or_else(|| {
            resource_metadata
                .as_ref()
                .and_then(|metadata| metadata.resource.clone())
        })
        .unwrap_or_else(|| row.url.clone());
    // THREAT[TM-MCP-008]: metadata absence must not remove audience binding.
    validate_oauth_resource(&resource, &row.url).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("OAuth protected resource blocked: {e}"),
        )
    })?;
    oauth.resource = Some(resource);
    let metadata = if !needs_server_discovery {
        OAuthServerMetadata {
            issuer: oauth.issuer.clone().unwrap_or_default(),
            authorization_endpoint: oauth.authorization_endpoint.clone().unwrap_or_default(),
            token_endpoint: oauth.token_endpoint.clone().unwrap_or_default(),
            registration_endpoint: oauth.registration_endpoint.clone(),
            scopes_supported: oauth.scopes_supported.clone(),
            client_id_metadata_document_supported: false,
            code_challenge_methods_supported: Vec::new(),
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
        oauth.issuer = Some(metadata.issuer.clone());
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
    let guarded = ServerOAuthEgress(egress);
    OAuthClient::new(&guarded, everruns_core::EgressRequestKind::Mcp)
        .discover_protected_resource(server_url)
        .await
        .map_err(oauth_error)
}

/// The authorization server to discover: the first one the protected-resource
/// metadata names, otherwise (no metadata, or none listed) the MCP server's
/// own origin, which is where legacy MCP servers host their OAuth.
pub(super) fn authorization_server_issuer(
    resource_metadata: Option<&OAuthProtectedResourceMetadata>,
    server_url: &str,
) -> Result<String, (StatusCode, String)> {
    everruns_core::mcp::oauth::protocol::authorization_server_issuer(resource_metadata, server_url)
        .map_err(oauth_error)
}

/// Discover with the same fallback order and issuer checks used by Framework login.
pub(super) async fn discover_oauth_server_metadata(
    egress: &dyn EgressService,
    issuer: &str,
) -> Result<OAuthServerMetadata, (StatusCode, String)> {
    let guarded = ServerOAuthEgress(egress);
    let metadata = OAuthClient::new(&guarded, everruns_core::EgressRequestKind::Mcp)
        .discover_authorization_server(issuer)
        .await
        .map_err(oauth_error)?;
    // Hosted metadata may not redirect a browser or later grant to an internal host.
    // Local hosts retain their own policy for loopback test/native integrations.
    for endpoint in [
        Some(&metadata.authorization_endpoint),
        Some(&metadata.token_endpoint),
        metadata.registration_endpoint.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_safe_url(endpoint)
            .map_err(|_| (StatusCode::BAD_REQUEST, "OAuth endpoint blocked".into()))?;
    }
    Ok(metadata)
}

async fn register_oauth_client(
    egress: &dyn EgressService,
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<OAuthClientRegistration, (StatusCode, String)> {
    use everruns_core::mcp::oauth::protocol::{ClientAuthMethod, ClientRegistration};
    let guarded = ServerOAuthEgress(egress);
    OAuthClient::new(&guarded, everruns_core::EgressRequestKind::Mcp)
        .register_client_with_auth_method(
            registration_endpoint,
            &ClientRegistration {
                client_name: "Everruns MCP".into(),
                redirect_uris: vec![redirect_uri.into()],
                scope: None,
            },
            ClientAuthMethod::ClientSecretPost,
        )
        .await
        .map_err(oauth_error)
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
