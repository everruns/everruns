//! Per-agent GitHub Apps: manifest creation, installation, and token minting.
//!
//! Design decisions:
//! - **One GitHub credential per agent.** "Connect GitHub" on an agent
//!   identity creates a GitHub App for that agent and installs it on the repos
//!   the user picks. The identity's `github` connection is then the only GitHub
//!   credential its sessions use: native GitHub tools, git, MCP servers bound
//!   to the connection, and GitHub-driven triggers all resolve through it.
//! - **Clicks, not copied secrets.** GitHub's App manifest flow returns the
//!   App's id, private key, client secret and webhook secret to our callback,
//!   so no one pastes a credential. Mirrors the Slack one-click install
//!   decision (`knowledge/integrations/slack-one-click-install.md`): one App per
//!   agent keeps a distinct bot identity (`<agent>[bot]`) and needs no operator
//!   setup on self-hosted deployments.
//! - **The App row id is in every GitHub-facing URL** (webhook, setup), so
//!   GitHub callbacks resolve their App without a lookup by untrusted input,
//!   and a setup callback can only ever bind installations of that one App
//!   (verified with the App's own JWT).
//! - Browser round trips carry an encrypted, expiring [`SetupState`] bound to
//!   the user who started the flow, instead of server-side pending rows.
//!
//! See `knowledge/integrations/github-apps.md`.

use anyhow::{Context, Result, bail};
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::storage::EncryptionService;

/// How long a started connect flow stays valid.
const SETUP_STATE_TTL_SECONDS: i64 = 60 * 60;

/// Permissions every agent App requests. Contents and metadata are read-only;
/// pull requests and issues are writable so the agent can comment.
pub fn default_permissions() -> Value {
    json!({
        "contents": "read",
        "metadata": "read",
        "pull_requests": "write",
        "issues": "write",
    })
}

/// Webhook events every agent App subscribes to.
pub const DEFAULT_EVENTS: &[&str] = &["pull_request", "issue_comment", "issues"];

/// GitHub endpoints. Overridable for GitHub Enterprise and for tests.
#[derive(Clone, Debug)]
pub struct GitHubEndpoints {
    /// REST API base, e.g. `https://api.github.com`.
    pub api_url: String,
    /// Web base, e.g. `https://github.com`.
    pub web_url: String,
}

impl Default for GitHubEndpoints {
    fn default() -> Self {
        Self {
            api_url: "https://api.github.com".to_string(),
            web_url: "https://github.com".to_string(),
        }
    }
}

impl GitHubEndpoints {
    /// Read `GITHUB_API_URL` / `GITHUB_WEB_URL`, defaulting to github.com.
    pub fn from_env() -> Self {
        let default = Self::default();
        let read = |name: &str, fallback: String| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().trim_end_matches('/').to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or(fallback)
        };
        Self {
            api_url: read("GITHUB_API_URL", default.api_url),
            web_url: read("GITHUB_WEB_URL", default.web_url),
        }
    }
}

/// Inputs to an App manifest.
pub struct ManifestInput<'a> {
    /// Display name GitHub shows for the App (and its bot).
    pub name: &'a str,
    /// Public base URL of this API, including any `/api` prefix.
    pub api_base_url: &'a str,
    /// Public UI origin; the App's homepage.
    pub frontend_url: &'a str,
    /// Row id the App will be stored under.
    pub app_row_id: Uuid,
}

/// Build the manifest GitHub uses to create the App.
///
/// GitHub refuses webhook URLs it cannot reach from the internet, so on a
/// local base URL the manifest omits the webhook; the App still works for
/// tools, and events need a public URL.
pub fn build_manifest(input: &ManifestInput<'_>) -> Value {
    let api = input.api_base_url.trim_end_matches('/');
    let mut manifest = json!({
        "name": input.name,
        "url": input.frontend_url,
        "redirect_url": format!("{api}/v1/github/app-manifest/callback"),
        "setup_url": format!("{api}/v1/github/apps/{}/setup", input.app_row_id),
        "setup_on_update": true,
        "public": false,
        "default_permissions": default_permissions(),
        "default_events": DEFAULT_EVENTS,
    });
    if is_publicly_reachable(api) {
        manifest["hook_attributes"] = json!({
            "url": format!("{api}/v1/github/apps/{}/webhook", input.app_row_id),
            "active": true,
        });
    } else {
        manifest["default_events"] = json!([]);
    }
    manifest
}

/// Whether GitHub could deliver webhooks to this base URL.
pub fn is_publicly_reachable(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.to_ascii_lowercase();
            host != "localhost" && !host.ends_with(".localhost") && !host.ends_with(".local")
        }
        Some(url::Host::Ipv4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified())
        }
        Some(url::Host::Ipv6(ip)) => !(ip.is_loopback() || ip.is_unspecified()),
        None => false,
    }
}

/// A default App name: GitHub requires globally unique names of at most 34
/// characters, so the agent name is shortened and given a random suffix. The
/// user can still rename it on GitHub's confirmation page.
pub fn default_app_name(agent_name: &str) -> String {
    let base: String = agent_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .trim_matches('-')
        .chars()
        .take(24)
        .collect();
    let base = if base.is_empty() {
        "agent".to_string()
    } else {
        base
    };
    let suffix = &Uuid::new_v4().simple().to_string()[..6];
    format!("{base}-{suffix}")
}

/// Where the browser posts the manifest: a personal account or an org.
pub fn manifest_form_action(web_url: &str, owner_org: Option<&str>, state: &str) -> String {
    let web = web_url.trim_end_matches('/');
    let path = match owner_org.filter(|org| !org.is_empty()) {
        Some(org) => format!(
            "{web}/organizations/{}/settings/apps/new",
            urlencoding::encode(org)
        ),
        None => format!("{web}/settings/apps/new"),
    };
    format!("{path}?state={}", urlencoding::encode(state))
}

/// Installation page for an App.
pub fn install_url(web_url: &str, slug: &str, state: &str) -> String {
    format!(
        "{}/apps/{}/installations/new?state={}",
        web_url.trim_end_matches('/'),
        urlencoding::encode(slug),
        urlencoding::encode(state)
    )
}

/// State carried through GitHub's redirects. Encrypted, so it cannot be forged
/// or altered, and bound to the user who started the flow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SetupState {
    pub org_id: i64,
    pub agent_identity_id: Uuid,
    pub user_id: Uuid,
    /// Row id the created App is stored under.
    pub app_row_id: Uuid,
    /// UI path to return to when done.
    pub return_to: Option<String>,
    /// Unix seconds after which the state is refused.
    pub expires_at: i64,
}

impl SetupState {
    pub fn new(
        org_id: i64,
        agent_identity_id: Uuid,
        user_id: Uuid,
        app_row_id: Uuid,
        return_to: Option<String>,
    ) -> Self {
        Self {
            org_id,
            agent_identity_id,
            user_id,
            app_row_id,
            return_to: return_to.filter(|path| is_safe_return_path(path)),
            expires_at: Utc::now().timestamp() + SETUP_STATE_TTL_SECONDS,
        }
    }

    pub fn seal(&self, encryption: &EncryptionService) -> Result<String> {
        let bytes = encryption.encrypt(&serde_json::to_vec(self)?)?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn open(sealed: &str, encryption: &EncryptionService) -> Result<Self> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sealed.trim())
            .context("state is not valid base64")?;
        let state: Self = serde_json::from_slice(&encryption.decrypt(&bytes)?)?;
        if state.expires_at < Utc::now().timestamp() {
            bail!("state expired");
        }
        Ok(state)
    }
}

/// Only same-origin UI paths are accepted as return targets.
pub fn is_safe_return_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
}

/// What GitHub returns when a manifest code is converted into an App.
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestConversion {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub html_url: String,
    pub owner: Option<GitHubAccount>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub webhook_secret: Option<String>,
    pub pem: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubAccount {
    pub id: i64,
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Installation {
    pub id: i64,
    pub app_id: i64,
    pub account: GitHubAccount,
    #[serde(default)]
    pub permissions: Value,
    #[serde(default)]
    pub repository_selection: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[cfg_attr(test, derive(Default))]
pub struct Repository {
    pub full_name: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub html_url: String,
}

/// Credentials of one App, decrypted for a single operation.
pub struct AppCredentials {
    pub app_id: i64,
    pub private_key_pem: String,
}

/// Thin GitHub REST client for App operations.
#[derive(Clone)]
pub struct GitHubAppApi {
    endpoints: GitHubEndpoints,
    http: reqwest::Client,
}

impl GitHubAppApi {
    pub fn new(endpoints: GitHubEndpoints) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("Everruns")
            .build()
            .unwrap_or_default();
        Self { endpoints, http }
    }

    pub fn endpoints(&self) -> &GitHubEndpoints {
        &self.endpoints
    }

    fn api(&self, path: &str) -> String {
        format!("{}{path}", self.endpoints.api_url)
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, self.api(path))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// Exchange the one-time manifest code for the new App's credentials.
    pub async fn convert_manifest(&self, code: &str) -> Result<ManifestConversion> {
        if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
            bail!("invalid manifest code");
        }
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/app-manifests/{code}/conversions"),
            )
            .send()
            .await
            .context("GitHub manifest conversion request failed")?;
        read_json(response, "converting App manifest").await
    }

    /// Short-lived JWT that authenticates as the App itself.
    pub fn app_jwt(credentials: &AppCredentials) -> Result<String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};
        let now = Utc::now().timestamp();
        let claims = json!({
            "iat": now - 60,
            "exp": now + 9 * 60,
            "iss": credentials.app_id.to_string(),
        });
        let key = EncodingKey::from_rsa_pem(credentials.private_key_pem.as_bytes())
            .context("GitHub App private key is not a PEM RSA key")?;
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
            .context("failed to sign GitHub App JWT")
    }

    /// Fetch an installation of this App. Fails when the installation belongs
    /// to another App, which is what makes a setup callback safe to trust.
    pub async fn get_installation(
        &self,
        credentials: &AppCredentials,
        installation_id: i64,
    ) -> Result<Installation> {
        let jwt = Self::app_jwt(credentials)?;
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("/app/installations/{installation_id}"),
            )
            .bearer_auth(jwt)
            .send()
            .await
            .context("GitHub installation request failed")?;
        read_json(response, "reading installation").await
    }

    /// Mint a one-hour installation access token.
    pub async fn mint_installation_token(
        &self,
        credentials: &AppCredentials,
        installation_id: i64,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct Token {
            token: String,
        }
        let jwt = Self::app_jwt(credentials)?;
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/app/installations/{installation_id}/access_tokens"),
            )
            .bearer_auth(jwt)
            .send()
            .await
            .context("GitHub installation token request failed")?;
        let token: Token = read_json(response, "minting installation token").await?;
        Ok(token.token)
    }

    /// Repositories an installation can access (up to 300).
    pub async fn list_installation_repositories(
        &self,
        installation_token: &str,
    ) -> Result<Vec<Repository>> {
        #[derive(Deserialize)]
        struct Page {
            repositories: Vec<Repository>,
        }
        let mut repositories = Vec::new();
        for page in 1..=3 {
            let response = self
                .request(
                    reqwest::Method::GET,
                    &format!("/installation/repositories?per_page=100&page={page}"),
                )
                .bearer_auth(installation_token)
                .send()
                .await
                .context("GitHub repositories request failed")?;
            let batch: Page = read_json(response, "listing repositories").await?;
            let done = batch.repositories.len() < 100;
            repositories.extend(batch.repositories);
            if done {
                break;
            }
        }
        repositories.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        Ok(repositories)
    }

    /// Uninstall the App from the account. Best effort on disconnect.
    pub async fn delete_installation(
        &self,
        credentials: &AppCredentials,
        installation_id: i64,
    ) -> Result<()> {
        let jwt = Self::app_jwt(credentials)?;
        let response = self
            .request(
                reqwest::Method::DELETE,
                &format!("/app/installations/{installation_id}"),
            )
            .bearer_auth(jwt)
            .send()
            .await
            .context("GitHub uninstall request failed")?;
        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        bail!("GitHub returned {} uninstalling the App", response.status())
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    action: &str,
) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        // The body can echo request details; keep it out of errors that may
        // reach a user and log only its size.
        let body_len = response.bytes().await.map(|b| b.len()).unwrap_or(0);
        tracing::warn!(%status, body_len, action, "GitHub API request failed");
        bail!("GitHub returned {status} {action}");
    }
    response
        .json::<T>()
        .await
        .with_context(|| format!("GitHub returned an unexpected response {action}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn encryption() -> EncryptionService {
        EncryptionService::new(
            &crate::storage::encryption::generate_encryption_key("github-apps-test"),
            &[],
        )
        .unwrap()
    }

    #[test]
    fn manifest_has_webhook_and_callbacks_on_a_public_url() {
        let id = Uuid::from_u128(7);
        let manifest = build_manifest(&ManifestInput {
            name: "pr-bot-abc123",
            api_base_url: "https://app.everruns.com/api/",
            frontend_url: "https://app.everruns.com",
            app_row_id: id,
        });
        assert_eq!(
            manifest["hook_attributes"]["url"],
            format!("https://app.everruns.com/api/v1/github/apps/{id}/webhook")
        );
        assert_eq!(
            manifest["setup_url"],
            format!("https://app.everruns.com/api/v1/github/apps/{id}/setup")
        );
        assert_eq!(
            manifest["redirect_url"],
            "https://app.everruns.com/api/v1/github/app-manifest/callback"
        );
        assert_eq!(manifest["default_permissions"]["pull_requests"], "write");
        assert_eq!(manifest["default_permissions"]["contents"], "read");
        assert_eq!(manifest["public"], false);
        assert!(
            manifest["default_events"]
                .as_array()
                .unwrap()
                .contains(&json!("pull_request"))
        );
    }

    #[test]
    fn manifest_omits_webhook_on_a_local_url() {
        let manifest = build_manifest(&ManifestInput {
            name: "local",
            api_base_url: "http://localhost:27100/api",
            frontend_url: "http://localhost:27100",
            app_row_id: Uuid::nil(),
        });
        assert!(manifest.get("hook_attributes").is_none());
        assert_eq!(manifest["default_events"], json!([]));
    }

    #[test]
    fn reachability_rejects_local_and_plain_http() {
        assert!(is_publicly_reachable("https://app.everruns.com/api"));
        assert!(!is_publicly_reachable("http://app.everruns.com/api"));
        assert!(!is_publicly_reachable("https://localhost/api"));
        assert!(!is_publicly_reachable("https://10.0.0.4/api"));
        assert!(!is_publicly_reachable("https://127.0.0.1/api"));
        assert!(!is_publicly_reachable("https://box.local/api"));
    }

    #[test]
    fn default_name_is_short_unique_and_github_safe() {
        let name = default_app_name("My PR Summarizer (prod)!");
        assert!(name.len() <= 34, "{name}");
        assert!(name.starts_with("My-PR-Summarizer-prod"), "{name}");
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_ne!(default_app_name("x"), default_app_name("x"));
        assert!(default_app_name("!!!").starts_with("agent-"));
    }

    #[test]
    fn form_action_targets_account_or_org() {
        assert_eq!(
            manifest_form_action("https://github.com", None, "s t"),
            "https://github.com/settings/apps/new?state=s%20t"
        );
        assert_eq!(
            manifest_form_action("https://github.com/", Some("acme"), "s"),
            "https://github.com/organizations/acme/settings/apps/new?state=s"
        );
    }

    #[test]
    fn setup_state_round_trips_and_rejects_tampering_and_expiry() {
        let encryption = encryption();
        let state = SetupState::new(
            1,
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
            Some("/agents/agent_1".to_string()),
        );
        let sealed = state.seal(&encryption).unwrap();
        assert_eq!(SetupState::open(&sealed, &encryption).unwrap(), state);

        let mut tampered = sealed.clone().into_bytes();
        let last = tampered.len() - 2;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        assert!(SetupState::open(&String::from_utf8(tampered).unwrap(), &encryption).is_err());

        let expired = SetupState {
            expires_at: Utc::now().timestamp() - 1,
            ..state
        };
        assert!(SetupState::open(&expired.seal(&encryption).unwrap(), &encryption).is_err());
    }

    #[test]
    fn return_paths_must_stay_on_origin() {
        assert!(is_safe_return_path("/agents/agent_1?tab=triggers"));
        assert!(!is_safe_return_path("https://evil.example"));
        assert!(!is_safe_return_path("//evil.example"));
        assert!(!is_safe_return_path("/\\evil.example"));
        let state = SetupState::new(
            1,
            Uuid::nil(),
            Uuid::nil(),
            Uuid::nil(),
            Some("//evil.example".to_string()),
        );
        assert_eq!(state.return_to, None);
    }

    #[tokio::test]
    async fn converts_a_manifest_code() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/app-manifests/abc123/conversions"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": 42,
                "slug": "pr-bot",
                "name": "pr-bot",
                "html_url": "https://github.com/apps/pr-bot",
                "owner": {"id": 9, "login": "acme"},
                "client_id": "Iv1.x",
                "client_secret": "cs",
                "webhook_secret": "whs",
                "pem": "-----BEGIN RSA PRIVATE KEY-----\n...",
            })))
            .mount(&server)
            .await;
        let api = GitHubAppApi::new(GitHubEndpoints {
            api_url: server.uri(),
            web_url: "https://github.com".to_string(),
        });
        let app = api.convert_manifest("abc123").await.unwrap();
        assert_eq!(app.id, 42);
        assert_eq!(app.webhook_secret.as_deref(), Some("whs"));
        assert_eq!(app.owner.unwrap().login, "acme");
        assert!(api.convert_manifest("../etc").await.is_err());
    }

    #[tokio::test]
    async fn lists_repositories_with_an_installation_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/installation/repositories"))
            .and(header("authorization", "Bearer ghs_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total_count": 2,
                "repositories": [
                    {"full_name": "acme/web", "private": true, "html_url": "https://github.com/acme/web"},
                    {"full_name": "acme/api", "private": false, "html_url": "https://github.com/acme/api"},
                ],
            })))
            .mount(&server)
            .await;
        let api = GitHubAppApi::new(GitHubEndpoints {
            api_url: server.uri(),
            web_url: "https://github.com".to_string(),
        });
        let repos = api
            .list_installation_repositories("ghs_token")
            .await
            .unwrap();
        let names: Vec<_> = repos.iter().map(|r| r.full_name.as_str()).collect();
        assert_eq!(names, vec!["acme/api", "acme/web"]);
    }

    #[tokio::test]
    async fn api_errors_do_not_echo_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/app-manifests/abc/conversions"))
            .respond_with(ResponseTemplate::new(404).set_body_string("secret-ish detail"))
            .mount(&server)
            .await;
        let api = GitHubAppApi::new(GitHubEndpoints {
            api_url: server.uri(),
            web_url: "https://github.com".to_string(),
        });
        let error = api.convert_manifest("abc").await.unwrap_err().to_string();
        assert!(error.contains("404"));
        assert!(!error.contains("secret-ish"));
    }
}
