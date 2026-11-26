//! Bounded HTTP operations with no implicit redirects or retries.
use crate::{Error, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

pub mod provider;

pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub max_response_bytes: usize,
}
impl Request {
    pub fn json(url: String, token: &str, body: &serde_json::Value) -> Self {
        Self {
            method: "POST".into(),
            url,
            headers: BTreeMap::from([
                ("authorization".into(), format!("Bearer {token}")),
                ("content-type".into(), "application/json".into()),
            ]),
            body: body.to_string().into_bytes(),
            max_response_bytes: 1024 * 1024,
        }
    }
}
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
impl Response {
    pub fn json(&self) -> Result<serde_json::Value> {
        serde_json::from_slice(&self.body).map_err(|_| Error::InvalidResponse)
    }
    pub fn rate_limit(&self) -> Option<Error> {
        if self.status == 429 {
            Some(Error::RateLimited {
                retry_after: Duration::from_secs(
                    self.headers
                        .get("retry-after")
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(1)
                        .min(60),
                ),
            })
        } else {
            None
        }
    }
}
#[async_trait]
pub trait Transport: Send + Sync {
    async fn execute(&self, request: Request) -> Result<Response>;
}
pub type SharedTransport = Arc<dyn Transport>;
pub struct ReqwestTransport {
    client: reqwest::Client,
}
impl ReqwestTransport {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| Error::Transport)?,
        })
    }
}
#[async_trait]
impl Transport for ReqwestTransport {
    async fn execute(&self, request: Request) -> Result<Response> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| Error::InvalidInput("HTTP method"))?;
        let mut builder = self.client.request(method, &request.url).body(request.body);
        for (key, value) in request.headers {
            builder = builder.header(key, value);
        }
        let response = builder.send().await.map_err(|_| Error::Transport)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().into(), v.into())))
            .collect();
        if response
            .content_length()
            .is_some_and(|length| length > request.max_response_bytes as u64)
        {
            return Err(Error::Capacity("HTTP response"));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Error::Transport)?;
            if body.len().saturating_add(chunk.len()) > request.max_response_bytes {
                return Err(Error::Capacity("HTTP response"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}
/// Credentials are sent only to the configured API origin, never a webhook URL.
pub fn validate_api_origin(url: &str) -> Result<()> {
    let url = url::Url::parse(url).map_err(|_| Error::InvalidInput("API origin"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidInput("API origin"));
    }
    if url.scheme() != "https"
        && !(url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
    {
        return Err(Error::InvalidInput("HTTPS API origin required"));
    }
    Ok(())
}
