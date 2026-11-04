// Org admin links for the shared Everruns GitHub App (D3): admin-only routes,
// the install and ownership proof reused from the person flow, an admin
// re-check on the callback, one org per installation, and the closed member
// path for agents' service virtual users.

use super::github_installation_ownership::{
    ATTACKER_INSTALLATION, VICTIM_INSTALLATION, github_state, location, mock_github,
};
use super::*;
use crate::auth::backend::AuthBackend;
use crate::auth::middleware::{AuthError, AuthMethod, OrgAdmin, OrgContext};
use crate::auth::routes::AuthConfigResponse;
use crate::domains::organizations::record::OrgMembership;
use crate::storage::{CreateOrganizationRow, CreateVirtualUserRow};
use axum::body::Body;
use axum::http::Request;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID};
use tower::ServiceExt;
use wiremock::MockServer;

/// Authenticates every request as one user with one role in the default org.
#[derive(Clone)]
struct OneUser {
    user_id: Uuid,
    role: OrgRole,
}

#[async_trait::async_trait]
impl AuthBackend for OneUser {
    async fn validate_token(&self, _token: &str) -> Result<ManagementUser, AuthError> {
        Ok(management_user(self.user_id, self.role))
    }

    async fn validate_personal_access_token(
        &self,
        _token: &str,
    ) -> Result<ManagementUser, AuthError> {
        Err(AuthError::unauthorized("not supported"))
    }

    fn auth_routes(&self) -> Option<Router> {
        None
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        AuthConfigResponse {
            mode: "full".to_string(),
            login_origin: None,
            password_auth_enabled: false,
            signup_enabled: false,
            oauth_providers: vec![],
            signup_email_confirm: false,
            captcha: None,
        }
    }
}

fn management_user(user_id: Uuid, role: OrgRole) -> ManagementUser {
    ManagementUser {
        id: user_id,
        email: format!("{user_id}@example.com"),
        name: "Test".to_string(),
        roles: vec!["user".to_string()],
        is_platform_user: false,
        auth_method: AuthMethod::Jwt,
        organizations: vec![OrgMembership {
            org_id: DEFAULT_ORG_ID,
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Default Organization".to_string(),
            role,
        }],
        pat_scope: None,
    }
}

/// A user of the default org with `role`, stored in the database too, so the
/// callback's membership re-check sees the same role.
async fn user_with_role(state: &AppState, role: OrgRole) -> Uuid {
    let user_id = state.db.create_test_user(Uuid::now_v7()).await;
    state
        .db
        .add_organization_member(
            DEFAULT_ORG_ID,
            user_id,
            match role {
                OrgRole::Owner => "owner",
                OrgRole::Admin => "admin",
                OrgRole::Member => "member",
            },
        )
        .await
        .unwrap();
    user_id
}

fn as_user(state: &AppState, user_id: Uuid, role: OrgRole) -> AppState {
    let mut state = state.clone();
    // `AuthMode::None` would admit everyone as the anonymous admin.
    let mut config = state.auth_config.clone();
    config.mode = crate::auth::config::AuthMode::Full;
    state.auth = AuthState::new(config, Arc::new(OneUser { user_id, role }));
    state
}

fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!(
            "/v1/orgs/{DEFAULT_ORG_PUBLIC_ID}/github/everruns-app{path}"
        ))
        .header("Authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap()
}

async fn status_of(state: &AppState, method: &str, path: &str) -> StatusCode {
    routes(state.clone())
        .oneshot(request(method, path))
        .await
        .unwrap()
        .status()
}

/// Start linking as an admin of the default org; returns the state cookie
/// jar and value.
async fn begin_link(state: &AppState, user_id: Uuid) -> (CookieJar, String) {
    let (jar, redirect) = github_org::authorize_link(
        State(state.clone()),
        OrgAdmin(OrgContext {
            org_id: DEFAULT_ORG_ID,
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Default Organization".to_string(),
            role: OrgRole::Admin,
        }),
        management_user(user_id, OrgRole::Admin),
        CookieJar::new(),
        Query(github_org::LinkQuery {
            return_to: None,
            popup: None,
        }),
    )
    .await
    .unwrap();
    assert!(location(redirect).contains("/apps/everruns-test/installations/new?state="));
    let setup_state = pending_state(&jar, "github").state;
    (jar, setup_state)
}

async fn finish_link(
    state: &AppState,
    user_id: Uuid,
    jar: CookieJar,
    setup_state: &str,
    installation_id: i64,
    code: &str,
) -> Result<String, (StatusCode, String)> {
    github_callback(
        State(state.clone()),
        Ok(ResolvedOrg {
            user_id: Some(user_id),
            ..test_org(user_id)
        }),
        jar,
        Query(GitHubInstallationCallbackQuery {
            installation_id: Some(installation_id),
            setup_action: Some("install".to_string()),
            state: Some(setup_state.to_string()),
            code: Some(code.to_string()),
        }),
    )
    .await
    .map(|(_, redirect)| location(redirect))
}

#[tokio::test]
async fn link_routes_are_admin_only() {
    let github = MockServer::start().await;
    let state = github_state(&github).await;
    let admin = user_with_role(&state, OrgRole::Admin).await;
    let member = user_with_role(&state, OrgRole::Member).await;
    let link_id = state
        .db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: DEFAULT_ORG_ID,
            name: "acme".to_string(),
            provider: "github".to_string(),
            connection_type: Some(crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE.to_string()),
            provider_username: Some("acme".to_string()),
            installation_id: Some(VICTIM_INSTALLATION),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;

    let as_member = as_user(&state, member, OrgRole::Member);
    assert_eq!(
        status_of(&as_member, "GET", "").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(&as_member, "GET", "/authorize").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status_of(&as_member, "DELETE", &format!("/{link_id}")).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        state
            .db
            .list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .len(),
        1,
        "a member cannot unlink"
    );

    let as_admin = as_user(&state, admin, OrgRole::Admin);
    let response = routes(as_admin.clone())
        .oneshot(request("GET", ""))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let links: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(links["configured"], true);
    assert_eq!(links["app_slug"], "everruns-test");
    assert_eq!(links["installations"][0]["account"], "acme");
    assert_eq!(links["installations"][0]["agent_count"], 0);
    assert_eq!(
        status_of(&as_admin, "GET", "/authorize").await,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        status_of(&as_admin, "DELETE", &format!("/{link_id}")).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status_of(&as_admin, "DELETE", &format!("/{link_id}")).await,
        StatusCode::NOT_FOUND
    );
    // The generic organization accounts never list a GitHub link.
    assert!(
        state
            .db
            .list_organization_connections(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_admin_links_an_installation_their_github_account_can_access() {
    let github = MockServer::start().await;
    mock_github(&github).await;
    let state = github_state(&github).await;
    let admin = user_with_role(&state, OrgRole::Admin).await;

    // A GitHub account without access to the installation proves nothing.
    let (jar, setup_state) = begin_link(&state, admin).await;
    let error = finish_link(
        &state,
        admin,
        jar,
        &setup_state,
        VICTIM_INSTALLATION,
        "attacker-code",
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert!(
        state
            .db
            .list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_empty()
    );

    let (jar, setup_state) = begin_link(&state, admin).await;
    let target = finish_link(
        &state,
        admin,
        jar,
        &setup_state,
        VICTIM_INSTALLATION,
        "owner-code",
    )
    .await
    .unwrap();
    assert!(
        target.ends_with("/settings/organization?linked=github"),
        "{target}"
    );
    let links = state
        .db
        .list_everruns_app_links(DEFAULT_ORG_ID)
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    let link = &links[0];
    assert_eq!(link.installation_id, Some(VICTIM_INSTALLATION));
    assert_eq!(link.owner_scope, "organization");
    assert_eq!(
        link.connection_type,
        crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE
    );
    assert_eq!(link.name.as_deref(), Some("account-4242"));
    assert_eq!(
        link.provider_metadata.as_ref().unwrap()["account_type"],
        "Organization"
    );

    // Linking it again refreshes the same link.
    let (jar, setup_state) = begin_link(&state, admin).await;
    finish_link(
        &state,
        admin,
        jar,
        &setup_state,
        VICTIM_INSTALLATION,
        "owner-code",
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .db
            .list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn the_callback_rechecks_admin_rights() {
    let github = MockServer::start().await;
    mock_github(&github).await;
    let state = github_state(&github).await;
    let admin = user_with_role(&state, OrgRole::Admin).await;

    let (jar, setup_state) = begin_link(&state, admin).await;
    state
        .db
        .update_organization_member_role(DEFAULT_ORG_ID, admin, "member")
        .await
        .unwrap();
    let error = finish_link(
        &state,
        admin,
        jar,
        &setup_state,
        VICTIM_INSTALLATION,
        "owner-code",
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert!(
        state
            .db
            .list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_empty()
    );

    // Another signed-in user cannot finish the admin's link either.
    state
        .db
        .update_organization_member_role(DEFAULT_ORG_ID, admin, "admin")
        .await
        .unwrap();
    let other = user_with_role(&state, OrgRole::Admin).await;
    let (jar, setup_state) = begin_link(&state, admin).await;
    let error = finish_link(
        &state,
        other,
        jar,
        &setup_state,
        VICTIM_INSTALLATION,
        "owner-code",
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_installation_belongs_to_one_org() {
    let github = MockServer::start().await;
    mock_github(&github).await;
    let state = github_state(&github).await;
    let admin = user_with_role(&state, OrgRole::Admin).await;
    let other_org = state
        .db
        .create_organization(CreateOrganizationRow {
            public_id: "org_000000000000000000000000000000e5".to_string(),
            name: "Other Org".to_string(),
            created_by: None,
        })
        .await
        .unwrap()
        .org_id;
    state
        .db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: other_org,
            name: "account-1717".to_string(),
            provider: "github".to_string(),
            connection_type: Some(crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE.to_string()),
            installation_id: Some(ATTACKER_INSTALLATION),
            ..Default::default()
        })
        .await
        .unwrap();

    // The admin's GitHub account can access it, but another org holds it.
    let (jar, setup_state) = begin_link(&state, admin).await;
    let error = finish_link(
        &state,
        admin,
        jar,
        &setup_state,
        ATTACKER_INSTALLATION,
        "attacker-code",
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert!(
        state
            .db
            .list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_empty()
    );

    // The index holds the same line when two inserts race.
    let duplicate = state
        .db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: DEFAULT_ORG_ID,
            name: "account-1717".to_string(),
            provider: "github".to_string(),
            connection_type: Some(crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE.to_string()),
            installation_id: Some(ATTACKER_INSTALLATION),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(crate::errors::is_database_unique_violation(&duplicate));
}

/// THREAT[TM-GITHUB-005]: without a user code the callback detours through
/// GitHub user authorization; the authorized state is bound to the installation
/// GitHub reported first, single use, and a rejected code proves nothing.
#[tokio::test]
async fn the_authorization_hop_is_bound_to_its_installation_and_single_use() {
    let github = MockServer::start().await;
    mock_github(&github).await;
    let state = github_state(&github).await;
    let admin = user_with_role(&state, OrgRole::Admin).await;
    let callback = |jar: CookieJar, setup_state: String, installation_id, code: Option<&str>| {
        github_callback(
            State(state.clone()),
            Ok(ResolvedOrg {
                user_id: Some(admin),
                ..test_org(admin)
            }),
            jar,
            Query(GitHubInstallationCallbackQuery {
                installation_id,
                setup_action: installation_id.map(|_| "install".to_string()),
                state: Some(setup_state),
                code: code.map(str::to_string),
            }),
        )
    };
    let no_links = || async {
        state
            .db
            .list_everruns_app_links(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_empty()
    };

    let (jar, setup_state) = begin_link(&state, admin).await;
    let (jar, redirect) = callback(jar, setup_state.clone(), Some(ATTACKER_INSTALLATION), None)
        .await
        .unwrap();
    assert!(location(redirect).starts_with(&format!(
        "{}/login/oauth/authorize?client_id=Iv1.test&state=",
        github.uri()
    )));
    let authorize_state = pending_state(&jar, "github").state;
    assert_ne!(authorize_state, setup_state);
    // Swapping the installation on the way back is rejected...
    let error = callback(
        jar.clone(),
        authorize_state.clone(),
        Some(VICTIM_INSTALLATION),
        Some("owner-code"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    // ...and the state cannot be replayed afterwards.
    let error = callback(jar, authorize_state, None, Some("owner-code"))
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(no_links().await);

    // A rejected or expired code proves nothing.
    let (jar, setup_state) = begin_link(&state, admin).await;
    let error = callback(
        jar,
        setup_state,
        Some(VICTIM_INSTALLATION),
        Some("expired-code"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(no_links().await);

    // The hop then the owner's code links the installation.
    let (jar, setup_state) = begin_link(&state, admin).await;
    let (jar, _) = callback(jar, setup_state, Some(VICTIM_INSTALLATION), None)
        .await
        .unwrap();
    let authorize_state = pending_state(&jar, "github").state;
    let _ = callback(jar, authorize_state, None, Some("owner-code"))
        .await
        .unwrap();
    assert!(!no_links().await);
}

#[tokio::test]
async fn members_cannot_attach_the_shared_app_to_an_agent_directly() {
    let github = MockServer::start().await;
    let state = github_state(&github).await;
    let user_id = user_with_role(&state, OrgRole::Member).await;
    let service = VirtualUserId::new();
    state
        .db
        .create_virtual_user(CreateVirtualUserRow {
            usage: "service".to_string(),
            org_id: DEFAULT_ORG_ID,
            id: service,
            name: "reviewer".to_string(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
    let error = github_authorize_inner(
        state.clone(),
        OAuthAuthority {
            org_id: DEFAULT_ORG_ID,
            caller: None,
            target_id: service.uuid(),
            management_user_id: Some(user_id),
            runtime_credential: None,
        },
        CookieJar::new(),
        None,
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(error.1.contains("Settings > Organization"), "{}", error.1);
}
