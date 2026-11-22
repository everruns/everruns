//! An org token for a load driver that runs against a data plane joined to a
//! real control plane (`agentd-drive load --control-plane`).
//!
//! Decisions:
//! - The driver acts as one member of one org, with an API key, the way a
//!   customer's client does. It exchanges the key at the control plane's
//!   `POST /v1/auth/token` and sends the token to the gateways as
//!   `x-everruns-org-token`; the token's org is the session's tenant.
//! - One token serves every request. It is fetched again a minute before it
//!   ends (halfway through a short life), as the edge does, so a 75 000
//!   session run asks the control plane about once every nine minutes, not
//!   once per session.
//! - A gateway that answers 401 drops the token it refused, and only that
//!   one: thousands of requests refused together cause one fetch, not a
//!   fetch each.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

/// Where the org token comes from.
pub struct OrgToken {
    http: reqwest::Client,
    url: String,
    api_key: String,
    org: String,
    /// The token and when to fetch the next one (unix seconds).
    current: RwLock<Option<(Arc<str>, i64)>>,
    /// One fetch at a time.
    fetching: tokio::sync::Mutex<()>,
}

/// When to fetch again a token that ends at `expires`.
pub fn refresh_at(now: i64, expires: i64) -> i64 {
    expires - ((expires - now) / 2).clamp(0, 60)
}

impl OrgToken {
    /// `control_plane` is the API base the token endpoint hangs off
    /// (`https://app.example.com/api`), `org` the org's public id.
    pub fn new(control_plane: &str, api_key: String, org: String) -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?,
            url: format!("{}/v1/auth/token", control_plane.trim_end_matches('/')),
            api_key,
            org,
            current: RwLock::new(None),
            fetching: tokio::sync::Mutex::new(()),
        })
    }

    /// A token good for now.
    pub async fn get(&self) -> anyhow::Result<Arc<str>> {
        if let Some(token) = self.fresh() {
            return Ok(token);
        }
        let _one = self.fetching.lock().await;
        if let Some(token) = self.fresh() {
            return Ok(token);
        }
        let mut wait = Duration::from_millis(500);
        let mut tries = 0;
        let (token, expires) = loop {
            match self.fetch().await {
                Ok(fetched) => break fetched,
                Err(error) if tries >= 6 => return Err(error),
                Err(error) => {
                    eprintln!("org token: {error:#}; retrying in {wait:?}");
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(Duration::from_secs(10));
                    tries += 1;
                }
            }
        };
        let now = chrono::Utc::now().timestamp();
        *self.current.write() = Some((token.clone(), refresh_at(now, expires)));
        Ok(token)
    }

    /// A gateway refused `token`: fetch a new one next time, unless another
    /// request already did.
    pub fn refused(&self, token: &str) {
        let mut current = self.current.write();
        if current.as_ref().is_some_and(|(held, _)| &**held == token) {
            *current = None;
        }
    }

    fn fresh(&self) -> Option<Arc<str>> {
        let now = chrono::Utc::now().timestamp();
        match &*self.current.read() {
            Some((token, refresh)) if now < *refresh => Some(token.clone()),
            _ => None,
        }
    }

    async fn fetch(&self) -> anyhow::Result<(Arc<str>, i64)> {
        let reply = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .header("x-org-id", &self.org)
            .json(&serde_json::json!({ "org": self.org }))
            .send()
            .await?;
        let status = reply.status();
        if !status.is_success() {
            anyhow::bail!(
                "POST {}: {status} {}",
                self.url,
                reply.text().await.unwrap_or_default()
            );
        }
        let body: serde_json::Value = reply.json().await?;
        let token = body["token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("token reply has no token"))?;
        let expires = body["expires_at_unix"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("token reply has no expires_at_unix"))?;
        Ok((Arc::from(token), expires))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Json;
    use axum::http::HeaderMap;
    use axum::routing::post;

    use super::*;

    #[test]
    fn refreshes_a_minute_early_or_halfway_through_a_short_life() {
        assert_eq!(refresh_at(1_000, 1_600), 1_540);
        assert_eq!(refresh_at(1_000, 1_040), 1_020);
        // Already past its end: fetch now.
        assert_eq!(refresh_at(1_000, 900), 900);
    }

    async fn control_plane(issued: Arc<AtomicUsize>) -> String {
        let app = axum::Router::new().route(
            "/api/v1/auth/token",
            post(
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let issued = issued.clone();
                    async move {
                        assert_eq!(headers["authorization"], "Bearer key");
                        assert_eq!(body["org"], "org_1");
                        let n = issued.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!({
                            "token": format!("t{n}"),
                            "expires_at_unix": chrono::Utc::now().timestamp() + 600,
                        }))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{addr}/api")
    }

    #[tokio::test]
    async fn one_token_serves_until_a_gateway_refuses_it() {
        let issued = Arc::new(AtomicUsize::new(0));
        let token = Arc::new(
            OrgToken::new(
                &control_plane(issued.clone()).await,
                "key".into(),
                "org_1".into(),
            )
            .unwrap(),
        );
        let first = futures::future::join_all((0..50).map(|_| {
            let token = token.clone();
            async move { token.get().await.unwrap() }
        }))
        .await;
        assert!(first.iter().all(|t| &**t == "t0"));
        assert_eq!(issued.load(Ordering::SeqCst), 1);

        // Many requests refused with the same token: one new fetch.
        for _ in 0..10 {
            token.refused("t0");
        }
        assert_eq!(&*token.get().await.unwrap(), "t1");
        token.refused("t0");
        assert_eq!(&*token.get().await.unwrap(), "t1");
        assert_eq!(issued.load(Ordering::SeqCst), 2);
    }
}
