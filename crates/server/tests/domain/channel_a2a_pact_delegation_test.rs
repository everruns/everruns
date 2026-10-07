//! PACT Delegated profile on an A2A channel: the OAuth 2.0 device-code
//! authorization server under `/v1/a2a/{channel_id}/oauth/`. A stand-in
//! company signs the sign-in assertions its login would POST to the consent
//! page, so the whole flow runs in-process: request scopes, sign in, consent,
//! redeem the device code, refresh.

use crate::channel_a2a_pact_test::{
    ISSUER, PersonalAgentKey, assert_no_a2a_body, call, create_channel, pact_config, server,
};
use crate::test_harness::{TestResponse, TestServer};

use axum::http::{Method, StatusCode};
use base64::Engine as _;
use serde_json::{Value, json};

const BRAND_ISSUER: &str = "https://brand.example";
const CONNECTED: &str = "https://brand.example/connected";

fn delegated_config(pa: &PersonalAgentKey, brand: &PersonalAgentKey) -> Value {
    let mut config = pact_config(pa);
    config["delegation"] = json!({
        "brand_name": "Skyline",
        "login_url": "https://brand.example/login",
        "login_issuer": BRAND_ISSUER,
        "login_jwks": { "keys": [brand.jwk] },
        "connected_url": CONNECTED,
        "scopes": [
            { "id": "flights:read", "description": "View your flights" },
            { "id": "flights:rebook", "description": "Rebook your flights" },
        ],
    });
    config
}

struct Fixture {
    server: TestServer,
    channel: String,
    pa: PersonalAgentKey,
    brand: PersonalAgentKey,
}

async fn fixture() -> Fixture {
    let (server, _) = server().await;
    let pa = PersonalAgentKey::generate();
    let brand = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(delegated_config(&pa, &brand))).await;
    Fixture {
        server,
        channel,
        pa,
        brand,
    }
}

impl Fixture {
    async fn form(&self, route: &str, token: Option<&str>, form: &[(&str, &str)]) -> TestResponse {
        let auth = token.map(|token| format!("Bearer {token}"));
        let mut headers = vec![("content-type", "application/x-www-form-urlencoded")];
        if let Some(auth) = auth.as_deref() {
            headers.push(("authorization", auth));
        }
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        self.server
            .request_raw(
                Method::POST,
                &format!("/v1/a2a/{}/oauth/{route}", self.channel),
                headers,
                body.into_bytes(),
            )
            .await
    }

    fn pa_token(&self) -> String {
        self.pa.token("pa-user-1")
    }

    async fn start(&self, scope: &str) -> Value {
        self.form(
            "device_authorization",
            Some(&self.pa_token()),
            &[("client_id", ISSUER), ("scope", scope)],
        )
        .await
        .assert_status(StatusCode::OK)
        .json()
    }

    async fn poll(&self, device_code: &str) -> TestResponse {
        self.form(
            "token",
            Some(&self.pa_token()),
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
                ("client_id", ISSUER),
            ],
        )
        .await
    }

    async fn metadata(&self) -> Value {
        self.server
            .get(&format!(
                "/v1/a2a/{}/oauth/.well-known/oauth-authorization-server",
                self.channel
            ))
            .await
            .assert_status(StatusCode::OK)
            .json()
    }

    /// The company's sign-in assertion for `user_code`, as its login would
    /// POST it to the consent page.
    async fn assertion(&self, user_code: &str, overrides: Value) -> String {
        let consent = format!(
            "{}/consent",
            self.metadata().await["issuer"].as_str().unwrap()
        );
        let mut claims = json!({
            "iss": BRAND_ISSUER,
            "aud": consent,
            "sub": "brand-user-42",
            "jti": uuid::Uuid::new_v4().to_string(),
            "user_code": user_code,
            "email": "jane@example.com",
        });
        for (key, value) in overrides.as_object().unwrap() {
            claims[key] = value.clone();
        }
        self.brand.token_with(claims)
    }

    /// Sign in with `assertion`, then answer the consent page.
    async fn consent(&self, assertion: &str, decision: &str, scopes: &[&str]) -> TestResponse {
        let page = self
            .form("consent", None, &[("assertion", assertion)])
            .await
            .assert_status(StatusCode::OK)
            .text();
        let session = input_value(&page, "session");
        let mut form = vec![("session", session.as_str()), ("decision", decision)];
        form.extend(scopes.iter().map(|scope| ("scope", *scope)));
        self.form("consent/decision", None, &form).await
    }
}

fn input_value(html: &str, name: &str) -> String {
    let at = html.find(&format!("name=\"{name}\" value=\"")).expect(name);
    let rest = &html[at + name.len() + 15..];
    rest[..rest.find('"').unwrap()].to_string()
}

fn oauth_error(response: TestResponse, status: StatusCode, error: &str) {
    let response = response.assert_status(status);
    let body: Value = response.json();
    assert_eq!(body["error"], error, "{body}");
}

fn claims(token: &str) -> Value {
    let payload = token.split('.').nth(1).unwrap();
    serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn card_and_metadata_advertise_the_device_code_flow() {
    let f = fixture().await;
    let card: Value = f
        .server
        .get(&format!(
            "/v1/a2a/{}/.well-known/agent-card.json",
            f.channel
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let scheme = &card["securitySchemes"]["userDelegation"]["oauth2SecurityScheme"];
    let flow = &scheme["flows"]["deviceCode"];
    assert_eq!(flow["scopes"]["flights:read"], "View your flights");
    assert_eq!(
        card["securityRequirements"][2],
        json!({ "schemes": { "paJwt": { "list": [] }, "userDelegation": { "list": [] } } })
    );
    // The JWT-only requirements stay (§5.1).
    assert_eq!(
        card["securityRequirements"][0],
        json!({ "schemes": { "paJwt": { "list": [] } } })
    );

    let metadata = f.metadata().await;
    assert_eq!(metadata["token_endpoint"], flow["tokenUrl"]);
    assert_eq!(
        metadata["device_authorization_endpoint"],
        flow["deviceAuthorizationUrl"]
    );
    assert_eq!(
        scheme["oauth2MetadataUrl"],
        format!(
            "{}/.well-known/oauth-authorization-server",
            metadata["issuer"].as_str().unwrap()
        )
    );
    assert_eq!(
        metadata["scopes_supported"],
        json!(["flights:read", "flights:rebook"])
    );

    let jwks: Value = f
        .server
        .get(&format!("/v1/a2a/{}/oauth/jwks.json", f.channel))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(jwks["keys"][0]["alg"], "ES256");
    assert!(jwks["keys"][0].get("d").is_none(), "private key published");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_grants_some_scopes_and_the_agent_gets_a_token() {
    let f = fixture().await;
    let started = f.start("flights:read flights:rebook").await;
    let device_code = started["device_code"].as_str().unwrap();
    let user_code = started["user_code"].as_str().unwrap();
    let complete = started["verification_uri_complete"].as_str().unwrap();
    assert!(complete.starts_with("https://brand.example/login?return_to="));
    assert!(complete.contains(user_code));

    oauth_error(
        f.poll(device_code).await,
        StatusCode::BAD_REQUEST,
        "authorization_pending",
    );
    oauth_error(
        f.poll(device_code).await,
        StatusCode::BAD_REQUEST,
        "slow_down",
    );

    let redirect = f
        .consent(
            &f.assertion(user_code, json!({})).await,
            "allow",
            &["flights:read"],
        )
        .await
        .assert_status(StatusCode::SEE_OTHER);
    let location = redirect
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        location.starts_with(&format!("{CONNECTED}?status=approved")),
        "{location}"
    );
    assert!(location.contains("scope=flights%3Aread"), "{location}");

    let token: Value = f
        .poll(device_code)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(token["scope"], "flights:read");
    assert_eq!(token["token_type"], "Bearer");
    let access = claims(token["access_token"].as_str().unwrap());
    let metadata = f.metadata().await;
    assert_eq!(access["iss"], metadata["issuer"]);
    assert_eq!(access["sub"], "brand-user-42");
    assert_eq!(access["client_id"], ISSUER);
    assert_eq!(access["scope"], "flights:read");
    assert!(
        access["grant_id"]
            .as_str()
            .unwrap()
            .starts_with("a2agrant_")
    );
    assert!(access["exp"].as_i64().unwrap() - access["iat"].as_i64().unwrap() <= 3600);

    // A redeemed device code is spent.
    oauth_error(
        f.poll(device_code).await,
        StatusCode::BAD_REQUEST,
        "invalid_grant",
    );

    // Refresh tokens rotate, and a used one is refused.
    let refresh = token["refresh_token"].as_str().unwrap();
    let refresh_form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", ISSUER),
    ];
    let refreshed: Value = f
        .form("token", Some(&f.pa_token()), &refresh_form)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(refreshed["scope"], "flights:read");
    assert_ne!(refreshed["refresh_token"], token["refresh_token"]);
    oauth_error(
        f.form("token", Some(&f.pa_token()), &refresh_form).await,
        StatusCode::BAD_REQUEST,
        "invalid_grant",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_denied_consent_is_access_denied() {
    let f = fixture().await;
    let started = f.start("flights:read").await;
    let assertion = f
        .assertion(started["user_code"].as_str().unwrap(), json!({}))
        .await;
    let redirect = f
        .consent(&assertion, "deny", &["flights:read"])
        .await
        .assert_status(StatusCode::SEE_OTHER);
    let location = redirect
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.contains("status=denied"), "{location}");
    oauth_error(
        f.poll(started["device_code"].as_str().unwrap()).await,
        StatusCode::BAD_REQUEST,
        "access_denied",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_endpoints_check_the_personal_agent() {
    let f = fixture().await;
    // No personal-agent JWT: the bare §3.4 401.
    let anonymous = f
        .form(
            "device_authorization",
            None,
            &[("client_id", ISSUER), ("scope", "flights:read")],
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    assert_eq!(
        anonymous.headers().get("www-authenticate").unwrap(),
        "Bearer realm=\"a2a\""
    );
    assert_no_a2a_body(&anonymous);

    // client_id must be the JWT's issuer.
    oauth_error(
        f.form(
            "device_authorization",
            Some(&f.pa_token()),
            &[
                ("client_id", "https://other.example"),
                ("scope", "flights:read"),
            ],
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "invalid_client",
    );

    for scope in ["", "flights:read orders:cancel"] {
        oauth_error(
            f.form(
                "device_authorization",
                Some(&f.pa_token()),
                &[("client_id", ISSUER), ("scope", scope)],
            )
            .await,
            StatusCode::BAD_REQUEST,
            "invalid_scope",
        );
    }

    oauth_error(
        f.form(
            "token",
            Some(&f.pa_token()),
            &[("grant_type", "password"), ("client_id", ISSUER)],
        )
        .await,
        StatusCode::BAD_REQUEST,
        "unsupported_grant_type",
    );
    oauth_error(
        f.poll("dc_not-a-real-code").await,
        StatusCode::BAD_REQUEST,
        "invalid_grant",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_consent_page_needs_a_fresh_company_assertion() {
    let f = fixture().await;
    let started = f.start("flights:read").await;
    let user_code = started["user_code"].as_str().unwrap();
    let now = chrono::Utc::now().timestamp();

    let forged = PersonalAgentKey::generate().token_with(json!({
        "iss": BRAND_ISSUER,
        "aud": format!("{}/consent", f.metadata().await["issuer"].as_str().unwrap()),
        "sub": "brand-user-42",
        "jti": "forged",
        "user_code": user_code,
    }));
    for bad in [
        forged,
        f.assertion(user_code, json!({ "iss": "https://evil.example" }))
            .await,
        f.assertion(
            user_code,
            json!({ "aud": "https://elsewhere.example/consent" }),
        )
        .await,
        f.assertion(user_code, json!({ "iat": now - 600, "exp": now - 400 }))
            .await,
        f.assertion(user_code, json!({ "jti": null })).await,
        "not-a-jwt".to_string(),
    ] {
        f.form("consent", None, &[("assertion", bad.as_str())])
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    // An assertion opens the consent page once.
    let assertion = f.assertion(user_code, json!({})).await;
    let page = f
        .form("consent", None, &[("assertion", assertion.as_str())])
        .await
        .assert_status(StatusCode::OK)
        .text();
    assert!(page.contains("View your flights"), "{page}");
    assert!(page.contains("pa.example"), "{page}");
    f.form("consent", None, &[("assertion", assertion.as_str())])
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // A user code with no pending request.
    f.form(
        "consent",
        None,
        &[(
            "assertion",
            f.assertion("BCDF-GHJK", json!({})).await.as_str(),
        )],
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);

    // A tampered consent session.
    f.form(
        "consent/decision",
        None,
        &[
            ("session", "a.b.c"),
            ("decision", "allow"),
            ("scope", "flights:read"),
        ],
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endpoints_without_delegation_are_not_found() {
    let (server, _) = server().await;
    let pa = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(pact_config(&pa))).await;
    for route in [
        "oauth/.well-known/oauth-authorization-server",
        "oauth/jwks.json",
    ] {
        call(&server, &channel, Method::GET, route, None, None)
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
    // Routing before authentication: no 401 for a route that does not exist.
    for route in ["oauth/device_authorization", "oauth/token", "oauth/consent"] {
        call(&server, &channel, Method::POST, route, None, Some(""))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
    let card: Value = server
        .get(&format!("/v1/a2a/{channel}/.well-known/agent-card.json"))
        .await
        .json();
    assert!(card["securitySchemes"].get("userDelegation").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delegation_config_is_validated() {
    let (server, _) = server().await;
    let pa = PersonalAgentKey::generate();
    let brand = PersonalAgentKey::generate();
    let mut config = delegated_config(&pa, &brand);
    config["delegation"]["login_url"] = json!("http://brand.example/login");
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": format!("pact-{}", uuid::Uuid::new_v4().simple()), "system_prompt": "x" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let response = server
        .post(
            &format!("/v1/agents/{}/channels", agent["id"].as_str().unwrap()),
            json!({
                "channel_type": "a2a",
                "channel_config": {
                    "session_mode": "shared_session",
                    "message": "{{a2a.text}}",
                    "api_key_hash": "h",
                    "api_key_prefix": "evra2a_h",
                    "pact": config,
                },
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert!(
        response.text().contains("pact.delegation"),
        "{}",
        response.text()
    );
}
