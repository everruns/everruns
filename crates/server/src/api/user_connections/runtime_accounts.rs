use super::*;

pub(super) fn form_schema_to_response(schema: &CoreFormSchema) -> FormSchemaResponse {
    FormSchemaResponse {
        instructions_markdown: schema.instructions_markdown.clone(),
        fields: schema
            .fields
            .iter()
            .map(|f| {
                let field_type = match f.field_type {
                    everruns_contracts::connector::FieldType::Password => "password",
                    everruns_contracts::connector::FieldType::Text => "text",
                    everruns_contracts::connector::FieldType::Url => "url",
                };
                FormFieldResponse {
                    name: f.name.clone(),
                    label: f.label.clone(),
                    field_type: field_type.to_string(),
                    required: f.required,
                    placeholder: f.placeholder.clone(),
                    help_text: f.help_text.clone(),
                }
            })
            .collect(),
    }
}

pub(super) fn oauth_state_cookie_name(provider: &str) -> String {
    format!("oauth_connection_state_{}", provider.replace(':', "_"))
}

pub(super) fn normalize_return_to(value: Option<&str>, default_path: &str) -> String {
    match value {
        Some(path) if is_safe_return_to(path) => path.to_string(),
        _ => default_path.to_string(),
    }
}

pub(super) fn mcp_oauth_redirect_uri(config: &AuthConfig, provider: &str) -> String {
    // base_url already includes any API prefix (set by AUTH_BASE_URL / BASE_URL env)
    format!(
        "{}/v1/user/connections/{provider}/callback",
        config.base_url.trim_end_matches('/')
    )
}

pub(super) fn parse_mcp_oauth_provider_id(provider: &str) -> Option<uuid::Uuid> {
    provider
        .strip_prefix("mcp_oauth_")
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
}

pub(super) fn generate_pkce_verifier() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub(super) async fn register_pending_setup(
    state: &AppState,
    pending: &PendingOAuthState,
) -> Result<(), (StatusCode, String)> {
    let hash = Sha256::digest(
        serde_json::to_vec(pending).map_err(|e| sanitized_internal_error("Setup state", &e))?,
    );
    state
        .db
        .register_connection_setup(&pending.state, &pending.provider, &hash)
        .await
        .map_err(|e| sanitized_internal_error("Setup state", &e))
}
// THREAT[TM-TOOL-041]: Capture the complete target and authority before redirect;
// hash binding, server expiry, and atomic consume prevent retargeting and replay.
pub(super) async fn consume_pending_setup(
    state: &AppState,
    pending: &PendingOAuthState,
) -> Result<(), (StatusCode, String)> {
    let hash = Sha256::digest(
        serde_json::to_vec(pending).map_err(|e| sanitized_internal_error("Setup state", &e))?,
    );
    if !state
        .db
        .consume_connection_setup(&pending.state, &pending.provider, &hash)
        .await
        .map_err(|e| sanitized_internal_error("Setup state", &e))?
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Setup expired, changed, or already completed; restart setup".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_pending_oauth_state(
    jar: &CookieJar,
    provider: &str,
    query_state: Option<&str>,
) -> Result<PendingOAuthState, (StatusCode, String)> {
    let cookie_name = oauth_state_cookie_name(provider);
    let cookie = jar.get(&cookie_name).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid or expired OAuth state".to_string(),
        )
    })?;
    let decoded = URL_SAFE_NO_PAD
        .decode(cookie.value())
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let pending: PendingOAuthState = serde_json::from_slice(&decoded)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let callback_state = query_state.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing state parameter".to_string(),
        )
    })?;
    if pending.state != callback_state {
        return Err((StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()));
    }
    Ok(pending)
}

pub(super) fn finalize_oauth_redirect(
    auth_config: &AuthConfig,
    return_to: &str,
    provider: &str,
    popup: bool,
) -> String {
    let frontend = auth_config.frontend_url.trim_end_matches('/');
    let encoded_provider = urlencoding::encode(provider);
    if popup {
        format!(
            "{}/connection-complete?provider={}&status=success&return_to={}",
            frontend,
            encoded_provider,
            urlencoding::encode(return_to),
        )
    } else if return_to.contains('?') {
        format!("{frontend}{return_to}&connected={encoded_provider}")
    } else {
        format!("{frontend}{return_to}?connected={encoded_provider}")
    }
}

/// Gate for authorizing a grant owned by an virtual user.
///
/// Requires the organization MCP-server management permission. Connecting your
/// own account stays ungated because it spends only your own access (EVE-1030).
pub(super) fn enforce_identity_grant_policy(
    state: &AppState,
    caller: &Caller,
) -> Result<(), (StatusCode, String)> {
    let resolver = state.auth.permission_resolver.as_ref();
    MCP_SERVER_MANAGE
        .evaluate_with(resolver, caller)
        .map_err(|_| {
            (
            StatusCode::FORBIDDEN,
            "Permission denied: authorizing an agent service grant requires MCP server management"
                .to_string(),
        )
        })?;
    Ok(())
}

pub(super) fn normalize_oauth_mode(mode: Option<&str>) -> Result<String, (StatusCode, String)> {
    match mode.unwrap_or("user") {
        "user" => Ok("user".to_string()),
        "session" => Ok("session".to_string()),
        // A grant owned by the agent itself, shared by every session and every
        // invoking user (EVE-1030).
        "identity" => Ok("identity".to_string()),
        "virtual_user" => Ok("virtual_user".to_string()),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid OAuth mode: {other}"),
        )),
    }
}

pub(super) fn resource_origin(url: &Url) -> Result<String, (StatusCode, String)> {
    let host = url.host_str().ok_or((
        StatusCode::BAD_REQUEST,
        "Invalid MCP server URL: missing host".to_string(),
    ))?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let mut origin = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Ok(origin)
}

pub(super) fn parse_and_validate_url(url: &str) -> Result<Url, (StatusCode, String)> {
    validate_safe_url(url).map_err(|e| (StatusCode::BAD_REQUEST, format!("Blocked URL: {e}")))?;
    Url::parse(url).map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid URL: {e}")))
}

#[utoipa::path(summary = "Start provider setup bound to the authorized virtual user.", get, path = "/v1/virtual-users/{identity_id}/connections/{provider}/authorize", params(("identity_id" = String, Path),("provider" = String, Path)),  responses((status = 303, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub(crate) async fn authorize_target_connection(
    State(state): State<AppState>,
    account: crate::auth::runtime::RuntimeAccount,
    jar: CookieJar,
    Path((id, provider)): Path<(String, String)>,
    Query(query): Query<OAuthAuthorizeQuery>,
    headers: axum::http::HeaderMap,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    // Browser navigation: a failure goes back to `return_to` when it is safe.
    let auth_config = state.auth_config.clone();
    let return_to = query.return_to.clone();
    let popup = query.popup.unwrap_or(false);
    let result =
        start_target_connection_inner(state, account, jar, id, provider.clone(), query, headers)
            .await;
    match redirect_on_connect_error(
        &auth_config,
        result,
        return_to.as_deref(),
        &provider,
        popup,
        None,
    )? {
        Ok(redirect) => Ok(redirect),
        Err(target) => Ok((CookieJar::new(), Redirect::to(&target))),
    }
}

async fn start_target_connection_inner(
    state: AppState,
    account: crate::auth::runtime::RuntimeAccount,
    jar: CookieJar,
    id: String,
    provider: String,
    mut query: OAuthAuthorizeQuery,
    headers: axum::http::HeaderMap,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    if !account
        .permits_provider(&state.db, &provider)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Provider availability unavailable".into(),
            )
        })?
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Provider not attached to this endpoint".into(),
        ));
    }
    let target = account
        .connection_target(&state.db, state.auth.permission_resolver.as_ref(), &id)
        .await
        .map_err(|_| {
            (
                StatusCode::FORBIDDEN,
                "Connection setup not authorized".into(),
            )
        })?;
    let credential = if account.management.is_none() {
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or((
                StatusCode::UNAUTHORIZED,
                "Runtime credential required".into(),
            ))?;
        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption unavailable".into(),
        ))?;
        Some(
            URL_SAFE_NO_PAD.encode(
                encryption
                    .encrypt_string(token)
                    .map_err(|e| sanitized_internal_error("Connection setup", &e))?,
            ),
        )
    } else {
        None
    };
    let authority = OAuthAuthority {
        org_id: account.org_id,
        caller: account.management.as_ref().map(Caller::from),
        target_id: target.uuid(),
        management_user_id: account.management.as_ref().and_then(|o| o.user_id),
        runtime_credential: credential,
    };
    query.mode = Some("virtual_user".into());
    if provider == "github" {
        return github_authorize_inner(state, authority, jar, query.return_to).await;
    }
    authorize_connection_inner(state, authority, jar, provider, query).await
}
/// Browser URL returned to a bearer-authenticated consumer before navigation.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectionSetupResponse {
    /// Navigate the browser to this provider authorization URL.
    #[schema(example = "https://provider.example/authorize?state=opaque")]
    pub authorization_url: String,
}
#[utoipa::path(post,path="/v1/virtual-users/{identity_id}/connections/{provider}/authorize", summary="Create a target-bound provider setup URL.",params(("identity_id"=String,Path),("provider"=String,Path)), request_body=OAuthAuthorizeQuery,responses((status=200,description="Browser setup created",body=ConnectionSetupResponse),(status=403,description="Setup not authorized")),tag="virtual-users")]
pub(super) async fn start_target_connection(
    state: State<AppState>,
    account: crate::auth::runtime::RuntimeAccount,
    jar: CookieJar,
    path: Path<(String, String)>,
    headers: axum::http::HeaderMap,
    Json(body): Json<OAuthAuthorizeQuery>,
) -> Result<(CookieJar, Json<ConnectionSetupResponse>), (StatusCode, String)> {
    // JSON variant: errors stay JSON-shaped status responses (the caller has
    // no page to return to); a blocked host carries a message naming the policy.
    let (State(state), Path((id, provider))) = (state, path);
    let (jar, redirect) =
        start_target_connection_inner(state, account, jar, id, provider, body, headers).await?;
    let response = redirect.into_response();
    let authorization_url = response
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Setup URL unavailable".into(),
        ))?
        .to_string();
    Ok((jar, Json(ConnectionSetupResponse { authorization_url })))
}

pub(super) async fn callback_authority(
    state: &AppState,
    pending: &PendingOAuthState,
    org: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
) -> Result<OAuthAuthority, (StatusCode, String)> {
    let target = pending
        .virtual_user_id
        .as_deref()
        .and_then(|id| id.parse::<VirtualUserId>().ok())
        .ok_or((StatusCode::BAD_REQUEST, "Missing runtime subject".into()))?;
    if let Some(credential) = &pending.runtime_credential {
        let bytes = URL_SAFE_NO_PAD
            .decode(credential)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid setup credential".into()))?;
        let token = state
            .encryption
            .as_ref()
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Encryption unavailable".into(),
            ))?
            .decrypt_to_string(&bytes)
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid setup credential".into()))?;
        let account = crate::auth::runtime::RuntimeAccount::from_token(&state.auth, &token)
            .await
            .map_err(|_| {
                (
                    StatusCode::UNAUTHORIZED,
                    "Runtime setup authority expired or revoked".into(),
                )
            })?;
        if pending.management_user_id.is_some()
            || pending.org_id != account.org_id
            || target != account.id
        {
            return Err((StatusCode::FORBIDDEN, "Setup subject mismatch".into()));
        }
        if !account
            .permits_provider(&state.db, &pending.provider)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Provider availability unavailable".into(),
                )
            })?
        {
            return Err((
                StatusCode::FORBIDDEN,
                "Provider no longer attached to this endpoint".into(),
            ));
        }
        return Ok(OAuthAuthority {
            org_id: account.org_id,
            caller: None,
            target_id: target.uuid(),
            management_user_id: None,
            runtime_credential: Some(credential.clone()),
        });
    }
    let org = org.map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            "Management authentication required".into(),
        )
    })?;
    if pending.management_user_id != org.user_id {
        return Err((
            StatusCode::FORBIDDEN,
            "OAuth setup account changed; restart setup".into(),
        ));
    }
    let caller = crate::auth::caller_resolution::caller_for_user(
        &state.db,
        pending.org_id,
        org.user_id.ok_or((
            StatusCode::UNAUTHORIZED,
            "Management authentication required".into(),
        ))?,
    )
    .await
    .map_err(|_| (StatusCode::FORBIDDEN, "Membership no longer active".into()))?;
    crate::domains::virtual_users::connection_target(
        &state.db,
        state.auth.permission_resolver.as_ref(),
        &caller,
        &target.to_string(),
    )
    .await
    .map_err(|_| {
        (
            StatusCode::FORBIDDEN,
            "Connection setup authority is no longer valid".into(),
        )
    })?;
    Ok(OAuthAuthority {
        org_id: pending.org_id,
        caller: Some(caller),
        target_id: target.uuid(),
        management_user_id: org.user_id,
        runtime_credential: None,
    })
}

#[utoipa::path(summary = "List legacy credentials requiring an explicit organization destination.", get, path = "/v1/user/connection-migrations",   responses((status = 200, description = "Success", body = serde_json::Value), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub(super) async fn list_pending_connection_migrations(
    State(state): State<AppState>,
    auth: ManagementUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .db
        .list_pending_connections(auth.id)
        .await
        .map_err(|e| sanitized_internal_error("Connection migration", &e))?;
    Ok(Json(
        serde_json::json!({"data":rows.into_iter().map(|r|serde_json::json!({"id":r.id,"provider":r.provider,"connection_type":r.connection_type,"provider_username":r.provider_username,"connected_at":r.created_at})).collect::<Vec<_>>()}),
    ))
}
#[utoipa::path(summary = "Move one legacy credential into the selected organization without copying it.", post, path = "/v1/user/connection-migrations/{connection_id}", params(("connection_id" = String, Path)),  responses((status = 204, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub(super) async fn migrate_pending_connection(
    State(state): State<AppState>,
    org: ResolvedOrg,
    auth: ManagementUser,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    match state
        .db
        .migrate_pending_connection(org.org_id, auth.id, id)
        .await
    {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err((StatusCode::NOT_FOUND, "Pending connection not found".into())),
        Err(_) => Err((
            StatusCode::CONFLICT,
            "Destination already has this provider or migration authority is unavailable".into(),
        )),
    }
}
