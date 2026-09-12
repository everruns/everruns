//! MCP OAuth authorize user switching, end to end.
//!
//! Drives the real browser flow in [`AuthMode::Full`] with two users: register
//! a client, load the authorize page as Alice, switch to Bob, approve as Bob,
//! and exchange the code for tokens issued to Bob. It also locks in the
//! cross-user consent-token rejection that makes the re-GET (via the Switch
//! account link) required after switching.
//!
//! Run with: `cargo test -p everruns-server --test mcp_oauth_user_switch_test`

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use everruns_server::auth::backend::AuthBackend;
use everruns_server::auth::config::{AuthConfig, AuthMode, JwtConfig};
use everruns_server::auth::{self, BuiltinAuthBackend};
use everruns_server::storage::StorageBackend;

const REDIRECT_URI: &str = "http://localhost:9999/callback";
// RFC 7636 Appendix B test vector, so no S256 helper (or extra dep) is needed.
const CODE_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CODE_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

/// Full-mode app router including the public MCP OAuth routes.
async fn oauth_router() -> Router {
    let db = Arc::new(StorageBackend::in_memory());
    let config = AuthConfig {
        mode: AuthMode::Full,
        frontend_url: "http://localhost:3000".to_string(),
        jwt: JwtConfig {
            secret: "test-secret-for-oauth-switch-tests".to_string(),
            access_token_lifetime: Duration::from_secs(900),
            refresh_token_lifetime: Duration::from_secs(86400),
        },
        ..Default::default()
    };
    let backend = BuiltinAuthBackend::new(
        config,
        db.clone(),
        Arc::new(everruns_server::platform::oss_host_composition()),
    );
    let mut router = auth::routes(backend.clone());
    if let Some(public) = backend.public_routes() {
        router = router.merge(public);
    }
    router
}

async fn send_raw(
    router: Router,
    method: Method,
    uri: &str,
    form: Option<&str>,
    cookie: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if form.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let req = if let Some(body) = form {
        builder.body(Body::from(body.to_string())).unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

fn extract_cookie_value(set_cookies: &[String], name: &str) -> Option<String> {
    for cookie in set_cookies {
        if cookie.starts_with(&format!("{name}=")) {
            return Some(
                cookie
                    .split(';')
                    .next()
                    .unwrap()
                    .trim_start_matches(&format!("{name}="))
                    .to_string(),
            );
        }
    }
    None
}

fn session_cookie(set_cookies: &[String]) -> String {
    let value = extract_cookie_value(set_cookies, "access_token").expect("access_token cookie");
    format!("access_token={value}")
}

/// Register a user; returns their session cookie.
async fn register_user(router: Router, name: &str, email: &str) -> String {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/auth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"name": name, "email": email, "password": "longenough123"}).to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "register {email}");
    let cookies: Vec<String> = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    session_cookie(&cookies)
}

fn extract_csrf(html: &str) -> String {
    let marker = r#"name="csrf_token" value=""#;
    let start = html.find(marker).expect("csrf token in page") + marker.len();
    let end = html[start..].find('"').unwrap() + start;
    html[start..end].to_string()
}

fn extract_switch_href(html: &str) -> Option<String> {
    let marker = ">Switch account</a>";
    let end = html.find(marker)?;
    let href_start = html[..end].rfind("href=\"")? + 6;
    let href_end = html[href_start..].find('"')? + href_start;
    Some(html[href_start..href_end].to_string())
}

/// Minimal base64url decode, just enough to read a JWT payload in tests.
fn base64url_decode(segment: &str) -> Vec<u8> {
    let mut bits: u32 = 0;
    let mut width = 0;
    let mut out = Vec::new();
    for ch in segment.bytes() {
        let val = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => break,
            _ => panic!("unexpected base64url char"),
        } as u32;
        bits = (bits << 6) | val;
        width += 6;
        if width >= 8 {
            width -= 8;
            out.push((bits >> width) as u8);
        }
    }
    out
}

fn jwt_sub(token: &str) -> String {
    let payload: Value = serde_json::from_slice(&base64url_decode(
        token.split('.').nth(1).expect("jwt payload"),
    ))
    .unwrap();
    payload["sub"].as_str().unwrap().to_string()
}

fn authorize_path(client_id: &str) -> String {
    format!(
        "/oauth/authorize?response_type=code&client_id={client_id}\
         &redirect_uri={REDIRECT_URI}&scope=mcp&state=xyz-state\
         &code_challenge={CODE_CHALLENGE}&code_challenge_method=S256"
    )
}

fn approve_form(client_id: &str, csrf_token: &str) -> String {
    format!(
        "client_id={client_id}&redirect_uri={REDIRECT_URI}&response_type=code\
         &code_challenge={CODE_CHALLENGE}&code_challenge_method=S256\
         &state=xyz-state&scope=mcp&csrf_token={csrf_token}"
    )
}

#[tokio::test]
async fn test_mcp_oauth_authorize_user_switch_end_to_end() {
    // One router (one database) for the whole flow.
    let router = oauth_router().await;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    // Register a public client.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/oauth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"client_name": "switch-test", "redirect_uris": [REDIRECT_URI]}).to_string(),
        ))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let client_id = serde_json::from_slice::<Value>(&bytes).unwrap()["client_id"]
        .as_str()
        .unwrap()
        .to_string();

    let alice = register_user(
        router.clone(),
        "Alice Switch",
        &format!("alice-switch-{nanos}@example.com"),
    )
    .await;
    let bob = register_user(
        router.clone(),
        "Bob Switch",
        &format!("bob-switch-{nanos}@example.com"),
    )
    .await;

    let path = authorize_path(&client_id);

    // Signed out: bounce to login, preserving the authorize request.
    let (status, headers, _) = send_raw(router.clone(), Method::GET, &path, None, None).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    let login_url = headers[header::LOCATION].to_str().unwrap().to_string();
    assert!(
        login_url.contains("/login?return_to=%2Foauth%2Fauthorize%3F"),
        "unexpected login url: {login_url}"
    );

    // Alice's page names her and offers the same login round-trip for switching.
    let (status, _, alice_html) =
        send_raw(router.clone(), Method::GET, &path, None, Some(&alice)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(alice_html.contains("Alice Switch"));
    let switch_href = extract_switch_href(&alice_html).expect("switch account link");
    assert_eq!(
        // Page HTML escapes `&` as `&amp;`.
        switch_href.replace("&amp;", "&"),
        login_url,
        "switch link must reuse the login round-trip"
    );
    let alice_token = extract_csrf(&alice_html);

    // Switching sessions re-renders the page for Bob with a fresh token.
    let (status, _, bob_html) =
        send_raw(router.clone(), Method::GET, &path, None, Some(&bob)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(bob_html.contains("Bob Switch"));
    assert!(!bob_html.contains("Alice Switch"));
    let bob_token = extract_csrf(&bob_html);
    assert_ne!(alice_token, bob_token);

    // Alice's token is useless in Bob's session: the switch must re-GET.
    let (status, _, body) = send_raw(
        router.clone(),
        Method::POST,
        "/oauth/authorize",
        Some(&approve_form(&client_id, &alice_token)),
        Some(&bob),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // Bob approves: a code is issued to his redirect URI.
    let (status, headers, _) = send_raw(
        router.clone(),
        Method::POST,
        "/oauth/authorize",
        Some(&approve_form(&client_id, &bob_token)),
        Some(&bob),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let redirect = headers[header::LOCATION].to_str().unwrap().to_string();
    assert!(redirect.starts_with(REDIRECT_URI), "{redirect}");
    assert!(redirect.contains("state=xyz-state"), "{redirect}");
    let code = redirect
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();

    // The code exchanges for tokens owned by Bob.
    let form = format!(
        "grant_type=authorization_code&code={code}&redirect_uri={REDIRECT_URI}\
         &client_id={client_id}&code_verifier={CODE_VERIFIER}"
    );
    let (status, _, token_body) = send_raw(
        router.clone(),
        Method::POST,
        "/oauth/token",
        Some(&form),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{token_body}");
    let access = serde_json::from_str::<Value>(&token_body).unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/v1/auth/me")
        .header(header::COOKIE, &bob)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let bob_id = serde_json::from_slice::<Value>(&bytes).unwrap()["id"].to_string();
    assert_eq!(jwt_sub(&access).trim_matches('"'), bob_id.trim_matches('"'));
}
