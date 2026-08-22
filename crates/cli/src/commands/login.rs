// CLI login command — interactive OAuth login with localhost callback
//
// Decision: Spawn a one-shot localhost HTTP server to receive the OAuth callback.
// Decision: browser launch goes through crate::browser, which spawns the
// platform URL handler directly.
// Decision: Fall back to --token flag for headless/SSH environments.

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::auth::{CredentialStore, Profile};

const DEFAULT_API_URL: &str = "https://app.everruns.com/api";

/// POST /v1/auth/cli/start request
#[derive(Serialize)]
struct CliStartRequest {
    redirect_port: u16,
}

/// POST /v1/auth/cli/start response
#[derive(Deserialize)]
struct CliStartResponse {
    auth_url: String,
    /// CSRF state nonce. Echoed back on the loopback callback and validated by
    /// the CLI to bind the callback to this login attempt.
    state: String,
}

/// POST /v1/auth/cli/exchange request
#[derive(Serialize)]
struct CliExchangeRequest {
    code: String,
    hostname: Option<String>,
    os: Option<String>,
}

/// POST /v1/auth/cli/exchange response
#[derive(Deserialize)]
struct CliExchangeResponse {
    personal_access_token: String,
    user: CliUserInfo,
    orgs: Vec<OrgInfo>,
}

#[derive(Deserialize)]
struct CliUserInfo {
    #[allow(dead_code)]
    id: String,
    email: String,
    name: String,
}

#[derive(Deserialize, Clone)]
struct OrgInfo {
    public_id: String,
    name: String,
    role: String,
}

/// Parsed `code`/`state` query parameters from the loopback OAuth callback.
#[derive(Debug, Default, PartialEq)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
}

impl CallbackParams {
    /// Return the `code` only if a `state` is present and exactly matches the
    /// `expected` value issued for this login attempt. Missing or mismatched
    /// `state`, or a missing `code`, yields `None` (fail closed — CSRF defense).
    fn validated_code(self, expected: &str) -> Option<String> {
        match (self.code, self.state) {
            (Some(code), Some(state)) if state == expected => Some(code),
            _ => None,
        }
    }
}

/// Parse the `code` and `state` query parameters from an HTTP request's first
/// line (`GET /callback?code=...&state=... HTTP/1.1`).
fn parse_request_line(request: &str) -> Option<CallbackParams> {
    let line = request.lines().next()?;
    let path = line.split_whitespace().nth(1)?;

    let mut params = CallbackParams::default();
    // A request line with no query string yields empty params (which then fail
    // validation), rather than `None` — only a malformed request line is `None`.
    if let Some(query) = path.split('?').nth(1) {
        for param in query.split('&') {
            if let Some(value) = param.strip_prefix("code=") {
                params.code = Some(value.to_string());
            } else if let Some(value) = param.strip_prefix("state=") {
                params.state = Some(value.to_string());
            }
        }
    }
    Some(params)
}

/// Run the login command
pub async fn run(api_url: Option<&str>, token: bool, profile: &str) -> Result<()> {
    let api_url = api_url
        .map(|s| s.to_string())
        .or_else(|| std::env::var("EVERRUNS_API_URL").ok())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string());

    if token {
        return run_token_login(&api_url, profile).await;
    }

    run_oauth_login(&api_url, profile).await
}

/// Interactive OAuth login with localhost callback
async fn run_oauth_login(api_url: &str, profile: &str) -> Result<()> {
    // 1. Find an available port for the localhost callback
    let listener = TcpListener::bind("127.0.0.1:0")
        .context("Failed to bind localhost port for OAuth callback")?;
    let port = listener.local_addr()?.port();
    drop(listener); // Release so our HTTP server can bind it

    // 2. Call POST /v1/auth/cli/start to get the auth URL
    let client = reqwest::Client::new();
    let start_resp = client
        .post(format!("{}/v1/auth/cli/start", api_url))
        .json(&CliStartRequest {
            redirect_port: port,
        })
        .send()
        .await
        .context("Failed to connect to Everruns server")?;

    if !start_resp.status().is_success() {
        let status = start_resp.status();
        let body = start_resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Server returned {} when starting CLI login: {}",
            status,
            body
        ));
    }

    let start: CliStartResponse = start_resp
        .json()
        .await
        .context("Failed to parse server response")?;

    // 3. Start localhost HTTP server to receive the callback
    let (code_tx, code_rx) = oneshot::channel::<String>();
    let code_tx = Arc::new(tokio::sync::Mutex::new(Some(code_tx)));
    let success_url = format!(
        "{}/cli/login-success",
        api_url.trim_end_matches("/api").trim_end_matches('/')
    );
    // Derive the server base URL for redirection
    // api_url might be "http://localhost:9300/api" or "https://app.everruns.com/api"
    let success_redirect = success_url.clone();

    // CSRF state issued for this login attempt. The loopback callback must echo
    // it back unchanged; otherwise we refuse to exchange the code.
    let expected_state = start.state.clone();

    let server = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .expect("Failed to bind localhost port");

        // Accept one connection
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("Failed to accept connection");
        let mut buf = vec![0u8; 4096];
        let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
            .await
            .expect("Failed to read request");
        let request = String::from_utf8_lossy(&buf[..n]);

        // Parse `code` and `state` from the GET request line, then enforce that
        // the returned `state` matches the value we issued (CSRF protection).
        let code = parse_request_line(&request)
            .and_then(|callback| callback.validated_code(&expected_state));

        // Respond with redirect to success page
        let response = if code.is_some() {
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {}\r\nConnection: close\r\n\r\n",
                success_redirect
            )
        } else {
            "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid login callback (missing code or state mismatch)"
                .to_string()
        };
        tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
            .await
            .ok();

        // Send code to main flow
        if let Some(code) = code
            && let Some(tx) = code_tx.lock().await.take()
        {
            let _ = tx.send(code);
        }
    });

    // 4. Open browser
    eprintln!("Opening browser for login...");
    eprintln!("If your browser doesn't open, visit:");
    eprintln!("  {}", start.auth_url);
    eprintln!();
    eprintln!("Waiting for login...");

    if crate::browser::open(&start.auth_url).is_err() {
        eprintln!("(Could not open browser automatically)");
    }

    // 5. Wait for the callback (with timeout)
    let code = tokio::time::timeout(std::time::Duration::from_secs(300), code_rx)
        .await
        .map_err(|_| anyhow!("Login timed out after 5 minutes"))?
        .map_err(|_| anyhow!("Login was cancelled"))?;

    server.abort();

    // 6. Exchange code for API key
    let hostname = hostname::get().ok().and_then(|h| h.into_string().ok());

    let exchange_resp = client
        .post(format!("{}/v1/auth/cli/exchange", api_url))
        .json(&CliExchangeRequest {
            code,
            hostname: hostname.clone(),
            os: Some(std::env::consts::OS.to_string()),
        })
        .send()
        .await
        .context("Failed to exchange code for API key")?;

    if !exchange_resp.status().is_success() {
        let status = exchange_resp.status();
        let body = exchange_resp.text().await.unwrap_or_default();
        return Err(anyhow!(
            "Server returned {} when exchanging code: {}",
            status,
            body
        ));
    }

    let exchange: CliExchangeResponse = exchange_resp
        .json()
        .await
        .context("Failed to parse exchange response")?;

    // 7. Select org
    let org_id = select_org(&exchange.orgs)?;

    // 8. Store credentials
    let mut store = CredentialStore::load().unwrap_or_default();
    store.set_profile(
        profile,
        Profile {
            api_url: api_url.to_string(),
            api_key: exchange.personal_access_token,
            org_id: Some(org_id),
            user_email: Some(exchange.user.email.clone()),
            user_name: Some(exchange.user.name.clone()),
        },
    );
    store.current_profile = profile.to_string();
    store.save()?;

    eprintln!();
    eprintln!(
        "Logged in as {} ({})",
        exchange.user.name, exchange.user.email
    );
    eprintln!(
        "Credentials saved to {:?}",
        crate::auth::credentials_path()?
    );

    Ok(())
}

/// Token-paste login for headless environments
async fn run_token_login(api_url: &str, profile: &str) -> Result<()> {
    eprintln!("Paste your personal access token (starts with evr_pat_):");

    let api_key: String = dialoguer::Input::new()
        .with_prompt("Personal access token")
        .validate_with(|input: &String| -> std::result::Result<(), String> {
            if input.starts_with("evr_pat_") && input.len() > 10 {
                Ok(())
            } else {
                Err(
                    "Personal access token must start with 'evr_pat_' and be at least 10 characters"
                        .to_string(),
                )
            }
        })
        .interact_text()
        .context("Failed to read personal access token")?;

    // Validate the token by calling /v1/auth/me
    let client = reqwest::Client::new();
    let me_resp = client
        .get(format!("{}/v1/auth/me", api_url))
        .header("Authorization", &api_key)
        .send()
        .await
        .context("Failed to connect to Everruns server")?;

    if !me_resp.status().is_success() {
        return Err(anyhow!(
            "Invalid personal access token (server returned {})",
            me_resp.status()
        ));
    }

    #[derive(Deserialize)]
    struct MeResponse {
        email: String,
        name: String,
        organizations: Option<Vec<OrgInfo>>,
    }

    let me: MeResponse = me_resp
        .json()
        .await
        .context("Failed to parse /v1/auth/me")?;
    let orgs = me.organizations.unwrap_or_default();
    let org_id = select_org(&orgs)?;

    let mut store = CredentialStore::load().unwrap_or_default();
    store.set_profile(
        profile,
        Profile {
            api_url: api_url.to_string(),
            api_key,
            org_id: Some(org_id),
            user_email: Some(me.email.clone()),
            user_name: Some(me.name.clone()),
        },
    );
    store.current_profile = profile.to_string();
    store.save()?;

    eprintln!("Logged in as {} ({})", me.name, me.email);
    Ok(())
}

/// Interactive org selection
fn select_org(orgs: &[OrgInfo]) -> Result<String> {
    if orgs.is_empty() {
        return Err(anyhow!("No organizations found for this user"));
    }

    if orgs.len() == 1 {
        eprintln!("Using organization: {}", orgs[0].name);
        return Ok(orgs[0].public_id.clone());
    }

    let items: Vec<String> = orgs
        .iter()
        .map(|o| format!("{} ({})", o.name, o.role))
        .collect();

    let selection = dialoguer::Select::new()
        .with_prompt("Select organization")
        .items(&items)
        .default(0)
        .interact()
        .context("Failed to select organization")?;

    Ok(orgs[selection].public_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(query: &str) -> String {
        format!(
            "GET /callback?{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
            query
        )
    }

    #[test]
    fn parses_code_and_state() {
        let params = parse_request_line(&req("code=abc123&state=nonce42")).unwrap();
        assert_eq!(
            params,
            CallbackParams {
                code: Some("abc123".to_string()),
                state: Some("nonce42".to_string()),
            }
        );
    }

    #[test]
    fn matching_state_yields_code() {
        let params = parse_request_line(&req("code=abc123&state=nonce42")).unwrap();
        assert_eq!(params.validated_code("nonce42"), Some("abc123".to_string()));
    }

    #[test]
    fn mismatched_state_is_rejected() {
        let params = parse_request_line(&req("code=abc123&state=attacker")).unwrap();
        assert_eq!(params.validated_code("nonce42"), None);
    }

    #[test]
    fn missing_state_is_rejected() {
        // A callback with only `code` (the pre-fix behavior) must now fail closed.
        let params = parse_request_line(&req("code=abc123")).unwrap();
        assert_eq!(params.state, None);
        assert_eq!(params.validated_code("nonce42"), None);
    }

    #[test]
    fn missing_code_is_rejected() {
        let params = parse_request_line(&req("state=nonce42")).unwrap();
        assert_eq!(params.validated_code("nonce42"), None);
    }

    #[test]
    fn empty_expected_state_does_not_accept_missing_state() {
        // Defensive: even if the issued state were empty, a callback without a
        // `state` parameter must not be accepted.
        let params = parse_request_line(&req("code=abc123")).unwrap();
        assert_eq!(params.validated_code(""), None);
    }

    #[test]
    fn no_query_string_yields_empty_params() {
        let params = parse_request_line("GET /callback HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!(params, CallbackParams::default());
        assert_eq!(params.validated_code("nonce42"), None);
    }
}
