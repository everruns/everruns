use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::Value;

/// Tries a contract command gets before its failure is reported.
const COMMAND_ATTEMPTS: u32 = 4;
/// Wait before the first retry; doubled for each later one.
const COMMAND_RETRY_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_millis(500)
};

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

    /// POST a contract command, retrying what a retry cannot make worse.
    ///
    /// Every attempt carries the same `Idempotency-Key`, so the server runs a
    /// mutating command at most once: a retry after a lost response gets the
    /// stored result back. That is what makes it safe to retry a request that
    /// may already have reached the server (a dropped connection, a timeout,
    /// a 502/503/504 from a proxy), and a 409 meaning the first attempt is
    /// still running. Anything else is reported at once.
    pub async fn post_command(
        &self,
        path: &str,
        body: &Value,
        idempotency_key: &str,
    ) -> Result<Value> {
        let mut delay = COMMAND_RETRY_DELAY;
        let mut attempt = 1;
        loop {
            let sent = self
                .request(Method::POST, path)
                .header("Idempotency-Key", idempotency_key)
                .json(body)
                .send()
                .await;
            let failure = match sent {
                Err(err) if err.is_builder() => {
                    return Err(err).with_context(|| format!("Failed to call POST {path}"));
                }
                Err(err) => anyhow::Error::new(err).context(format!("Failed to call POST {path}")),
                Ok(response) if response.status().is_success() => {
                    return response
                        .json()
                        .await
                        .with_context(|| format!("Failed to parse response from POST {path}"));
                }
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    let failure = anyhow::anyhow!("API request failed ({status}): {text}");
                    if !retryable_status(status, &text) {
                        return Err(failure);
                    }
                    failure
                }
            };
            if attempt >= COMMAND_ATTEMPTS {
                return Err(failure);
            }
            tokio::time::sleep(delay).await;
            delay *= 2;
            attempt += 1;
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

/// Gateway failures may have happened after the command ran; a 409 for an
/// idempotency key means the first attempt is still running.
fn retryable_status(status: StatusCode, body: &str) -> bool {
    match status {
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT => {
            true
        }
        StatusCode::CONFLICT => serde_json::from_str::<Value>(body)
            .is_ok_and(|problem| problem["code"] == "idempotency_key_in_progress"),
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Answer one connection per scripted response, reporting each request's
    /// `Idempotency-Key`. `None` drops the connection without answering.
    async fn serve(
        responses: Vec<Option<(u16, &'static str)>>,
    ) -> (String, tokio::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (keys, received) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = vec![0u8; 8192];
                let read = socket.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
                let key = request
                    .lines()
                    .find_map(|line| line.strip_prefix("idempotency-key: "))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                keys.send(key).await.ok();
                let Some((status, body)) = response else {
                    continue;
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (url, received)
    }

    #[tokio::test]
    async fn retries_lost_responses_with_the_same_key() {
        let (url, mut keys) = serve(vec![
            None,
            Some((503, "")),
            Some((409, r#"{"code":"idempotency_key_in_progress"}"#)),
            Some((200, r#"{"output":{"id":"agent_1"}}"#)),
        ])
        .await;
        let client = ApiClient::new(&url, "key", None);

        let response = client
            .post_command("/v1/commands/create_agent", &serde_json::json!({}), "k-1")
            .await
            .unwrap();

        assert_eq!(response["output"]["id"], "agent_1");
        for _ in 0..4 {
            assert_eq!(keys.recv().await.unwrap(), "k-1");
        }
    }

    #[tokio::test]
    async fn does_not_retry_a_rejected_command() {
        let (url, mut keys) = serve(vec![
            Some((409, r#"{"code":"conflict"}"#)),
            Some((200, "{}")),
        ])
        .await;
        let client = ApiClient::new(&url, "key", None);

        let err = client
            .post_command("/v1/commands/create_agent", &serde_json::json!({}), "k-2")
            .await
            .unwrap_err();

        assert!(err.to_string().contains("409"), "{err}");
        assert_eq!(keys.recv().await.unwrap(), "k-2");
        assert!(
            keys.try_recv().is_err(),
            "a rejected command is not retried"
        );
    }

    #[tokio::test]
    async fn gives_up_after_the_last_attempt() {
        let (url, _keys) = serve(vec![Some((503, "")); 4]).await;
        let client = ApiClient::new(&url, "key", None);

        let err = client
            .post_command("/v1/commands/create_agent", &serde_json::json!({}), "k-3")
            .await
            .unwrap_err();

        assert!(err.to_string().contains("503"), "{err}");
    }
}
