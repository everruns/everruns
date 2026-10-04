//! Browser authorization with a loopback receiver. The host launches the URL and saves the result.
use super::{ChatGptRegistration, CodexAuth, OpenSourceGrant, oauth::*};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use rand::RngExt;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub struct LoginAttempt {
    pub authorize_url: String,
    listener: TcpListener,
    endpoints: Endpoints,
    redirect_uri: String,
    registration: Option<ChatGptRegistration>,
    state: String,
    nonce: String,
    verifier: String,
}
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
impl LoginAttempt {
    pub async fn start(
        endpoints: Endpoints,
        agent_name: &str,
        host_id: &str,
        registration: Option<ChatGptRegistration>,
        id_token_hint: Option<&str>,
        force_consent: bool,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("bind ChatGPT callback")?;
        let redirect_uri = format!(
            "http://127.0.0.1:{}/auth/callback",
            listener.local_addr()?.port()
        );
        let state = random_token();
        let nonce = random_token();
        let verifier = random_token();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let authorize_url = authorize_url(
            &endpoints,
            &AuthorizeParams {
                agent_name,
                host_id,
                registration: registration.as_ref(),
                id_token_hint,
                force_consent,
                redirect_uri: &redirect_uri,
                state: &state,
                nonce: &nonce,
                code_challenge: &challenge,
            },
        )?
        .to_string();
        Ok(Self {
            authorize_url,
            listener,
            endpoints,
            redirect_uri,
            registration,
            state,
            nonce,
            verifier,
        })
    }
    pub fn nonce(&self) -> &str {
        &self.nonce
    }
    pub async fn finish(self) -> Result<CodexAuth> {
        let params = tokio::time::timeout(Duration::from_secs(600), self.callback())
            .await
            .map_err(|_| anyhow!("ChatGPT sign-in timed out"))??;
        if params.contains_key("error") {
            bail!("ChatGPT sign-in was declined or did not complete");
        }
        let code = params
            .get("code")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("ChatGPT callback has no authorization code"))?;
        let client_id = issued_client_id(
            self.registration.as_ref().map(|r| r.client_id.as_str()),
            params.get("client_id").map(String::as_str),
        )?;
        let token = exchange_code(
            &self.endpoints.token,
            &client_id,
            code,
            &self.verifier,
            &self.redirect_uri,
        )
        .await?;
        let id_token = token
            .id_token
            .as_ref()
            .ok_or_else(|| anyhow!("ChatGPT sign-in returned no ID token"))?;
        let identity = validate_id_token(
            id_token,
            &fetch_jwks(&self.endpoints.jwks).await?,
            &self.endpoints.issuer,
            &client_id,
            &self.nonce,
            now_epoch_millis() / 1000,
        )?;
        if self
            .registration
            .as_ref()
            .is_some_and(|r| r.subject != identity.subject)
        {
            bail!("ChatGPT returned a different account for this registration");
        }
        let scopes = token.scopes();
        Ok(CodexAuth {
            expires_at: token
                .expires_in
                .filter(|s| *s > 0)
                .map(|s| now_epoch_millis().saturating_add(s.saturating_mul(1000))),
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            client_id: Some(client_id),
            email: identity.email,
            account_id: None,
            open_source: Some(OpenSourceGrant {
                id_token: token.id_token,
                scopes,
                subject: Some(identity.subject),
            }),
        })
    }
    async fn callback(&self) -> Result<HashMap<String, String>> {
        loop {
            let (mut socket, _) = self.listener.accept().await?;
            let mut request = Vec::with_capacity(4096);
            let read = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await?;
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() >= 16 * 1024 {
                        break;
                    }
                }
                Ok::<_, std::io::Error>(())
            })
            .await;
            if !matches!(read, Ok(Ok(()))) || request.len() >= 16 * 1024 {
                continue;
            }
            let req = String::from_utf8_lossy(&request);
            let mut line = req.lines().next().unwrap_or("").split_whitespace();
            let method = line.next();
            let path = line.next().unwrap_or("/");
            let Ok(parsed) = reqwest::Url::parse(&format!("http://127.0.0.1{path}")) else {
                continue;
            };
            let pairs: Vec<_> = parsed.query_pairs().into_owned().collect();
            let params: HashMap<_, _> = pairs.iter().cloned().collect();
            let valid = method == Some("GET")
                && parsed.path() == "/auth/callback"
                && pairs.len() == params.len()
                && params.get("state") == Some(&self.state);
            let status = if valid { "200 OK" } else { "400 Bad Request" };
            // Receiving a code is not proof of a completed login; verification follows.
            let body = if valid {
                "Sign-in received. Return to the application to check the result."
            } else {
                "This sign-in callback does not match the pending attempt."
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                socket.write_all(response.as_bytes()),
            )
            .await;
            if valid {
                return Ok(params);
            }
        }
    }
}
