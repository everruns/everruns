use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Method, StatusCode};
use serde_json::Value;

/// What the CLI prints when a personal access token's scope does not cover a
/// command (`403` with problem code `insufficient_scope`). `read` is the only
/// narrow scope, so the line can name it.
pub const INSUFFICIENT_SCOPE_MESSAGE: &str =
    "This personal access token is read-only; this command needs a full-access token.";

/// A non-success API response, kept typed so [`explain`] can recognize the
/// failures a person can act on, whichever command or request path hit them.
#[derive(Debug)]
pub struct ApiError {
    /// What was being attempted, the start of the message.
    pub action: String,
    pub status: StatusCode,
    pub body: String,
}

impl ApiError {
    /// The error for a response that was not a success. Reads the body.
    pub async fn from_response(action: impl Into<String>, response: reqwest::Response) -> Self {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Self {
            action: action.into(),
            status,
            body,
        }
    }

    /// The problem document's `code`, when the body is one.
    fn code(&self) -> Option<String> {
        let problem: Value = serde_json::from_str(&self.body).ok()?;
        problem.get("code")?.as_str().map(ToOwned::to_owned)
    }

    /// A personal access token whose scope does not cover the request.
    pub fn is_insufficient_scope(&self) -> bool {
        self.status == StatusCode::FORBIDDEN && self.code().as_deref() == Some("insufficient_scope")
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.action, self.status, self.body)
    }
}

impl std::error::Error for ApiError {}

/// The error to report for a failed command.
///
/// Decision: scope refusals are explained here, once, for every command,
/// rather than at each call site. A request path only has to fail with an
/// [`ApiError`] somewhere in its error chain; `main` passes every failure
/// through this function. Anything else is reported unchanged.
pub fn explain(error: anyhow::Error) -> anyhow::Error {
    let refused = error.chain().any(|cause| {
        cause
            .downcast_ref::<ApiError>()
            .is_some_and(ApiError::is_insufficient_scope)
    });
    if refused {
        anyhow::anyhow!(INSUFFICIENT_SCOPE_MESSAGE)
    } else {
        error
    }
}

/// Tries a contract command gets before its failure is reported.
const COMMAND_ATTEMPTS: u32 = 4;
/// Wait before the first retry; doubled for each later one.
const COMMAND_RETRY_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_millis(500)
};

/// REST request header carrying a change reason; the server records it in the
/// changed entity's history. The value is UTF-8 percent-encoded, which is how
/// the server decodes it (`change_history::intent`).
pub const CHANGE_REASON_HEADER: &str = "Everruns-Change-Reason";

/// `request` with the change reason header, when there is a reason.
pub fn with_change_reason(
    request: reqwest::RequestBuilder,
    reason: Option<&str>,
) -> reqwest::RequestBuilder {
    match reason {
        Some(reason) => request.header(CHANGE_REASON_HEADER, urlencoding::encode(reason).as_ref()),
        None => request,
    }
}

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
        self.send(Method::GET, path, None, None).await
    }

    pub async fn post(&self, path: &str, body: Option<&Value>) -> Result<Value> {
        self.send(Method::POST, path, body, None).await
    }

    /// POST to a REST route, recording `reason` (when given) in the changed
    /// entity's history. Contract commands carry it in the envelope instead.
    pub async fn post_with_reason(
        &self,
        path: &str,
        body: Option<&Value>,
        reason: Option<&str>,
    ) -> Result<Value> {
        self.send(Method::POST, path, body, reason).await
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
                    let failure = ApiError::from_response("API request failed", response).await;
                    if !retryable_status(failure.status, &failure.body) {
                        return Err(failure.into());
                    }
                    failure.into()
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

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        reason: Option<&str>,
    ) -> Result<Value> {
        let mut request = with_change_reason(self.request(method.clone(), path), reason);
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request
            .send()
            .await
            .with_context(|| format!("Failed to call {method} {path}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::from_response("API request failed", response)
                .await
                .into());
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

/// A one-shot HTTP server for tests that need to see what the CLI sent.
#[cfg(test)]
pub(crate) mod capture {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// One request as received: header lines (names lowercased) and the body.
    pub struct Captured {
        pub headers: Vec<String>,
        pub body: String,
    }

    impl Captured {
        pub fn header(&self, name: &str) -> Option<String> {
            let prefix = format!("{}: ", name.to_lowercase());
            self.headers
                .iter()
                .find_map(|line| line.strip_prefix(&prefix))
                .map(|value| value.trim().to_string())
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_str(&self.body).expect("a JSON body")
        }
    }

    /// Answer one request with `status` and `body`, reporting what was sent.
    pub async fn serve_once(
        status: u16,
        body: &'static str,
    ) -> (String, tokio::sync::oneshot::Receiver<Captured>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, received) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let mut chunk = vec![0u8; 8192];
            let (head_len, content_length) = loop {
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "connection closed before the request ended");
                buffer.extend_from_slice(&chunk[..read]);
                let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                break (end + 4, length);
            };
            while buffer.len() < head_len + content_length {
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "connection closed before the body ended");
                buffer.extend_from_slice(&chunk[..read]);
            }
            // Names lowercased for lookup; values kept as sent.
            let headers = String::from_utf8_lossy(&buffer[..head_len])
                .lines()
                .map(|line| match line.split_once(':') {
                    Some((name, value)) => format!("{}:{value}", name.to_lowercase()),
                    None => line.to_owned(),
                })
                .collect();
            let request_body =
                String::from_utf8_lossy(&buffer[head_len..head_len + content_length]).into_owned();
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            sender
                .send(Captured {
                    headers,
                    body: request_body,
                })
                .ok();
        });
        (url, received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn a_reason_is_sent_percent_encoded_in_the_header() {
        let (url, captured) = capture::serve_once(200, "{}").await;
        let client = ApiClient::new(&url, "key", None);
        client
            .post_with_reason("/v1/agents/import", None, Some("kid friendly: ok"))
            .await
            .unwrap();
        let request = captured.await.unwrap();
        assert_eq!(
            request.header(CHANGE_REASON_HEADER).as_deref(),
            Some("kid%20friendly%3A%20ok")
        );
    }

    #[tokio::test]
    async fn no_reason_sends_no_header() {
        let (url, captured) = capture::serve_once(200, "{}").await;
        let client = ApiClient::new(&url, "key", None);
        client
            .post_with_reason("/v1/agents/import", None, None)
            .await
            .unwrap();
        assert_eq!(captured.await.unwrap().header(CHANGE_REASON_HEADER), None);
    }

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

    const REFUSAL: &str = r#"{"status":403,"detail":"read-only","code":"insufficient_scope"}"#;

    #[tokio::test]
    async fn a_scope_refusal_is_explained_in_one_line() {
        let (url, _keys) = serve(vec![Some((403, REFUSAL))]).await;
        let client = ApiClient::new(&url, "evr_pat_read", None);
        let err = client
            .post_command("/v1/commands/create_agent", &serde_json::json!({}), "k-4")
            .await
            .unwrap_err();
        assert_eq!(explain(err).to_string(), INSUFFICIENT_SCOPE_MESSAGE);
    }

    #[tokio::test]
    async fn a_scope_refusal_on_a_rest_route_is_explained_through_context() {
        let (url, _captured) = capture::serve_once(403, REFUSAL).await;
        let client = ApiClient::new(&url, "evr_pat_read", None);
        let err = client
            .post("/v1/agents/import", None)
            .await
            .context("Failed to import")
            .unwrap_err();
        // `{:#}` prints the whole chain: the context is replaced, not prefixed.
        assert_eq!(format!("{:#}", explain(err)), INSUFFICIENT_SCOPE_MESSAGE);
    }

    #[test]
    fn other_failures_are_reported_unchanged() {
        let forbidden = ApiError {
            action: "API request failed".into(),
            status: StatusCode::FORBIDDEN,
            body: r#"{"code":"forbidden"}"#.into(),
        };
        assert!(!forbidden.is_insufficient_scope());
        assert_eq!(
            explain(forbidden.into()).to_string(),
            r#"API request failed (403 Forbidden): {"code":"forbidden"}"#
        );

        // The code alone is not enough: the refusal is a 403.
        let not_forbidden = ApiError {
            action: "API request failed".into(),
            status: StatusCode::BAD_REQUEST,
            body: REFUSAL.into(),
        };
        assert!(!not_forbidden.is_insufficient_scope());
        assert!(
            explain(anyhow::anyhow!("plain failure"))
                .to_string()
                .contains("plain failure")
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
