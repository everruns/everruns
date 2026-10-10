//! Connected AI clients (MCP OAuth grants), end to end.
//!
//! Drives the real flow in [`AuthMode::Full`]: register a client, approve it on
//! the consent page, exchange the code, then check the token on the `/mcp`
//! authentication path, list the client over the REST API, revoke it, and
//! confirm the token, its refresh token and a re-approval behave.
//!
//! Run with: `cargo test -p everruns-server --test domain mcp_connected_clients_test::`

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use everruns_server::api::state::ApiState;
use everruns_server::auth::backend::AuthBackend;
use everruns_server::auth::config::{AuthConfig, AuthMode, JwtConfig};
use everruns_server::auth::middleware::extract_mcp_auth_user;
use everruns_server::auth::{self, AuthState, BuiltinAuthBackend};
use everruns_server::services::CapabilityService;
use everruns_server::storage::StorageBackend;

const REDIRECT_URI: &str = "http://localhost:9999/callback";
// RFC 7636 Appendix B test vector.
const CODE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

struct Harness {
    router: Router,
    auth_state: AuthState,
}

fn harness() -> Harness {
    let db = Arc::new(StorageBackend::test_database());
    let config = AuthConfig {
        mode: AuthMode::Full,
        frontend_url: "http://localhost:3000".to_string(),
        jwt: JwtConfig {
            secret: "test-secret-for-connected-clients".to_string(),
            access_token_lifetime: Duration::from_secs(900),
            refresh_token_lifetime: Duration::from_secs(86400),
        },
        ..Default::default()
    };
    let backend = BuiltinAuthBackend::new(
        config.clone(),
        db.clone(),
        Arc::new(everruns_server::platform::oss_host_composition()),
    );
    let auth_state = AuthState::new(config, Arc::new(backend.clone()));
    let api_state = ApiState::basic(
        db.clone(),
        None,
        Arc::new(CapabilityService::new(db, None)),
        auth_state.clone(),
    );
    let mut router = auth::routes(backend.clone())
        .merge(everruns_server::api::connected_clients::routes(api_state));
    if let Some(public) = backend.public_routes() {
        router = router.merge(public);
    }
    Harness { router, auth_state }
}

async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<(&str, String)>,
    cookie: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some((content_type, _)) = &body {
        builder = builder.header(header::CONTENT_TYPE, *content_type);
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let req = builder
        .body(body.map(|(_, b)| Body::from(b)).unwrap_or_else(Body::empty))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn register_user(router: &Router, email: &str) -> String {
    let (status, headers, body) = send(
        router,
        Method::POST,
        "/v1/auth/register",
        Some((
            "application/json",
            json!({"name": "Ava Clients", "email": email, "password": "longenough123"}).to_string(),
        )),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let cookie = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|c| c.starts_with("access_token="))
        .expect("session cookie");
    cookie.split(';').next().unwrap().to_string()
}

async fn register_client(router: &Router) -> String {
    let (status, _, body) = send(
        router,
        Method::POST,
        "/oauth/register",
        Some((
            "application/json",
            json!({"client_name": "Cursor", "redirect_uris": [REDIRECT_URI]}).to_string(),
        )),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    serde_json::from_str::<Value>(&body).unwrap()["client_id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Approve the client on the consent page and return the authorization code.
async fn approve(router: &Router, client_id: &str, cookie: &str) -> String {
    let path = format!(
        "/oauth/authorize?response_type=code&client_id={client_id}\
         &redirect_uri={REDIRECT_URI}&scope=mcp&state=s1\
         &code_challenge={CODE_CHALLENGE}&code_challenge_method=S256"
    );
    let (status, _, html) = send(router, Method::GET, &path, None, Some(cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let marker = r#"name="csrf_token" value=""#;
    let start = html.find(marker).unwrap() + marker.len();
    let csrf = &html[start..start + html[start..].find('"').unwrap()];

    let form = format!(
        "client_id={client_id}&redirect_uri={REDIRECT_URI}&response_type=code\
         &code_challenge={CODE_CHALLENGE}&code_challenge_method=S256\
         &state=s1&scope=mcp&csrf_token={csrf}"
    );
    let (status, headers, _) = send(
        router,
        Method::POST,
        "/oauth/authorize",
        Some(("application/x-www-form-urlencoded", form)),
        Some(cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = headers[header::LOCATION].to_str().unwrap();
    location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string()
}

async fn token_request(router: &Router, body: Value) -> (StatusCode, Value) {
    let (status, _, body) = send(
        router,
        Method::POST,
        "/oauth/token",
        Some(("application/json", body.to_string())),
        None,
    )
    .await;
    (status, serde_json::from_str(&body).unwrap())
}

async fn exchange(router: &Router, client_id: &str, code: &str) -> Value {
    let (status, body) = token_request(
        router,
        json!({
            "grant_type": "authorization_code",
            "code": code,
            "client_id": client_id,
            "redirect_uri": REDIRECT_URI,
            "code_verifier": CODE_VERIFIER,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn claims(token: &str) -> Value {
    use base64::Engine;
    let payload = token.split('.').nth(1).unwrap();
    serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap(),
    )
    .unwrap()
}

/// Authenticate a bearer token the way the `/mcp` endpoint does.
async fn mcp_auth(auth_state: &AuthState, token: &str) -> Result<(), StatusCode> {
    let resource = claims(token)["aud"].as_str().unwrap().to_string();
    let (mut parts, _) = Request::builder()
        .uri("/mcp")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(())
        .unwrap()
        .into_parts();
    extract_mcp_auth_user(&mut parts, auth_state, Some(&resource))
        .await
        .map(|_| ())
        .map_err(|e| e.status)
}

#[tokio::test]
async fn revoked_client_is_cut_off_and_reapproval_starts_a_new_grant() {
    let Harness { router, auth_state } = harness();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let cookie = register_user(&router, &format!("clients-{nanos}@example.com")).await;
    let client_id = register_client(&router).await;

    // Approve and exchange: the access token names the client and its grant.
    let code = approve(&router, &client_id, &cookie).await;
    let tokens = exchange(&router, &client_id, &code).await;
    let access = tokens["access_token"].as_str().unwrap().to_string();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_string();
    let token_claims = claims(&access);
    assert_eq!(token_claims["client_id"], client_id.as_str());
    let grant_id = token_claims["grant_id"].as_str().unwrap().to_string();
    assert_eq!(mcp_auth(&auth_state, &access).await, Ok(()));

    // The person sees the client, with its redirect host.
    let (status, _, body) = send(
        &router,
        Method::GET,
        "/v1/user/connected-clients",
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: Value = serde_json::from_str(&body).unwrap();
    let items = list["data"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], grant_id.as_str());
    assert_eq!(items[0]["client_name"], "Cursor");
    assert_eq!(items[0]["redirect_hosts"], json!(["localhost"]));
    assert_eq!(items[0]["access"], "read_and_run");
    assert_eq!(items[0]["all_organizations"], true);
    assert!(
        items[0]["last_used_at"].is_string(),
        "the /mcp check stamps last_used_at: {body}"
    );

    // Revoke.
    let (status, _, body) = send(
        &router,
        Method::DELETE,
        &format!("/v1/user/connected-clients/{grant_id}"),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // The still-unexpired access token is rejected on /mcp.
    assert_eq!(
        mcp_auth(&auth_state, &access).await,
        Err(StatusCode::UNAUTHORIZED)
    );
    // Its refresh token is gone.
    let (status, body) = token_request(
        &router,
        json!({"grant_type": "refresh_token", "refresh_token": refresh, "client_id": client_id}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_grant");
    // And the list is empty; revoking twice is a 404.
    let (_, _, body) = send(
        &router,
        Method::GET,
        "/v1/user/connected-clients",
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["data"],
        json!([])
    );
    let (status, _, _) = send(
        &router,
        Method::DELETE,
        &format!("/v1/user/connected-clients/{grant_id}"),
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Reconnecting creates a new grant; the old token stays dead.
    let code = approve(&router, &client_id, &cookie).await;
    let tokens = exchange(&router, &client_id, &code).await;
    let new_access = tokens["access_token"].as_str().unwrap();
    assert_ne!(claims(new_access)["grant_id"], grant_id.as_str());
    assert_eq!(mcp_auth(&auth_state, new_access).await, Ok(()));
    assert_eq!(
        mcp_auth(&auth_state, &access).await,
        Err(StatusCode::UNAUTHORIZED)
    );
}

#[tokio::test]
async fn connected_clients_require_a_session() {
    let Harness { router, .. } = harness();
    let (status, _, _) = send(
        &router,
        Method::GET,
        "/v1/user/connected-clients",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
