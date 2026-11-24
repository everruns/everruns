use super::*;
use crate::auth::config::AuthConfig;

#[test]
fn test_generate_oauth_state_is_unique() {
    let s1 = generate_oauth_state();
    let s2 = generate_oauth_state();
    assert_ne!(s1, s2);
    assert_eq!(s1.len(), 32); // 16 bytes hex-encoded
}

// EVE-632 / TM-AUTH-014: password registration must not disclose that an
// account exists. OAuth-link conflicts are different: that caller has
// already proved mailbox ownership, so the callback may safely return its
// dedicated 409 category.
#[tokio::test]
async fn registration_existing_account_message_does_not_leak_existence() {
    use axum::response::IntoResponse;

    let response = AuthError::unauthorized(GENERIC_REGISTRATION_FAILED).into_response();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body = String::from_utf8(bytes.to_vec()).unwrap().to_lowercase();

    for leak in [
        "already exists",
        "account with this email",
        "existing credentials",
    ] {
        assert!(
            !body.contains(leak),
            "OAuth signup error must not disclose account existence (leaked '{leak}'): {body}"
        );
    }
    assert_eq!(GENERIC_REGISTRATION_FAILED, "Registration failed");
}

fn oauth_link_candidate(verified: bool, provider: Option<&str>) -> UserRow {
    let now = Utc::now();
    UserRow {
        id: Uuid::now_v7(),
        email: "victim@example.com".to_string(),
        name: "Victim".to_string(),
        avatar_url: None,
        roles: serde_json::json!(["user"]),
        password_hash: Some("argon2-hash".to_string()),
        email_verified: verified,
        auth_provider: provider.map(str::to_string),
        auth_provider_id: None,
        created_at: now,
        updated_at: now,
        external_id: None,
    }
}

#[test]
fn oauth_link_rejects_unverified_existing_email_account() {
    let user = oauth_link_candidate(false, Some("local"));

    assert_eq!(
        existing_oauth_link_rejection_reason(&user, "google"),
        Some("existing_email_unverified")
    );
}

#[test]
fn oauth_link_allows_verified_local_account() {
    let user = oauth_link_candidate(true, Some("local"));

    assert_eq!(existing_oauth_link_rejection_reason(&user, "google"), None);
}

#[test]
fn oauth_link_rejects_verified_account_from_another_provider() {
    let user = oauth_link_candidate(true, Some("github"));

    assert_eq!(
        existing_oauth_link_rejection_reason(&user, "google"),
        Some("email_bound_to_different_provider")
    );
}

#[test]
fn oauth_link_allows_verified_account_from_same_provider() {
    let user = oauth_link_candidate(true, Some("github"));

    assert_eq!(existing_oauth_link_rejection_reason(&user, "github"), None);
}

#[test]
fn test_oauth_providers_hidden_when_oauth_disabled() {
    let mut config = AuthConfig::default();
    config.mode = AuthMode::External;
    config.google = Some(crate::auth::config::GoogleOAuthConfig {
        base: crate::auth::config::OAuthProviderConfig {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
        },
        allowed_domains: None,
    });
    config.github = Some(crate::auth::config::GitHubOAuthConfig {
        base: crate::auth::config::OAuthProviderConfig {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
        },
    });

    assert!(oauth_providers(&config).is_empty());
    assert!(ensure_oauth_enabled(&config).is_err());
}

fn make_google_user(email: &str, verified: bool) -> super::super::oauth::OAuthUserInfo {
    super::super::oauth::OAuthUserInfo {
        provider_id: "google-sub-1".to_string(),
        email: email.to_string(),
        name: "User".to_string(),
        avatar_url: None,
        email_verified: verified,
    }
}

fn config_with_google(allowed_domains: Option<Vec<String>>) -> AuthConfig {
    let mut config = AuthConfig::default();
    config.mode = AuthMode::Full;
    config.google = Some(crate::auth::config::GoogleOAuthConfig {
        base: crate::auth::config::OAuthProviderConfig {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
        },
        allowed_domains,
    });
    config
}

#[test]
fn test_email_domain_allowed_matches_case_insensitive() {
    let allowed = vec!["Example.com".to_string(), "Other.io".to_string()];
    assert!(email_domain_allowed("user@example.com", &allowed));
    assert!(email_domain_allowed("user@EXAMPLE.COM", &allowed));
    assert!(email_domain_allowed("user@other.io", &allowed));
    assert!(!email_domain_allowed("user@evil.com", &allowed));
    assert!(!email_domain_allowed("not-an-email", &allowed));
    assert!(!email_domain_allowed("user@", &allowed));
}

// EVE-451 (review feedback): operators sometimes write `@company.com` —
// accept it as an equivalent of `company.com` rather than silently
// failing closed.
#[test]
fn test_email_domain_allowed_accepts_at_prefixed_entries() {
    let allowed = vec!["@company.com".to_string(), " @other.io ".to_string()];
    assert!(email_domain_allowed("user@company.com", &allowed));
    assert!(email_domain_allowed("user@other.io", &allowed));
    assert!(!email_domain_allowed("user@evil.com", &allowed));
}

// EVE-451 (review feedback): exact-domain semantics — `company.com` must
// not authorize an `attacker.company.com` subdomain. Operators who want
// subdomains must list each one.
#[test]
fn test_email_domain_allowed_does_not_match_subdomains() {
    let allowed = vec!["company.com".to_string()];
    assert!(!email_domain_allowed("user@attacker.company.com", &allowed));
    assert!(!email_domain_allowed("user@xcompany.com", &allowed));
}

#[test]
fn test_normalize_allowed_domain_drops_empty_entries() {
    assert_eq!(normalize_allowed_domain(""), None);
    assert_eq!(normalize_allowed_domain("   "), None);
    assert_eq!(normalize_allowed_domain("@"), None);
    assert_eq!(normalize_allowed_domain(" @  "), None);
    assert_eq!(
        normalize_allowed_domain("@Company.COM"),
        Some("company.com".to_string())
    );
    assert_eq!(
        normalize_allowed_domain("  company.com  "),
        Some("company.com".to_string())
    );
}

// EVE-451: Google OAuth must require email_verified=true.
#[test]
fn test_google_rejects_unverified_email() {
    let config = config_with_google(None);
    let user = make_google_user("user@example.com", false);
    assert_eq!(
        oauth_identity_rejection_reason(super::super::oauth::OAuthProvider::Google, &config, &user),
        Some("email_unverified")
    );
}

// EVE-451: When allowed_domains is set, mismatched email domains are rejected.
#[test]
fn test_google_rejects_disallowed_domain() {
    let config = config_with_google(Some(vec!["company.com".to_string()]));
    let user = make_google_user("user@evil.com", true);
    assert_eq!(
        oauth_identity_rejection_reason(super::super::oauth::OAuthProvider::Google, &config, &user),
        Some("domain_not_allowed")
    );
}

// EVE-451: Allowed domain on a verified email passes.
#[test]
fn test_google_accepts_allowed_domain_verified() {
    let config = config_with_google(Some(vec!["company.com".to_string()]));
    let user = make_google_user("user@Company.COM", true);
    assert!(
        oauth_identity_rejection_reason(super::super::oauth::OAuthProvider::Google, &config, &user)
            .is_none()
    );
}

// EVE-702: GitHub identities with an unverified email are rejected, mirroring
// Google, so a hardcoded email_verified=true can no longer pre-empt accounts.
#[test]
fn test_github_rejects_unverified_email() {
    let config = AuthConfig::default();
    let user = make_google_user("user@example.com", false);
    assert_eq!(
        oauth_identity_rejection_reason(super::super::oauth::OAuthProvider::GitHub, &config, &user),
        Some("email_unverified")
    );
}

// EVE-702: a verified GitHub email passes the gate.
#[test]
fn test_github_accepts_verified_email() {
    let config = AuthConfig::default();
    let user = make_google_user("user@example.com", true);
    assert!(
        oauth_identity_rejection_reason(super::super::oauth::OAuthProvider::GitHub, &config, &user)
            .is_none()
    );
}

// EVE-451: No allowed_domains config means any verified domain is accepted.
#[test]
fn test_google_accepts_any_verified_domain_when_unrestricted() {
    let config = config_with_google(None);
    let user = make_google_user("user@anywhere.io", true);
    assert!(
        oauth_identity_rejection_reason(super::super::oauth::OAuthProvider::Google, &config, &user)
            .is_none()
    );
}

// EVE-451: Empty allowed_domains list (only whitespace) does not lock everyone out.
// Treats it as "no restriction" rather than "deny all" so an operator who
// sets `AUTH_GOOGLE_ALLOWED_DOMAINS=` does not accidentally brick login.
#[test]
fn test_google_empty_allowed_domains_does_not_lock_out() {
    let config = config_with_google(Some(vec!["   ".to_string()]));
    let user = make_google_user("user@anywhere.io", true);
    assert!(
        oauth_identity_rejection_reason(super::super::oauth::OAuthProvider::Google, &config, &user)
            .is_none()
    );
}

// EVE-702 replaced EVE-451's "GitHub has no gates" assertion: GitHub now
// gates on email_verified. See test_github_rejects_unverified_email and
// test_github_accepts_verified_email above.

#[test]
fn test_oauth_providers_visible_when_oauth_enabled() {
    let mut config = AuthConfig::default();
    config.mode = AuthMode::Full;
    config.google = Some(crate::auth::config::GoogleOAuthConfig {
        base: crate::auth::config::OAuthProviderConfig {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
        },
        allowed_domains: None,
    });
    config.github = Some(crate::auth::config::GitHubOAuthConfig {
        base: crate::auth::config::OAuthProviderConfig {
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "http://localhost/callback".to_string(),
        },
    });

    assert_eq!(
        oauth_providers(&config),
        vec!["google".to_string(), "github".to_string()]
    );
    assert!(ensure_oauth_enabled(&config).is_ok());
}

// Minimal signup derives a display name from the email local-part when the
// client omits `name`.
#[test]
fn test_display_name_from_email() {
    assert_eq!(display_name_from_email("eli@acme.com"), "Eli");
    assert_eq!(display_name_from_email("eli.wong@x.com"), "Eli Wong");
    assert_eq!(display_name_from_email("eli_wong@x.com"), "Eli Wong");
    assert_eq!(display_name_from_email("eli-wong@x.com"), "Eli Wong");
    // Degenerate local-parts fall back to the raw email rather than "".
    assert_eq!(display_name_from_email("@x.com"), "@x.com");
}
