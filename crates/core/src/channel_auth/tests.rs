use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::Value;

use super::claims::{
    extract_basic_credentials, principal_from_claims, validate_claim_requirements,
    verify_jwt_with_jwks, verify_shared_secret,
};
use super::verifier::resolve_and_validate;
use super::*;

const EXAMPLE_USERNAME: &str = "example";
const EXAMPLE_PASSWORD: &str = "YExample0";

fn example_basic_auth_header() -> HeaderValue {
    let credentials = BASE64_STANDARD.encode(format!("{EXAMPLE_USERNAME}:{EXAMPLE_PASSWORD}"));
    HeaderValue::from_bytes(format!("basic {credentials}").as_bytes()).unwrap()
}

fn basic_auth(password_hash: Option<&str>) -> ChannelAuthConfig {
    ChannelAuthConfig {
        mode: ChannelAuthMode::HttpBasic,
        provider: Some(ChannelAuthProviderConfig::HttpBasic {
            username: EXAMPLE_USERNAME.to_string(),
            password: None,
            password_hash: password_hash.map(str::to_string),
            password_configured: false,
        }),
        requirements: ChannelAuthRequirements::default(),
    }
}

/// A stand-in for the host's password store: `plain:<password>`.
fn plain_check() -> PasswordCheck {
    Arc::new(|password, hash| Ok(hash.strip_prefix("plain:") == Some(password)))
}

#[test]
fn extracts_basic_credentials() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, example_basic_auth_header());
    assert_eq!(
        extract_basic_credentials(&headers),
        Some((EXAMPLE_USERNAME.to_string(), EXAMPLE_PASSWORD.to_string()))
    );
}

#[test]
fn shared_secret_uses_bearer() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("bearer YExample0"));
    assert!(verify_shared_secret(&headers, EXAMPLE_PASSWORD).is_ok());
    assert_eq!(
        verify_shared_secret(&headers, "other").unwrap_err(),
        ChannelAuthError::Unauthorized
    );
}

#[test]
fn basic_auth_checks_the_password_with_the_host_check() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, example_basic_auth_header());
    let verifier = ChannelAuthVerifier::new().with_password_check(plain_check());
    assert!(
        verifier
            .verify_basic(&basic_auth(Some("plain:YExample0")), &headers)
            .is_ok()
    );
    assert_eq!(
        verifier
            .verify_basic(&basic_auth(Some("plain:other")), &headers)
            .unwrap_err(),
        ChannelAuthError::Unauthorized
    );
}

#[test]
fn basic_auth_without_a_password_check_fails_closed() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, example_basic_auth_header());
    assert_eq!(
        ChannelAuthVerifier::new()
            .verify_basic(&basic_auth(Some("plain:YExample0")), &headers)
            .unwrap_err(),
        ChannelAuthError::Misconfigured
    );
}

#[test]
fn claim_requirements_check_scope_domain_and_claims() {
    let claims = serde_json::json!({
        "sub": "user-1",
        "email": "a@example.com",
        "scope": "read write",
        "tier": "prod"
    });
    let requirements = ChannelAuthRequirements {
        audiences: vec!["api://test".to_string()],
        scopes: vec!["read".to_string()],
        domains: vec!["example.com".to_string()],
        subjects: vec!["user-1".to_string()],
        claims: [("tier".to_string(), Value::String("prod".to_string()))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    assert!(validate_claim_requirements(&claims, &requirements).is_err());

    let claims = serde_json::json!({
        "sub": "user-1",
        "email": "a@example.com",
        "aud": ["api://test"],
        "scope": "read write",
        "tier": "prod"
    });
    assert!(validate_claim_requirements(&claims, &requirements).is_ok());
}

#[test]
fn principal_identity_realm_is_bound_to_verifier_authority() {
    let claims = serde_json::json!({
        "iss": "https://victim.example/",
        "sub": "user-1"
    });
    let trusted =
        principal_from_claims(&claims, "oidc:https://victim.example/.well-known/jwks.json")
            .unwrap();
    let spoofed = principal_from_claims(
        &claims,
        "oauth2_introspection:https://attacker.example/introspect",
    )
    .unwrap();

    assert_eq!(trusted.issuer, spoofed.issuer);
    assert_eq!(trusted.subject, spoofed.subject);
    assert_ne!(trusted.identity_realm, spoofed.identity_realm);
    assert!(!trusted.identity_realm.contains(".well-known"));
    assert!(!spoofed.identity_realm.contains("attacker.example"));
}

#[test]
fn principal_identity_realm_is_a_stable_sha256_hex() {
    // The realm is persisted on virtual-user bindings, so its encoding must
    // not drift: lowercase hex of SHA-256 over the verifier authority.
    let claims = serde_json::json!({ "iss": "https://idp.example", "sub": "user-1" });
    let principal = principal_from_claims(&claims, "abc").unwrap();
    assert_eq!(
        principal.identity_realm,
        "https://idp.example#ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

mod oidc {
    use super::*;
    use crate::channel_auth::test_keys::{signer, token};

    const ISSUER: &str = "https://idp.example";
    const JWKS: &str = "https://idp.example/keys";

    fn auth() -> ChannelAuthConfig {
        ChannelAuthConfig {
            mode: ChannelAuthMode::Oidc,
            provider: Some(ChannelAuthProviderConfig::Oidc {
                issuer: ISSUER.to_string(),
                jwks_url: None,
            }),
            requirements: ChannelAuthRequirements {
                audiences: vec!["serve".to_string()],
                ..Default::default()
            },
        }
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn verifies_a_token_against_primed_discovery() {
        let signer = signer();
        let verifier = ChannelAuthVerifier::new();
        verifier.prime_oidc(ISSUER, JWKS, signer.jwks.clone()).await;
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": ISSUER, "aud": "serve", "sub": "user-1", "iat": now, "exp": now + 600,
        });
        let principal = verifier
            .verify_principal(
                &auth(),
                &bearer(&token(&signer, claims.clone())),
                LegacyChannelAuth::default(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(principal.provider, OIDC_PROVIDER);
        assert_eq!(principal.subject, "user-1");

        let mut wrong_audience = claims;
        wrong_audience["aud"] = Value::String("other".to_string());
        assert_eq!(
            verifier
                .verify_principal(
                    &auth(),
                    &bearer(&token(&signer, wrong_audience)),
                    LegacyChannelAuth::default(),
                )
                .await
                .unwrap_err(),
            ChannelAuthError::Unauthorized
        );
    }

    #[tokio::test]
    async fn a_token_from_another_key_is_rejected() {
        let verifier = ChannelAuthVerifier::new();
        verifier.prime_oidc(ISSUER, JWKS, signer().jwks).await;
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": ISSUER, "aud": "serve", "sub": "user-1", "exp": now + 600,
        });
        assert_eq!(
            verifier
                .verify_principal(
                    &auth(),
                    &bearer(&token(&signer(), claims)),
                    LegacyChannelAuth::default(),
                )
                .await
                .unwrap_err(),
            ChannelAuthError::Unauthorized
        );
    }
}

mod agentid {
    use super::*;
    use crate::channel_auth::test_keys::{Signer, signer, token};

    const CLIENT_ID: &str = "agentid-client";

    fn claims(actor_type: Option<&str>) -> Value {
        let now = chrono::Utc::now().timestamp();
        let mut claims = serde_json::json!({
            "iss": AGENTID_ISSUER,
            "aud": CLIENT_ID,
            "sub": "lM9vT2aR7sK4qN8wE1xC6bY0uF3hJ5pD9gL2zV7oA4Q",
            "email": "research@acme.agentmail.to",
            "iat": now,
            "exp": now + 600,
        });
        if let Some(actor_type) = actor_type {
            claims["actor_type"] = Value::String(actor_type.to_string());
        }
        claims
    }

    fn verify(
        signer: &Signer,
        token: &str,
        auth: &ChannelAuthConfig,
        authority: &str,
    ) -> Result<ChannelAuthPrincipal, ChannelAuthError> {
        verify_jwt_with_jwks(
            token,
            AGENTID_ISSUER,
            &signer.jwks,
            &auth.requirements,
            authority,
            auth.is_agentid(),
        )
    }

    const DISCOVERY: &str = "oidc-discovery:https://auth.agentid.com";

    #[test]
    fn accepts_an_agent_token_and_binds_it_to_the_agentid_realm() {
        let signer = signer();
        let auth = ChannelAuthConfig::agentid_preset(CLIENT_ID);
        let principal = verify(
            &signer,
            &token(&signer, claims(Some("agent"))),
            &auth,
            DISCOVERY,
        )
        .unwrap();
        assert_eq!(principal.provider, AGENTID_PROVIDER);
        assert_eq!(principal.identity_realm, AGENTID_ISSUER);
        assert_eq!(
            principal.subject,
            "lM9vT2aR7sK4qN8wE1xC6bY0uF3hJ5pD9gL2zV7oA4Q"
        );
    }

    #[test]
    fn rejects_missing_or_non_agent_actor_type() {
        let signer = signer();
        let auth = ChannelAuthConfig::agentid_preset(CLIENT_ID);
        for actor_type in [None, Some("human"), Some("Agent")] {
            assert_eq!(
                verify(
                    &signer,
                    &token(&signer, claims(actor_type)),
                    &auth,
                    DISCOVERY
                )
                .unwrap_err(),
                ChannelAuthError::Unauthorized,
                "accepted actor_type {actor_type:?}"
            );
        }
    }

    #[test]
    fn rejects_another_audience() {
        let signer = signer();
        let auth = ChannelAuthConfig::agentid_preset("someone-else");
        assert_eq!(
            verify(
                &signer,
                &token(&signer, claims(Some("agent"))),
                &auth,
                DISCOVERY
            )
            .unwrap_err(),
            ChannelAuthError::Unauthorized
        );
    }

    #[test]
    fn operator_jwks_claiming_the_agentid_issuer_stays_a_plain_oidc_identity() {
        // THREAT[TM-AUTH-031]: an endpoint owner who points a channel at
        // their own JWKS can sign any `iss`. Those subjects must never land
        // in the shared `agentid` namespace.
        let signer = signer();
        let mut auth = ChannelAuthConfig::agentid_preset(CLIENT_ID);
        auth.provider = Some(ChannelAuthProviderConfig::Oidc {
            issuer: AGENTID_ISSUER.to_string(),
            jwks_url: Some("https://attacker.example/jwks.json".to_string()),
        });
        assert!(!auth.is_agentid());
        let principal = verify(
            &signer,
            &token(&signer, claims(Some("agent"))),
            &auth,
            "oidc-jwks:https://attacker.example/jwks.json",
        )
        .unwrap();
        assert_eq!(principal.provider, OIDC_PROVIDER);
        assert_ne!(principal.identity_realm, AGENTID_ISSUER);
        assert!(
            principal
                .identity_realm
                .starts_with("https://auth.agentid.com#")
        );
    }
}

fn mtls_auth() -> ChannelAuthConfig {
    ChannelAuthConfig {
        mode: ChannelAuthMode::Mtls,
        provider: Some(ChannelAuthProviderConfig::Mtls {
            header_name: "x-client-cert".to_string(),
            allowed_values: vec!["CN=trusted".to_string()],
            proxy_secret_header: Some("x-proxy-secret".to_string()),
            proxy_secret: Some("supersecret".to_string()),
            proxy_secret_configured: false,
        }),
        requirements: ChannelAuthRequirements::default(),
    }
}

#[test]
fn mtls_requires_both_cert_and_proxy_secret() {
    let auth = mtls_auth();
    let verifier = ChannelAuthVerifier::new();

    // No headers — Unauthorized (cert missing).
    assert_eq!(
        verifier.verify_mtls(&auth, &HeaderMap::new()).unwrap_err(),
        ChannelAuthError::Unauthorized
    );

    // Cert header only — Unauthorized (proxy secret missing). This is the
    // spoofing case EVE-545 guards against.
    let mut headers = HeaderMap::new();
    headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
    assert_eq!(
        verifier.verify_mtls(&auth, &headers).unwrap_err(),
        ChannelAuthError::Unauthorized,
        "spoofed cert header alone must not authenticate"
    );

    // Cert + wrong proxy secret — Unauthorized.
    let mut headers = HeaderMap::new();
    headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
    headers.insert("x-proxy-secret", HeaderValue::from_static("wrongsecret"));
    assert_eq!(
        verifier.verify_mtls(&auth, &headers).unwrap_err(),
        ChannelAuthError::Unauthorized
    );

    // Both correct — Ok.
    let mut headers = HeaderMap::new();
    headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
    headers.insert("x-proxy-secret", HeaderValue::from_static("supersecret"));
    assert!(verifier.verify_mtls(&auth, &headers).is_ok());
}

#[test]
fn mtls_without_proxy_secret_config_is_misconfigured() {
    // Legacy configs that predate EVE-545 (no proxy_secret fields) must fail
    // closed so they cannot be exploited after an upgrade.
    let auth = ChannelAuthConfig {
        mode: ChannelAuthMode::Mtls,
        provider: Some(ChannelAuthProviderConfig::Mtls {
            header_name: "x-client-cert".to_string(),
            allowed_values: vec!["CN=trusted".to_string()],
            proxy_secret_header: None,
            proxy_secret: None,
            proxy_secret_configured: false,
        }),
        requirements: ChannelAuthRequirements::default(),
    };
    let verifier = ChannelAuthVerifier::new();
    let mut headers = HeaderMap::new();
    headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
    assert_eq!(
        verifier.verify_mtls(&auth, &headers).unwrap_err(),
        ChannelAuthError::Misconfigured
    );
}

#[tokio::test]
async fn resolve_and_validate_rejects_literal_loopback() {
    assert!(resolve_and_validate("http://127.0.0.1/").await.is_err());
    assert!(resolve_and_validate("http://[::1]/").await.is_err());
}

#[tokio::test]
async fn resolve_and_validate_rejects_literal_private_ranges() {
    assert!(resolve_and_validate("http://10.0.0.1/").await.is_err());
    assert!(resolve_and_validate("http://192.168.1.1/").await.is_err());
    assert!(
        resolve_and_validate("http://169.254.169.254/")
            .await
            .is_err()
    );
}
