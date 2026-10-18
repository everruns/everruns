//! Native HTTPS authority transport. No transport code participates in agent decisions.
use crate::client::AuthorityTransport;
use async_trait::async_trait;
use everruns_contracts::{
    error::{AgentLoopError, Result},
    typed_id::SessionId,
};
use everruns_core::host::session_actor::store::actor_lease_lost;
use serde_json::Value;
use std::time::Duration;

pub(crate) struct HttpAuthorityTransport {
    base: reqwest::Url,
    token: reqwest::header::HeaderValue,
    http: reqwest::Client,
}
impl HttpAuthorityTransport {
    pub fn new(base: &str, token: &str) -> Result<Self> {
        let base = reqwest::Url::parse(base)
            .map_err(|_| AgentLoopError::config("invalid authority URL"))?;
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(base.scheme() == "https" || base.scheme() == "http" && loopback)
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(AgentLoopError::config(
                "authority URL must be an HTTPS origin (loopback HTTP is allowed)",
            ));
        }
        if token.len() < 32 {
            return Err(AgentLoopError::config(
                "authority secret must contain at least 32 bytes",
            ));
        }
        let mut token = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| AgentLoopError::config("invalid authority credential"))?;
        token.set_sensitive(true);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| AgentLoopError::config("cannot build authority transport"))?;
        Ok(Self { base, token, http })
    }
}
#[async_trait]
impl AuthorityTransport for HttpAuthorityTransport {
    fn credential_matches(&self, token: &str) -> bool {
        self.token.as_bytes() == format!("Bearer {token}").as_bytes()
    }
    async fn invoke(
        &self,
        tenant: &str,
        session: SessionId,
        operation: &str,
        body: Value,
    ) -> Result<Value> {
        let url = self
            .base
            .join(&format!("/tenants/{tenant}/sessions/{session}/{operation}"))
            .map_err(|_| AgentLoopError::config("invalid authority path"))?;
        let mut response = self
            .http
            .post(url)
            .header(reqwest::header::AUTHORIZATION, self.token.clone())
            .json(&body)
            .send()
            .await
            .map_err(|_| AgentLoopError::store("authority transport unavailable"))?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(actor_lease_lost(
                session,
                "authority rejected stale ownership",
            ));
        }
        if !response.status().is_success() {
            return Err(crate::client::rejected(response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AgentLoopError::store("authority response interrupted"))?
        {
            if bytes.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
                return Err(AgentLoopError::store("authority response exceeds limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| AgentLoopError::store("invalid authority response"))
    }
}
