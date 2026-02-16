// MCP OAuth service for OAuth 2.1 authentication with MCP servers
//
// Implements the MCP Authorization specification (2025-06-18):
// - RFC 9728: Protected Resource Metadata discovery
// - RFC 8414: OAuth Authorization Server Metadata
// - RFC 8707: Resource Indicators
// - PKCE (S256): Required for all flows

use crate::storage::{
    CreateMcpOAuthState, EncryptionService, McpOAuthStateRow, McpServerRow, McpUserTokenRow,
    StorageBackend, UpsertMcpUserToken,
};
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use everruns_core::{
    AuthorizationServerMetadata, McpOAuthStatus, McpServerAuthType, McpUserToken,
    OAuthTokenResponse, ProtectedResourceMetadata,
};
use rand::Rng;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

/// HTTP client timeout for OAuth metadata discovery
const OAUTH_CLIENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// OAuth state expiration (10 minutes)
const OAUTH_STATE_TTL_MINUTES: i64 = 10;

pub struct McpOAuthService {
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    /// External base URL (e.g. http://localhost:9300)
    base_url: String,
    /// API prefix for constructing callback URLs (e.g. "/api")
    api_prefix: String,
}

impl McpOAuthService {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Arc<EncryptionService>,
        base_url: String,
        api_prefix: String,
    ) -> Self {
        Self {
            db,
            encryption,
            base_url,
            api_prefix,
        }
    }

    /// Get OAuth authorization status for a user and MCP server
    pub async fn get_oauth_status(
        &self,
        mcp_server_id: Uuid,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<McpOAuthStatus> {
        let server = self
            .db
            .get_mcp_server(org_id, mcp_server_id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;

        let auth_type = McpServerAuthType::from(server.auth_type.as_str());

        if auth_type != McpServerAuthType::OAuth {
            return Ok(McpOAuthStatus {
                auth_type,
                authorized: true, // Non-OAuth servers don't need user authorization
                expires_at: None,
                scopes: vec![],
                authorization_url: None,
            });
        }

        // Check if user has valid tokens
        let token = self.db.get_mcp_user_token(mcp_server_id, user_id).await?;

        let (authorized, expires_at, scopes) = if let Some(ref t) = token {
            // Check if token is expired
            let is_valid = t.expires_at.is_none() || t.expires_at.unwrap() > Utc::now();
            let scopes: Vec<String> = t
                .scope
                .as_ref()
                .map(|s| s.split_whitespace().map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();
            (is_valid, t.expires_at, scopes)
        } else {
            (false, None, vec![])
        };

        let authorization_url = if !authorized {
            Some(format!(
                "{}{}/v1/mcp-servers/{}/oauth/authorize",
                self.base_url, self.api_prefix, mcp_server_id
            ))
        } else {
            None
        };

        Ok(McpOAuthStatus {
            auth_type,
            authorized,
            expires_at,
            scopes,
            authorization_url,
        })
    }

    /// Start OAuth authorization flow
    /// Returns the authorization URL to redirect the user to
    pub async fn start_authorization(
        &self,
        mcp_server_id: Uuid,
        org_id: i64,
        user_id: Uuid,
        return_url: Option<String>,
    ) -> Result<String> {
        let server = self
            .db
            .get_mcp_server(org_id, mcp_server_id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;

        if McpServerAuthType::from(server.auth_type.as_str()) != McpServerAuthType::OAuth {
            return Err(anyhow!("MCP server does not use OAuth authentication"));
        }

        // Get OAuth endpoints (from config or discover)
        let (auth_url, _token_url) = self.get_oauth_endpoints(&server).await?;

        // Generate PKCE code verifier and challenge
        let code_verifier = generate_code_verifier();
        let code_challenge = generate_code_challenge(&code_verifier);

        // Single stable callback URL (server ID recovered from state parameter)
        let redirect_uri = format!("{}{}/v1/oauth/callback", self.base_url, self.api_prefix);

        // Store state in database
        let state = self
            .db
            .create_mcp_oauth_state(CreateMcpOAuthState {
                mcp_server_id,
                org_id,
                user_id,
                code_verifier,
                redirect_uri: redirect_uri.clone(),
                return_url,
                expires_at: Utc::now() + Duration::minutes(OAUTH_STATE_TTL_MINUTES),
            })
            .await?;

        // Build authorization URL
        let client_id = server
            .oauth_client_id
            .ok_or_else(|| anyhow!("OAuth client_id not configured"))?;

        let scopes: Vec<String> = server
            .oauth_scopes
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()).unwrap_or_default())
            .unwrap_or_default();

        let scope = scopes.join(" ");

        let mut auth_params = vec![
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code".to_string()),
            ("state", state.id.to_string()),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256".to_string()),
        ];

        if !scope.is_empty() {
            auth_params.push(("scope", scope));
        }

        // Add resource indicator (RFC 8707)
        auth_params.push(("resource", server.url.clone()));

        let auth_url_with_params = format!(
            "{}?{}",
            auth_url,
            auth_params
                .iter()
                .map(|(k, v)| {
                    let encoded: String = v
                        .chars()
                        .map(|c| match c {
                            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                                c.to_string()
                            }
                            _ => format!("%{:02X}", c as u8),
                        })
                        .collect();
                    format!("{}={}", k, encoded)
                })
                .collect::<Vec<_>>()
                .join("&")
        );

        Ok(auth_url_with_params)
    }

    /// Handle OAuth callback
    /// Exchanges authorization code for tokens and stores them
    pub async fn handle_callback(
        &self,
        state_id: Uuid,
        code: &str,
    ) -> Result<(Uuid, Option<String>)> {
        // Consume state (also validates expiration)
        let state = self
            .db
            .consume_mcp_oauth_state(state_id)
            .await?
            .ok_or_else(|| anyhow!("Invalid or expired OAuth state"))?;

        // Get server config
        let server = self
            .db
            .get_mcp_server(state.org_id, state.mcp_server_id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;

        // Get OAuth endpoints
        let (_, token_url) = self.get_oauth_endpoints(&server).await?;

        // Get client credentials
        let client_id = server
            .oauth_client_id
            .ok_or_else(|| anyhow!("OAuth client_id not configured"))?;

        let client_secret = if let Some(encrypted) = &server.oauth_client_secret_encrypted {
            Some(self.encryption.decrypt_to_string(encrypted)?)
        } else {
            None
        };

        // Exchange code for tokens
        let token_response = exchange_code_for_tokens(
            &token_url,
            &client_id,
            client_secret.as_deref(),
            code,
            &state,
        )
        .await?;

        // Calculate expiration
        let expires_at = token_response
            .expires_in
            .map(|secs| Utc::now() + Duration::seconds(secs));

        // Encrypt and store tokens
        let access_token_encrypted = self
            .encryption
            .encrypt_string(&token_response.access_token)?;
        let refresh_token_encrypted = if let Some(ref rt) = token_response.refresh_token {
            Some(self.encryption.encrypt_string(rt)?)
        } else {
            None
        };

        self.db
            .upsert_mcp_user_token(UpsertMcpUserToken {
                mcp_server_id: state.mcp_server_id,
                user_id: state.user_id,
                access_token_encrypted,
                refresh_token_encrypted,
                token_type: token_response.token_type,
                scope: token_response.scope,
                expires_at,
            })
            .await?;

        Ok((state.mcp_server_id, state.return_url))
    }

    /// Get decrypted access token for a user and MCP server
    /// Refreshes the token if expired and refresh_token is available
    pub async fn get_access_token(
        &self,
        mcp_server_id: Uuid,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Option<String>> {
        let token = self.db.get_mcp_user_token(mcp_server_id, user_id).await?;

        let Some(token) = token else {
            return Ok(None);
        };

        // Check if token is expired
        let is_expired = token
            .expires_at
            .map(|exp| exp <= Utc::now())
            .unwrap_or(false);

        if is_expired && token.refresh_token_encrypted.is_some() {
            // Try to refresh the token
            match self
                .refresh_token(mcp_server_id, org_id, user_id, &token)
                .await
            {
                Ok(new_token) => {
                    return Ok(Some(new_token));
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to refresh token for MCP server {}: {}",
                        mcp_server_id,
                        e
                    );
                    // Return None to indicate re-authorization is needed
                    return Ok(None);
                }
            }
        }

        if is_expired {
            return Ok(None);
        }

        // Decrypt and return access token
        let access_token = self
            .encryption
            .decrypt_to_string(&token.access_token_encrypted)?;
        Ok(Some(access_token))
    }

    /// Refresh an expired access token
    async fn refresh_token(
        &self,
        mcp_server_id: Uuid,
        org_id: i64,
        user_id: Uuid,
        token: &McpUserTokenRow,
    ) -> Result<String> {
        let server = self
            .db
            .get_mcp_server(org_id, mcp_server_id)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found"))?;

        let (_, token_url) = self.get_oauth_endpoints(&server).await?;

        let client_id = server
            .oauth_client_id
            .ok_or_else(|| anyhow!("OAuth client_id not configured"))?;

        let client_secret = if let Some(encrypted) = &server.oauth_client_secret_encrypted {
            Some(self.encryption.decrypt_to_string(encrypted)?)
        } else {
            None
        };

        let refresh_token = token
            .refresh_token_encrypted
            .as_ref()
            .ok_or_else(|| anyhow!("No refresh token available"))?;
        let refresh_token = self.encryption.decrypt_to_string(refresh_token)?;

        // Refresh the token
        let token_response = refresh_access_token(
            &token_url,
            &client_id,
            client_secret.as_deref(),
            &refresh_token,
            &server.url,
        )
        .await?;

        // Calculate expiration
        let expires_at = token_response
            .expires_in
            .map(|secs| Utc::now() + Duration::seconds(secs));

        // Encrypt and store new tokens
        let access_token_encrypted = self
            .encryption
            .encrypt_string(&token_response.access_token)?;
        let refresh_token_encrypted = if let Some(ref rt) = token_response.refresh_token {
            Some(self.encryption.encrypt_string(rt)?)
        } else {
            // Keep old refresh token if new one wasn't provided
            token.refresh_token_encrypted.clone()
        };

        self.db
            .upsert_mcp_user_token(UpsertMcpUserToken {
                mcp_server_id,
                user_id,
                access_token_encrypted: access_token_encrypted.clone(),
                refresh_token_encrypted,
                token_type: token_response.token_type,
                scope: token_response.scope.or_else(|| token.scope.clone()),
                expires_at,
            })
            .await?;

        // Return the new access token
        Ok(token_response.access_token)
    }

    /// Revoke OAuth authorization for a user
    pub async fn revoke_authorization(&self, mcp_server_id: Uuid, user_id: Uuid) -> Result<bool> {
        self.db.delete_mcp_user_token(mcp_server_id, user_id).await
    }

    /// Get user token info (without decrypting)
    pub async fn get_user_token(
        &self,
        mcp_server_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<McpUserToken>> {
        let token = self.db.get_mcp_user_token(mcp_server_id, user_id).await?;
        Ok(token.map(|t| McpUserToken {
            id: t.id,
            mcp_server_id: t.mcp_server_id,
            user_id: t.user_id,
            token_type: t.token_type,
            scope: t.scope,
            expires_at: t.expires_at,
            created_at: t.created_at,
            updated_at: t.updated_at,
        }))
    }

    /// Get OAuth endpoints from server config or discover via metadata
    async fn get_oauth_endpoints(&self, server: &McpServerRow) -> Result<(String, String)> {
        // First, check if endpoints are explicitly configured
        if let (Some(auth_url), Some(token_url)) =
            (&server.oauth_authorization_url, &server.oauth_token_url)
        {
            return Ok((auth_url.clone(), token_url.clone()));
        }

        // Try to discover via Protected Resource Metadata (RFC 9728)
        if let Some(metadata_url) = &server.oauth_resource_metadata_url {
            let resource_metadata = fetch_protected_resource_metadata(metadata_url).await?;
            if let Some(auth_server_url) = resource_metadata.authorization_servers.first() {
                let as_metadata = fetch_authorization_server_metadata(auth_server_url).await?;
                return Ok((
                    as_metadata.authorization_endpoint,
                    as_metadata.token_endpoint,
                ));
            }
        }

        // Try to discover from MCP server URL
        let base_url = reqwest::Url::parse(&server.url)?;
        let metadata_url = format!(
            "{}://{}/{}",
            base_url.scheme(),
            base_url.host_str().unwrap_or(""),
            ".well-known/oauth-protected-resource"
        );

        match fetch_protected_resource_metadata(&metadata_url).await {
            Ok(resource_metadata) => {
                if let Some(auth_server_url) = resource_metadata.authorization_servers.first() {
                    let as_metadata = fetch_authorization_server_metadata(auth_server_url).await?;
                    return Ok((
                        as_metadata.authorization_endpoint,
                        as_metadata.token_endpoint,
                    ));
                }
            }
            Err(e) => {
                tracing::debug!("Failed to discover OAuth metadata: {}", e);
            }
        }

        Err(anyhow!(
            "OAuth endpoints not configured and could not be discovered"
        ))
    }
}

/// Generate PKCE code verifier (43-128 character random string)
fn generate_code_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Generate PKCE code challenge (S256)
fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

/// Fetch Protected Resource Metadata (RFC 9728)
async fn fetch_protected_resource_metadata(url: &str) -> Result<ProtectedResourceMetadata> {
    let client = reqwest::Client::builder()
        .timeout(OAUTH_CLIENT_TIMEOUT)
        .build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to fetch protected resource metadata: {}",
            response.status()
        ));
    }

    let metadata: ProtectedResourceMetadata = response.json().await?;
    Ok(metadata)
}

/// Fetch Authorization Server Metadata (RFC 8414)
async fn fetch_authorization_server_metadata(issuer: &str) -> Result<AuthorizationServerMetadata> {
    let client = reqwest::Client::builder()
        .timeout(OAUTH_CLIENT_TIMEOUT)
        .build()?;

    // Try RFC 8414 path first
    let metadata_url = format!("{}/.well-known/oauth-authorization-server", issuer);

    let response = client.get(&metadata_url).send().await?;

    if !response.status().is_success() {
        // Try OpenID Connect Discovery as fallback
        let oidc_url = format!("{}/.well-known/openid-configuration", issuer);
        let response = client.get(&oidc_url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch authorization server metadata: {}",
                response.status()
            ));
        }

        return Ok(response.json().await?);
    }

    Ok(response.json().await?)
}

/// Exchange authorization code for tokens
async fn exchange_code_for_tokens(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    state: &McpOAuthStateRow,
) -> Result<OAuthTokenResponse> {
    let client = reqwest::Client::builder()
        .timeout(OAUTH_CLIENT_TIMEOUT)
        .build()?;

    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("redirect_uri", &state.redirect_uri),
        ("code_verifier", &state.code_verifier),
    ];

    // Note: We don't add resource indicator here because some providers don't support it
    // in token requests (they infer it from the authorization request)

    if let Some(secret) = client_secret {
        params.push(("client_secret", secret));
    }

    let response = client
        .post(token_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(anyhow!("Token exchange failed: {}", error_text));
    }

    let token_response: OAuthTokenResponse = response.json().await?;
    Ok(token_response)
}

/// Refresh access token
async fn refresh_access_token(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    resource: &str,
) -> Result<OAuthTokenResponse> {
    let client = reqwest::Client::builder()
        .timeout(OAUTH_CLIENT_TIMEOUT)
        .build()?;

    let mut params = vec![
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token),
        ("resource", resource),
    ];

    if let Some(secret) = client_secret {
        params.push(("client_secret", secret));
    }

    let response = client
        .post(token_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(anyhow!("Token refresh failed: {}", error_text));
    }

    let token_response: OAuthTokenResponse = response.json().await?;
    Ok(token_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_code_verifier() {
        let verifier = generate_code_verifier();
        // Should be 43 characters (32 bytes base64 encoded)
        assert_eq!(verifier.len(), 43);
        // Should be URL-safe characters only
        assert!(
            verifier
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn test_generate_code_challenge() {
        let verifier = "test_verifier_string";
        let challenge = generate_code_challenge(verifier);
        // Should be 43 characters (SHA256 base64 encoded)
        assert_eq!(challenge.len(), 43);
        // Should be URL-safe characters only
        assert!(
            challenge
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn test_code_challenge_deterministic() {
        let verifier = "test_verifier";
        let challenge1 = generate_code_challenge(verifier);
        let challenge2 = generate_code_challenge(verifier);
        assert_eq!(challenge1, challenge2);
    }

    #[test]
    fn test_code_challenge_different_for_different_verifiers() {
        let challenge1 = generate_code_challenge("verifier1");
        let challenge2 = generate_code_challenge("verifier2");
        assert_ne!(challenge1, challenge2);
    }
}
