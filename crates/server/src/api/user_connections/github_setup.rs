// Shared GitHub App connection setup: installation redirect and callback.
// Decision: the callback links an installation only after a GitHub
// user-to-server token proves the user can access it (EVE-1193, TM-GITHUB-005).
// Decision: one callback serves both targets GitHub can return to: a virtual
// user's own connection and an org admin's link (`mode = organization`,
// `github_org.rs`). The App has one setup URL, so the pending state, not the
// URL, says which target the proven installation is stored for.

use super::*;

/// GET /v1/user/connections/github/authorize — Redirect to GitHub App installation
pub async fn github_authorize(
    State(state): State<AppState>,
    _auth: ConnectionUser,
    jar: CookieJar,
    Query(_params): Query<std::collections::HashMap<String, String>>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    github_authorize_inner(
        state,
        OAuthAuthority {
            org_id: _auth.org_id,
            caller: None,
            target_id: _auth.id,
            management_user_id: Some(_auth.management_user_id),
            runtime_credential: None,
        },
        jar,
        None,
    )
    .await
}
pub(super) async fn github_authorize_inner(
    state: AppState,
    authority: OAuthAuthority,
    jar: CookieJar,
    return_to: Option<String>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let config = state
        .auth_config
        .github_connection
        .as_ref()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub App not configured".to_string(),
            )
        })?;

    // THREAT[TM-GITHUB-005]: an agent's service virtual user acts as the
    // shared App only through an org admin's link (D3), never through a
    // member attaching an installation directly.
    refuse_service_target(&state, authority.org_id, authority.target_id).await?;

    let pending = PendingOAuthState {
        state: new_setup_state(),
        org_id: authority.org_id,
        management_user_id: authority.management_user_id,
        runtime_credential: authority.runtime_credential,
        provider: "github".into(),
        return_to: normalize_return_to(
            return_to.as_deref(),
            "/settings/connections?connected=github",
        ),
        mode: "virtual_user".into(),
        session_id: None,
        agent_id: None,
        virtual_user_id: Some(VirtualUserId::from_uuid(authority.target_id).to_string()),
        popup: false,
        code_verifier: String::new(),
        github_installation_id: None,
    };
    begin_installation(&state, config, jar, pending).await
}

/// Register `pending` and send the browser to the shared App's install page.
pub(super) async fn begin_installation(
    state: &AppState,
    config: &crate::auth::config::GitHubConnectionConfig,
    jar: CookieJar,
    pending: PendingOAuthState,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    register_pending_setup(state, &pending).await?;
    let jar = jar.add(github_setup_cookie(&pending)?);
    let auth_url = GitHubAppService::new(config).installation_url(&pending.state);
    Ok((jar, Redirect::to(&auth_url.url)))
}

pub(super) fn new_setup_state() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    hex::encode(bytes)
}

pub(super) fn github_setup_cookie(
    pending: &PendingOAuthState,
) -> Result<Cookie<'static>, (StatusCode, String)> {
    Ok(Cookie::build((
        oauth_state_cookie_name("github"),
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(pending)
                .map_err(|e| sanitized_internal_error("GitHub setup", &e))?,
        ),
    ))
    .path("/")
    .http_only(true)
    .secure(true)
    .same_site(SameSite::Lax)
    .max_age(time::Duration::minutes(10))
    .build())
}

/// GET /v1/user/connections/github/callback — GitHub App installation callback
///
/// After user installs the GitHub App on their repos, GitHub redirects here
/// with the installation_id. Validates the single-use setup state, proves the
/// GitHub user completing setup can access the installation, and stores it.
pub async fn github_callback(
    State(state): State<AppState>,
    org: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
    jar: CookieJar,
    Query(query): Query<GitHubInstallationCallbackQuery>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let pending = validate_pending_oauth_state(&jar, "github", query.state.as_deref())?;
    consume_pending_setup(&state, &pending).await?;
    let target = if pending.mode == ORG_LINK_MODE {
        SetupTarget::Org(org_link_authority(&state, &pending, org).await?)
    } else {
        let auth = callback_authority(&state, &pending, org).await?;
        refuse_service_target(&state, auth.org_id, auth.target_id).await?;
        SetupTarget::VirtualUser(auth)
    };
    let jar = jar.remove(Cookie::from(oauth_state_cookie_name("github")));
    let config = state
        .auth_config
        .github_connection
        .as_ref()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub App not configured".to_string(),
            )
        })?;

    let service = GitHubAppService::new(config);

    // The authorization hop's state carries the installation GitHub reported
    // first; the return leg may not swap it for another.
    let installation_id = match (pending.github_installation_id, query.installation_id) {
        (Some(bound), Some(claimed)) if bound != claimed => {
            return Err((
                StatusCode::BAD_REQUEST,
                "GitHub installation changed during setup; restart setup".to_string(),
            ));
        }
        (Some(id), _) | (None, Some(id)) => id,
        (None, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                "GitHub did not report an installation. If an organization owner must approve \
                 it, connect again after approval"
                    .to_string(),
            ));
        }
    };

    // THREAT[TM-GITHUB-005]: a valid setup state proves who started setup, not
    // that their GitHub account can access `installation_id`. Without a user
    // authorization code yet, bind the claim to a fresh single-use state and
    // send the browser through GitHub user authorization.
    let Some(code) = query.code.as_deref() else {
        if pending.github_installation_id.is_some() {
            return Err((
                StatusCode::BAD_REQUEST,
                "GitHub user authorization did not complete; restart setup".to_string(),
            ));
        }
        let authorize = PendingOAuthState {
            state: new_setup_state(),
            github_installation_id: Some(installation_id),
            ..pending
        };
        let url = service
            .user_authorization_url(&authorize.state)
            .map_err(|e| sanitized_internal_error("GitHub setup", &e))?;
        register_pending_setup(&state, &authorize).await?;
        let jar = jar.add(github_setup_cookie(&authorize)?);
        return Ok((jar, Redirect::to(&url)));
    };

    let can_access = service
        .user_can_access_installation(code, installation_id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "GitHub user authorization failed");
            (
                StatusCode::BAD_REQUEST,
                "GitHub user authorization failed; restart setup".to_string(),
            )
        })?;
    if !can_access {
        tracing::warn!(
            target = %target.subject(),
            installation_id,
            "GitHub user cannot access the claimed installation"
        );
        return Err((
            StatusCode::FORBIDDEN,
            "Your GitHub account cannot access this GitHub App installation".to_string(),
        ));
    }

    // Verify the installation exists and get account details
    let result = service
        .verify_installation(installation_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "GitHub App installation verification failed");
            (
                StatusCode::BAD_REQUEST,
                "GitHub App installation verification failed".to_string(),
            )
        })?;

    let frontend_url = state.auth_config.frontend_url.trim_end_matches('/');
    let auth = match target {
        SetupTarget::Org(authority) => {
            store_org_link(&state, &authority, result).await?;
            return Ok((
                jar,
                Redirect::to(&format!("{}{}", frontend_url, pending.return_to)),
            ));
        }
        SetupTarget::VirtualUser(auth) => auth,
    };

    // Prevent installation hijacking across users: an installation already linked
    // to another user must not be claimable via callback replay/forgery.
    if let Some(existing_owner_id) = state
        .db
        .get_user_id_by_installation_id("github", result.installation_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to resolve GitHub installation owner");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?
        && existing_owner_id != auth.target_id
    {
        tracing::warn!(
            user_id = %auth.target_id,
            existing_owner_id = %existing_owner_id,
            installation_id = result.installation_id,
            "GitHub installation already linked to another user"
        );
        return Err((
            StatusCode::CONFLICT,
            "GitHub installation is already linked to another user".to_string(),
        ));
    }

    // Store installation_id (no OAuth token needed — tokens minted on demand)
    state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.target_id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some(result.account_id),
            provider_username: Some(result.account_login),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some(result.permissions),
            expires_at: None,
            installation_id: Some(result.installation_id),
            // Lets an agent's GitHub card suggest this account as an owner.
            provider_metadata: result
                .account_type
                .map(|account_type| serde_json::json!({ "account_type": account_type })),
        })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to store GitHub App installation");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?;

    Ok((
        jar,
        Redirect::to(&format!("{}{}", frontend_url, pending.return_to)),
    ))
}

/// Where a proven installation is stored.
enum SetupTarget {
    VirtualUser(OAuthAuthority),
    Org(OrgLinkAuthority),
}

impl SetupTarget {
    /// The target, for logs: a virtual user id or an org id.
    fn subject(&self) -> String {
        match self {
            Self::VirtualUser(auth) => format!("virtual_user:{}", auth.target_id),
            Self::Org(authority) => format!("org:{}", authority.org_id),
        }
    }
}
