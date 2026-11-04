use super::*;

fn authorize_query() -> OAuthAuthorizeQuery {
    OAuthAuthorizeQuery {
        client_id: "mcp_client_test".to_string(),
        redirect_uri: "https://client.example/callback?next=1".to_string(),
        response_type: "code".to_string(),
        code_challenge: "challenge".to_string(),
        code_challenge_method: "S256".to_string(),
        state: "state value".to_string(),
        scope: "mcp".to_string(),
        resource: None,
    }
}

fn auth_user_for_render() -> AuthUser {
    AuthUser {
        mcp_client: None,
        id: uuid::Uuid::nil(),
        email: "ava@example.com".to_string(),
        name: "Ava Root".to_string(),
        roles: vec!["admin".to_string()],
        is_platform_user: false,
        auth_method: crate::auth::middleware::AuthMethod::Jwt,
        organizations: vec![crate::domains::organizations::record::OrgMembership {
            org_id: 42,
            public_id: "org_test".to_string(),
            name: "User Personal".to_string(),
            role: everruns_core::OrgRole::Admin,
        }],
        pat_scope: None,
    }
}

#[test]
fn consent_rejects_unimplemented_scopes_and_explains_localhost_trust() {
    for scope in ["mcp:read", "mcp read", "custom", "MCP"] {
        assert!(validate_scope(scope).is_err());
    }
    assert!(validate_scope("mcp").is_ok());
    assert!(validate_scope(" ").is_ok());
    let mut query = authorize_query();
    query.redirect_uri = "http://localhost:9999/callback".into();
    let html = render_authorize_confirm_page(
        &query,
        "<Trusted App>",
        &auth_user_for_render(),
        "csrf",
        "https://example.com",
        None,
    );
    assert!(html.contains("&lt;Trusted App&gt;"));
    assert!(html.contains("self-declared name"));
    assert!(html.contains("Localhost does not verify"));
    assert!(html.contains("all organizations"));
    assert!(html.contains("not read-only"));
    assert!(html.contains("Selected redirect host"));
}

#[test]
fn test_generate_random_hex_uniqueness() {
    let a = generate_random_hex();
    let b = generate_random_hex();
    assert_ne!(a, b);
    assert_eq!(a.len(), 64); // 32 bytes = 64 hex chars
}

#[test]
fn token_hash_matches_sha256_known_vectors() {
    for (input, expected) in [
        (
            "",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
    ] {
        assert_eq!(hash_value(input), expected);
    }
}

#[test]
fn test_pkce_s256_verification() {
    use base64::Engine;

    // Generate a verifier and compute the challenge
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

    assert!(verify_pkce_s256(verifier, &challenge));
    assert!(!verify_pkce_s256("wrong-verifier", &challenge));
}

#[test]
fn test_redirect_url_with_existing_query_preserves_pairs() {
    let s = build_oauth_redirect_url(
        "https://app.example.com/cb?next=1",
        &[("code", "C&D"), ("state", "x y")],
    )
    .unwrap();
    assert!(s.starts_with("https://app.example.com/cb?next=1&"));
    assert!(s.contains("code=C%26D"));
    assert!(s.contains("state=x+y"));
    // No double `?` or naive concatenation.
    assert_eq!(s.matches('?').count(), 1);
}

#[test]
fn test_pkce_s256_rfc_example() {
    // RFC 7636 Appendix B test vector
    // verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
    // challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    assert!(verify_pkce_s256(verifier, challenge));
}

#[test]
fn test_authorize_confirm_page_lists_grant_details() {
    let html = render_authorize_confirm_page(
        &authorize_query(),
        "Cursor",
        &auth_user_for_render(),
        "csrf",
        "https://app.example.com",
        None,
    );

    assert!(html.contains("Authorize this MCP client?"));
    assert!(html.contains("<strong>Cursor</strong>"));
    assert!(html.contains("Ava Root &lt;ava@example.com&gt;"));
    // The consent page is intentionally org-free: MCP OAuth tokens are
    // user-scoped and resolve org per request, so showing a single org
    // here would be misleading.
    assert!(!html.contains("Organization"));
    assert!(html.contains("Read and change data, run agents, and send messages"));
    assert!(html.contains("access token and refresh token"));
    assert!(html.contains(r#"<span class="scope">mcp</span>"#));
    assert!(html.contains(r#"name="client_id" value="mcp_client_test""#));
    assert!(
        html.contains(
            "href=\"https://client.example/callback?next=1&amp;error=access_denied&amp;state=state+value&amp;iss=https%3A%2F%2Fapp.example.com\""
        )
    );
}

#[test]
fn test_authorize_confirm_page_escapes_dynamic_values() {
    let mut query = authorize_query();
    query.client_id = "client_<id>".to_string();
    query.redirect_uri = "https://client.example/callback?next=<bad>&ok=1".to_string();
    query.state = "a&b".to_string();
    query.scope = "mcp custom<scope>".to_string();
    let mut user = auth_user_for_render();
    user.name = "Ava \"Root\" <Ops>".to_string();
    user.email = "ava+ops@example.com".to_string();

    let html = render_authorize_confirm_page(
        &query,
        "Cursor <script>alert(\"x\")</script>",
        &user,
        "csrf&token",
        "https://app.example.com",
        None,
    );

    assert!(!html.contains("<script>alert"));
    assert!(html.contains("Cursor &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"));
    assert!(html.contains("client_&lt;id&gt;"));
    assert!(html.contains("Ava &quot;Root&quot; &lt;Ops&gt;"));
    assert!(!html.contains("custom&lt;scope&gt;"));
    assert!(html.contains("csrf&amp;token"));
    assert!(html.contains(
        "next=%3Cbad%3E&amp;ok=1&amp;error=access_denied&amp;state=a%26b&amp;iss=https%3A%2F%2Fapp.example.com"
    ));
}

#[test]
fn test_authorize_confirm_page_switch_account_link() {
    let html = render_authorize_confirm_page(
        &authorize_query(),
        "Cursor",
        &auth_user_for_render(),
        "csrf",
        "https://app.example.com",
        Some("https://app.example.com/login?return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dx%26a=b"),
    );

    assert!(html.contains("Not Ava Root?"));
    assert!(html.contains(
        "href=\"https://app.example.com/login?return_to=%2Foauth%2Fauthorize%3Fclient_id%3Dx%26a=b\"",
    ));
    assert!(html.contains(">Switch account</a>"));
}

#[test]
fn test_authorize_confirm_page_normalizes_empty_scope() {
    let mut query = authorize_query();
    query.scope = " \t ".to_string();

    let html = render_authorize_confirm_page(
        &query,
        "ChatGPT",
        &auth_user_for_render(),
        "csrf",
        "https://app.example.com",
        None,
    );

    assert!(html.contains(r#"<span class="scope">mcp</span>"#));
    assert!(html.contains(r#"name="scope" value="mcp""#));
    // No account to switch to: the link stays hidden.
    assert!(!html.contains("Switch account"));
}

#[test]
fn test_application_type_deserializes_from_oidc_values() {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        #[serde(default)]
        application_type: Option<OAuthApplicationType>,
    }

    let native: Wrapper = serde_json::from_str(r#"{"application_type":"native"}"#).unwrap();
    assert_eq!(native.application_type, Some(OAuthApplicationType::Native));
    let web: Wrapper = serde_json::from_str(r#"{"application_type":"web"}"#).unwrap();
    assert_eq!(web.application_type, Some(OAuthApplicationType::Web));
    // Omitted stays unstated rather than defaulting to `web`, so the MCP
    // clients in the field today keep registering loopback callbacks.
    let absent: Wrapper = serde_json::from_str("{}").unwrap();
    assert_eq!(absent.application_type, None);
    assert!(serde_json::from_str::<Wrapper>(r#"{"application_type":"desktop"}"#).is_err());
}

#[test]
fn test_authorize_redirect_carries_rfc_9207_issuer() {
    let url = build_oauth_redirect_url(
        "https://client.example/callback",
        &[
            ("code", "abc"),
            ("state", "xyz"),
            ("iss", "https://app.example.com"),
        ],
    )
    .unwrap();
    assert!(
        url.contains("iss=https%3A%2F%2Fapp.example.com"),
        "authorization response must identify the issuer: {url}"
    );
}
