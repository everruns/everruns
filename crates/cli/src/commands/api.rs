use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::Value;

pub struct ApiClient<'a> {
    api_url: &'a str,
    api_key: &'a str,
    org_id: Option<&'a str>,
    http: reqwest::Client,
}

impl<'a> ApiClient<'a> {
    pub fn new(api_url: &'a str, api_key: &'a str, org_id: Option<&'a str>) -> Self {
        Self {
            api_url,
            api_key,
            org_id,
            http: reqwest::Client::new(),
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.send(Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: Option<&Value>) -> Result<Value> {
        self.send(Method::POST, path, body).await
    }

    pub async fn patch(&self, path: &str, body: &Value) -> Result<Value> {
        self.send(Method::PATCH, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value> {
        self.send(Method::DELETE, path, None).await
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{}{}", self.api_url.trim_end_matches('/'), path);
        let mut request = self
            .http
            .request(method.clone(), &url)
            .header("Authorization", format!("Bearer {}", self.api_key));

        let env_org = std::env::var("EVERRUNS_ORG_ID").ok();
        if let Some(org_id) = self.org_id.or(env_org.as_deref()) {
            request = request.header("X-Org-Id", org_id);
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to call {method} {path}"))?;
        let status = response.status();
        if !status.is_success() {
            let response_body = response.text().await.unwrap_or_default();
            anyhow::bail!("API request failed ({status}): {response_body}");
        }
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        response
            .json()
            .await
            .with_context(|| format!("Failed to parse response from {method} {path}"))
    }
}
