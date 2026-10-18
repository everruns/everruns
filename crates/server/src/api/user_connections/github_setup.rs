// Shared GitHub App connection setup: installation redirect and callback.
// Decision: the callback links an installation only after a GitHub
// user-to-server token proves the user can access it (EVE-1193, TM-GITHUB-005).
// Decision: installations of the shared App belong to the org, so the callback
// stores one only as an org admin's link (`mode = organization`,
// `github_org.rs`). Step 3b closed the older target, a virtual user's own
// connection: it made a person's turns act as the shared App's bot, and agents
// choose a link instead (D3). A setup state from that flow is refused.

use super::*;

/// GET /v1/user/connections/github/authorize — refused since step 3b: the
/// shared App is installed for the org by an admin (`github_org.rs`).
pub async fn github_authorize(
    _auth: ConnectionUser,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    Err(refused_virtual_user_install())
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
    if pending.mode != ORG_LINK_MODE {
        return Err(refused_virtual_user_install());
    }
    let authority = org_link_authority(&state, &pending, org).await?;
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
    let frontend_url = state.auth_config.frontend_url.trim_end_matches('/');
    Ok((
        jar,
        Redirect::to(&format!("{}{}", frontend_url, pending.return_to)),
    ))
}
