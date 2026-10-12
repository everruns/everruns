// Extractor enforcement of personal access token scopes
// (knowledge/security/pat-scopes.md, Tests item 4), through the real
// classification layer and the real `AuthUser` extractor.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use tower::ServiceExt;

use super::*;
use crate::auth::backend::AuthBackend;
use crate::auth::config::{AuthConfig, AuthMode};
use crate::auth::middleware::{AuthError, AuthMethod, AuthState, AuthUser};
use crate::auth::routes::AuthConfigResponse;

const READ_PAT: &str = "evr_pat_read";
const FULL_PAT: &str = "evr_pat_full";
const LEGACY_PAT: &str = "evr_pat_legacy";

/// Resolves a PAT's scope from its (fake) plaintext; JWTs carry no scope.
struct ScopedBackend;

fn user(pat_scope: Option<PatScope>) -> AuthUser {
    AuthUser {
        auth_method: if pat_scope.is_some() {
            AuthMethod::PersonalAccessToken
        } else {
            AuthMethod::Jwt
        },
        pat_scope,
        ..AuthUser::anonymous()
    }
}

#[async_trait]
impl AuthBackend for ScopedBackend {
    async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
        Ok(user(None))
    }

    async fn validate_personal_access_token(&self, token: &str) -> Result<AuthUser, AuthError> {
        let scope = match token {
            READ_PAT => PatScope::Read,
            FULL_PAT => PatScope::Full,
            LEGACY_PAT => PatScope::NoAccess,
            _ => return Err(AuthError::unauthorized("Invalid personal access token")),
        };
        Ok(user(Some(scope)))
    }

    fn auth_routes(&self) -> Option<Router> {
        None
    }

    fn auth_config_response(&self) -> AuthConfigResponse {
        AuthConfigResponse {
            mode: "full".into(),
            login_origin: None,
            password_auth_enabled: false,
            oauth_providers: vec![],
            signup_enabled: false,
            signup_email_confirm: false,
            captcha: None,
        }
    }
}

fn app(extra: ExtraRouteClasses) -> Router {
    async fn ok(_user: AuthUser) -> &'static str {
        "ok"
    }
    async fn command(_user: AuthUser, scope: Option<axum::Extension<PatScope>>) -> String {
        format!("{:?}", scope.map(|axum::Extension(scope)| scope))
    }
    let state = AuthState::new(
        AuthConfig {
            mode: AuthMode::Full,
            ..AuthConfig::default()
        },
        Arc::new(ScopedBackend),
    );
    let routes = Router::new()
        .route("/v1/agents", get(ok).post(ok))
        .route("/v1/users/me/export", get(ok))
        .route("/v1/commands/{name}", post(command))
        .route("/v1/unlisted", get(ok))
        .route("/v1/embedder/usage", get(ok).post(ok))
        .with_state(state);
    classify_api_routes(routes, "", extra)
}

async fn call(app: Router, method: &str, uri: &str, auth: Option<(&str, String)>) -> Response {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some((name, value)) = auth {
        request = request.header(name, value);
    }
    app.oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn bearer(token: &str) -> Option<(&'static str, String)> {
    Some((header::AUTHORIZATION.as_str(), format!("Bearer {token}")))
}

async fn assert_insufficient_scope(response: Response, what: &str) {
    assert_eq!(response.status(), StatusCode::FORBIDDEN, "{what}");
    assert_eq!(
        response.headers().get(header::WWW_AUTHENTICATE).unwrap(),
        "Bearer error=\"insufficient_scope\"",
        "{what}"
    );
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], INSUFFICIENT_SCOPE, "{what}: {body}");
    assert_eq!(body["detail"], INSUFFICIENT_SCOPE_MESSAGE, "{what}: {body}");
}

#[tokio::test]
async fn read_token_reaches_read_routes() {
    let app = app(ExtraRouteClasses::default());
    let response = call(app.clone(), "GET", "/v1/agents", bearer(READ_PAT)).await;
    assert_eq!(response.status(), StatusCode::OK);
    // `PerCommand` passes the route; the command is checked by the handler.
    let response = call(app, "POST", "/v1/commands/list_agents", bearer(READ_PAT)).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn read_token_is_denied_write_never_and_unclassified_routes() {
    let app = app(ExtraRouteClasses::default());
    for (method, uri) in [
        ("POST", "/v1/agents"),
        ("GET", "/v1/users/me/export"),
        ("GET", "/v1/unlisted"),
        ("GET", "/v1/embedder/usage"),
    ] {
        let response = call(app.clone(), method, uri, bearer(READ_PAT)).await;
        assert_insufficient_scope(response, &format!("{method} {uri}")).await;
    }
}

#[tokio::test]
async fn every_pat_header_form_is_checked() {
    let app = app(ExtraRouteClasses::default());
    for auth in [
        format!("Bearer {READ_PAT}"),
        format!("bearer {READ_PAT}"),
        format!("ApiKey {READ_PAT}"),
        READ_PAT.to_string(),
    ] {
        let response = call(
            app.clone(),
            "POST",
            "/v1/agents",
            Some((header::AUTHORIZATION.as_str(), auth.clone())),
        )
        .await;
        assert_insufficient_scope(response, &auth).await;
    }
}

#[tokio::test]
async fn legacy_narrow_token_reaches_nothing() {
    let app = app(ExtraRouteClasses::default());
    for (method, uri) in [("GET", "/v1/agents"), ("POST", "/v1/commands/list_agents")] {
        let response = call(app.clone(), method, uri, bearer(LEGACY_PAT)).await;
        assert_insufficient_scope(response, &format!("{method} {uri}")).await;
    }
}

#[tokio::test]
async fn full_token_jwt_and_cookie_are_unaffected() {
    let app = app(ExtraRouteClasses::default());
    let credentials = [
        bearer(FULL_PAT),
        bearer("header.payload.signature"),
        Some((
            header::COOKIE.as_str(),
            "access_token=header.payload.signature".to_string(),
        )),
    ];
    for auth in credentials {
        for (method, uri) in [
            ("POST", "/v1/agents"),
            ("GET", "/v1/users/me/export"),
            ("GET", "/v1/unlisted"),
        ] {
            let response = call(app.clone(), method, uri, auth.clone()).await;
            assert_eq!(response.status(), StatusCode::OK, "{auth:?} {method} {uri}");
        }
    }
}

#[tokio::test]
async fn unauthenticated_stays_401_before_scope() {
    let app = app(ExtraRouteClasses::default());
    let response = call(app.clone(), "POST", "/v1/agents", None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = call(app, "POST", "/v1/agents", bearer("evr_pat_unknown")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn extractor_leaves_the_pat_scope_for_the_command_check() {
    let app = app(ExtraRouteClasses::default());
    let body = |response: Response| async move {
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    };
    let uri = "/v1/commands/list_agents";
    let read = call(app.clone(), "POST", uri, bearer(READ_PAT)).await;
    assert_eq!(body(read).await, "Some(Read)");
    let full = call(app.clone(), "POST", uri, bearer(FULL_PAT)).await;
    assert_eq!(body(full).await, "Some(Full)");
    let jwt = call(app, "POST", uri, bearer("header.payload.signature")).await;
    assert_eq!(body(jwt).await, "None");
}

// --- Embedder hook ----------------------------------------------------------

#[tokio::test]
async fn embedder_classifications_open_only_unclassified_routes() {
    let mut extra = ExtraRouteClasses::default();
    extra.insert("GET", "/v1/embedder/usage", RouteAccess::Read);
    extra.insert("POST", "/v1/embedder/usage", RouteAccess::Write);
    // Built-in classification wins: these do not open anything.
    extra.insert("POST", "/v1/agents", RouteAccess::Read);
    extra.insert("GET", "/v1/users/me/export", RouteAccess::Read);
    let app = app(extra);

    let response = call(app.clone(), "GET", "/v1/embedder/usage", bearer(READ_PAT)).await;
    assert_eq!(response.status(), StatusCode::OK);
    for (method, uri) in [
        ("POST", "/v1/embedder/usage"),
        ("POST", "/v1/agents"),
        ("GET", "/v1/users/me/export"),
    ] {
        let response = call(app.clone(), method, uri, bearer(READ_PAT)).await;
        assert_insufficient_scope(response, &format!("{method} {uri}")).await;
    }
}

#[test]
#[should_panic(expected = "cannot be PerCommand")]
fn embedder_cannot_register_per_command() {
    ExtraRouteClasses::default().insert("POST", "/v1/embedder/run", RouteAccess::PerCommand);
}

// --- Per-command check ------------------------------------------------------

fn command_of_class(class: RouteAccess) -> &'static CommandDescriptor {
    inventory::iter::<CommandDescriptor>
        .into_iter()
        .find(|desc| {
            crate::services::command_catalog::catalog::is_exposed(&(desc.meta)())
                && classify_command(desc) == class
        })
        .unwrap_or_else(|| panic!("no exposed command of class {class:?}"))
}

#[test]
fn command_check_follows_the_command_class() {
    let read = command_of_class(RouteAccess::Read);
    let write = command_of_class(RouteAccess::Write);
    let user = Some(uuid::Uuid::nil());

    assert!(enforce_command(Some(PatScope::Read), user, read).is_ok());
    let denied = enforce_command(Some(PatScope::Read), user, write).unwrap_err();
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.code, Some(INSUFFICIENT_SCOPE));
    assert!(enforce_command(Some(PatScope::NoAccess), user, read).is_err());

    for desc in [read, write] {
        assert!(enforce_command(Some(PatScope::Full), user, desc).is_ok());
        assert!(enforce_command(None, user, desc).is_ok(), "non-PAT caller");
    }
}
