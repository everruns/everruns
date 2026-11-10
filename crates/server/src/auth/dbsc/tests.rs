use super::*;
use crate::auth::backend::AuthBackend;
use crate::auth::config::{AuthConfig, AuthMode};
use crate::storage::StorageBackend;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use axum::{Router, body::Body, http::Request as HttpRequest};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use std::sync::Arc;
use tower::ServiceExt;

struct DeviceKey {
    pair: EcdsaKeyPair,
}

impl DeviceKey {
    fn new() -> Self {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .unwrap();
        Self {
            pair: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref())
                .unwrap(),
        }
    }

    fn jwk(&self) -> Value {
        let point = self.pair.public_key().as_ref();
        json!({"kty": "EC", "crv": "P-256", "x": B64.encode(&point[1..33]), "y": B64.encode(&point[33..65])})
    }

    fn proof_with(&self, header: Value, jti: &str) -> String {
        let input = format!(
            "{}.{}",
            B64.encode(header.to_string()),
            B64.encode(json!({"jti": jti}).to_string())
        );
        let sig = self
            .pair
            .sign(&SystemRandom::new(), input.as_bytes())
            .unwrap();
        format!("{input}.{}", B64.encode(sig.as_ref()))
    }

    fn registration(&self, jti: &str) -> String {
        self.proof_with(
            json!({"typ": "dbsc+jwt", "alg": "ES256", "jwk": self.jwk()}),
            jti,
        )
    }

    fn refresh(&self, jti: &str) -> String {
        self.proof_with(json!({"typ": "dbsc+jwt", "alg": "ES256"}), jti)
    }
}

#[test]
fn registration_proofs_bind_their_key_and_our_challenge() {
    let key = DeviceKey::new();
    let (alg, stored) = verify_registration(&key.registration("c1"), "c1").unwrap();
    assert_eq!(alg, ProofAlg::Es256);
    assert_eq!(
        verify_registration(&key.registration("c1"), "c2"),
        Err(ProofError("challenge mismatch"))
    );
    // The stored key verifies later refresh proofs; another key does not.
    assert_eq!(
        verify_refresh(&key.refresh("n"), alg, &stored).unwrap(),
        "n"
    );
    let other = DeviceKey::new();
    assert!(verify_refresh(&other.refresh("n"), alg, &stored).is_err());
    // Refresh proofs never carry a key, and the algorithm must not change.
    assert!(verify_refresh(&key.registration("n"), alg, &stored).is_err());
    assert!(verify_refresh(&key.refresh("n"), ProofAlg::Rs256, &stored).is_err());
}

#[test]
fn malformed_unsigned_or_mistyped_proofs_are_refused() {
    let key = DeviceKey::new();
    let forged = DeviceKey::new().proof_with(
        json!({"typ": "dbsc+jwt", "alg": "ES256", "jwk": key.jwk()}),
        "c",
    );
    assert_eq!(
        verify_registration(&forged, "c"),
        Err(ProofError("bad signature"))
    );
    let none = format!(
        "{}.{}.",
        B64.encode(json!({"typ": "dbsc+jwt", "alg": "none"}).to_string()),
        B64.encode(json!({"jti": "c"}).to_string())
    );
    assert!(verify_registration(&none, "c").is_err());
    let typed = key.proof_with(
        json!({"typ": "JWT+x", "alg": "ES256", "jwk": key.jwk()}),
        "c",
    );
    assert_eq!(
        verify_registration(&typed, "c"),
        Err(ProofError("proof type mismatch"))
    );
    let legacy = key.proof_with(json!({"typ": "jwt", "alg": "ES256", "jwk": key.jwk()}), "c");
    assert!(verify_registration(&legacy, "c").is_ok());
    let mismatched = key.proof_with(
        json!({"typ": "dbsc+jwt", "alg": "RS256", "jwk": key.jwk()}),
        "c",
    );
    assert!(verify_registration(&mismatched, "c").is_err());
    let small: Jwk =
        serde_json::from_value(json!({"kty": "RSA", "n": B64.encode([0xffu8; 128]), "e": "AQAB"}))
            .unwrap();
    assert_eq!(
        device_key(ProofAlg::Rs256, &small).unwrap_err(),
        ProofError("RSA key too small")
    );
}

#[test]
fn registration_paths_survive_a_proxy_prefix() {
    assert_eq!(relative_path("/v1/auth/login", REGISTER_PATH), "dbsc/start");
    assert_eq!(
        relative_path("/v1/auth/callback/google", REGISTER_PATH),
        "../dbsc/start"
    );
    assert_eq!(
        relative_path("/v1/auth/oidc/callback", REGISTER_PATH),
        "../dbsc/start"
    );
    assert_eq!(relative_path(REGISTER_PATH, REFRESH_PATH), "refresh");
    assert_eq!(
        relative_path("/login", "/v1/auth/dbsc/start"),
        "v1/auth/dbsc/start"
    );
    assert_eq!(
        registration_header("/v1/auth/login", "c"),
        "(ES256 RS256);path=\"dbsc/start\";challenge=\"c\""
    );
}

#[test]
fn sf_strings_parse_with_escapes() {
    assert_eq!(sf_string("\"a\\\"b\";id=\"x\"").as_deref(), Some("a\"b"));
    assert_eq!(sf_string("plain"), None);
    assert_eq!(sf_string("\"open"), None);
}

#[test]
fn only_new_refresh_cookies_offer_registration() {
    let mut headers = HeaderMap::new();
    headers.append(
        SET_COOKIE,
        "access_token=a; Path=/; Max-Age=900".parse().unwrap(),
    );
    assert_eq!(issued_refresh_token(&headers), None);
    headers.append(
        SET_COOKIE,
        "refresh_token=; Path=/; Max-Age=0".parse().unwrap(),
    );
    assert_eq!(issued_refresh_token(&headers), None);
    headers.append(
        SET_COOKIE,
        "refresh_token=r; Path=/; HttpOnly; Max-Age=2592000"
            .parse()
            .unwrap(),
    );
    assert_eq!(issued_refresh_token(&headers).as_deref(), Some("r"));
}

// ---- HTTP flow through the real auth router and database ----

fn app() -> (Router, BuiltinAuthBackend) {
    let state = BuiltinAuthBackend::new(
        AuthConfig {
            mode: AuthMode::Full,
            ..Default::default()
        },
        Arc::new(StorageBackend::test_database()),
        Arc::new(crate::platform::oss_host_composition()),
    );
    (state.auth_routes().unwrap(), state)
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Reply {
    fn cookie(&self, name: &str) -> Option<Cookie<'static>> {
        self.headers.get_all(SET_COOKIE).iter().find_map(|v| {
            let c = Cookie::parse(v.to_str().ok()?.to_string()).ok()?;
            (c.name() == name).then_some(c)
        })
    }
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    }
    fn challenge(&self) -> String {
        sf_string(&self.header(CHALLENGE_HEADER).expect("challenge header")).unwrap()
    }
}

async fn call(app: &Router, path: &str, headers: &[(&str, String)], body: Option<Value>) -> Reply {
    let mut request = HttpRequest::builder().method("POST").uri(path);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let body = match body {
        Some(json) => {
            request = request.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn sign_up(app: &Router, email: &str) -> Reply {
    let reply = call(
        app,
        "/v1/auth/register",
        &[],
        Some(json!({"email": email, "password": "password12345"})),
    )
    .await;
    assert!(reply.status.is_success(), "register: {}", reply.status);
    reply
}

fn sf(value: &str) -> String {
    format!("\"{value}\"")
}

#[tokio::test]
async fn browser_session_binds_refreshes_and_ends_with_sign_out() {
    let (app, state) = app();
    let login = sign_up(&app, "bound@example.com").await;
    let refresh_token = login.cookie("refresh_token").unwrap().value().to_string();
    let offer = login
        .header(REGISTRATION_HEADER)
        .expect("sign-in offers DBSC");
    assert_eq!(
        offer,
        registration_header("/v1/auth/register", &registration_challenge(&refresh_token))
    );
    let key = DeviceKey::new();
    let cookie = |name: &str, value: &str| ("cookie", format!("{name}={value}"));

    // Registration needs the new session's cookie and a proof over its challenge.
    let proof = key.registration(&registration_challenge(&refresh_token));
    let no_cookie = call(&app, REGISTER_PATH, &[(RESPONSE_HEADER, sf(&proof))], None).await;
    assert_eq!(no_cookie.status, StatusCode::BAD_REQUEST);
    let wrong = call(
        &app,
        REGISTER_PATH,
        &[
            cookie("refresh_token", &refresh_token),
            (RESPONSE_HEADER, sf(&key.registration("other"))),
        ],
        None,
    )
    .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    let bound = call(
        &app,
        REGISTER_PATH,
        &[
            cookie("refresh_token", &refresh_token),
            (RESPONSE_HEADER, sf(&proof)),
        ],
        None,
    )
    .await;
    assert_eq!(bound.status, StatusCode::OK);
    assert_eq!(bound.header(REGISTRATION_HEADER), None);
    let session_id = bound.body["session_identifier"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(bound.body["refresh_url"], "refresh");
    assert_eq!(bound.body["credentials"][0]["name"], "access_token");
    assert_eq!(
        bound.cookie("refresh_token").unwrap().value(),
        "",
        "long-lived cookie removed"
    );
    assert_eq!(bound.cookie(SESSION_COOKIE).unwrap().value(), session_id);
    let access = bound.cookie("access_token").unwrap();
    assert!(
        state
            .jwt_service
            .validate_access_token(access.value())
            .is_ok()
    );
    let mut challenge = bound.challenge();

    // A second key cannot take the session over.
    let again = call(
        &app,
        REGISTER_PATH,
        &[
            cookie("refresh_token", &refresh_token),
            (
                RESPONSE_HEADER,
                sf(&DeviceKey::new().registration(&registration_challenge(&refresh_token))),
            ),
        ],
        None,
    )
    .await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);

    // The refresh token copied before binding is dead for the classic refresh.
    let stolen = call(
        &app,
        "/v1/auth/refresh",
        &[cookie("refresh_token", &refresh_token)],
        None,
    )
    .await;
    assert_eq!(stolen.status, StatusCode::UNAUTHORIZED);

    // Refresh: challenge first, then only the device key earns a new access token.
    let id = || (SESSION_ID_HEADER, sf(&session_id));
    let ask = call(&app, REFRESH_PATH, &[id()], None).await;
    assert_eq!(ask.status, StatusCode::FORBIDDEN);
    assert_eq!(
        ask.challenge(),
        challenge,
        "a live challenge is reused, not churned"
    );
    let foreign = call(
        &app,
        REFRESH_PATH,
        &[
            id(),
            (RESPONSE_HEADER, sf(&DeviceKey::new().refresh(&challenge))),
        ],
        None,
    )
    .await;
    assert_eq!(foreign.status, StatusCode::BAD_REQUEST);
    let good = key.refresh(&challenge);
    let renewed = call(
        &app,
        REFRESH_PATH,
        &[id(), (RESPONSE_HEADER, sf(&good))],
        None,
    )
    .await;
    assert_eq!(renewed.status, StatusCode::OK);
    assert_eq!(renewed.body["session_identifier"], session_id.as_str());
    assert!(
        state
            .jwt_service
            .validate_access_token(renewed.cookie("access_token").unwrap().value())
            .is_ok()
    );
    let next = renewed.challenge();
    assert_ne!(next, challenge);
    // Replaying a proof answers a consumed challenge and earns only a new ask.
    let replay = call(
        &app,
        REFRESH_PATH,
        &[id(), (RESPONSE_HEADER, sf(&good))],
        None,
    )
    .await;
    assert_eq!(replay.status, StatusCode::FORBIDDEN);
    assert_eq!(replay.challenge(), next);
    challenge = next;
    let renewed = call(
        &app,
        REFRESH_PATH,
        &[id(), (RESPONSE_HEADER, sf(&key.refresh(&challenge)))],
        None,
    )
    .await;
    assert_eq!(renewed.status, StatusCode::OK);

    // Unknown sessions and missing identifiers.
    let unknown = call(&app, REFRESH_PATH, &[(SESSION_ID_HEADER, sf("nope"))], None).await;
    assert_eq!(unknown.body, json!({"continue": false}));
    assert_eq!(
        call(&app, REFRESH_PATH, &[], None).await.status,
        StatusCode::BAD_REQUEST
    );

    // Sign-out through the DBSC session cookie revokes the bound row.
    let out = call(
        &app,
        "/v1/auth/logout",
        &[cookie(SESSION_COOKIE, &session_id)],
        None,
    )
    .await;
    assert!(out.status.is_success());
    assert_eq!(out.cookie(SESSION_COOKIE).unwrap().value(), "");
    let after = call(&app, REFRESH_PATH, &[id()], None).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(after.body, json!({"continue": false}));
}

#[tokio::test]
async fn browsers_without_dbsc_keep_the_classic_refresh() {
    let (app, _state) = app();
    let login = sign_up(&app, "classic@example.com").await;
    let refresh_token = login.cookie("refresh_token").unwrap().value().to_string();
    let rotated = call(
        &app,
        "/v1/auth/refresh",
        &[("cookie", format!("refresh_token={refresh_token}"))],
        None,
    )
    .await;
    assert_eq!(rotated.status, StatusCode::OK);
    // Each rotated refresh token is offered for binding too.
    let next = rotated.cookie("refresh_token").unwrap().value().to_string();
    assert_eq!(
        rotated.header(REGISTRATION_HEADER),
        Some(registration_header(
            "/v1/auth/refresh",
            &registration_challenge(&next)
        ))
    );
}
