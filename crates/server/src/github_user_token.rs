//! A person's own GitHub sign-in (D1 and step 4 in
//! knowledge/integrations/github-model.md): user-to-server tokens of the
//! deployment's shared App, issued through its OAuth client.
//!
//! Decision: a person's `github` connection stores the user token GitHub
//! issues at sign-in, never an installation, so it acts as the person. GitHub
//! Apps that expire user tokens return a refresh token with an 8-hour access
//! token; the resolver refreshes it before use. A token without an expiry is
//! used as is.
//!
//! THREAT[TM-GITHUB-008]: the code, the tokens and GitHub's error responses
//! never reach logs or user-visible errors.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::github_apps::GitHubEndpoints;

/// The shared App's OAuth client credentials.
#[derive(Clone)]
pub struct GitHubUserClient {
    pub client_id: String,
    pub client_secret: String,
}

impl GitHubUserClient {
    /// The client from `GITHUB_APP_CLIENT_ID` and `GITHUB_APP_CLIENT_SECRET`,
    /// when both are set.
    pub fn new(client_id: Option<&str>, client_secret: Option<&str>) -> Option<Self> {
        let client_id = client_id.map(str::trim).filter(|id| !id.is_empty())?;
        let client_secret = client_secret.filter(|secret| !secret.is_empty())?;
        Some(Self {
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        })
    }

    pub fn from_env() -> Option<Self> {
        Self::new(
            std::env::var("GITHUB_APP_CLIENT_ID").ok().as_deref(),
            std::env::var("GITHUB_APP_CLIENT_SECRET").ok().as_deref(),
        )
    }
}

/// A user-to-server token as GitHub issued it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubUserToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// When the access token expires; `None` when the App does not expire them.
    pub expires_at: Option<DateTime<Utc>>,
}

/// The GitHub account a user token belongs to.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GitHubUser {
    pub id: i64,
    pub login: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    error: Option<String>,
}

/// GitHub's OAuth endpoints for the shared App's user tokens.
#[derive(Clone)]
pub struct GitHubUserTokens {
    endpoints: GitHubEndpoints,
    http: reqwest::Client,
}

impl GitHubUserTokens {
    pub fn new(endpoints: GitHubEndpoints) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent("Everruns")
            .build()
            .unwrap_or_default();
        Self { endpoints, http }
    }

    /// Exchange a sign-in `code` for the person's token.
    pub async fn exchange_code(
        &self,
        client: &GitHubUserClient,
        code: &str,
    ) -> Result<GitHubUserToken> {
        self.token_request(client, json!({ "code": code }), "exchanging a code")
            .await
    }

    /// Trade a refresh token for a new token pair.
    pub async fn refresh(
        &self,
        client: &GitHubUserClient,
        refresh_token: &str,
    ) -> Result<GitHubUserToken> {
        self.token_request(
            client,
            json!({ "grant_type": "refresh_token", "refresh_token": refresh_token }),
            "refreshing a token",
        )
        .await
    }

    /// The account `access_token` signs in as.
    pub async fn user(&self, access_token: &str) -> Result<GitHubUser> {
        let response = self
            .http
            .get(format!("{}/user", self.endpoints.api_url))
            .bearer_auth(access_token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("GitHub user request failed")?;
        if !response.status().is_success() {
            bail!("GitHub returned {} reading the user", response.status());
        }
        response
            .json()
            .await
            .context("GitHub returned an unexpected user")
    }

    async fn token_request(
        &self,
        client: &GitHubUserClient,
        grant: serde_json::Value,
        action: &str,
    ) -> Result<GitHubUserToken> {
        let mut body = json!({
            "client_id": client.client_id,
            "client_secret": client.client_secret,
        });
        if let (Some(body), Some(grant)) = (body.as_object_mut(), grant.as_object()) {
            body.extend(grant.clone());
        }
        let response = self
            .http
            .post(format!(
                "{}/login/oauth/access_token",
                self.endpoints.web_url
            ))
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("GitHub token request failed {action}"))?;
        if !response.status().is_success() {
            bail!("GitHub returned {} {action}", response.status());
        }
        // A bad, used or expired code or refresh token is a 200 with `error`.
        let token: TokenResponse = response
            .json()
            .await
            .with_context(|| format!("GitHub returned an unexpected token response {action}"))?;
        let Some(access_token) = token.access_token.filter(|t| !t.is_empty()) else {
            bail!(
                "GitHub refused {action}: {}",
                token.error.as_deref().unwrap_or("no access token")
            );
        };
        Ok(GitHubUserToken {
            access_token,
            refresh_token: token.refresh_token.filter(|t| !t.is_empty()),
            expires_at: token
                .expires_in
                .filter(|seconds| *seconds > 0)
                .map(|seconds| Utc::now() + Duration::seconds(seconds)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client() -> GitHubUserClient {
        GitHubUserClient::new(Some("Iv1.test"), Some("secret")).unwrap()
    }

    fn tokens(github: &MockServer) -> GitHubUserTokens {
        GitHubUserTokens::new(GitHubEndpoints {
            api_url: github.uri(),
            web_url: github.uri(),
        })
    }

    #[test]
    fn a_client_needs_both_credentials() {
        assert!(GitHubUserClient::new(Some("Iv1.test"), None).is_none());
        assert!(GitHubUserClient::new(Some(" "), Some("secret")).is_none());
        assert!(GitHubUserClient::new(None, Some("secret")).is_none());
    }

    #[tokio::test]
    async fn a_code_becomes_an_expiring_token_pair() {
        let github = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .and(body_partial_json(json!({
                "client_id": "Iv1.test", "client_secret": "secret", "code": "c1",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "ghu_1", "expires_in": 28800,
                "refresh_token": "ghr_1", "refresh_token_expires_in": 15897600,
                "token_type": "bearer",
            })))
            .mount(&github)
            .await;
        let token = tokens(&github)
            .exchange_code(&client(), "c1")
            .await
            .unwrap();
        assert_eq!(token.access_token, "ghu_1");
        assert_eq!(token.refresh_token.as_deref(), Some("ghr_1"));
        let left = token.expires_at.unwrap() - Utc::now();
        assert!(left > Duration::hours(7) && left <= Duration::hours(8));
    }

    #[tokio::test]
    async fn a_refused_grant_is_an_error_and_a_token_without_expiry_stays_one() {
        let github = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .and(body_partial_json(json!({ "refresh_token": "used" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "error": "bad_refresh_token" })),
            )
            .mount(&github)
            .await;
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .and(body_partial_json(json!({ "code": "c2" })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "access_token": "ghu_2" })),
            )
            .mount(&github)
            .await;
        let error = tokens(&github)
            .refresh(&client(), "used")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("bad_refresh_token"), "{error}");
        let token = tokens(&github)
            .exchange_code(&client(), "c2")
            .await
            .unwrap();
        assert_eq!(token.expires_at, None);
        assert_eq!(token.refresh_token, None);
    }

    #[tokio::test]
    async fn the_user_is_read_with_the_token() {
        let github = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("authorization", "Bearer ghu_1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "id": 7, "login": "octo" })),
            )
            .mount(&github)
            .await;
        assert_eq!(
            tokens(&github).user("ghu_1").await.unwrap(),
            GitHubUser {
                id: 7,
                login: "octo".to_string()
            }
        );
    }
}
