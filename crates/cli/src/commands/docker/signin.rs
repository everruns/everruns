//! Point a CLI profile at the local instance.
//!
//! Decision: the CLI signs itself in with a personal access token it creates,
//! not the admin password, so changing the password in the UI later does not
//! log the CLI out. A saved token that still works is reused; `up` only mints
//! a new one when the profile is missing, points elsewhere, or was revoked
//! (moving away from no-sign-in revokes the tokens made under it).

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use super::AuthChoice;
use crate::auth::{CredentialStore, Profile};
use crate::commands::login::{OrgInfo, select_org};

pub struct SignedIn {
    pub profile: String,
    pub current: bool,
    pub email: String,
}

#[derive(Deserialize)]
struct Me {
    email: String,
    name: String,
    organizations: Option<Vec<OrgInfo>>,
}

pub struct Credentials<'a> {
    pub auth: AuthChoice,
    pub email: &'a str,
    pub password: &'a str,
    /// Where the person changes the saved password, named when sign-in fails.
    pub password_hint: &'a str,
}

/// Sign in at `api_url` and save the long-lived `profile_api_url`. They differ
/// when Kubernetes bootstraps the sign-in through a temporary port-forward.
pub async fn ensure_profile(
    api_url: &str,
    profile_api_url: &str,
    profile: &str,
    creds: Credentials<'_>,
    switch: SwitchPolicy,
) -> Result<SignedIn> {
    let http = reqwest::Client::new();
    let management_url = management_url(&http, api_url).await?;
    let api_url = management_url.as_str();
    let mut store = CredentialStore::load().unwrap_or_default();

    let existing = store
        .profiles
        .get(profile)
        .filter(|p| p.api_url == profile_api_url)
        .cloned();
    let reusable = match &existing {
        Some(p) => me(&http, api_url, &p.api_key)
            .await
            .ok()
            .map(|m| (p.clone(), m)),
        None => None,
    };

    let (saved, me) = match reusable {
        Some(found) => found,
        None => {
            let token = mint_token(&http, api_url, &creds).await?;
            let me = me(&http, api_url, &token).await?;
            let org_id = select_org(me.organizations.as_deref().unwrap_or_default())?;
            let saved = Profile {
                api_url: profile_api_url.to_string(),
                api_key: token,
                org_id: Some(org_id),
                user_email: Some(me.email.clone()),
                user_name: Some(me.name.clone()),
            };
            (saved, me)
        }
    };
    store.profiles.insert(profile.to_string(), saved);

    let current = decide_current(&store, profile, switch)?;
    if current {
        store.current_profile = profile.to_string();
    }
    store.save()?;
    Ok(SignedIn {
        profile: profile.to_string(),
        current,
        email: me.email,
    })
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SwitchPolicy {
    /// Ask when another profile is current.
    Ask,
    /// Switch without asking (`--yes`).
    Always,
    /// Never switch away from another profile (no terminal to ask on).
    Never,
}

/// Becomes current when nothing else is in use, or the person agrees.
fn decide_current(store: &CredentialStore, profile: &str, switch: SwitchPolicy) -> Result<bool> {
    let other = match store.current_profile.as_str() {
        "" => None,
        name if name == profile => None,
        name if !store.profiles.contains_key(name) => None,
        name => Some(name.to_string()),
    };
    let Some(other) = other else {
        return Ok(true);
    };
    Ok(match switch {
        SwitchPolicy::Always => true,
        SwitchPolicy::Never => false,
        SwitchPolicy::Ask => dialoguer::Confirm::with_theme(crate::ui::theme())
            .with_prompt(format!(
                "Use this instance for CLI commands? (now: profile '{other}')"
            ))
            .default(true)
            .interact()
            .context("Failed to read the answer")?,
    })
}

// Published self-hosted images can predate the Platform namespace. Select a
// namespace with an unauthenticated read before sending credentials or writes.
async fn management_url(http: &reqwest::Client, api_url: &str) -> Result<String> {
    let base = api_url.trim_end_matches('/');
    for prefix in ["v1/platform", "v1"] {
        let url = format!("{base}/{prefix}");
        let response = http
            .get(format!("{url}/auth/config"))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .context("Failed to reach the server's authentication configuration")?;
        if response.status().is_success() {
            return Ok(url);
        }
        if prefix != "v1/platform" || response.status() != reqwest::StatusCode::NOT_FOUND {
            bail!(
                "authentication configuration refused ({})",
                response.status()
            );
        }
    }
    bail!("the server provides no supported management configuration endpoint")
}

async fn me(http: &reqwest::Client, api_url: &str, token: &str) -> Result<Me> {
    let resp = http
        .get(format!("{api_url}/auth/me"))
        .bearer_auth(token)
        .send()
        .await
        .context("Failed to reach the server")?;
    if !resp.status().is_success() {
        bail!("the server refused the token ({})", resp.status());
    }
    resp.json()
        .await
        .context("Failed to parse the current user")
}

fn token_name() -> String {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());
    format!("everruns-cli@{host}")
}

async fn mint_token(
    http: &reqwest::Client,
    api_url: &str,
    creds: &Credentials<'_>,
) -> Result<String> {
    let session = match creds.auth {
        // Without sign-in every request is the built-in admin.
        AuthChoice::None => None,
        AuthChoice::Single => Some(login(http, api_url, creds).await?),
        AuthChoice::Accounts => {
            let session = match login(http, api_url, creds).await {
                Ok(token) => token,
                // First run: the admin account does not exist yet.
                Err(_) => {
                    register(http, api_url, creds).await?;
                    login(http, api_url, creds).await?
                }
            };
            ensure_organization(http, api_url, &session).await?;
            Some(session)
        }
    };

    let mut req = http
        .post(format!("{api_url}/auth/personal-access-tokens"))
        .json(&serde_json::json!({ "name": token_name() }));
    if let Some(session) = session {
        req = req.bearer_auth(session);
    }
    let resp = req.send().await.context("Failed to create a CLI token")?;
    if !resp.status().is_success() {
        bail!("creating a CLI token failed ({})", resp.status());
    }
    #[derive(Deserialize)]
    struct Created {
        token: String,
    }
    let created: Created = resp.json().await.context("Failed to parse the new token")?;
    Ok(created.token)
}

/// A new account starts without an organization; the UI asks for one on
/// first sign-in. The CLI needs one to work in, so it creates it the same way.
async fn ensure_organization(http: &reqwest::Client, api_url: &str, session: &str) -> Result<()> {
    let me = me(http, api_url, session).await?;
    if !me.organizations.unwrap_or_default().is_empty() {
        return Ok(());
    }
    let resp = http
        .post(format!("{api_url}/orgs"))
        .bearer_auth(session)
        .json(&serde_json::json!({ "name": "Everruns" }))
        .send()
        .await
        .context("Failed to reach the server")?;
    if !resp.status().is_success() {
        bail!("creating an organization failed ({})", resp.status());
    }
    Ok(())
}

async fn login(http: &reqwest::Client, api_url: &str, creds: &Credentials<'_>) -> Result<String> {
    let resp = http
        .post(format!("{api_url}/auth/login"))
        .json(&serde_json::json!({ "email": creds.email, "password": creds.password }))
        .send()
        .await
        .context("Failed to reach the server")?;
    if !resp.status().is_success() {
        bail!(
            "signing in as {} failed ({}). If you changed the password in the UI, {}",
            creds.email,
            resp.status(),
            creds.password_hint
        );
    }
    #[derive(Deserialize)]
    struct LoginResponse {
        access_token: String,
    }
    let session: LoginResponse = resp.json().await.context("Failed to parse the sign-in")?;
    Ok(session.access_token)
}

async fn register(http: &reqwest::Client, api_url: &str, creds: &Credentials<'_>) -> Result<()> {
    let resp = http
        .post(format!("{api_url}/auth/register"))
        .json(&serde_json::json!({ "email": creds.email, "password": creds.password }))
        .send()
        .await
        .context("Failed to reach the server")?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("creating the admin account failed: {body}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn configuration_server(
        statuses: &[u16],
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api", listener.local_addr().unwrap());
        let statuses = statuses.to_vec();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for status in statuses {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = BufReader::new(socket);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    assert!(socket.read_line(&mut line).await.unwrap() > 0);
                    request.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                requests.push(request);
                socket.get_mut().write_all(format!("HTTP/1.1 {status} X\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}").as_bytes()).await.unwrap();
            }
            requests
        });
        (url, task)
    }

    #[tokio::test]
    async fn launcher_prefers_platform_before_sending_credentials() {
        let (url, requests) = configuration_server(&[200]).await;
        assert_eq!(
            management_url(&reqwest::Client::new(), &format!("{url}/"))
                .await
                .unwrap(),
            format!("{url}/v1/platform")
        );
        let requests = requests.await.unwrap();
        assert!(requests[0].starts_with("GET /api/v1/platform/auth/config "));
        assert!(!requests[0].to_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn launcher_selects_legacy_for_older_published_images() {
        let (url, requests) = configuration_server(&[404, 200]).await;
        assert_eq!(
            management_url(&reqwest::Client::new(), &url).await.unwrap(),
            format!("{url}/v1")
        );
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /api/v1/platform/auth/config "));
        assert!(requests[1].starts_with("GET /api/v1/auth/config "));
        assert!(
            requests
                .iter()
                .all(|r| !r.to_lowercase().contains("authorization:"))
        );
    }

    #[tokio::test]
    async fn launcher_does_not_fall_back_on_denial_or_server_errors() {
        for status in [401, 403, 500] {
            let (url, requests) = configuration_server(&[status]).await;
            let error = management_url(&reqwest::Client::new(), &url)
                .await
                .unwrap_err();
            assert!(error.to_string().contains(&status.to_string()));
            assert_eq!(requests.await.unwrap().len(), 1);
        }
    }

    fn store(current: &str, names: &[&str]) -> CredentialStore {
        let mut store = CredentialStore::default();
        for name in names {
            store.profiles.insert(
                (*name).to_string(),
                Profile {
                    api_url: "https://app.everruns.com/api".into(),
                    api_key: "evr_pat_x".into(),
                    org_id: None,
                    user_email: None,
                    user_name: None,
                },
            );
        }
        store.current_profile = current.to_string();
        store
    }

    #[test]
    fn becomes_current_when_nothing_else_is_in_use() {
        for policy in [SwitchPolicy::Ask, SwitchPolicy::Never] {
            assert!(decide_current(&store("", &[]), "local", policy).unwrap());
            assert!(decide_current(&store("local", &["local"]), "local", policy).unwrap());
            // A current profile that no longer exists is not in use.
            assert!(decide_current(&store("gone", &[]), "local", policy).unwrap());
        }
    }

    #[test]
    fn never_takes_over_another_profile_silently() {
        let s = store("default", &["default"]);
        assert!(!decide_current(&s, "local", SwitchPolicy::Never).unwrap());
        assert!(decide_current(&s, "local", SwitchPolicy::Always).unwrap());
    }
}
