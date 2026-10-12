use super::*;
use crate::api::channel_auth::test_keys::{Signer, signer, token};
use wiremock::matchers::{body_string_contains, header as header_is, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT_ID: &str = "agentid-registered-client";
const SUBJECT: &str = "lM9vT2aR7sK4qN8wE1xC6bY0uF3hJ5pD9gL2zV7oA4Q";
const OWNER: &str = "oW7xN2pQ4mT8vL1kR6sC9dF3gH5jB0aE2uY4zI6nP8A";

fn config(issuer_base: &str, owner_scopes: bool) -> AgentIdLoginConfig {
    AgentIdLoginConfig {
        client_id: CLIENT_ID.into(),
        client_secret: ClientSecret("s3cret".into()),
        redirect_uri: "https://everruns.test/api/v1/agentid/callback".into(),
        owner_scopes,
        default_channel: None,
        frontend_url: "https://everruns.test".into(),
        issuer_base: issuer_base.into(),
    }
}

fn login() -> AgentIdLoginState {
    AgentIdLoginState {
        org_id: 1,
        channel_id: "appchan_test".into(),
        code_verifier: "the-verifier".into(),
        nonce: "the-nonce".into(),
        login_hint: None,
    }
}

fn id_claims() -> Value {
    let now = chrono::Utc::now().timestamp();
    serde_json::json!({
        "iss": AGENTID_ISSUER,
        "aud": CLIENT_ID,
        "sub": SUBJECT,
        "actor_type": "agent",
        "nonce": "the-nonce",
        "email": "research@acme.agentmail.to",
        "iat": now,
        "exp": now + 600,
    })
}

fn userinfo() -> Value {
    serde_json::json!({
        "sub": SUBJECT,
        "actor_type": "agent",
        "email": "research@acme.agentmail.to",
        "preferred_username": "research",
        "owner_sub": OWNER,
        "owner_name": "Maya Chen",
        "owner_email": "maya@acme.com",
    })
}

async fn agentid(signer: &Signer, id_token_claims: Value, info: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v0/token"))
        .and(body_string_contains("code_verifier=the-verifier"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id_token": token(signer, id_token_claims),
            "access_token": "access-1",
            "token_type": "Bearer",
            "expires_in": 600,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/userinfo"))
        .and(header_is("authorization", "Bearer access-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(info))
        .mount(&server)
        .await;
    server
}

async fn run(
    id_token_claims: Value,
    info: Value,
    owner_scopes: bool,
) -> Result<AgentIdAgent, SignInError> {
    let signer = signer();
    let server = agentid(&signer, id_token_claims, info).await;
    finish(
        &config(&server.uri(), owner_scopes),
        &reqwest::Client::new(),
        &signer.jwks,
        "code-1",
        &login(),
    )
    .await
}

#[tokio::test]
async fn a_verified_agent_signs_in_by_sub_and_keeps_its_own_name() {
    let agent = run(id_claims(), userinfo(), false).await.unwrap();
    assert_eq!(agent.subject, SUBJECT);
    assert_eq!(agent.owner_sub, OWNER);
    // The owner's name and email never leak in without the explicit opt-in.
    assert_eq!(agent.display_name, "research");
    assert_eq!(agent.owner_email, None);
}

#[tokio::test]
async fn owner_email_is_kept_only_with_the_owner_scope_opt_in() {
    let agent = run(id_claims(), userinfo(), true).await.unwrap();
    assert_eq!(agent.owner_email.as_deref(), Some("maya@acme.com"));
    assert_eq!(agent.display_name, "research");
}

#[tokio::test]
async fn a_token_without_the_agent_actor_type_is_rejected() {
    let mut claims = id_claims();
    claims["actor_type"] = Value::String("human".into());
    assert_eq!(
        run(claims, userinfo(), false).await.unwrap_err(),
        SignInError::Rejected
    );
}

#[tokio::test]
async fn an_unsigned_agent_claim_decides_nothing() {
    // Correct claims under a key AgentID did not publish.
    let signer = signer();
    let other = crate::api::channel_auth::test_keys::signer();
    let server = agentid(&other, id_claims(), userinfo()).await;
    let result = finish(
        &config(&server.uri(), false),
        &reqwest::Client::new(),
        &signer.jwks,
        "code-1",
        &login(),
    )
    .await;
    assert_eq!(result.unwrap_err(), SignInError::Rejected);
}

#[tokio::test]
async fn a_replayed_nonce_or_another_audience_is_rejected() {
    let mut nonce = id_claims();
    nonce["nonce"] = Value::String("other".into());
    assert_eq!(
        run(nonce, userinfo(), false).await.unwrap_err(),
        SignInError::Rejected
    );
    let mut audience = id_claims();
    audience["aud"] = Value::String("someone-else".into());
    assert_eq!(
        run(audience, userinfo(), false).await.unwrap_err(),
        SignInError::Rejected
    );
}

#[tokio::test]
async fn userinfo_for_another_subject_is_rejected() {
    let mut info = userinfo();
    info["sub"] = Value::String("another-agent".into());
    assert_eq!(
        run(id_claims(), info, false).await.unwrap_err(),
        SignInError::Rejected
    );
}

#[tokio::test]
async fn a_sign_in_without_owner_sub_fails() {
    let mut info = userinfo();
    info.as_object_mut().unwrap().remove("owner_sub");
    assert_eq!(
        run(id_claims(), info, false).await.unwrap_err(),
        SignInError::MissingOwner
    );
}

#[test]
fn the_authorize_request_uses_pkce_nonce_and_the_profile_scope() {
    let url = config(AGENTID_ISSUER, false).authorize_url(
        "st",
        "ch",
        "nn",
        Some("research@acme.agentmail.to"),
    );
    let parsed = url::Url::parse(&url).unwrap();
    let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(parsed.path(), "/v0/authorize");
    assert_eq!(query["scope"], "openid email profile");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["login_hint"], "research@acme.agentmail.to");
    assert_eq!(query["client_id"], CLIENT_ID);
    assert!(!url.contains("s3cret"));
    let owner = config(AGENTID_ISSUER, true).authorize_url("st", "ch", "nn", None);
    assert!(owner.contains("owner_profile"));
}

#[test]
fn the_client_secret_never_prints() {
    let printed = format!("{:?}", config(AGENTID_ISSUER, false));
    assert!(!printed.contains("s3cret"));
}
