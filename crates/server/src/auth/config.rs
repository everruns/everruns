// Authentication configuration loaded from environment variables.
// Decision: Follow Langfuse pattern with AUTH_ prefix for all auth config
// Decision: Default to "none" mode for local development
//
// Trust boundary (External mode + OAuth providers): when `AUTH_MODE=external`
// the platform delegates user identity to a third-party provider (PropelAuth,
// Auth0, Clerk, etc.) and the built-in OAuth flow is intentionally disabled.
// Since #1492 the runtime quietly hides configured providers and returns 401 on
// `/v1/auth/oauth/{provider}` and `/v1/auth/callback/{provider}` (the login
// OAuth handlers — distinct from the MCP `/oauth/...` handlers). That made
// hybrid deployments fail silently — operators saw 401s only at request
// time. We now fail fast at startup: `AuthConfig::validate()` rejects
// `mode == External` when any OAuth provider config is present, with a clear
// message that names both the conflicting mode and the specific providers.
// Operators must remove `google`/`github` config or change `AUTH_MODE`
// before the server starts. See `specs/authentication.md` (External Mode
// and OAuth Providers).

use everruns_core::config::env_opt_string_any;
use std::time::Duration;

const DEFAULT_PUBLIC_APP_URL: &str = "http://localhost:9300";
const DEFAULT_API_PREFIX: &str = "/api";

/// Authentication mode
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthMode {
    /// No authentication required (local development)
    #[default]
    None,
    /// Admin-only mode via environment variables
    Admin,
    /// Full authentication (password + OAuth + API keys)
    Full,
    /// External authentication (third-party provider like PropelAuth, Auth0, Clerk)
    External,
}

impl AuthMode {
    /// Parse a recognized `AUTH_MODE` value, returning `None` for anything
    /// unrecognized so callers can fail closed.
    ///
    /// The previous lenient `parse` mapped any unknown value (including typos
    /// like `AUTH_MODE=production`) to `AuthMode::None` — which resolves every
    /// request to an anonymous admin. Unknown values must be rejected, not
    /// silently downgraded to no-auth. See EVE-621 / TM-AUTH-011.
    pub fn parse_known(s: &str) -> Option<AuthMode> {
        match s.trim().to_lowercase().as_str() {
            "none" => Some(AuthMode::None),
            "admin" => Some(AuthMode::Admin),
            "full" => Some(AuthMode::Full),
            "external" => Some(AuthMode::External),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMode::None => "none",
            AuthMode::Admin => "admin",
            AuthMode::Full => "full",
            AuthMode::External => "external",
        }
    }
}

/// Resolve the effective authentication mode from the raw `AUTH_MODE` value and
/// the deployment grade, failing closed against the fail-open default
/// (EVE-621 / TM-AUTH-011):
///
/// - Unrecognized `AUTH_MODE` values are rejected — a typo such as
///   `AUTH_MODE=production` must not silently fall back to no-auth.
/// - `none` mode resolves every request to an anonymous admin, so it is only
///   permitted in dev deployments. A non-dev deployment that omits `AUTH_MODE`
///   (or sets it to `none`) fails startup instead of coming up unauthenticated.
fn resolve_auth_mode(
    raw: Option<&str>,
    grade: everruns_core::DeploymentGrade,
) -> Result<AuthMode, String> {
    let mode = match raw {
        Some(s) => AuthMode::parse_known(s).ok_or_else(|| {
            format!(
                "AUTH_MODE='{s}' is not a recognized value \
                 (expected one of: none, admin, full, external)"
            )
        })?,
        None => AuthMode::None,
    };

    if mode == AuthMode::None && !grade.is_dev() {
        return Err(format!(
            "AUTH_MODE=none disables authentication — every request is treated as an \
             anonymous admin with full access — and is only allowed in dev deployments \
             (current deployment grade: {grade}). Set AUTH_MODE=admin|full|external for \
             this deployment, or set DEPLOYMENT_GRADE=dev (or DEV_MODE=true) to explicitly \
             opt into no-auth dev mode."
        ));
    }

    Ok(mode)
}

/// OAuth provider configuration
#[derive(Debug, Clone)]
pub struct OAuthProviderConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

/// Google OAuth configuration
#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    pub base: OAuthProviderConfig,
    /// Optional: restrict to specific domains
    pub allowed_domains: Option<Vec<String>>,
}

/// GitHub OAuth configuration (for login)
#[derive(Debug, Clone)]
pub struct GitHubOAuthConfig {
    pub base: OAuthProviderConfig,
}

/// GitHub App configuration (for repo access via GitHub App installation tokens)
#[derive(Debug, Clone)]
pub struct GitHubConnectionConfig {
    /// GitHub App ID (numeric, from App settings page)
    pub app_id: String,
    /// PEM-encoded RSA private key for JWT signing
    pub private_key: String,
    /// App slug for constructing installation URLs (e.g. "everruns")
    pub app_slug: String,
    /// Callback URL after user installs the GitHub App
    pub setup_url: String,
}

/// Admin user configuration (for admin-only mode or initial setup)
#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub email: String,
    pub password: String,
}

/// JWT configuration
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// Secret key for signing JWTs
    pub secret: String,
    /// Access token lifetime
    pub access_token_lifetime: Duration,
    /// Refresh token lifetime
    pub refresh_token_lifetime: Duration,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            access_token_lifetime: Duration::from_secs(15 * 60), // 15 minutes
            refresh_token_lifetime: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
        }
    }
}

/// Complete authentication configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Authentication mode
    pub mode: AuthMode,
    /// Base URL for OAuth callbacks (backend origin)
    pub base_url: String,
    /// Frontend URL for post-auth redirects (UI origin, same as base_url in production)
    pub frontend_url: String,
    /// JWT configuration
    pub jwt: JwtConfig,
    /// Admin user (for admin mode or initial setup)
    pub admin: Option<AdminConfig>,
    /// Google OAuth configuration
    pub google: Option<GoogleOAuthConfig>,
    /// GitHub OAuth configuration (login)
    pub github: Option<GitHubOAuthConfig>,
    /// GitHub Connection OAuth configuration (repo access, separate OAuth App)
    pub github_connection: Option<GitHubConnectionConfig>,
    /// Whether to disable password authentication
    pub disable_password_auth: bool,
    /// Whether to disable signup (registration)
    pub disable_signup: bool,
    /// Session max age in seconds (default: 30 days)
    pub session_max_age: Duration,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: AuthMode::None,
            base_url: "http://localhost:9300/api".to_string(),
            frontend_url: "http://localhost:9300".to_string(),
            jwt: JwtConfig::default(),
            admin: None,
            google: None,
            github: None,
            github_connection: None,
            disable_password_auth: false,
            disable_signup: false,
            session_max_age: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
        }
    }
}

impl AuthConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        // Fail closed: reject unknown AUTH_MODE values and forbid no-auth
        // `none` mode outside dev deployments, so a production server cannot
        // silently come up unauthenticated by omission or typo (EVE-621).
        let grade = everruns_core::DeploymentGrade::from_env();
        let mode = resolve_auth_mode(std::env::var("AUTH_MODE").ok().as_deref(), grade)
            .unwrap_or_else(|err| panic!("{err}"));
        if mode == AuthMode::None {
            tracing::warn!(
                "AUTH_MODE=none: authentication is DISABLED — every request is treated as an \
                 anonymous admin with full access. Intended for local development only."
            );
        }

        let api_prefix =
            std::env::var("API_PREFIX").unwrap_or_else(|_| DEFAULT_API_PREFIX.to_string());
        let public_app_url = env_opt_string_any(&["PUBLIC_APP_URL", "APP_URL", "FRONTEND_URL"]);
        let base_url = env_opt_string_any(&["AUTH_BASE_URL", "BASE_URL"])
            .or_else(|| {
                public_app_url
                    .as_deref()
                    .map(|url| auth_base_url_from_public_app_url(url, &api_prefix))
            })
            .unwrap_or_else(|| {
                auth_base_url_from_public_app_url(DEFAULT_PUBLIC_APP_URL, &api_prefix)
            });

        // Post-auth redirects land on the public app origin. In single-origin
        // deployments this is the same host as AUTH_BASE_URL, without API_PREFIX.
        let frontend_url = env_opt_string_any(&["FRONTEND_URL"])
            .or(public_app_url.clone())
            .unwrap_or_else(|| frontend_url_from_auth_base_url(&base_url, &api_prefix));

        // JWT configuration
        // TM-AUTH-002: Require AUTH_JWT_SECRET when authentication is enabled.
        // In AuthMode::None, generate a random secret (tokens are not validated).
        // In AuthMode::Admin or AuthMode::Full, refuse to start without a secret.
        let jwt_secret = std::env::var("AUTH_JWT_SECRET").unwrap_or_else(|_| {
            if mode == AuthMode::None {
                use rand::RngExt;
                let bytes: [u8; 32] = rand::rng().random();
                hex::encode(bytes)
            } else {
                panic!(
                    "AUTH_JWT_SECRET must be set when AUTH_MODE is '{}'. \
                     Without it, JWTs can be forged by anyone.",
                    mode.as_str()
                )
            }
        });

        let access_token_lifetime = std::env::var("AUTH_JWT_ACCESS_TOKEN_LIFETIME")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(15 * 60));

        let refresh_token_lifetime = std::env::var("AUTH_JWT_REFRESH_TOKEN_LIFETIME")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(30 * 24 * 60 * 60));

        let jwt = JwtConfig {
            secret: jwt_secret,
            access_token_lifetime,
            refresh_token_lifetime,
        };

        // Admin configuration
        let admin = match (
            std::env::var("AUTH_ADMIN_EMAIL"),
            std::env::var("AUTH_ADMIN_PASSWORD"),
        ) {
            (Ok(email), Ok(password)) if !email.is_empty() && !password.is_empty() => {
                Some(AdminConfig { email, password })
            }
            _ => None,
        };

        // Google OAuth configuration
        let google = match (
            std::env::var("AUTH_GOOGLE_CLIENT_ID"),
            std::env::var("AUTH_GOOGLE_CLIENT_SECRET"),
        ) {
            (Ok(client_id), Ok(client_secret))
                if !client_id.is_empty() && !client_secret.is_empty() =>
            {
                let redirect_uri = std::env::var("AUTH_GOOGLE_REDIRECT_URI").unwrap_or_else(|_| {
                    format!("{}/v1/auth/callback/google", base_url.trim_end_matches('/'))
                });
                let allowed_domains = std::env::var("AUTH_GOOGLE_ALLOWED_DOMAINS")
                    .ok()
                    .map(|s| s.split(',').map(|s| s.trim().to_string()).collect());
                Some(GoogleOAuthConfig {
                    base: OAuthProviderConfig {
                        client_id,
                        client_secret,
                        redirect_uri,
                    },
                    allowed_domains,
                })
            }
            _ => None,
        };

        // GitHub OAuth configuration
        let github = match (
            std::env::var("AUTH_GITHUB_CLIENT_ID"),
            std::env::var("AUTH_GITHUB_CLIENT_SECRET"),
        ) {
            (Ok(client_id), Ok(client_secret))
                if !client_id.is_empty() && !client_secret.is_empty() =>
            {
                let redirect_uri = std::env::var("AUTH_GITHUB_REDIRECT_URI").unwrap_or_else(|_| {
                    format!("{}/v1/auth/callback/github", base_url.trim_end_matches('/'))
                });
                Some(GitHubOAuthConfig {
                    base: OAuthProviderConfig {
                        client_id,
                        client_secret,
                        redirect_uri,
                    },
                })
            }
            _ => None,
        };

        // GitHub App for repo access (installation-based tokens)
        let github_connection = match (
            std::env::var("GITHUB_APP_ID"),
            std::env::var("GITHUB_APP_PRIVATE_KEY"),
        ) {
            (Ok(app_id), Ok(private_key)) if !app_id.is_empty() && !private_key.is_empty() => {
                // Support escaped newlines in env vars (common in Docker/CI)
                let private_key = private_key.replace("\\n", "\n");
                let app_slug =
                    std::env::var("GITHUB_APP_SLUG").unwrap_or_else(|_| "everruns".to_string());
                let setup_url = std::env::var("GITHUB_APP_SETUP_URL").unwrap_or_else(|_| {
                    format!(
                        "{}/v1/user/connections/github/callback",
                        base_url.trim_end_matches('/')
                    )
                });
                Some(GitHubConnectionConfig {
                    app_id,
                    private_key,
                    app_slug,
                    setup_url,
                })
            }
            _ => None,
        };

        let disable_password_auth = std::env::var("AUTH_DISABLE_PASSWORD")
            .map(|s| s.to_lowercase() == "true" || s == "1")
            .unwrap_or(false);

        let disable_signup = std::env::var("AUTH_DISABLE_SIGNUP")
            .map(|s| s.to_lowercase() == "true" || s == "1")
            .unwrap_or(false);

        let session_max_age = std::env::var("AUTH_SESSION_MAX_AGE")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(|mins: u64| Duration::from_secs(mins * 60))
            .unwrap_or_else(|| Duration::from_secs(30 * 24 * 60 * 60));

        let config = Self {
            mode,
            base_url,
            frontend_url,
            jwt,
            admin,
            google,
            github,
            github_connection,
            disable_password_auth,
            disable_signup,
            session_max_age,
        };

        // Fail fast on conflicting auth-config combinations so operators see
        // the problem at startup instead of as silent 401s at request time.
        // See top-of-file decision comment for rationale.
        if let Err(err) = config.validate() {
            panic!("{err}");
        }
        config
    }

    /// Validate cross-field invariants on the assembled config. Returns a
    /// human-readable error message that operators can paste into a chat
    /// channel without further translation.
    ///
    /// Currently enforces:
    /// - `mode == External` is incompatible with any built-in OAuth provider
    ///   (`google` or `github`). External mode delegates identity to a
    ///   third-party provider; configuring both at once is ambiguous and
    ///   was the source of the silent 401s in #1492.
    pub fn validate(&self) -> Result<(), String> {
        if self.mode == AuthMode::External {
            let mut configured: Vec<&str> = Vec::new();
            if self.google.is_some() {
                configured.push("AUTH_GOOGLE_CLIENT_ID, AUTH_GOOGLE_CLIENT_SECRET");
            }
            if self.github.is_some() {
                configured.push("AUTH_GITHUB_CLIENT_ID, AUTH_GITHUB_CLIENT_SECRET");
            }
            if !configured.is_empty() {
                return Err(format!(
                    "AUTH_MODE=external is incompatible with built-in OAuth providers \
                     ({}). External mode delegates identity to a third-party provider; \
                     remove the OAuth env vars or set AUTH_MODE=full to use the built-in \
                     OAuth flow. See specs/authentication.md (External Mode and OAuth \
                     Providers).",
                    configured.join("; ")
                ));
            }
        }
        Ok(())
    }

    /// Check if authentication is enabled
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.mode != AuthMode::None
    }

    /// Check if password authentication is available
    pub fn password_auth_enabled(&self) -> bool {
        // Admin mode always has password auth (that's how you log in with admin credentials)
        // Full mode has password auth unless explicitly disabled
        // External mode: password auth is managed by the external provider, not us
        self.mode == AuthMode::Admin || (self.mode == AuthMode::Full && !self.disable_password_auth)
    }

    /// Check if OAuth is available
    pub fn oauth_enabled(&self) -> bool {
        self.mode == AuthMode::Full && (self.google.is_some() || self.github.is_some())
    }

    /// Check if user signup (registration) is enabled
    pub fn signup_enabled(&self) -> bool {
        self.mode != AuthMode::Admin && self.mode != AuthMode::External && !self.disable_signup
    }

    /// Check if API key authentication is available
    #[allow(dead_code)]
    pub fn api_key_auth_enabled(&self) -> bool {
        self.mode == AuthMode::Full
            || self.mode == AuthMode::Admin
            || self.mode == AuthMode::External
    }
}

fn normalize_api_prefix(api_prefix: &str) -> String {
    let trimmed = api_prefix.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

fn auth_base_url_from_public_app_url(public_app_url: &str, api_prefix: &str) -> String {
    let origin = public_app_url.trim_end_matches('/');
    let api_prefix = normalize_api_prefix(api_prefix);
    if api_prefix.is_empty() {
        origin.to_string()
    } else {
        format!("{origin}{api_prefix}")
    }
}

fn frontend_url_from_auth_base_url(auth_base_url: &str, api_prefix: &str) -> String {
    let base_url = auth_base_url.trim_end_matches('/');
    let api_prefix = normalize_api_prefix(api_prefix);

    if !api_prefix.is_empty()
        && let Some(root) = base_url.strip_suffix(&api_prefix)
    {
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            DEFAULT_PUBLIC_APP_URL.to_string()
        } else {
            root.to_string()
        }
    } else if base_url.is_empty() {
        DEFAULT_PUBLIC_APP_URL.to_string()
    } else {
        base_url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const AUTH_URL_ENV_VARS: &[&str] = &[
        "API_PREFIX",
        "APP_URL",
        "AUTH_BASE_URL",
        "BASE_URL",
        "FRONTEND_URL",
        "PUBLIC_APP_URL",
    ];

    fn clear_auth_url_env() {
        for key in AUTH_URL_ENV_VARS {
            // SAFETY: tests that mutate these process-wide env vars hold ENV_LOCK.
            unsafe { std::env::remove_var(key) };
        }
    }

    #[test]
    fn test_auth_mode_parsing() {
        assert_eq!(AuthMode::parse_known("none"), Some(AuthMode::None));
        assert_eq!(AuthMode::parse_known("NONE"), Some(AuthMode::None));
        assert_eq!(AuthMode::parse_known("admin"), Some(AuthMode::Admin));
        assert_eq!(AuthMode::parse_known("ADMIN"), Some(AuthMode::Admin));
        assert_eq!(AuthMode::parse_known("full"), Some(AuthMode::Full));
        assert_eq!(AuthMode::parse_known("FULL"), Some(AuthMode::Full));
        assert_eq!(AuthMode::parse_known("external"), Some(AuthMode::External));
        assert_eq!(AuthMode::parse_known("EXTERNAL"), Some(AuthMode::External));
        // EVE-621: unknown values are rejected, not silently mapped to no-auth.
        assert_eq!(AuthMode::parse_known("invalid"), None);
        assert_eq!(AuthMode::parse_known("production"), None);
    }

    #[test]
    fn test_resolve_auth_mode_fails_closed() {
        use everruns_core::DeploymentGrade;

        // Unknown AUTH_MODE is rejected in every grade (no silent no-auth).
        assert!(resolve_auth_mode(Some("production"), DeploymentGrade::Dev).is_err());
        assert!(resolve_auth_mode(Some("production"), DeploymentGrade::Prod).is_err());

        // `none` (explicit or by omission) is only allowed in dev.
        assert_eq!(
            resolve_auth_mode(None, DeploymentGrade::Dev),
            Ok(AuthMode::None)
        );
        assert_eq!(
            resolve_auth_mode(Some("none"), DeploymentGrade::Dev),
            Ok(AuthMode::None)
        );
        assert!(resolve_auth_mode(None, DeploymentGrade::Prod).is_err());
        assert!(resolve_auth_mode(Some("none"), DeploymentGrade::Prod).is_err());
        assert!(resolve_auth_mode(None, DeploymentGrade::Preview).is_err());
        assert!(resolve_auth_mode(None, DeploymentGrade::Poc).is_err());

        // Real auth modes are accepted regardless of grade.
        for grade in [DeploymentGrade::Dev, DeploymentGrade::Prod] {
            assert_eq!(resolve_auth_mode(Some("admin"), grade), Ok(AuthMode::Admin));
            assert_eq!(resolve_auth_mode(Some("full"), grade), Ok(AuthMode::Full));
            assert_eq!(
                resolve_auth_mode(Some("external"), grade),
                Ok(AuthMode::External)
            );
        }
    }

    #[test]
    fn test_default_config() {
        let config = AuthConfig::default();
        assert_eq!(config.mode, AuthMode::None);
        assert!(!config.is_enabled());
        assert!(!config.password_auth_enabled());
        assert!(!config.oauth_enabled());
    }

    #[test]
    fn test_auth_base_url_from_public_app_url() {
        assert_eq!(
            auth_base_url_from_public_app_url("https://app.example.com/", "/api"),
            "https://app.example.com/api"
        );
        assert_eq!(
            auth_base_url_from_public_app_url("https://app.example.com", "custom-api"),
            "https://app.example.com/custom-api"
        );
        assert_eq!(
            auth_base_url_from_public_app_url("https://app.example.com", ""),
            "https://app.example.com"
        );
    }

    #[test]
    fn test_frontend_url_from_auth_base_url() {
        assert_eq!(
            frontend_url_from_auth_base_url("https://app.example.com/api", "/api"),
            "https://app.example.com"
        );
        assert_eq!(
            frontend_url_from_auth_base_url("https://api.example.com", "/api"),
            "https://api.example.com"
        );
        assert_eq!(
            frontend_url_from_auth_base_url("", "/api"),
            DEFAULT_PUBLIC_APP_URL
        );
        assert_eq!(
            frontend_url_from_auth_base_url("/api", "/api"),
            DEFAULT_PUBLIC_APP_URL
        );
    }

    #[test]
    fn test_from_env_derives_base_url_from_api_prefix_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_auth_url_env();
        // SAFETY: this test holds ENV_LOCK.
        // `none` mode (the default) is only permitted in dev deployments, so
        // pin the grade for this base-URL-derivation test (EVE-621).
        unsafe {
            std::env::set_var("DEPLOYMENT_GRADE", "dev");
            std::env::set_var("API_PREFIX", "/custom-api");
        }

        let config = AuthConfig::from_env();

        assert_eq!(config.base_url, "http://localhost:9300/custom-api");
        assert_eq!(config.frontend_url, DEFAULT_PUBLIC_APP_URL);
        clear_auth_url_env();
        // SAFETY: this test holds ENV_LOCK.
        unsafe { std::env::remove_var("DEPLOYMENT_GRADE") };
    }

    #[test]
    fn test_from_env_preserves_empty_api_prefix() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_auth_url_env();
        // SAFETY: this test holds ENV_LOCK.
        // `none` mode (the default) is only permitted in dev deployments, so
        // pin the grade for this base-URL-derivation test (EVE-621).
        unsafe {
            std::env::set_var("DEPLOYMENT_GRADE", "dev");
            std::env::set_var("API_PREFIX", "");
        }

        let config = AuthConfig::from_env();

        assert_eq!(config.base_url, DEFAULT_PUBLIC_APP_URL);
        assert_eq!(config.frontend_url, DEFAULT_PUBLIC_APP_URL);
        clear_auth_url_env();
        // SAFETY: this test holds ENV_LOCK.
        unsafe { std::env::remove_var("DEPLOYMENT_GRADE") };
    }

    #[test]
    fn test_admin_mode_has_password_auth() {
        let config = AuthConfig {
            mode: AuthMode::Admin,
            ..Default::default()
        };
        assert!(config.is_enabled());
        assert!(
            config.password_auth_enabled(),
            "Admin mode should have password auth enabled"
        );
        assert!(!config.oauth_enabled(), "Admin mode should not have OAuth");
    }

    #[test]
    fn test_full_mode_password_auth() {
        // Full mode with password auth enabled (default)
        let config = AuthConfig {
            mode: AuthMode::Full,
            disable_password_auth: false,
            ..Default::default()
        };
        assert!(
            config.password_auth_enabled(),
            "Full mode should have password auth by default"
        );

        // Full mode with password auth disabled
        let config_disabled = AuthConfig {
            mode: AuthMode::Full,
            disable_password_auth: true,
            ..Default::default()
        };
        assert!(
            !config_disabled.password_auth_enabled(),
            "Full mode with disable_password_auth should not have password auth"
        );
    }

    #[test]
    fn test_none_mode_no_password_auth() {
        let config = AuthConfig {
            mode: AuthMode::None,
            ..Default::default()
        };
        assert!(
            !config.password_auth_enabled(),
            "None mode should not have password auth"
        );
    }

    #[test]
    fn test_admin_config_credentials() {
        // Test that AdminConfig stores credentials correctly
        let admin = AdminConfig {
            email: "admin@example.com".to_string(),
            password: "secret123".to_string(),
        };

        assert_eq!(admin.email, "admin@example.com");
        assert_eq!(admin.password, "secret123");

        // Simulate credential check (same logic as login handler)
        let test_email = "admin@example.com";
        let test_password = "secret123";
        assert!(
            test_email == admin.email && test_password == admin.password,
            "Matching credentials should pass"
        );

        // Wrong password
        let wrong_password = "wrongpass";
        assert!(
            !(test_email == admin.email && wrong_password == admin.password),
            "Wrong password should fail"
        );

        // Wrong email
        let wrong_email = "user@example.com";
        assert!(
            !(wrong_email == admin.email && test_password == admin.password),
            "Wrong email should fail"
        );
    }

    #[test]
    fn test_admin_mode_requires_admin_config() {
        // Admin mode without admin config
        let config_no_admin = AuthConfig {
            mode: AuthMode::Admin,
            admin: None,
            ..Default::default()
        };
        assert!(config_no_admin.admin.is_none(), "Admin config can be None");

        // Admin mode with admin config
        let config_with_admin = AuthConfig {
            mode: AuthMode::Admin,
            admin: Some(AdminConfig {
                email: "admin@example.com".to_string(),
                password: "changeme".to_string(),
            }),
            ..Default::default()
        };
        assert!(
            config_with_admin.admin.is_some(),
            "Admin config should be set"
        );
        let admin = config_with_admin.admin.unwrap();
        assert_eq!(admin.email, "admin@example.com");
        assert_eq!(admin.password, "changeme");
    }

    #[test]
    fn test_external_mode() {
        let config = AuthConfig {
            mode: AuthMode::External,
            ..Default::default()
        };
        assert!(config.is_enabled());
        assert!(
            !config.password_auth_enabled(),
            "External mode should not have password auth (managed by external provider)"
        );
        assert!(
            !config.oauth_enabled(),
            "External mode should not have built-in OAuth"
        );
        assert!(
            config.api_key_auth_enabled(),
            "External mode should support API keys"
        );
    }

    #[test]
    fn test_admin_mode_signup_should_be_disabled() {
        let config = AuthConfig {
            mode: AuthMode::Admin,
            disable_signup: false, // Even if not explicitly disabled
            ..Default::default()
        };
        assert!(
            !config.signup_enabled(),
            "Admin mode should never allow signup"
        );
    }

    fn make_google_config() -> GoogleOAuthConfig {
        GoogleOAuthConfig {
            base: OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "https://example.com/callback".to_string(),
            },
            allowed_domains: None,
        }
    }

    fn make_github_config() -> GitHubOAuthConfig {
        GitHubOAuthConfig {
            base: OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "https://example.com/callback".to_string(),
            },
        }
    }

    #[test]
    fn test_validate_external_mode_with_google_oauth_rejected() {
        let config = AuthConfig {
            mode: AuthMode::External,
            google: Some(make_google_config()),
            ..Default::default()
        };
        let err = config.validate().expect_err("expected validation error");
        assert!(err.contains("AUTH_MODE=external"), "got: {err}");
        assert!(err.contains("AUTH_GOOGLE_CLIENT_ID"), "got: {err}");
    }

    #[test]
    fn test_validate_external_mode_with_github_oauth_rejected() {
        let config = AuthConfig {
            mode: AuthMode::External,
            github: Some(make_github_config()),
            ..Default::default()
        };
        let err = config.validate().expect_err("expected validation error");
        assert!(err.contains("AUTH_MODE=external"), "got: {err}");
        assert!(err.contains("AUTH_GITHUB_CLIENT_ID"), "got: {err}");
    }

    #[test]
    fn test_validate_external_mode_with_both_providers_lists_both() {
        let config = AuthConfig {
            mode: AuthMode::External,
            google: Some(make_google_config()),
            github: Some(make_github_config()),
            ..Default::default()
        };
        let err = config.validate().expect_err("expected validation error");
        assert!(err.contains("AUTH_GOOGLE_CLIENT_ID"), "got: {err}");
        assert!(err.contains("AUTH_GITHUB_CLIENT_ID"), "got: {err}");
    }

    #[test]
    fn test_validate_external_mode_without_oauth_passes() {
        // External mode without any built-in OAuth config is the supported
        // setup — third-party provider handles identity, no conflict.
        let config = AuthConfig {
            mode: AuthMode::External,
            ..Default::default()
        };
        config.validate().expect("expected validation to pass");
    }

    #[test]
    fn test_validate_full_mode_with_oauth_passes() {
        // Full mode supports built-in OAuth when providers are configured;
        // that combination must validate successfully. (Full mode is also
        // valid without OAuth — password / API-key auth still work — and
        // that case is covered by `test_full_mode_password_auth`.)
        let config = AuthConfig {
            mode: AuthMode::Full,
            google: Some(make_google_config()),
            github: Some(make_github_config()),
            ..Default::default()
        };
        config.validate().expect("expected validation to pass");
    }

    #[test]
    fn test_validate_admin_mode_with_oauth_passes() {
        // Admin mode doesn't surface OAuth either, but the validator only
        // catches the External-mode conflict — Admin + configured OAuth is
        // unusual but harmless (login goes through admin credentials).
        let config = AuthConfig {
            mode: AuthMode::Admin,
            google: Some(make_google_config()),
            ..Default::default()
        };
        config.validate().expect("expected validation to pass");
    }

    #[test]
    fn test_validate_github_connection_does_not_conflict_with_external() {
        // The repo-access GitHub App (GITHUB_APP_ID / GITHUB_APP_PRIVATE_KEY)
        // is unrelated to the login OAuth flow — it's a per-user connection
        // for repo access. Operators running External mode can still wire
        // the GitHub App for repo features.
        let config = AuthConfig {
            mode: AuthMode::External,
            github_connection: Some(GitHubConnectionConfig {
                app_id: "1".to_string(),
                private_key: "key".to_string(),
                app_slug: "everruns".to_string(),
                setup_url: "https://example.com/setup".to_string(),
            }),
            ..Default::default()
        };
        config.validate().expect("expected validation to pass");
    }

    // TM-AUTH-002: Verify the insecure hardcoded fallback is removed
    #[test]
    fn test_default_jwt_config_has_no_insecure_secret() {
        // AuthConfig::default() should not contain the old hardcoded fallback
        let config = AuthConfig::default();
        assert_ne!(
            config.jwt.secret, "insecure-dev-secret-change-me",
            "Default config must not use the insecure hardcoded secret"
        );
    }
}
