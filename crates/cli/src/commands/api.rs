use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::Value;

#[derive(Clone)]
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

    /// An authenticated request to `path` under the API URL, for callers that
    /// need the raw response (a stream, a file).
    pub fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.api_url.trim_end_matches('/'), path);
        let request = self
            .http
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.api_key));
        let env_org = std::env::var("EVERRUNS_ORG_ID").ok();
        match self.org_id.or(env_org.as_deref()) {
            Some(org_id) => request.header("X-Org-Id", org_id),
            None => request,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut request = self.request(method.clone(), path);
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

/// `path` with `query` appended, each value percent-encoded.
pub fn with_query(path: &str, query: &[(&str, &str)]) -> String {
    if query.is_empty() {
        return path.to_string();
    }
    let pairs: Vec<String> = query
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect();
    format!("{path}?{}", pairs.join("&"))
}
