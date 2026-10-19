//! Auth on the agent routes over a real socket, through `ServerBuilder`.
//! Uses the agents of `wire_tests` (`tester`, the default).

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};
use base64::Engine as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};

use crate::app::Mode;
use crate::auth::{AuthMethod, ChannelAuthVerifier};
use crate::hosting::{Server, ServerBuilder};
use crate::wire_tests::app;

struct Running {
    base: String,
    client: reqwest::Client,
    _task: tokio::task::JoinHandle<()>,
}

async fn run(builder: ServerBuilder) -> Running {
    let server = builder.build().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, server.router()).await.unwrap();
    });
    Running {
        base: format!("http://{addr}"),
        client: reqwest::Client::new(),
        _task: task,
    }
}

impl Running {
    async fn create(&self, bearer: Option<&str>) -> reqwest::Response {
        let mut request = self
            .client
            .post(format!("{}/v1/channels/tester/sessions", self.base))
            .json(&json!({}));
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        request.send().await.unwrap()
    }

    async fn get(&self, path: &str, bearer: Option<&str>) -> reqwest::Response {
        let mut request = self.client.get(format!("{}{path}", self.base));
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        request.send().await.unwrap()
    }

    async fn card_auth(&self) -> Value {
        let response = self.get("/v1/channels/tester", None).await;
        assert_eq!(response.status(), 200);
        response.json::<Value>().await.unwrap()["auth"].clone()
    }
}

fn assert_unauthorized(response: &reqwest::Response) {
    assert_eq!(response.status(), 401);
    assert_eq!(response.headers()["www-authenticate"], "Bearer");
}

#[tokio::test]
async fn without_a_method_the_api_stays_open() {
    let server = run(Server::builder(app(), Mode::Eval)).await;
    assert_eq!(server.create(None).await.status(), 201);
    assert_eq!(server.card_auth().await, json!([]));
}

#[tokio::test]
async fn a_static_key_is_required_on_every_agent_route() {
    let server = run(Server::builder(app(), Mode::Eval).auth([AuthMethod::api_key("k-1")])).await;

    assert_unauthorized(&server.create(None).await);
    assert_unauthorized(&server.create(Some("k-2")).await);
    assert_unauthorized(&server.create(Some("k-1-and-more")).await);
    let created = server.create(Some("k-1")).await;
    assert_eq!(created.status(), 201);
    let id = created.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The root session routes and the per-session ones are behind it too.
    let events = format!("/v1/channels/tester/sessions/{id}/events");
    assert_unauthorized(&server.get(&events, None).await);
    assert_eq!(server.get(&events, Some("k-1")).await.status(), 200);
    assert_unauthorized(&server.get(&format!("/v1/sessions/{id}"), None).await);
    assert_unauthorized(&server.get("/v1/agent", None).await);
    let root = server
        .client
        .post(format!("{}/v1/sessions", server.base))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_unauthorized(&root);

    // Discovery stays public, and the card names the method.
    assert_eq!(server.get("/health", None).await.status(), 200);
    assert_eq!(server.card_auth().await, json!([{ "type": "agent_key" }]));
}

/// An ES256 key pair, its JWKS, and a way to sign claims with it.
struct Issuer {
    encoding: EncodingKey,
    jwks: jsonwebtoken::jwk::JwkSet,
}

impl Issuer {
    fn new() -> Self {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap();
        let point = pair.public_key().as_ref();
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let jwks = serde_json::from_value(json!({
            "keys": [{
                "kty": "EC", "crv": "P-256", "kid": "k1", "alg": "ES256", "use": "sig",
                "x": b64(&point[1..33]), "y": b64(&point[33..65]),
            }]
        }))
        .unwrap();
        Self {
            encoding: EncodingKey::from_ec_der(pkcs8.as_ref()),
            jwks,
        }
    }

    fn token(&self, audience: &str) -> String {
        let now = chrono::Utc::now().timestamp();
        let claims = json!({
            "iss": ISSUER, "aud": audience, "sub": "user-1", "iat": now, "exp": now + 600,
        });
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("k1".to_string());
        encode(&header, &claims, &self.encoding).unwrap()
    }
}

const ISSUER: &str = "https://login.example.com";

#[tokio::test]
async fn an_oidc_token_from_the_configured_issuer_is_accepted() {
    let issuer = Issuer::new();
    // The identity provider is not reachable from a test; prime its
    // discovery and keys instead.
    let verifier = ChannelAuthVerifier::new();
    verifier
        .prime_oidc(
            ISSUER,
            "https://login.example.com/keys",
            issuer.jwks.clone(),
        )
        .await;
    let server = run(Server::builder(app(), Mode::Eval)
        .auth([
            AuthMethod::api_key("k-1"),
            AuthMethod::oidc(ISSUER, ["support"]),
        ])
        .auth_verifier(verifier))
    .await;

    assert_eq!(
        server.create(Some(&issuer.token("support"))).await.status(),
        201
    );
    assert_unauthorized(&server.create(Some(&issuer.token("other"))).await);
    assert_unauthorized(&server.create(Some(&Issuer::new().token("support"))).await);
    // The static key still works beside it.
    assert_eq!(server.create(Some("k-1")).await.status(), 201);
    assert_eq!(
        server.card_auth().await,
        json!([{ "type": "agent_key" }, { "type": "oidc", "issuer": ISSUER }])
    );
}

#[test]
fn an_unenforceable_method_fails_the_boot() {
    let result = Server::builder(app(), Mode::Eval)
        .auth([AuthMethod::oidc(ISSUER, Vec::<String>::new())])
        .build();
    assert!(result.is_err());
}

/// The A2A Agent Card names the schemes a caller must satisfy once auth is on,
/// in the A2A 1.0 shape, and none while the API is open.
#[cfg(feature = "a2a")]
#[tokio::test]
async fn the_a2a_card_lists_security_schemes_only_when_auth_is_on() {
    const CARD: &str = "/v1/channels/tester/a2a/.well-known/agent-card.json";

    let open = run(Server::builder(app(), Mode::Eval)).await;
    let card: Value = open.get(CARD, None).await.json().await.unwrap();
    assert!(card.get("securitySchemes").is_none(), "{card}");
    assert!(card.get("securityRequirements").is_none(), "{card}");

    let protected = run(Server::builder(app(), Mode::Eval).auth([
        AuthMethod::api_key("k-1"),
        AuthMethod::api_key("k-2"),
        AuthMethod::oidc(ISSUER, ["support"]).requirements(crate::auth::ChannelAuthRequirements {
            audiences: vec!["support".into()],
            scopes: vec!["agent:call".into()],
            ..Default::default()
        }),
    ]))
    .await;
    // Public, so a caller can learn how to authenticate before it can.
    let response = protected.get(CARD, None).await;
    assert_eq!(response.status(), 200);
    let card: Value = response.json().await.unwrap();
    assert_eq!(
        card["securitySchemes"],
        json!({
            "apiKey": { "httpAuthSecurityScheme": { "scheme": "bearer" } },
            "oidc": { "openIdConnectSecurityScheme": {
                "openIdConnectUrl": "https://login.example.com/.well-known/openid-configuration"
            } },
        })
    );
    assert_eq!(
        card["securityRequirements"],
        json!([
            { "schemes": { "apiKey": [] } },
            { "schemes": { "oidc": ["agent:call"] } },
        ])
    );
    // An A2A 1.0 client reads it back as the same requirements.
    let parsed: ::a2a::AgentCard = serde_json::from_value(card).unwrap();
    let requirements = parsed.security_requirements.unwrap();
    assert_eq!(requirements.len(), 2);
    assert_eq!(requirements[0]["apiKey"], Vec::<String>::new());
    assert_eq!(requirements[1]["oidc"], vec!["agent:call".to_string()]);
    assert_eq!(parsed.security_schemes.unwrap().len(), 2);
}

/// `eval --against` reaches a protected app with `--credential`, and without
/// one fails with a pointer to the flag rather than a bare 401.
#[tokio::test]
async fn a_remote_eval_sends_its_credential() {
    let server = run(Server::builder(app(), Mode::Eval).auth([AuthMethod::api_key("k-1")])).await;

    let mut anonymous = crate::eval::EvalCx::remote_for_test(server.base.clone());
    let error = anonymous.send("one").await.unwrap_err().to_string();
    assert!(error.contains("--credential"), "{error}");
    assert!(error.contains("SERVE_CREDENTIAL"), "{error}");

    let mut wrong =
        crate::eval::EvalCx::remote_with_credential_for_test(server.base.clone(), Some("k-2"));
    let error = wrong.send("one").await.unwrap_err().to_string();
    assert!(
        !error.contains("k-2"),
        "the credential is never echoed: {error}"
    );

    let mut eval =
        crate::eval::EvalCx::remote_with_credential_for_test(server.base.clone(), Some("k-1"));
    eval.send("one").await.unwrap();
    eval.completed().unwrap().called_tool("shout").unwrap();
    // Approvals are answered over the same authenticated client.
    eval.send("two").await.unwrap();
    eval.completed().unwrap().asked_approval("guarded").unwrap();
}
