// "Connect GitHub" for agent identities: per-agent GitHub Apps.
//
// Flow (all clicks, no copied credentials):
// 1. `POST /v1/agent-identities/{id}/connections/github/app` (authenticated)
//    returns either a manifest form for the browser to post to GitHub (first
//    connect) or the App's install URL (the App already exists).
// 2. GitHub creates the App and redirects the browser to
//    `GET /v1/github/app-manifest/callback?code&state`. We exchange the code
//    for the App's credentials, store them encrypted, and send the browser on
//    to the App's installation page.
// 3. GitHub redirects to `GET /v1/github/apps/{app_row_id}/setup` after the
//    install. We verify the installation with the App's own JWT and store it
//    as the identity's `github` connection.
//
// The two GET routes are browser redirects from GitHub and carry none of our
// auth. See `crate::github_apps` for why they are safe anyway, and
// THREAT[TM-GHAPP-*] in `knowledge/security/threat-model.md`.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::common::{ErrorResponse, impl_auth_state};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::agent_identities::AGENT_IDENTITY_MANAGE;
use crate::github_apps::{
    AppCredentials, GitHubAppApi, ManifestInput, SetupState, build_manifest, default_app_name,
    install_url, manifest_form_action,
};
use crate::kernel_imports::{Caller, everruns_provider::typed_id::AgentIdentityId};
use crate::storage::github_app_rows::{CreateGitHubAppRow, GitHubAppRow};
use crate::storage::models::CreateAgentIdentityConnectionRow;
use crate::storage::{EncryptionService, StorageBackend};

pub const GITHUB_PROVIDER: &str = "github";
pub const GITHUB_APP_CONNECTION_TYPE: &str = "github_app";

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    pub github: GitHubAppApi,
}

impl AppState {
    /// Shares the identity-connections state: "Connect GitHub" is one more way
    /// to create an identity connection.
    pub fn from_connections(state: &super::agent_identity_connections::AppState) -> Self {
        Self {
            db: state.db.clone(),
            encryption: state.encryption.clone(),
            auth: state.auth.clone(),
            github: GitHubAppApi::new(crate::github_apps::GitHubEndpoints::from_env()),
        }
    }

    /// Public base URL of this API, as GitHub must call it.
    fn api_base_url(&self) -> &str {
        &self.auth.config.base_url
    }

    /// Public UI origin; where the browser returns when a flow ends.
    fn frontend_url(&self) -> &str {
        &self.auth.config.frontend_url
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agent-identities/{identity_id}/connections/github/app",
            post(begin_connect).delete(disconnect),
        )
        .route(
            "/v1/agent-identities/{identity_id}/connections/github/repositories",
            get(list_repositories),
        )
        .route("/v1/github/app-manifest/callback", get(manifest_callback))
        .route("/v1/github/apps/{app_row_id}/setup", get(setup_callback))
        .with_state(state)
}

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Default, Deserialize)]
pub struct BeginConnectRequest {
    /// GitHub organization that should own the App. Omit for the user's
    /// personal account.
    #[serde(default)]
    pub owner_org: Option<String>,
    /// UI path to return to when the flow ends.
    #[serde(default)]
    pub return_to: Option<String>,
}

/// What the browser does next.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BeginConnectResponse {
    /// Post `manifest` as the `manifest` form field to `action` (a top-level
    /// navigation, so GitHub's confirmation page is shown).
    CreateApp { action: String, manifest: String },
    /// The agent already has an App; open its installation page.
    Install { url: String, app_slug: String },
}

#[derive(Debug, Serialize)]
pub struct RepositoryResponse {
    pub full_name: String,
    pub private: bool,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
pub struct ManifestCallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetupCallbackQuery {
    installation_id: Option<i64>,
    state: Option<String>,
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn error(status: StatusCode, message: &str) -> ApiError {
    ErrorResponse::new(message.to_string()).into_response(status)
}

fn internal(context: &str, err: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %err, "{context}");
    error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
}

// ============================================================================
// Authenticated routes
// ============================================================================

async fn resolve_identity(
    state: &AppState,
    org: &ResolvedOrg,
    raw_id: &str,
) -> Result<(AgentIdentityId, String), ApiError> {
    let identity_id: AgentIdentityId = raw_id
        .parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid identity ID"))?;
    let caller = Caller::from(org);
    AGENT_IDENTITY_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(|_| error(StatusCode::FORBIDDEN, "Permission denied"))?;
    let identity = state
        .db
        .get_agent_identity(org.org_id, identity_id)
        .await
        .map_err(|e| internal("Failed to get agent identity", e))?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Agent identity not found"))?;
    Ok((identity_id, identity.name))
}

fn encryption(state: &AppState) -> Result<&EncryptionService, ApiError> {
    state
        .encryption
        .as_deref()
        .ok_or_else(|| error(StatusCode::SERVICE_UNAVAILABLE, "Encryption not configured"))
}

/// POST /v1/agent-identities/{identity_id}/connections/github/app
async fn begin_connect(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
    body: Option<Json<BeginConnectRequest>>,
) -> Result<Json<BeginConnectResponse>, ApiError> {
    let (identity_id, identity_name) = resolve_identity(&state, &org, &identity_id).await?;
    let encryption = encryption(&state)?;
    let request = body.map(|Json(body)| body).unwrap_or_default();
    let user_id = org
        .user_id
        .ok_or_else(|| error(StatusCode::FORBIDDEN, "A signed-in user is required"))?;

    let existing = state
        .db
        .get_github_app_for_identity(org.org_id, identity_id)
        .await
        .map_err(|e| internal("Failed to look up GitHub App", e))?;

    let app_row_id = existing.as_ref().map_or_else(Uuid::new_v4, |app| app.id);
    let setup_state = SetupState::new(
        org.org_id,
        identity_id.uuid(),
        user_id,
        app_row_id,
        request.return_to,
    )
    .seal(encryption)
    .map_err(|e| internal("Failed to seal GitHub setup state", e))?;

    let web_url = &state.github.endpoints().web_url;
    if let Some(app) = existing {
        return Ok(Json(BeginConnectResponse::Install {
            url: install_url(web_url, &app.slug, &setup_state),
            app_slug: app.slug,
        }));
    }

    let name = default_app_name(&identity_name);
    let manifest = build_manifest(&ManifestInput {
        name: &name,
        api_base_url: state.api_base_url(),
        frontend_url: state.frontend_url(),
        app_row_id,
    });
    Ok(Json(BeginConnectResponse::CreateApp {
        action: manifest_form_action(web_url, request.owner_org.as_deref(), &setup_state),
        manifest: manifest.to_string(),
    }))
}

/// DELETE /v1/agent-identities/{identity_id}/connections/github/app
///
/// Uninstalls the App (best effort) and removes the identity's `github`
/// connection. The App itself stays so a reconnect reuses it; GitHub offers no
/// API to delete an App.
async fn disconnect(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (identity_id, _) = resolve_identity(&state, &org, &identity_id).await?;
    let connection = state
        .db
        .get_agent_identity_connection(identity_id, GITHUB_PROVIDER)
        .await
        .map_err(|e| internal("Failed to look up GitHub connection", e))?;
    let app = state
        .db
        .get_github_app_for_identity(org.org_id, identity_id)
        .await
        .map_err(|e| internal("Failed to look up GitHub App", e))?;

    if let (Some(app), Some(installation_id)) = (
        app.as_ref(),
        connection.as_ref().and_then(|c| c.installation_id),
    ) {
        match app_credentials(&state, app) {
            Ok(credentials) => {
                if let Err(err) = state
                    .github
                    .delete_installation(&credentials, installation_id)
                    .await
                {
                    tracing::warn!(error = %err, app_id = app.app_id, "GitHub uninstall failed");
                }
            }
            Err(err) => tracing::warn!(error = %err, "GitHub App key unavailable for uninstall"),
        }
    }

    if connection.is_none() {
        return Err(error(StatusCode::NOT_FOUND, "Connection not found"));
    }
    state
        .db
        .delete_agent_identity_connection(identity_id, GITHUB_PROVIDER)
        .await
        .map_err(|e| internal("Failed to delete GitHub connection", e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v1/agent-identities/{identity_id}/connections/github/repositories
///
/// Repositories the agent's installation can reach, for repo pickers.
async fn list_repositories(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<Json<Vec<RepositoryResponse>>, ApiError> {
    let (identity_id, _) = resolve_identity(&state, &org, &identity_id).await?;
    let app = state
        .db
        .get_github_app_for_identity(org.org_id, identity_id)
        .await
        .map_err(|e| internal("Failed to look up GitHub App", e))?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "GitHub is not connected"))?;
    let installation_id = state
        .db
        .get_agent_identity_connection(identity_id, GITHUB_PROVIDER)
        .await
        .map_err(|e| internal("Failed to look up GitHub connection", e))?
        .and_then(|c| c.installation_id)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "GitHub is not connected"))?;

    let credentials =
        app_credentials(&state, &app).map_err(|e| internal("GitHub App key unavailable", e))?;
    let token = state
        .github
        .mint_installation_token(&credentials, installation_id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Failed to mint GitHub installation token");
            error(StatusCode::BAD_GATEWAY, "GitHub rejected the installation")
        })?;
    let repositories = state
        .github
        .list_installation_repositories(&token)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Failed to list GitHub repositories");
            error(
                StatusCode::BAD_GATEWAY,
                "Could not list GitHub repositories",
            )
        })?;
    Ok(Json(
        repositories
            .into_iter()
            .map(|repo| RepositoryResponse {
                full_name: repo.full_name,
                private: repo.private,
                html_url: repo.html_url,
            })
            .collect(),
    ))
}

fn app_credentials(state: &AppState, app: &GitHubAppRow) -> anyhow::Result<AppCredentials> {
    let encryption = state
        .encryption
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("encryption not configured"))?;
    Ok(AppCredentials {
        app_id: app.app_id,
        private_key_pem: encryption.decrypt_to_string(&app.private_key_encrypted)?,
    })
}

// ============================================================================
// GitHub redirects (unauthenticated)
// ============================================================================

fn ui_redirect(state: &AppState, return_to: Option<&str>, outcome: &str) -> Response {
    let base = state.frontend_url().trim_end_matches('/');
    let path = return_to.unwrap_or("/agents");
    let separator = if path.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{base}{path}{separator}github_connect={outcome}")).into_response()
}

/// GET /v1/github/app-manifest/callback
///
/// THREAT[TM-GHAPP-001]: reached by a browser redirect, so it cannot be
/// authenticated. The encrypted, expiring state names the org, identity and
/// App row; the one-time manifest code can only be redeemed once at GitHub.
async fn manifest_callback(
    State(state): State<AppState>,
    Query(query): Query<ManifestCallbackQuery>,
) -> Response {
    let Some(encryption) = state.encryption.as_deref() else {
        return ui_redirect(&state, None, "failed");
    };
    let setup = match query
        .state
        .as_deref()
        .map(|raw| SetupState::open(raw, encryption))
    {
        Some(Ok(setup)) => setup,
        _ => {
            tracing::warn!("GitHub manifest callback with missing or invalid state");
            return ui_redirect(&state, None, "failed");
        }
    };
    let return_to = setup.return_to.clone();
    match finish_manifest(&state, encryption, &setup, query.code.as_deref()).await {
        Ok(slug) => {
            // Reuse the same sealed state for the install hop.
            let sealed = query.state.unwrap_or_default();
            Redirect::to(&install_url(
                &state.github.endpoints().web_url,
                &slug,
                &sealed,
            ))
            .into_response()
        }
        Err(reason) => {
            tracing::warn!(reason, "GitHub manifest callback rejected");
            ui_redirect(&state, return_to.as_deref(), "failed")
        }
    }
}

async fn finish_manifest(
    state: &AppState,
    encryption: &EncryptionService,
    setup: &SetupState,
    code: Option<&str>,
) -> Result<String, &'static str> {
    let identity_id = AgentIdentityId::from_uuid(setup.agent_identity_id);
    // The identity may have been deleted, or have finished another connect
    // flow, since the state was minted.
    if state
        .db
        .get_agent_identity(setup.org_id, identity_id)
        .await
        .map_err(|_| "identity lookup failed")?
        .is_none()
    {
        return Err("identity no longer exists");
    }
    if let Some(existing) = state
        .db
        .get_github_app_for_identity(setup.org_id, identity_id)
        .await
        .map_err(|_| "app lookup failed")?
    {
        // A second create raced the first. The new App is orphaned on GitHub;
        // the user can delete it there. Keep the one we already hold.
        tracing::warn!(
            app_id = existing.app_id,
            "identity already has a GitHub App"
        );
        return Ok(existing.slug);
    }

    let code = code.filter(|c| !c.is_empty()).ok_or("missing code")?;
    let app = state
        .github
        .convert_manifest(code)
        .await
        .map_err(|_| "manifest conversion failed")?;

    let encrypt = |value: &str| {
        encryption
            .encrypt_string(value)
            .map_err(|_| "encrypt failed")
    };
    let row = CreateGitHubAppRow {
        id: setup.app_row_id,
        org_id: setup.org_id,
        agent_identity_id: identity_id,
        app_id: app.id,
        slug: app.slug.clone(),
        name: app.name,
        html_url: app.html_url,
        owner_login: app.owner.map(|owner| owner.login),
        client_id: app.client_id,
        client_secret_encrypted: app.client_secret.as_deref().map(encrypt).transpose()?,
        private_key_encrypted: encrypt(&app.pem)?,
        webhook_secret_encrypted: app.webhook_secret.as_deref().map(encrypt).transpose()?,
        created_by_user_id: Some(setup.user_id),
    };
    state
        .db
        .create_github_app(row)
        .await
        .map_err(|_| "storing the App failed")?;
    Ok(app.slug)
}

/// GET /v1/github/apps/{app_row_id}/setup
///
/// THREAT[TM-GHAPP-002]: the installation id is attacker-controllable, so it is
/// verified with the App's own JWT; GitHub only returns installations of that
/// App. The App belongs to exactly one identity, so the binding needs no state:
/// state only carries where to send the browser. This also covers installs and
/// repository changes a user makes directly on GitHub (`setup_on_update`).
async fn setup_callback(
    State(state): State<AppState>,
    Path(app_row_id): Path<Uuid>,
    Query(query): Query<SetupCallbackQuery>,
) -> Response {
    let return_to = state
        .encryption
        .as_deref()
        .zip(query.state.as_deref())
        .and_then(|(encryption, raw)| SetupState::open(raw, encryption).ok())
        .filter(|setup| setup.app_row_id == app_row_id)
        .and_then(|setup| setup.return_to);
    match finish_setup(&state, app_row_id, query.installation_id).await {
        Ok(()) => ui_redirect(&state, return_to.as_deref(), "ok"),
        Err(reason) => {
            tracing::warn!(%app_row_id, reason, "GitHub setup callback rejected");
            ui_redirect(&state, return_to.as_deref(), "failed")
        }
    }
}

async fn finish_setup(
    state: &AppState,
    app_row_id: Uuid,
    installation_id: Option<i64>,
) -> Result<(), &'static str> {
    let installation_id = installation_id.ok_or("missing installation_id")?;
    let app = state
        .db
        .get_github_app_unscoped(app_row_id)
        .await
        .map_err(|_| "app lookup failed")?
        .ok_or("unknown app")?;
    let credentials = app_credentials(state, &app).map_err(|_| "app key unavailable")?;
    let installation = state
        .github
        .get_installation(&credentials, installation_id)
        .await
        .map_err(|_| "installation not found for this app")?;
    if installation.app_id != app.app_id {
        return Err("installation belongs to another app");
    }

    let scopes = installation
        .permissions
        .as_object()
        .map(|permissions| {
            let mut scopes: Vec<String> = permissions
                .iter()
                .map(|(name, level)| format!("{name}:{}", level.as_str().unwrap_or("?")))
                .collect();
            scopes.sort();
            scopes.join(",")
        })
        .filter(|scopes| !scopes.is_empty());

    state
        .db
        .upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
            agent_identity_id: app.agent_identity_id,
            provider: GITHUB_PROVIDER.to_string(),
            connection_type: GITHUB_APP_CONNECTION_TYPE.to_string(),
            provider_user_id: Some(installation.account.id.to_string()),
            provider_username: Some(installation.account.login.clone()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes,
            expires_at: None,
            installation_id: Some(installation.id),
            provider_metadata: Some(serde_json::json!({
                "app_id": app.app_id,
                "app_slug": app.slug,
                "app_url": app.html_url,
                "repository_selection": installation.repository_selection,
            })),
        })
        .await
        .map_err(|_| "storing the connection failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::DEFAULT_ORG_ID;
    use crate::storage::models::CreateAgentIdentityRow;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn pem() -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/test-server-key.pem",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    async fn state(github: &MockServer) -> (AppState, AgentIdentityId) {
        let db = Arc::new(StorageBackend::in_memory());
        let identity_id = AgentIdentityId::from_seed(21);
        db.create_agent_identity(CreateAgentIdentityRow {
            org_id: DEFAULT_ORG_ID,
            id: identity_id,
            name: "PR Summarizer".to_string(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
        let encryption = EncryptionService::new(
            &crate::storage::encryption::generate_encryption_key("github-apps-api-test"),
            &[],
        )
        .unwrap();
        let auth = AuthState::builtin(crate::auth::config::AuthConfig::default(), db.clone());
        let state = AppState {
            db,
            encryption: Some(Arc::new(encryption)),
            auth,
            github: GitHubAppApi::new(crate::github_apps::GitHubEndpoints {
                api_url: github.uri(),
                web_url: "https://github.example".to_string(),
            }),
        };
        (state, identity_id)
    }

    fn setup_for(identity_id: AgentIdentityId, app_row_id: Uuid) -> SetupState {
        SetupState::new(
            DEFAULT_ORG_ID,
            identity_id.uuid(),
            Uuid::now_v7(),
            app_row_id,
            Some("/agents/x".to_string()),
        )
    }

    async fn mock_conversion(github: &MockServer, app_id: i64) {
        Mock::given(method("POST"))
            .and(path("/app-manifests/abc123/conversions"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": app_id,
                "slug": "pr-summarizer-1a2b3c",
                "name": "PR-Summarizer-1a2b3c",
                "html_url": "https://github.example/apps/pr-summarizer-1a2b3c",
                "owner": { "id": 7, "login": "acme" },
                "client_id": "Iv1.abc",
                "client_secret": "client-secret",
                "webhook_secret": "hook-secret",
                "pem": pem(),
            })))
            .mount(github)
            .await;
    }

    #[tokio::test]
    async fn manifest_callback_stores_the_app_encrypted_and_only_once() {
        let github = MockServer::start().await;
        mock_conversion(&github, 555).await;
        let (state, identity_id) = state(&github).await;
        let encryption = state.encryption.clone().unwrap();
        let row_id = Uuid::now_v7();
        let setup = setup_for(identity_id, row_id);

        let slug = finish_manifest(&state, &encryption, &setup, Some("abc123"))
            .await
            .unwrap();
        assert_eq!(slug, "pr-summarizer-1a2b3c");

        let app = state
            .db
            .get_github_app_unscoped(row_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(app.app_id, 555);
        assert_eq!(app.agent_identity_id, identity_id);
        assert_eq!(app.owner_login.as_deref(), Some("acme"));
        // Secrets never land in plaintext.
        assert!(!String::from_utf8_lossy(&app.private_key_encrypted).contains("PRIVATE KEY"));
        assert_eq!(
            encryption
                .decrypt_to_string(app.webhook_secret_encrypted.as_ref().unwrap())
                .unwrap(),
            "hook-secret"
        );

        // A second flow for the same identity keeps the first App.
        let again = setup_for(identity_id, Uuid::now_v7());
        assert_eq!(
            finish_manifest(&state, &encryption, &again, Some("abc123"))
                .await
                .unwrap(),
            "pr-summarizer-1a2b3c"
        );
        assert!(
            state
                .db
                .get_github_app_unscoped(again.app_row_id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn manifest_callback_rejects_missing_code_and_unknown_identity() {
        let github = MockServer::start().await;
        let (state, identity_id) = state(&github).await;
        let encryption = state.encryption.clone().unwrap();
        let setup = setup_for(identity_id, Uuid::now_v7());
        assert!(
            finish_manifest(&state, &encryption, &setup, None)
                .await
                .is_err()
        );

        let stranger = setup_for(AgentIdentityId::from_seed(99), Uuid::now_v7());
        assert_eq!(
            finish_manifest(&state, &encryption, &stranger, Some("abc123")).await,
            Err("identity no longer exists")
        );
    }

    #[tokio::test]
    async fn setup_binds_only_installations_of_this_app() {
        let github = MockServer::start().await;
        mock_conversion(&github, 555).await;
        Mock::given(method("GET"))
            .and(path("/app/installations/4242"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 4242,
                "app_id": 555,
                "account": { "id": 7, "login": "acme" },
                "permissions": { "pull_requests": "write", "contents": "read" },
                "repository_selection": "selected",
            })))
            .mount(&github)
            .await;
        // GitHub answers 404 for an installation of another App.
        Mock::given(method("GET"))
            .and(path("/app/installations/9999"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&github)
            .await;

        let (state, identity_id) = state(&github).await;
        let encryption = state.encryption.clone().unwrap();
        let row_id = Uuid::now_v7();
        let setup = setup_for(identity_id, row_id);
        finish_manifest(&state, &encryption, &setup, Some("abc123"))
            .await
            .unwrap();

        assert!(finish_setup(&state, row_id, Some(9999)).await.is_err());
        assert!(
            finish_setup(&state, Uuid::now_v7(), Some(4242))
                .await
                .is_err()
        );
        assert!(finish_setup(&state, row_id, None).await.is_err());
        assert!(
            state
                .db
                .get_agent_identity_connection(identity_id, GITHUB_PROVIDER)
                .await
                .unwrap()
                .is_none()
        );

        finish_setup(&state, row_id, Some(4242)).await.unwrap();
        let connection = state
            .db
            .get_agent_identity_connection(identity_id, GITHUB_PROVIDER)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connection.installation_id, Some(4242));
        assert_eq!(connection.connection_type, GITHUB_APP_CONNECTION_TYPE);
        assert_eq!(connection.provider_username.as_deref(), Some("acme"));
        assert_eq!(
            connection.scopes.as_deref(),
            Some("contents:read,pull_requests:write")
        );
    }

    #[tokio::test]
    async fn ui_redirect_appends_the_outcome() {
        let github = MockServer::start().await;
        let (state, _) = state(&github).await;
        let location = |response: Response| {
            response.headers()[axum::http::header::LOCATION]
                .to_str()
                .unwrap()
                .to_string()
        };
        let base = state.frontend_url().trim_end_matches('/').to_string();
        assert_eq!(
            location(ui_redirect(&state, Some("/agents/a?tab=identity"), "ok")),
            format!("{base}/agents/a?tab=identity&github_connect=ok")
        );
        assert_eq!(
            location(ui_redirect(&state, None, "failed")),
            format!("{base}/agents?github_connect=failed")
        );
    }

    /// "Connect GitHub" routes sit beside the generic `{provider}` routes; the
    /// generic ones must still match for `github`.
    #[tokio::test]
    async fn github_routes_do_not_shadow_generic_provider_routes() {
        use axum::body::Body;
        use tower::ServiceExt;
        let app: Router = Router::new()
            .route(
                "/v1/agent-identities/{identity_id}/connections/{provider}",
                post(|| async { "generic" }).delete(|| async { "generic-delete" }),
            )
            .route(
                "/v1/agent-identities/{identity_id}/connections/{provider}/verify",
                post(|| async { "verify" }),
            )
            .merge(
                Router::new()
                    .route(
                        "/v1/agent-identities/{identity_id}/connections/github/app",
                        post(|| async { "app" }),
                    )
                    .route(
                        "/v1/agent-identities/{identity_id}/connections/github/repositories",
                        get(|| async { "repos" }),
                    ),
            );
        for (verb, uri, expected) in [
            (
                "POST",
                "/v1/agent-identities/i/connections/github",
                "generic",
            ),
            (
                "DELETE",
                "/v1/agent-identities/i/connections/github",
                "generic-delete",
            ),
            (
                "POST",
                "/v1/agent-identities/i/connections/github/verify",
                "verify",
            ),
            (
                "POST",
                "/v1/agent-identities/i/connections/linear/verify",
                "verify",
            ),
            (
                "POST",
                "/v1/agent-identities/i/connections/github/app",
                "app",
            ),
            (
                "GET",
                "/v1/agent-identities/i/connections/github/repositories",
                "repos",
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(verb)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{verb} {uri}");
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            assert_eq!(body, expected, "{verb} {uri}");
        }
    }
}
