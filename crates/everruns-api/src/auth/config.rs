// Authentication configuration loaded from environment variables.
// Decision: Follow Langfuse pattern with AUTH_ prefix for all auth config
// Decision: Default to "none" mode for local development

use std::time::Duration;

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
}

impl AuthMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "admin" => AuthMode::Admin,
            "full" => AuthMode::Full,
            _ => AuthMode::None,
        }
    }
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

/// GitHub OAuth configuration
#[derive(Debug, Clone)]
pub struct GitHubOAuthConfig {
    pub base: OAuthProviderConfig,
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
#[allow(dead_code)]
pub struct AuthConfig {
    /// Authentication mode
    pub mode: AuthMode,
    /// Base URL for OAuth callbacks
    pub base_url: String,
    /// JWT configuration
    pub jwt: JwtConfig,
    /// Admin user (for admin mode or initial setup)
    pub admin: Option<AdminConfig>,
    /// Google OAuth configuration
    pub google: Option<GoogleOAuthConfig>,
    /// GitHub OAuth configuration
    pub github: Option<GitHubOAuthConfig>,
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
            base_url: "http://localhost:9000".to_string(),
            jwt: JwtConfig::default(),
            admin: None,
            google: None,
            github: None,
            disable_password_auth: false,
            disable_signup: false,
            session_max_age: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
        }
    }
}

impl AuthConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let mode = std::env::var("AUTH_MODE")
            .map(|s| AuthMode::from_str(&s))
            .unwrap_or_default();

        let base_url = std::env::var("AUTH_BASE_URL")
            .or_else(|_| std::env::var("BASE_URL"))
            .unwrap_or_else(|_| "http://localhost:9000".to_string());

        // JWT configuration
        let jwt_secret = std::env::var("AUTH_JWT_SECRET").unwrap_or_else(|_| {
            if mode == AuthMode::None {
                // Generate a random secret for dev mode
                use rand::Rng;
                let bytes: [u8; 32] = rand::thread_rng().gen();
                hex::encode(bytes)
            } else {
                tracing::warn!("AUTH_JWT_SECRET not set, using insecure default");
                "insecure-dev-secret-change-me".to_string()
            }
        });

        let access_token_lifetime = std::env::var("AUTH_ACCESS_TOKEN_LIFETIME")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(15 * 60));

        let refresh_token_lifetime = std::env::var("AUTH_REFRESH_TOKEN_LIFETIME")
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
            std::env::var("ADMIN_EMAIL"),
            std::env::var("ADMIN_PASSWORD"),
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
                let redirect_uri = std::env::var("AUTH_GOOGLE_REDIRECT_URI")
                    .unwrap_or_else(|_| format!("{}/api/auth/callback/google", base_url));
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
                let redirect_uri = std::env::var("AUTH_GITHUB_REDIRECT_URI")
                    .unwrap_or_else(|_| format!("{}/api/auth/callback/github", base_url));
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

        Self {
            mode,
            base_url,
            jwt,
            admin,
            google,
            github,
            disable_password_auth,
            disable_signup,
            session_max_age,
        }
    }

    /// Check if authentication is enabled
    #[allow(dead_code)]
    pub fn is_enabled(&self) -> bool {
        self.mode != AuthMode::None
    }

    /// Check if password authentication is available
    pub fn password_auth_enabled(&self) -> bool {
        self.mode == AuthMode::Full && !self.disable_password_auth
    }

    /// Check if OAuth is available
    pub fn oauth_enabled(&self) -> bool {
        self.mode == AuthMode::Full && (self.google.is_some() || self.github.is_some())
    }

    /// Check if API key authentication is available
    #[allow(dead_code)]
    pub fn api_key_auth_enabled(&self) -> bool {
        self.mode == AuthMode::Full || self.mode == AuthMode::Admin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_mode_parsing() {
        assert_eq!(AuthMode::from_str("none"), AuthMode::None);
        assert_eq!(AuthMode::from_str("NONE"), AuthMode::None);
        assert_eq!(AuthMode::from_str("admin"), AuthMode::Admin);
        assert_eq!(AuthMode::from_str("ADMIN"), AuthMode::Admin);
        assert_eq!(AuthMode::from_str("full"), AuthMode::Full);
        assert_eq!(AuthMode::from_str("FULL"), AuthMode::Full);
        assert_eq!(AuthMode::from_str("invalid"), AuthMode::None);
    }

    #[test]
    fn test_default_config() {
        let config = AuthConfig::default();
        assert_eq!(config.mode, AuthMode::None);
        assert!(!config.is_enabled());
        assert!(!config.password_auth_enabled());
        assert!(!config.oauth_enabled());
    }
}
