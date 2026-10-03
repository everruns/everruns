//! Rotating OAuth authentication. Storage leases serialize refresh across processes.
use super::{CodexAuth, oauth};
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::{AgentLoopError, LlmErrorKind, ProviderAuth, ProviderAuthRequest};
use std::sync::Arc;

#[async_trait]
pub trait TokenStore: Send + Sync {
    /// Hold an exclusive, account-scoped lease until its returned guard drops.
    async fn lock(&self) -> anyhow::Result<Box<dyn Send>>;
    async fn load(&self) -> anyhow::Result<Option<CodexAuth>>;
    async fn save(&self, auth: CodexAuth) -> anyhow::Result<()>;
    /// Hosts with writers outside the refresh lease must implement an atomic comparison.
    async fn compare_and_save(
        &self,
        previous: &CodexAuth,
        auth: CodexAuth,
    ) -> anyhow::Result<bool> {
        if self.load().await?.as_ref() != Some(previous) {
            return Ok(false);
        }
        self.save(auth).await?;
        Ok(true)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenRoute {
    ChatGptPlan,
    Codex,
}
#[derive(Clone)]
pub struct RotatingAuth {
    store: Arc<dyn TokenStore>,
    token_url: String,
    route: TokenRoute,
    originator: String,
}
impl RotatingAuth {
    pub fn new(store: Arc<dyn TokenStore>, route: TokenRoute) -> Self {
        Self {
            store,
            token_url: match route {
                TokenRoute::ChatGptPlan => oauth::Endpoints::production().token,
                TokenRoute::Codex => "https://auth.openai.com/oauth/token".into(),
            },
            route,
            originator: "everruns".into(),
        }
    }
    pub fn with_token_url(mut self, url: impl Into<String>) -> Self {
        self.token_url = url.into();
        self
    }
    pub fn with_originator(mut self, name: impl Into<String>) -> Self {
        self.originator = name.into();
        self
    }
    pub async fn token(&self) -> Result<CodexAuth> {
        let _lease = self.store.lock().await.map_err(storage_error)?;
        let mut auth = self
            .store
            .load()
            .await
            .map_err(storage_error)?
            .ok_or_else(|| authentication("Sign in with ChatGPT to reconnect this provider."))?;
        if self.route == TokenRoute::ChatGptPlan
            && auth
                .client_id
                .as_deref()
                .is_none_or(|id| oauth::validate_client_id(id).is_err())
        {
            return Err(authentication(
                "This ChatGPT grant has no valid issuing client. Reconnect this account.",
            ));
        }
        if self.route == TokenRoute::ChatGptPlan
            && !auth
                .open_source
                .as_ref()
                .is_some_and(|g| oauth::grants_plan_usage(&g.scopes))
        {
            return Err(authentication(
                "ChatGPT plan use was not granted. Sign in again to allow plan use.",
            ));
        }
        if auth.access_token.is_empty() {
            return Err(authentication(
                "The ChatGPT connection has no access token.",
            ));
        }
        if auth
            .expires_at
            .is_some_and(|t| t <= oauth::now_epoch_millis() + 60_000)
        {
            let previous = auth.clone();
            let refresh = auth.refresh_token.as_deref().ok_or_else(|| {
                authentication("ChatGPT sign-in has expired. Reconnect this account.")
            })?;
            let client = auth
                .client_id
                .as_deref()
                .unwrap_or("app_EMoamEEZ73f0CkXaXp7hrann");
            let fresh = match self.route {
                TokenRoute::ChatGptPlan => {
                    oauth::refresh_with_token_at(&self.token_url, client, refresh).await
                }
                TokenRoute::Codex => refresh_codex(&self.token_url, client, refresh).await,
            }
            .map_err(|error| {
                let kind = match error
                    .downcast_ref::<oauth::TokenHttpError>()
                    .map(|e| e.status)
                {
                    Some(400 | 401 | 403) => LlmErrorKind::Authentication,
                    Some(429) => LlmErrorKind::RateLimited,
                    _ => LlmErrorKind::Unavailable,
                };
                AgentLoopError::llm_kind(
                    kind,
                    if kind == LlmErrorKind::Authentication {
                        "ChatGPT token refresh failed. Reconnect this account."
                    } else {
                        "ChatGPT token refresh is temporarily unavailable. Retry later."
                    },
                )
            })?;
            auth.access_token = fresh.access_token;
            auth.refresh_token = fresh.refresh_token.or(auth.refresh_token);
            auth.expires_at = fresh.expires_at;
            if let (Some(grant), Some(fresh)) = (&mut auth.open_source, fresh.open_source) {
                if fresh.id_token.is_some() {
                    grant.id_token = fresh.id_token;
                }
                if !fresh.scopes.is_empty() {
                    grant.scopes = fresh.scopes;
                }
            }
            // THREAT[TM-LLM-048]: Rotation is durable before credentials leave the host lease.
            // Persist the entire rotated pair before exposing the access token.
            if !self
                .store
                .compare_and_save(&previous, auth.clone())
                .await
                .map_err(storage_error)?
            {
                return Err(AgentLoopError::llm_kind(
                    LlmErrorKind::Unavailable,
                    "The ChatGPT connection changed during refresh. Retry with the current account.",
                ));
            }
        }
        if self.route == TokenRoute::ChatGptPlan
            && !auth
                .open_source
                .as_ref()
                .is_some_and(|g| oauth::grants_plan_usage(&g.scopes))
        {
            return Err(authentication(
                "ChatGPT plan use was not granted. Sign in again to allow plan use.",
            ));
        }
        Ok(auth)
    }
}
#[async_trait]
impl ProviderAuth for RotatingAuth {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        let auth = self.token().await?;
        let mut headers = vec![(
            "authorization".into(),
            format!("Bearer {}", auth.access_token),
        )];
        if self.route == TokenRoute::Codex {
            headers.push(("originator".into(), self.originator.clone()));
            headers.push(("openai-beta".into(), "responses=experimental".into()));
            if let Some(account) = auth.account_id {
                headers.push(("chatgpt-account-id".into(), account));
            }
        }
        Ok(headers)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
fn authentication(message: &str) -> AgentLoopError {
    AgentLoopError::llm_kind(LlmErrorKind::Authentication, message)
}
fn storage_error(_: anyhow::Error) -> AgentLoopError {
    AgentLoopError::llm("Cannot load or persist the ChatGPT connection credentials.")
}
async fn refresh_codex(url: &str, client: &str, refresh: &str) -> anyhow::Result<CodexAuth> {
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client),
            ("refresh_token", refresh),
        ])
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(oauth::TokenHttpError {
            status: response.status().as_u16(),
        }
        .into());
    }
    let token: oauth::TokenResponse = response.json().await?;
    Ok(CodexAuth {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: token
            .expires_in
            .map(|n| oauth::now_epoch_millis() + n.saturating_mul(1000)),
        account_id: None,
        email: None,
        client_id: Some(client.into()),
        open_source: None,
    })
}
