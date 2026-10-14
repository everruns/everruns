// Org admin links for the shared Everruns GitHub App (D3 in
// knowledge/integrations/github-model.md).
//
// Decision: a shared App installation belongs to the Everruns org, not to the
// person who clicked Install, so linking one is an org admin action
// (`OrgAdmin` on start, admin re-checked on the callback) and the link is an
// organization connection (`connection_type = everruns_app`). Agents then
// choose a link explicitly (`domains::agents::github::everruns_app`).
//
// Decision: the install and ownership proof are the person flow's
// (`github_setup.rs`): GitHub's install redirect, then user authorization and
// `GET /user/installations` to prove the admin's GitHub account can reach the
// installation. Only the pending state's target differs: an org, not a
// virtual user.
//
// Decision: unlinking forgets the link and every agent choice of it; it does
// not uninstall the App on GitHub, which other orgs' admins or the GitHub org
// may still rely on. An installation already linked to another org is refused.

use super::*;
use crate::auth::middleware::OrgAdmin;
use crate::auth::oauth::GitHubInstallationResult;
use crate::github_apps::{EVERRUNS_APP_CONNECTION_TYPE, GITHUB_PROVIDER};
use crate::storage::CreateOrganizationConnectionRow;
use everruns_core::OrgRole;

/// `PendingOAuthState::mode` of an org admin's link.
pub(super) const ORG_LINK_MODE: &str = "organization";

const DEFAULT_RETURN_TO: &str = "/settings/organization?linked=github";

pub(super) fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/orgs/{org}/github/everruns-app", get(list_links))
        .route(
            "/v1/orgs/{org}/github/everruns-app/authorize",
            get(authorize_link),
        )
        .route(
            "/v1/orgs/{org}/github/everruns-app/{connection_id}",
            delete(unlink),
        )
        .with_state(state)
}

/// The org's links to the shared Everruns GitHub App.
#[derive(Debug, Serialize, ToSchema)]
pub struct EverrunsAppLinks {
    /// The deployment has a shared App; without one nothing can be linked.
    pub configured: bool,
    /// The shared App's slug; agents that choose it show up as `<slug>[bot]`.
    pub app_slug: Option<String>,
    pub installations: Vec<EverrunsAppLink>,
}

/// One shared App installation linked to the org.
#[derive(Debug, Serialize, ToSchema)]
pub struct EverrunsAppLink {
    /// Connection id agents choose the link by.
    pub id: uuid::Uuid,
    /// GitHub installation id.
    #[schema(example = 4242)]
    pub installation_id: i64,
    /// GitHub account the installation is on.
    #[schema(example = "acme")]
    pub account: String,
    /// GitHub's account type: `User` or `Organization`.
    pub account_type: Option<String>,
    /// `all` or `selected`.
    pub repository_selection: Option<String>,
    /// Agents in the org acting through this link.
    pub agent_count: usize,
    pub linked_at: DateTime<Utc>,
}

/// Where the browser returns after linking.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LinkQuery {
    /// Same-origin path; defaults to Settings > Organization.
    pub return_to: Option<String>,
}

fn internal(context: &str, error: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!(%error, "{context}");
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal error".into())
}

#[utoipa::path(get, path = "/v1/orgs/{org}/github/everruns-app", summary = "List the org's Everruns GitHub App links", description = "List the organization's links to the shared Everruns GitHub App.", params(("org" = String, Path)), responses((status = 200, body = EverrunsAppLinks), (status = 403, description = "Admin access required")), tag = "Organizations")]
pub(crate) async fn list_links(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
) -> Result<Json<EverrunsAppLinks>, (StatusCode, String)> {
    let rows = state
        .db
        .list_everruns_app_links(org.org_id)
        .await
        .map_err(|e| internal("Failed to list Everruns App links", e))?;
    let mut installations = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(installation_id) = row.installation_id else {
            continue;
        };
        let agent_count = state
            .db
            .list_everruns_app_identities(org.org_id, installation_id)
            .await
            .map_err(|e| internal("Failed to count Everruns App agents", e))?
            .len();
        let choice = crate::domains::agents::github::EverrunsAppInstallation::from_link(&row);
        installations.push(EverrunsAppLink {
            id: row.id,
            installation_id,
            account: choice.account,
            account_type: choice.account_type,
            repository_selection: choice.repository_selection,
            agent_count,
            linked_at: row.created_at,
        });
    }
    let shared = state.auth_config.github_connection.as_ref();
    Ok(Json(EverrunsAppLinks {
        configured: shared.is_some(),
        app_slug: shared.map(|config| config.app_slug.clone()),
        installations,
    }))
}

#[utoipa::path(get, path = "/v1/orgs/{org}/github/everruns-app/authorize", summary = "Link an Everruns GitHub App installation", description = "Start linking a shared Everruns GitHub App installation to the organization.", params(("org" = String, Path), ("return_to" = Option<String>, Query)), responses((status = 303, description = "Redirect to GitHub"), (status = 403, description = "Admin access required")), tag = "Organizations")]
pub(crate) async fn authorize_link(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    management: ManagementUser,
    jar: CookieJar,
    Query(query): Query<LinkQuery>,
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
    let pending = PendingOAuthState {
        state: new_setup_state(),
        org_id: org.org_id,
        management_user_id: Some(management.id),
        runtime_credential: None,
        provider: GITHUB_PROVIDER.into(),
        return_to: normalize_return_to(query.return_to.as_deref(), DEFAULT_RETURN_TO),
        mode: ORG_LINK_MODE.into(),
        session_id: None,
        agent_id: None,
        virtual_user_id: None,
        popup: false,
        code_verifier: String::new(),
        github_installation_id: None,
    };
    begin_installation(&state, config, jar, pending).await
}

#[utoipa::path(delete, path = "/v1/orgs/{org}/github/everruns-app/{connection_id}", summary = "Unlink an Everruns GitHub App installation", description = "Unlink a shared Everruns GitHub App installation and remove every agent's choice of it.", params(("org" = String, Path), ("connection_id" = String, Path)), responses((status = 204, description = "Unlinked"), (status = 403, description = "Admin access required"), (status = 404, description = "Link not found")), tag = "Organizations")]
pub(crate) async fn unlink(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    Path((_org, connection_id)): Path<(String, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
    match state
        .db
        .unlink_everruns_app(org.org_id, connection_id)
        .await
        .map_err(|e| internal("Failed to unlink Everruns App", e))?
    {
        Some(agents) => {
            tracing::info!(org_id = org.org_id, %connection_id, agents, "Everruns App unlinked");
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err((StatusCode::NOT_FOUND, "Link not found".into())),
    }
}

/// Who may store an org link on the callback: the admin who started it, still
/// an admin of that org.
pub(super) struct OrgLinkAuthority {
    pub(super) org_id: i64,
}

pub(super) async fn org_link_authority(
    state: &AppState,
    pending: &PendingOAuthState,
    org: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
) -> Result<OrgLinkAuthority, (StatusCode, String)> {
    let user_id = org.ok().and_then(|org| org.user_id).ok_or((
        StatusCode::UNAUTHORIZED,
        "Management authentication required".into(),
    ))?;
    if pending.management_user_id != Some(user_id) {
        return Err((
            StatusCode::FORBIDDEN,
            "Setup account changed; restart linking".into(),
        ));
    }
    let caller =
        crate::auth::caller_resolution::caller_for_user(&state.db, pending.org_id, user_id)
            .await
            .map_err(|_| (StatusCode::FORBIDDEN, "Membership no longer active".into()))?;
    if !caller.role.has_permission(OrgRole::Admin) {
        return Err((
            StatusCode::FORBIDDEN,
            "Linking the Everruns GitHub App needs organization admin rights".into(),
        ));
    }
    Ok(OrgLinkAuthority {
        org_id: pending.org_id,
    })
}

/// Store a proven installation as the org's link, or refresh the link the org
/// already has for it. An installation another org links is refused.
pub(super) async fn store_org_link(
    state: &AppState,
    authority: &OrgLinkAuthority,
    result: GitHubInstallationResult,
) -> Result<(), (StatusCode, String)> {
    let conflict = || {
        (
            StatusCode::CONFLICT,
            "This GitHub installation is already linked to another organization".to_string(),
        )
    };
    let mut metadata = serde_json::Map::new();
    if let Some(account_type) = result.account_type {
        metadata.insert("account_type".into(), account_type.into());
    }
    if let Some(selection) = result.repository_selection {
        metadata.insert("repository_selection".into(), selection.into());
    }
    let metadata = Some(serde_json::Value::Object(metadata));

    if let Some(existing) = state
        .db
        .find_everruns_app_link(result.installation_id)
        .await
        .map_err(|e| internal("Failed to look up Everruns App link", e))?
    {
        if existing.org_id != authority.org_id {
            tracing::warn!(
                org_id = authority.org_id,
                installation_id = result.installation_id,
                "GitHub installation already linked to another org"
            );
            return Err(conflict());
        }
        state
            .db
            .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
                virtual_user_id: existing.row.virtual_user_id,
                provider: GITHUB_PROVIDER.into(),
                connection_type: EVERRUNS_APP_CONNECTION_TYPE.into(),
                provider_user_id: Some(result.account_id),
                provider_username: Some(result.account_login),
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                scopes: Some(result.permissions),
                expires_at: None,
                installation_id: Some(result.installation_id),
                provider_metadata: metadata,
            })
            .await
            .map_err(|e| internal("Failed to refresh Everruns App link", e))?;
        return Ok(());
    }

    match state
        .db
        .create_organization_connection(CreateOrganizationConnectionRow {
            org_id: authority.org_id,
            name: result.account_login.clone(),
            provider: GITHUB_PROVIDER.into(),
            connection_type: Some(EVERRUNS_APP_CONNECTION_TYPE.into()),
            access_token_encrypted: None,
            provider_user_id: Some(result.account_id),
            provider_username: Some(result.account_login),
            scopes: Some(result.permissions),
            installation_id: Some(result.installation_id),
            provider_metadata: metadata,
        })
        .await
    {
        Ok(_) => Ok(()),
        // Another org linked it between the lookup and the insert.
        Err(e) if crate::errors::is_database_unique_violation(&e) => Err(conflict()),
        Err(e) => Err(internal("Failed to store Everruns App link", e)),
    }
}

/// Refuse to attach a shared App installation directly to an agent's service
/// virtual user: agents act as the shared App only through an org link (D3).
/// People's own connections are untouched until they move to `person` (3b).
pub(super) async fn refuse_service_target(
    state: &AppState,
    org_id: i64,
    target_id: uuid::Uuid,
) -> Result<(), (StatusCode, String)> {
    let target = state
        .db
        .get_virtual_user(org_id, VirtualUserId::from_uuid(target_id))
        .await
        .map_err(|e| internal("Failed to load virtual user", e))?;
    if target.is_some_and(|target| target.usage == "service") {
        return Err((
            StatusCode::BAD_REQUEST,
            "Agents use the Everruns GitHub App through the organization: an org admin links an \
             installation in Settings > Organization, then choose \"Use the Everruns app\" on the \
             agent's GitHub section"
                .into(),
        ));
    }
    Ok(())
}
