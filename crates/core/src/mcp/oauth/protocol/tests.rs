use super::*;
use crate::egress::{EgressError, EgressResponse, EgressResult};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

#[test]
fn client_metadata_capability_requires_a_boolean() {
    for (field, expected) in [(None, false), (Some(false), false), (Some(true), true)] {
        let mut value = serde_json::json!({
            "issuer": "https://auth.example",
            "authorization_endpoint": "https://auth.example/authorize",
            "token_endpoint": "https://auth.example/token"
        });
        if let Some(field) = field {
            value["client_id_metadata_document_supported"] = field.into();
        }
        let metadata: AuthorizationServerMetadata = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(metadata.client_id_metadata_document_supported, expected);
        for invalid in [
            serde_json::json!("true"),
            serde_json::json!(1),
            serde_json::Value::Null,
        ] {
            value["client_id_metadata_document_supported"] = invalid;
            assert!(serde_json::from_value::<AuthorizationServerMetadata>(value.clone()).is_err());
        }
    }
}

#[test]
fn client_metadata_url_obeys_cimd_identifier_rules() {
    for valid in [
        "https://client.example/oauth.json",
        "https://client.example:8443/oauth/client.json",
        // Queries are discouraged, but not forbidden by the draft.
        "https://client.example/oauth.json?version=1",
    ] {
        validate_client_metadata_url(valid).unwrap();
    }
    for invalid in [
        "",
        "client.json",
        "http://client.example/client.json",
        "http://127.0.0.1/client.json",
        "https://client.example",
        "https://client.example/",
        "https://client.example/#fragment",
        "https://client.example/client.json#fragment",
        "https://user@client.example/client.json",
        "https://user:pass@client.example/client.json",
        "https://@client.example/client.json",
        "https://client.example/./client.json",
        "https://client.example/a/../client.json",
        "https://client.example/%2e/client.json",
        "https://client.example/.%2E/client.json",
        "https://client.example/%2e%2E/client.json",
        " https://client.example/client.json",
        "https://client.example/cli\nent.json",
        "https://client.example/a\\client.json",
    ] {
        assert!(
            validate_client_metadata_url(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}

/// Egress double: answers by URL, records what was sent. Tests exercise
/// the whole client without a socket, which is the payoff of routing OAuth
/// through the egress seam instead of a private HTTP client.
#[derive(Default)]
struct FakeEgress {
    responses: Mutex<Vec<(String, u16, String)>>,
    sent: Arc<Mutex<Vec<EgressRequest>>>,
}

impl FakeEgress {
    fn with(routes: Vec<(&str, u16, &str)>) -> Self {
        Self {
            responses: Mutex::new(
                routes
                    .into_iter()
                    .map(|(url, status, body)| (url.to_string(), status, body.to_string()))
                    .collect(),
            ),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl EgressService for FakeEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        self.sent.lock().unwrap().push(request.clone());
        let responses = self.responses.lock().unwrap();
        let matched = responses
            .iter()
            .find(|(url, _, _)| *url == request.url)
            .map(|(_, status, body)| (*status, body.clone()));
        match matched {
            Some((status, body)) => Ok(EgressResponse {
                status,
                headers: BTreeMap::new(),
                body: body.into_bytes(),
            }),
            None => Ok(EgressResponse {
                status: 404,
                headers: BTreeMap::new(),
                body: Vec::new(),
            }),
        }
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> EgressResult<crate::egress::EgressStreamResponse> {
        Err(EgressError::Transport(
            "streaming not supported".to_string(),
        ))
    }
}

fn metadata_json() -> &'static str {
    r#"{
        "issuer": "https://auth.example.com",
        "authorization_endpoint": "https://auth.example.com/authorize",
        "token_endpoint": "https://auth.example.com/token",
        "registration_endpoint": "https://auth.example.com/register",
        "code_challenge_methods_supported": ["S256"]
    }"#
}

#[test]
fn pkce_challenge_is_the_s256_of_the_verifier() {
    let pair = PkcePair::generate();
    let expected = base64_url(Sha256::digest(pair.verifier.as_bytes()).as_slice());
    assert_eq!(pair.challenge, expected);
    assert_ne!(pair.verifier, pair.challenge);
    // Two generations must not collide.
    assert_ne!(PkcePair::generate().verifier, pair.verifier);
}

#[test]
fn plaintext_endpoints_are_rejected_except_on_loopback() {
    assert!(require_secure("https://auth.example.com/token", "token endpoint").is_ok());
    assert!(require_secure("http://127.0.0.1:1455/callback", "redirect").is_ok());
    assert!(require_secure("http://auth.example.com/token", "token endpoint").is_err());
    assert!(require_secure("not-a-url", "token endpoint").is_err());
}

#[test]
fn authorize_url_carries_pkce_state_and_resource() {
    let metadata: AuthorizationServerMetadata = serde_json::from_str(metadata_json()).unwrap();
    let url = authorize_url(
        &metadata,
        &AuthorizeParams {
            client_id: "client-1",
            redirect_uri: "http://127.0.0.1:1455/callback",
            code_challenge: "challenge",
            state: "state-1",
            scope: Some("mcp:read"),
            resource: Some("https://mcp.example.com/"),
        },
    )
    .unwrap();

    assert!(url.starts_with("https://auth.example.com/authorize?"));
    for expected in [
        "response_type=code",
        "client_id=client-1",
        "code_challenge=challenge",
        "code_challenge_method=S256",
        "state=state-1",
        "scope=mcp%3Aread",
        "resource=https%3A%2F%2Fmcp.example.com%2F",
    ] {
        assert!(url.contains(expected), "missing {expected} in {url}");
    }
}

#[test]
fn callback_issuer_must_match_when_present() {
    assert!(validate_callback_issuer("https://auth.example.com", None).is_ok());
    assert!(
        validate_callback_issuer("https://auth.example.com", Some("https://auth.example.com"))
            .is_ok()
    );
    assert!(matches!(
        validate_callback_issuer("https://auth.example.com", Some("https://evil.example.com")),
        Err(OAuthError::IssuerMismatch { .. })
    ));
}

#[test]
fn well_known_inserts_the_suffix_before_an_issuer_path() {
    let plain = Url::parse("https://auth.example.com").unwrap();
    assert_eq!(
        well_known(&plain, "oauth-authorization-server"),
        "https://auth.example.com/.well-known/oauth-authorization-server"
    );
    let tenanted = Url::parse("https://auth.example.com/tenant-a").unwrap();
    assert_eq!(
        well_known(&tenanted, "oauth-authorization-server"),
        "https://auth.example.com/.well-known/oauth-authorization-server/tenant-a"
    );
}

#[tokio::test]
async fn discovery_falls_back_to_the_openid_document() {
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/.well-known/openid-configuration",
        200,
        metadata_json(),
    )]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

    let metadata = client
        .discover_authorization_server("https://auth.example.com")
        .await
        .expect("discovery should fall back");

    assert_eq!(metadata.token_endpoint, "https://auth.example.com/token");
    // The RFC 8414 location is tried first, then OpenID.
    let sent = egress.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].url.contains("oauth-authorization-server"));
    assert!(sent[1].url.contains("openid-configuration"));
}

#[tokio::test]
async fn discovery_requires_secure_endpoints_in_the_metadata() {
    // A compromised or sloppy server advertising a plaintext token endpoint
    // must not be used, even though the metadata document itself was HTTPS.
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/.well-known/oauth-authorization-server",
        200,
        r#"{
            "issuer": "https://auth.example.com",
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "http://auth.example.com/token"
        }"#,
    )]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

    assert!(matches!(
        client
            .discover_authorization_server("https://auth.example.com")
            .await,
        Err(OAuthError::InsecureUrl { .. })
    ));
}

#[tokio::test]
async fn missing_protected_resource_document_is_not_an_error() {
    let egress = FakeEgress::with(vec![]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

    let found = client
        .discover_protected_resource("https://mcp.example.com/sse")
        .await
        .expect("a 404 means 'not published', not a failure");

    assert!(found.is_none());
}

#[tokio::test]
async fn code_exchange_posts_pkce_and_returns_a_dated_token_set() {
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/token",
        200,
        r#"{"access_token":"at-1","refresh_token":"rt-1","token_type":"Bearer","expires_in":3600}"#,
    )]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

    let tokens = client
        .exchange_code(
            "https://auth.example.com/token",
            &RegisteredClient {
                client_id: "client-1".to_string(),
                client_secret: None,
            },
            "code-1",
            "verifier-1",
            "http://127.0.0.1:1455/callback",
            Some("https://mcp.example.com/"),
        )
        .await
        .expect("exchange should succeed");

    assert_eq!(tokens.access_token, "at-1");
    assert_eq!(tokens.authorization_value(), "Bearer at-1");
    assert!(tokens.expires_at.is_some());
    assert_eq!(
        tokens.token_endpoint.as_deref(),
        Some("https://auth.example.com/token")
    );

    let sent = egress.sent.lock().unwrap();
    let body = String::from_utf8(sent[0].body.clone()).unwrap();
    assert!(body.contains("grant_type=authorization_code"));
    assert!(body.contains("code_verifier=verifier-1"));
    assert!(body.contains("resource=https"));
    assert!(sent[0].dns_pinning_required, "token calls must be pinned");
}

#[tokio::test]
async fn refresh_carries_the_old_refresh_token_when_the_server_does_not_rotate() {
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/token",
        200,
        r#"{"access_token":"at-2","token_type":"Bearer","expires_in":3600}"#,
    )]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);
    let existing = TokenSet {
        access_token: "at-1".to_string(),
        refresh_token: Some("rt-1".to_string()),
        token_type: "Bearer".to_string(),
        expires_at: Some(Utc::now() - ChronoDuration::seconds(1)),
        scope: None,
        token_endpoint: Some("https://auth.example.com/token".to_string()),
        resource: Some("https://mcp.example.com/".to_string()),
        client_id: Some("client-1".to_string()),
        client_secret: None,
    };

    let renewed = client.refresh(&existing).await.expect("refresh");

    assert_eq!(renewed.access_token, "at-2");
    assert_eq!(renewed.resource, existing.resource);
    assert_eq!(
        renewed.refresh_token.as_deref(),
        Some("rt-1"),
        "a non-rotating server must not cost us the refresh token"
    );
}

#[tokio::test]
async fn refresh_without_a_refresh_token_is_a_typed_error() {
    let egress = FakeEgress::with(vec![]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);
    let tokens = TokenSet {
        access_token: "at-1".to_string(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_at: None,
        scope: None,
        token_endpoint: Some("https://auth.example.com/token".to_string()),
        resource: Some("https://mcp.example.com/".to_string()),
        client_id: None,
        client_secret: None,
    };

    assert!(matches!(
        client.refresh(&tokens).await,
        Err(OAuthError::NotRefreshable)
    ));
}

#[tokio::test]
async fn token_endpoint_errors_surface_status_and_body() {
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/token",
        400,
        r#"{"error":"invalid_grant"}"#,
    )]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

    let error = client
        .exchange_code(
            "https://auth.example.com/token",
            &RegisteredClient {
                client_id: "client-1".to_string(),
                client_secret: None,
            },
            "code-1",
            "verifier-1",
            "http://127.0.0.1:1455/callback",
            None,
        )
        .await
        .expect_err("a 400 must not be read as success");

    match error {
        OAuthError::Http { status, body, .. } => {
            assert_eq!(status, 400);
            assert!(body.contains("invalid_grant"));
        }
        other => panic!("expected an HTTP error, got {other}"),
    }
}

#[tokio::test]
async fn discovery_rejects_a_different_issuer() {
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/.well-known/oauth-authorization-server/tenant",
        200,
        r#"{"issuer":"https://other.example.com","authorization_endpoint":"https://auth.example.com/authorize","token_endpoint":"https://auth.example.com/token"}"#,
    )]);
    assert!(
        OAuthClient::new(&egress, EgressRequestKind::Mcp)
            .discover_authorization_server("https://auth.example.com/tenant")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn oauth_errors_keep_classification_without_retaining_secrets() {
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/token",
        400,
        r#"{"error":"invalid_grant","error_description":"client-secret refresh-secret code-secret"}"#,
    )]);
    let tokens = TokenSet {
        access_token: "access-secret".into(),
        refresh_token: Some("refresh-secret".into()),
        token_type: "Bearer".into(),
        expires_at: None,
        scope: None,
        token_endpoint: Some("https://auth.example.com/token".into()),
        resource: Some("https://mcp.example.com/mcp".into()),
        client_id: Some("client".into()),
        client_secret: Some("client-secret".into()),
    };
    let error = OAuthClient::new(&egress, EgressRequestKind::Mcp)
        .refresh(&tokens)
        .await
        .unwrap_err();
    let diagnostic = format!("{error} {error:?}");
    assert!(diagnostic.contains("invalid_grant"));
    for secret in ["client-secret", "refresh-secret", "code-secret"] {
        assert!(
            !diagnostic.contains(secret),
            "OAuth errors must redact credentials"
        );
    }
}

#[tokio::test]
async fn unrepresentable_token_lifetimes_are_errors_instead_of_panics() {
    let egress = FakeEgress::with(vec![(
        "https://auth.example.com/token",
        200,
        r#"{"access_token":"secret","expires_in":9223372036854775807}"#,
    )]);
    let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);
    let result = client
        .exchange_code(
            "https://auth.example.com/token",
            &RegisteredClient {
                client_id: "client".into(),
                client_secret: None,
            },
            "code",
            "verifier",
            "http://localhost/callback",
            None,
        )
        .await;
    assert!(matches!(result, Err(OAuthError::Malformed { .. })));
}

#[test]
fn token_set_debug_redacts_every_credential_field() {
    let tokens = TokenSet {
        access_token: "at-secret".to_string(),
        refresh_token: Some("rt-secret".to_string()),
        token_type: "Bearer".to_string(),
        expires_at: None,
        scope: Some("mcp:read".to_string()),
        token_endpoint: Some("https://auth.example.com/token".to_string()),
        resource: Some("https://mcp.example.com/".to_string()),
        client_id: Some("client-1".to_string()),
        client_secret: Some("cs-secret".to_string()),
    };
    let rendered = format!("{tokens:?}");
    for secret in ["at-secret", "rt-secret", "cs-secret"] {
        assert!(!rendered.contains(secret), "{secret} leaked: {rendered}");
    }
    assert!(rendered.contains("[REDACTED]"));
    // Non-secret bookkeeping stays visible for debugging.
    assert!(rendered.contains("client-1"));
    assert!(rendered.contains("https://auth.example.com/token"));
}

#[test]
fn registered_client_debug_redacts_the_confidential_secret() {
    let secret = "registration-secret-sentinel";
    let client = RegisteredClient {
        client_id: "public-client-id".into(),
        client_secret: Some(secret.into()),
    };
    let debug = format!("{client:?}");
    assert!(!debug.contains(secret));
    assert!(debug.contains("public-client-id"));
    // Secret-store serialization remains available and complete.
    let stored = serde_json::to_string(&client).unwrap();
    assert!(stored.contains(secret));
}

#[test]
fn pkce_debug_redacts_the_verifier() {
    let pair = PkcePair::generate();
    let rendered = format!("{pair:?}");
    assert!(!rendered.contains(&pair.verifier), "{rendered}");
    assert!(rendered.contains(&pair.challenge));
}

#[test]
fn expiry_and_refresh_windows_are_distinct() {
    let now = Utc::now();
    let tokens = TokenSet {
        access_token: "at".to_string(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_at: Some(now + ChronoDuration::seconds(30)),
        scope: None,
        token_endpoint: None,
        resource: None,
        client_id: None,
        client_secret: None,
    };

    assert!(!tokens.is_expired(now), "still valid for another 30s");
    assert!(
        tokens.needs_refresh(now, DEFAULT_REFRESH_SKEW_SECONDS),
        "but inside the refresh window, so renew before using it"
    );

    let no_expiry = TokenSet {
        expires_at: None,
        ..tokens
    };
    assert!(!no_expiry.is_expired(now));
    assert!(!no_expiry.needs_refresh(now, DEFAULT_REFRESH_SKEW_SECONDS));
}
