// Shared GitHub App setup: an org admin's installation link and a person's own
// sign-in, both returning to one callback.
// Decision: the callback links an installation only after a GitHub
// user-to-server token proves the user can access it (EVE-1193, TM-GITHUB-005).
// Decision: installations of the shared App belong to the org, so the callback
// stores one only as an org admin's link (`mode = organization`,
// `github_org.rs`). A person's own connection holds their user token instead
// (D1, step 4), so their uses act as them, never as the shared App's bot.
// Agents never sign in: they act through their own App or a link (D3).

use super::*;

/// GET /v1/user/connections/github/authorize — sign the caller in to GitHub
pub async fn github_authorize(
    State(state): State<AppState>,
    auth: ConnectionUser,
    jar: CookieJar,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    github_authorize_inner(
        state,
        OAuthAuthority {
            org_id: auth.org_id,
            caller: None,
            target_id: auth.id,
            management_user_id: Some(auth.management_user_id),
            runtime_credential: None,
        },
        jar,
        params.get("return_to").cloned(),
        params.get("popup").is_some_and(|value| value == "true"),
    )
    .await
}

/// Start a person's GitHub sign-in for `authority.target_id`: GitHub user
/// authorization of the shared App, returning a code to `github_callback`.
pub(super) async fn github_authorize_inner(
    state: AppState,
    authority: OAuthAuthority,
    jar: CookieJar,
    return_to: Option<String>,
    popup: bool,
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
        popup,
        code_verifier: String::new(),
        github_installation_id: None,
    };
    let url = GitHubAppService::new(config)
        .user_authorization_url(&pending.state)
        .map_err(|e| sanitized_internal_error("GitHub sign-in", &e))?;
    register_pending_setup(&state, &pending).await?;
    let jar = jar.add(github_setup_cookie(&pending)?);
    Ok((jar, Redirect::to(&url)))
}

/// Agents act on GitHub through their own App or an org link, never through a
/// sign-in or an installation on their service virtual user (D3).
async fn refuse_service_target(
    state: &AppState,
    org_id: i64,
    target_id: uuid::Uuid,
) -> Result<(), (StatusCode, String)> {
    let target = state
        .db
        .get_virtual_user(org_id, VirtualUserId::from_uuid(target_id))
        .await
        .map_err(|e| sanitized_internal_error("GitHub sign-in", &e))?;
    if target.is_some_and(|target| target.usage == "service") {
        return Err(refused_shared_app_install());
    }
    Ok(())
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

/// Where the browser goes after a successful setup: a popup reports back to
/// its opener (the agent's Sync dialog) and closes; a full page returns to
/// `return_to` as before.
fn setup_done_redirect(state: &AppState, pending: &PendingOAuthState) -> String {
    if pending.popup {
        return finalize_oauth_redirect(&state.auth_config, &pending.return_to, "github", true);
    }
    let frontend_url = state.auth_config.frontend_url.trim_end_matches('/');
    format!("{}{}", frontend_url, pending.return_to)
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
    let jar = jar.remove(Cookie::from(oauth_state_cookie_name("github")));
    if pending.mode != ORG_LINK_MODE {
        return person_sign_in(&state, pending, org, jar, query).await;
    }
    let authority = org_link_authority(&state, &pending, org).await?;
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
            org_id = authority.org_id,
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

    store_org_link(&state, &authority, result).await?;
    Ok((jar, Redirect::to(&setup_done_redirect(&state, &pending))))
}

/// The return leg of a person's sign-in: exchange the code for their user
/// token and store it on their own `github` connection, replacing whatever it
/// held. THREAT[TM-GITHUB-008]: the account is the one the token reads back
/// from GitHub, never a claim from the query.
async fn person_sign_in(
    state: &AppState,
    pending: PendingOAuthState,
    org: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
    jar: CookieJar,
    query: GitHubInstallationCallbackQuery,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let auth = callback_authority(state, &pending, org).await?;
    refuse_service_target(state, auth.org_id, auth.target_id).await?;
    // A setup state from before step 3b started an installation; it carries
    // no sign-in to finish.
    if query.installation_id.is_some() || pending.github_installation_id.is_some() {
        return Err(refused_shared_app_install());
    }
    let Some(code) = query.code.as_deref() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "GitHub sign-in did not complete; connect GitHub again".to_string(),
        ));
    };
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
    let client = crate::github_user_token::GitHubUserClient::new(
        config.client_id.as_deref(),
        config.client_secret.as_deref(),
    )
    .ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "GitHub App client credentials not configured".to_string(),
        )
    })?;
    let encryption = state.encryption.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        )
    })?;
    let github = crate::github_user_token::GitHubUserTokens::new(config.endpoints.clone());
    let refused = |e: anyhow::Error| {
        tracing::warn!(error = %e, "GitHub sign-in failed");
        (
            StatusCode::BAD_REQUEST,
            "GitHub sign-in failed; connect GitHub again".to_string(),
        )
    };
    let token = github.exchange_code(&client, code).await.map_err(refused)?;
    let user = github.user(&token.access_token).await.map_err(refused)?;
    let encrypt = |value: &str| {
        encryption
            .encrypt_string(value)
            .map_err(|e| sanitized_internal_error("GitHub sign-in", &e))
    };
    state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.target_id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some(user.id.to_string()),
            provider_username: Some(user.login),
            access_token_encrypted: Some(encrypt(&token.access_token)?),
            refresh_token_encrypted: token.refresh_token.as_deref().map(encrypt).transpose()?,
            scopes: None,
            expires_at: token.expires_at,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .map_err(|e| sanitized_internal_error("GitHub sign-in", &e))?;
    Ok((jar, Redirect::to(&setup_done_redirect(state, &pending))))
}
