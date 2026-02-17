// OAuth service for Google and GitHub authentication
// Decision: Manual OAuth2 implementation to avoid http crate version conflicts
// Decision: Support account linking by email

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::config::{GitHubConnectionConfig, GitHubOAuthConfig, GoogleOAuthConfig};

/// OAuth provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProvider {
    Google,
    GitHub,
}

impl OAuthProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            OAuthProvider::Google => "google",
            OAuthProvider::GitHub => "github",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "google" => Some(OAuthProvider::Google),
            "github" => Some(OAuthProvider::GitHub),
            _ => None,
        }
    }
}

/// User info from OAuth provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthUserInfo {
    /// Provider user ID
    pub provider_id: String,
    /// User email
    pub email: String,
    /// User name
    pub name: String,
    /// Avatar URL
    pub avatar_url: Option<String>,
    /// Email verified status
    pub email_verified: bool,
}

/// OAuth authorization URL with state
#[derive(Debug)]
#[allow(dead_code)]
pub struct OAuthAuthorizationUrl {
    pub url: String,
    pub state: String,
}

/// Google OAuth service
pub struct GoogleOAuthService {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    #[allow(dead_code)]
    allowed_domains: Option<Vec<String>>,
}

impl GoogleOAuthService {
    pub fn new(config: &GoogleOAuthConfig) -> Result<Self> {
        Ok(Self {
            client_id: config.base.client_id.clone(),
            client_secret: config.base.client_secret.clone(),
            redirect_uri: config.base.redirect_uri.clone(),
            allowed_domains: config.allowed_domains.clone(),
        })
    }

    /// Generate authorization URL for OAuth flow
    pub fn authorization_url(&self, state: &str) -> OAuthAuthorizationUrl {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", state),
            ("access_type", "offline"),
        ];

        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        OAuthAuthorizationUrl {
            url: format!("https://accounts.google.com/o/oauth2/v2/auth?{}", query),
            state: state.to_string(),
        }
    }

    /// Exchange authorization code for user info
    pub async fn exchange_code(&self, code: &str) -> Result<OAuthUserInfo> {
        let client = reqwest::Client::new();

        // Exchange code for token
        let token_response: GoogleTokenResponse = client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await
            .context("Failed to exchange code")?
            .json()
            .await
            .context("Failed to parse token response")?;

        // Fetch user info
        let user_info: GoogleUserInfo = client
            .get("https://www.googleapis.com/oauth2/v3/userinfo")
            .bearer_auth(&token_response.access_token)
            .send()
            .await
            .context("Failed to fetch user info")?
            .json()
            .await
            .context("Failed to parse user info")?;

        Ok(OAuthUserInfo {
            provider_id: user_info.sub,
            email: user_info.email,
            name: user_info.name.unwrap_or_default(),
            avatar_url: user_info.picture,
            email_verified: user_info.email_verified.unwrap_or(false),
        })
    }
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    #[allow(dead_code)]
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    sub: String,
    email: String,
    name: Option<String>,
    picture: Option<String>,
    email_verified: Option<bool>,
}

/// GitHub OAuth service
pub struct GitHubOAuthService {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

impl GitHubOAuthService {
    pub fn new(config: &GitHubOAuthConfig) -> Result<Self> {
        Ok(Self {
            client_id: config.base.client_id.clone(),
            client_secret: config.base.client_secret.clone(),
            redirect_uri: config.base.redirect_uri.clone(),
        })
    }

    /// Generate authorization URL for OAuth flow
    pub fn authorization_url(&self, state: &str) -> OAuthAuthorizationUrl {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", "user:email read:user"),
            ("state", state),
        ];

        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        OAuthAuthorizationUrl {
            url: format!("https://github.com/login/oauth/authorize?{}", query),
            state: state.to_string(),
        }
    }

    /// Exchange authorization code for user info
    pub async fn exchange_code(&self, code: &str) -> Result<OAuthUserInfo> {
        let client = reqwest::Client::new();

        // Exchange code for token
        let token_response: GitHubTokenResponse = client
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", self.redirect_uri.as_str()),
            ])
            .send()
            .await
            .context("Failed to exchange code")?
            .json()
            .await
            .context("Failed to parse token response")?;

        let access_token = &token_response.access_token;

        // Fetch user info
        let user_info: GitHubUserInfo = client
            .get("https://api.github.com/user")
            .header("User-Agent", "Everruns")
            .bearer_auth(access_token)
            .send()
            .await
            .context("Failed to fetch user info")?
            .json()
            .await
            .context("Failed to parse user info")?;

        // GitHub may not return email in user info, need to fetch from emails endpoint
        let email = if let Some(email) = user_info.email {
            email
        } else {
            // Fetch primary email
            let emails: Vec<GitHubEmail> = client
                .get("https://api.github.com/user/emails")
                .header("User-Agent", "Everruns")
                .bearer_auth(access_token)
                .send()
                .await
                .context("Failed to fetch user emails")?
                .json()
                .await
                .context("Failed to parse user emails")?;

            emails
                .into_iter()
                .find(|e| e.primary)
                .map(|e| e.email)
                .ok_or_else(|| anyhow::anyhow!("No primary email found"))?
        };

        Ok(OAuthUserInfo {
            provider_id: user_info.id.to_string(),
            email,
            name: user_info.name.unwrap_or_else(|| user_info.login.clone()),
            avatar_url: Some(user_info.avatar_url),
            email_verified: true, // GitHub emails are verified
        })
    }
}

#[derive(Debug, Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: String,
    #[allow(dead_code)]
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubUserInfo {
    id: i64,
    login: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    #[allow(dead_code)]
    verified: bool,
}

/// Result of GitHub App installation verification
#[derive(Debug, Clone)]
pub struct GitHubInstallationResult {
    /// GitHub App installation ID
    pub installation_id: i64,
    /// Account (user/org) that installed the app
    pub account_id: String,
    /// Account login (username or org name)
    pub account_login: String,
    /// Permissions granted to the installation (e.g. "contents: read")
    pub permissions: String,
}

/// GitHub App service for installation-based repo access.
///
/// Unlike OAuth Apps, GitHub Apps use:
/// 1. App installation flow (user installs app on repos, not OAuth consent)
/// 2. Short-lived installation access tokens (1h TTL, minted on demand)
/// 3. Granular per-repo permissions instead of broad OAuth scopes
pub struct GitHubAppService {
    app_id: String,
    private_key: String,
    app_slug: String,
}

impl GitHubAppService {
    pub fn new(config: &GitHubConnectionConfig) -> Self {
        Self {
            app_id: config.app_id.clone(),
            private_key: config.private_key.clone(),
            app_slug: config.app_slug.clone(),
        }
    }

    /// Generate URL for GitHub App installation.
    /// User clicks this to install the app on their repos.
    pub fn installation_url(&self, state: &str) -> OAuthAuthorizationUrl {
        let url = format!(
            "https://github.com/apps/{}/installations/new?state={}",
            self.app_slug,
            urlencoding::encode(state)
        );
        OAuthAuthorizationUrl {
            url,
            state: state.to_string(),
        }
    }

    /// Create a JWT for authenticating as the GitHub App.
    /// Valid for up to 10 minutes. Used to call GitHub API as the App.
    pub fn create_app_jwt(&self) -> Result<String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};

        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iat": now - 60,          // 60s in past for clock drift
            "exp": now + (10 * 60),   // 10 minute max
            "iss": self.app_id,
        });

        let key = EncodingKey::from_rsa_pem(self.private_key.as_bytes())
            .context("Invalid GitHub App private key (must be PEM-encoded RSA)")?;
        let header = Header::new(Algorithm::RS256);
        let token = jsonwebtoken::encode(&header, &claims, &key)
            .context("Failed to sign GitHub App JWT")?;
        Ok(token)
    }

    /// Verify an installation exists and return its details.
    /// Called during the installation callback to validate the installation_id.
    pub async fn verify_installation(
        &self,
        installation_id: i64,
    ) -> Result<GitHubInstallationResult> {
        let jwt = self.create_app_jwt()?;
        let client = reqwest::Client::new();

        let response = client
            .get(format!(
                "https://api.github.com/app/installations/{installation_id}"
            ))
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "Everruns")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("Failed to verify GitHub App installation")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "GitHub API returned {status} verifying installation {installation_id}: {body}"
            );
        }

        let installation: GitHubInstallation = response
            .json()
            .await
            .context("Failed to parse installation response")?;

        // Format permissions as human-readable string
        let permissions = installation
            .permissions
            .as_object()
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or("unknown")))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();

        Ok(GitHubInstallationResult {
            installation_id,
            account_id: installation.account.id.to_string(),
            account_login: installation.account.login,
            permissions,
        })
    }

    /// Mint a short-lived installation access token (1h TTL).
    /// Called at tool execution time to get a fresh token for git operations.
    pub async fn mint_installation_token(&self, installation_id: i64) -> Result<String> {
        let jwt = self.create_app_jwt()?;
        let client = reqwest::Client::new();

        let response = client
            .post(format!(
                "https://api.github.com/app/installations/{installation_id}/access_tokens"
            ))
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "Everruns")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("Failed to mint GitHub installation token")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "GitHub API returned {status} minting installation token for {installation_id}: {body}"
            );
        }

        let token_response: GitHubInstallationTokenResponse = response
            .json()
            .await
            .context("Failed to parse installation token response")?;

        Ok(token_response.token)
    }
}

#[derive(Debug, Deserialize)]
struct GitHubInstallation {
    account: GitHubInstallationAccount,
    permissions: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct GitHubInstallationAccount {
    id: i64,
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubInstallationTokenResponse {
    token: String,
}

/// URL encoding helper
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::new();
        for c in s.chars() {
            match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
                ' ' => result.push_str("%20"),
                _ => {
                    for byte in c.to_string().as_bytes() {
                        result.push_str(&format!("%{:02X}", byte));
                    }
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth_provider_from_str() {
        assert_eq!(OAuthProvider::parse("google"), Some(OAuthProvider::Google));
        assert_eq!(OAuthProvider::parse("GOOGLE"), Some(OAuthProvider::Google));
        assert_eq!(OAuthProvider::parse("github"), Some(OAuthProvider::GitHub));
        assert_eq!(OAuthProvider::parse("GITHUB"), Some(OAuthProvider::GitHub));
        assert_eq!(OAuthProvider::parse("invalid"), None);
    }

    #[test]
    fn test_oauth_provider_as_str() {
        assert_eq!(OAuthProvider::Google.as_str(), "google");
        assert_eq!(OAuthProvider::GitHub.as_str(), "github");
    }

    #[test]
    fn test_url_encoding() {
        assert_eq!(urlencoding::encode("hello world"), "hello%20world");
        assert_eq!(
            urlencoding::encode("test@example.com"),
            "test%40example.com"
        );
    }
}
