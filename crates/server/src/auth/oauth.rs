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
pub struct OAuthAuthorizationUrl {
    pub url: String,
    pub state: String,
}

/// Google OAuth service
pub struct GoogleOAuthService {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
}

impl GoogleOAuthService {
    pub fn new(config: &GoogleOAuthConfig) -> Result<Self> {
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

        // GitHub's /user endpoint returns the public profile email but carries
        // no verification bit, and the public email may be absent entirely.
        // Always consult /user/emails (granted by the user:email scope) so the
        // real `verified` flag drives the identity gate instead of assuming
        // verification (EVE-702).
        let emails: Vec<GitHubEmail> = client
            .get("https://api.github.com/user/emails")
            .header("User-Agent", "Everruns")
            .bearer_auth(access_token)
            .send()
            .await
            .context("Failed to fetch user emails")?
            .json()
            .await
            .unwrap_or_default();

        let (email, email_verified) = select_github_email(user_info.email, &emails)
            .ok_or_else(|| anyhow::anyhow!("No email available from GitHub"))?;

        Ok(OAuthUserInfo {
            provider_id: user_info.id.to_string(),
            email,
            name: user_info.name.unwrap_or_else(|| user_info.login.clone()),
            avatar_url: Some(user_info.avatar_url),
            email_verified,
        })
    }
}

/// Choose which GitHub email to trust and its verification status.
///
/// GitHub's public `/user` email carries no verification flag, so we look it up
/// in the `/user/emails` list to learn whether it is verified — a public email
/// absent from that list is treated as unverified, failing closed. When no
/// public email is exposed we fall back to the account's primary address. The
/// caller gate rejects unverified addresses, so an honest `false` here blocks
/// account pre-emption via an unverified GitHub identity. Returns `None` only
/// when GitHub surfaces no usable email at all.
fn select_github_email(public: Option<String>, emails: &[GitHubEmail]) -> Option<(String, bool)> {
    if let Some(public) = public {
        let verified = emails
            .iter()
            .any(|e| e.email.eq_ignore_ascii_case(&public) && e.verified);
        return Some((public, verified));
    }
    emails
        .iter()
        .find(|e| e.primary)
        .map(|e| (e.email.clone(), e.verified))
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

    fn gh_email(email: &str, primary: bool, verified: bool) -> GitHubEmail {
        GitHubEmail {
            email: email.to_string(),
            primary,
            verified,
        }
    }

    // EVE-702: a public profile email inherits the verified flag from the
    // matching /user/emails entry.
    #[test]
    fn github_email_uses_verified_flag_of_public_email() {
        let emails = vec![gh_email("Pub@x.com", true, true)];
        assert_eq!(
            select_github_email(Some("pub@x.com".to_string()), &emails),
            Some(("pub@x.com".to_string(), true))
        );
    }

    // EVE-702: a public email GitHub does not list as verified fails closed.
    #[test]
    fn github_email_public_absent_from_list_is_unverified() {
        let emails = vec![gh_email("other@x.com", true, true)];
        assert_eq!(
            select_github_email(Some("pub@x.com".to_string()), &emails),
            Some(("pub@x.com".to_string(), false))
        );
    }

    // EVE-702: with no public email, fall back to the primary and carry its
    // real verified flag (here: primary but unverified).
    #[test]
    fn github_email_falls_back_to_primary_with_real_flag() {
        let emails = vec![
            gh_email("sec@x.com", false, true),
            gh_email("primary@x.com", true, false),
        ];
        assert_eq!(
            select_github_email(None, &emails),
            Some(("primary@x.com".to_string(), false))
        );
    }

    // EVE-702: no usable email at all yields None (surfaced as an error).
    #[test]
    fn github_email_none_when_no_email_available() {
        assert_eq!(select_github_email(None, &[]), None);
    }
}
