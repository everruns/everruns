//! Complete a browser-authorized MCP grant using its captured binding.

use super::*;

pub(super) async fn complete_mcp_oauth_callback(
    state: &AppState,
    org: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
    server_id: uuid::Uuid,
    provider: &str,
    pending: &PendingOAuthState,
    query: &OAuthCallbackQuery,
) -> Result<(), (StatusCode, String)> {
    consume_pending_setup(state, pending).await?;
    let authority = callback_authority(state, pending, org).await?;
    if let Some(error) = query.error.as_deref() {
        return Err((
            StatusCode::BAD_REQUEST,
            oauth_refusal_message(error, query.error_description.as_deref()),
        ));
    }
    let code = query.code.as_deref().ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth callback is missing the authorization code".to_string(),
    ))?;

    let owned = state
        .db
        .get_mcp_server_with_owner(authority.org_id, server_id)
        .await
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "MCP server not found".to_string()))?;
    // Re-check ownership against the subject the grant will be written for;
    // the state cookie is browser-bound but not signed.
    if let Some(owner) = owned.owner_virtual_user_id {
        let subject = pending
            .virtual_user_id
            .as_deref()
            .and_then(|id| id.parse::<VirtualUserId>().ok())
            .map(|id| id.uuid());
        if pending.mode == "identity" || subject != Some(owner) {
            return Err((StatusCode::NOT_FOUND, "MCP server not found".to_string()));
        }
    }
    let catalog_preset = owned.owner_virtual_user_id.is_none();
    let row = owned.row;
    if row.status != "active" {
        return Err((
            StatusCode::BAD_REQUEST,
            "MCP server is not active".to_string(),
        ));
    }
    let settings = McpServerService::settings_from_row(&row);
    let oauth = settings.oauth.clone().ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth metadata missing for MCP server".to_string(),
    ))?;
    let config_hash = mcp_oauth_config_hash(&row.url, &oauth);
    if pending.mcp_config_hash.as_deref() != Some(config_hash.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "MCP OAuth settings changed; sign in again".into(),
        ));
    }
    let resource = oauth.resource.clone().ok_or((
        StatusCode::BAD_REQUEST,
        "MCP OAuth resource binding is missing; sign in again".into(),
    ))?;
    let client_id = oauth.client_id.ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth client_id missing".to_string(),
    ))?;
    let client_secret = oauth
        .client_secret_encrypted
        .as_deref()
        .map(|v| state.mcp_service.decrypt_string_from_b64(v))
        .transpose()
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
    let token_endpoint = oauth.token_endpoint.ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth token endpoint missing".to_string(),
    ))?;
    let redirect_uri = mcp_oauth_redirect_uri(&state.auth_config, provider);

    let egress = org_scoped_egress(state.mcp_service.egress_service(), authority.org_id);
    let token = exchange_oauth_code(
        egress.as_ref(),
        OAuthCodeExchangeRequest {
            token_endpoint: &token_endpoint,
            client_id: &client_id,
            client_secret: client_secret.as_deref(),
            redirect_uri: &redirect_uri,
            code,
            code_verifier: &pending.code_verifier,
            resource: oauth.resource.as_deref(),
        },
    )
    .await?;

    let refresh_grant = token.refresh_token.as_ref().map(|refresh_token| {
        McpRefreshGrant {
            refresh_token: refresh_token.clone(),
            resource,
            config_hash,
        }
        .encode()
    });

    let expires_at = token
        .expires_in
        .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds));

    if pending.mode == "session" {
        let session_id = pending
            .session_id
            .as_deref()
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing session_id for session OAuth flow".to_string(),
            ))?
            .parse::<SessionId>()
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid session_id: {e}")))?;
        state
            .db
            .get_session(authority.org_id, session_id)
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
            .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;
        // Verify caller identity: ensure the authenticated user initiated this session's OAuth
        // flow. The session is already org-scoped via get_session, but we also verify the
        // OAuth state cookie was set in *this* browser (validated above via
        // validate_pending_oauth_state). This prevents cross-user token injection since the
        // state cookie + PKCE verifier are browser-bound.
        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        ))?;
        state
            .db
            .upsert_mcp_oauth_session_credentials(UpsertMcpOAuthSessionCredentials {
                virtual_user_id: pending
                    .virtual_user_id
                    .as_deref()
                    .and_then(|id| id.parse::<VirtualUserId>().ok()),
                session_id,
                server_id,
                access_token_encrypted: encryption
                    .encrypt_string(&token.access_token)
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                refresh_token_encrypted: refresh_grant
                    .as_deref()
                    .map(|value| encryption.encrypt_string(value))
                    .transpose()
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                expires_at_encrypted: expires_at
                    .map(|value| encryption.encrypt_string(&value.to_rfc3339()))
                    .transpose()
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
            })
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
    } else if pending.mode == "identity" {
        let agent_id = pending
            .agent_id
            .as_deref()
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing agent_id for identity OAuth flow".to_string(),
            ))?
            .parse::<AgentId>()
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid agent_id: {e}")))?;
        let identity_id = pending
            .virtual_user_id
            .as_deref()
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing virtual_user_id for identity OAuth flow".to_string(),
            ))?
            .parse::<VirtualUserId>()
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid virtual_user_id: {e}"),
                )
            })?;

        // Re-check both the permission and org ownership here rather than
        // trusting the cookie. The state cookie is browser-bound but not
        // signed, so a planted one must not be able to aim a grant at another
        // tenant's identity or clear a gate the authorizing user never passed.
        let caller = authority.management_caller()?;
        enforce_identity_grant_policy(state, caller)?;

        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        ))?;
        let connection = state
            .db
            .upsert_virtual_user_connection_for_active_agent(
                authority.org_id,
                agent_id,
                CreateVirtualUserConnectionRow {
                    virtual_user_id: identity_id,
                    provider: provider.to_string(),
                    connection_type: "oauth".to_string(),
                    provider_user_id: None,
                    provider_username: Some(row.name.clone()),
                    access_token_encrypted: Some(
                        encryption
                            .encrypt_string(&token.access_token)
                            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                    ),
                    refresh_token_encrypted: refresh_grant
                        .as_deref()
                        .map(|value| encryption.encrypt_string(value))
                        .transpose()
                        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                    scopes: token.scope.clone(),
                    expires_at,
                    installation_id: None,
                    provider_metadata: None,
                },
            )
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        if connection.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Agent is no longer active with the authorized identity".to_string(),
            ));
        }
    } else {
        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        ))?;
        let subject = pending
            .virtual_user_id
            .as_deref()
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing runtime subject".to_string(),
            ))?
            .parse::<VirtualUserId>()
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid runtime subject".to_string(),
                )
            })?
            .uuid();
        state
            .db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id: subject,
                provider: provider.to_string(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: Some(row.name.clone()),
                access_token_encrypted: Some(
                    encryption
                        .encrypt_string(&token.access_token)
                        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                ),
                refresh_token_encrypted: refresh_grant
                    .as_deref()
                    .map(|value| encryption.encrypt_string(value))
                    .transpose()
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                scopes: token.scope.clone(),
                expires_at,
                installation_id: None,
                provider_metadata: None,
            })
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        if catalog_preset {
            list_signed_in_catalog_server(state, authority.org_id, subject, server_id).await;
        }
    }
    Ok(())
}
