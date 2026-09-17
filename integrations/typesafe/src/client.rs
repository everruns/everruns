//! HTTP client for the TypeSafe System One endpoint.

use std::time::Duration;

use serde_json::Value;
use tracing::debug;

use crate::{Error, Evaluation, Judgment, Result};

/// Default API root. Override for a proxy or a test double.
pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
/// Environment variable read by [`TypeSafeClient::from_env`].
pub const API_KEY_ENV: &str = "TYPESAFE_API_KEY";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);
/// Upper bound on a honored `Retry-After`, so a hostile or confused header
/// cannot park a request past the caller's own deadline.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
/// Upper bound on the upstream message copied into [`Error::Api`].
const MAX_ERROR_MESSAGE: usize = 500;

/// How a client retries failures that could still succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts, including the first. 1 disables retrying.
    pub max_attempts: u32,
    /// Delay before the second attempt; doubles for each attempt after it.
    pub backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            backoff: DEFAULT_BACKOFF,
        }
    }
}

impl RetryPolicy {
    /// Never retry.
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            backoff: Duration::ZERO,
        }
    }
}

/// A client for TypeSafe's System One API.
///
/// Cloning shares the underlying connection pool.
#[derive(Clone)]
pub struct TypeSafeClient {
    http: reqwest::Client,
    api_key: String,
    endpoint: String,
    retry: RetryPolicy,
}

impl std::fmt::Debug for TypeSafeClient {
    /// Never renders the API key: clients end up inside capability and agent
    /// debug output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeSafeClient")
            .field("endpoint", &self.endpoint)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl TypeSafeClient {
    /// A client with default endpoint, timeout, and retry policy.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::builder(api_key).build()
    }

    /// Read the key from [`API_KEY_ENV`] once, at startup.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var(API_KEY_ENV).map_err(|_| Error::MissingApiKey(API_KEY_ENV))?;
        if key.trim().is_empty() {
            return Err(Error::MissingApiKey(API_KEY_ENV));
        }
        Ok(Self::new(key))
    }

    /// Configure endpoint, timeout, retries, or a pre-built HTTP client.
    pub fn builder(api_key: impl Into<String>) -> TypeSafeClientBuilder {
        TypeSafeClientBuilder {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            retry: RetryPolicy::default(),
            http: None,
        }
    }

    /// Ask every question in `evaluation` in one round trip.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), everruns_integrations_typesafe::Error> {
    /// use everruns_integrations_typesafe::{Evaluation, Question, TypeSafeClient};
    ///
    /// let client = TypeSafeClient::from_env()?;
    /// let judgment = client
    ///     .evaluate(
    ///         Evaluation::new("Why did the chicken cross the road? To get to the other side.")
    ///             .ask("is_funny", Question::noul("Would a general audience laugh at this?")),
    ///     )
    ///     .await?;
    /// assert!(judgment.noul("is_funny")? <= 1.0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn evaluate(&self, evaluation: Evaluation) -> Result<Judgment> {
        evaluation.validate()?;
        let body = serde_json::to_value(&evaluation)
            .map_err(|e| Error::InvalidRequest(format!("request is not serializable: {e}")))?;

        let mut delay = self.retry.backoff;
        let mut attempt = 1;
        loop {
            let outcome = self.send_once(&body).await;
            let error = match outcome {
                Ok(judgment) => return Ok(judgment),
                Err(error) => error,
            };

            if attempt >= self.retry.max_attempts || !error.is_retryable() {
                return Err(error);
            }
            let wait = retry_after(&error).unwrap_or(delay);
            debug!(
                attempt,
                max_attempts = self.retry.max_attempts,
                wait_ms = wait.as_millis() as u64,
                "typesafe: retrying after {error}"
            );
            tokio::time::sleep(wait).await;
            delay = delay.saturating_mul(2);
            attempt += 1;
        }
    }

    async fn send_once(&self, body: &Value) -> Result<Judgment> {
        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| Error::Transport(sanitize_transport(&e)))?;

        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let text = response
            .text()
            .await
            .map_err(|e| Error::Transport(sanitize_transport(&e)))?;

        if !(200..300).contains(&status) {
            return Err(Error::Api {
                status,
                message: upstream_message(&text, retry_after),
            });
        }
        serde_json::from_str(&text).map_err(|e| Error::Decode(e.to_string()))
    }
}

/// Builder for [`TypeSafeClient`].
pub struct TypeSafeClientBuilder {
    api_key: String,
    base_url: String,
    timeout: Duration,
    retry: RetryPolicy,
    http: Option<reqwest::Client>,
}

impl TypeSafeClientBuilder {
    /// Point at a different API root, such as a mock server or a proxy.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Per-attempt request timeout. Ignored when [`Self::http_client`] is set.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Replace the retry policy.
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Supply an HTTP client, to share a pool or a custom TLS setup.
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Build the client.
    pub fn build(self) -> TypeSafeClient {
        let http = self.http.unwrap_or_else(|| {
            reqwest::Client::builder()
                .timeout(self.timeout)
                .build()
                .unwrap_or_default()
        });
        TypeSafeClient {
            http,
            api_key: self.api_key,
            endpoint: format!("{}/v1/systemone", self.base_url.trim_end_matches('/')),
            retry: self.retry,
        }
    }
}

/// Honor `Retry-After` when the API sent one, capped.
fn retry_after(error: &Error) -> Option<Duration> {
    let Error::Api { message, .. } = error else {
        return None;
    };
    message
        .as_deref()?
        .strip_prefix("retry after ")?
        .strip_suffix('s')?
        .parse::<u64>()
        .ok()
        .map(|seconds| Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

/// Pull a message out of an error body without ever surfacing the raw body:
/// upstream errors can echo request headers, and ours carries the API key.
fn upstream_message(body: &str, retry_after_seconds: Option<u64>) -> Option<String> {
    if let Some(seconds) = retry_after_seconds {
        return Some(format!("retry after {seconds}s"));
    }
    let parsed: Value = serde_json::from_str(body).ok()?;
    let message = ["message", "error", "detail"]
        .into_iter()
        .find_map(|key| parsed.get(key).and_then(Value::as_str))?;
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    let mut end = message.len().min(MAX_ERROR_MESSAGE);
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }
    Some(message[..end].to_string())
}

/// reqwest renders the full URL in transport errors; the endpoint is not a
/// secret, but the message is kept short and stable for logs.
fn sanitize_transport(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "request timed out".to_string();
    }
    if error.is_connect() {
        return "could not connect to the TypeSafe API".to_string();
    }
    if error.is_decode() {
        return "could not read the TypeSafe response".to_string();
    }
    "request failed".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_renders_the_api_key() {
        let client = TypeSafeClient::new("sentinel-typesafe-credential");
        assert!(!format!("{client:?}").contains("sentinel-typesafe-credential"));
    }

    #[test]
    fn endpoint_tolerates_a_trailing_slash_on_the_base_url() {
        let client = TypeSafeClient::builder("k")
            .base_url("http://host/")
            .build();
        assert_eq!(client.endpoint, "http://host/v1/systemone");
    }

    #[test]
    fn upstream_message_prefers_documented_fields_and_drops_raw_bodies() {
        assert_eq!(
            upstream_message(r#"{"message":"questions.0 is malformed"}"#, None).as_deref(),
            Some("questions.0 is malformed")
        );
        assert_eq!(
            upstream_message(r#"{"detail":"bad field"}"#, None).as_deref(),
            Some("bad field")
        );
        assert_eq!(upstream_message("Bearer sk-live-leaked", None), None);
        assert_eq!(upstream_message(r#"{"message":"  "}"#, None), None);
    }

    #[test]
    fn upstream_message_truncates_on_a_char_boundary() {
        let body = serde_json::json!({ "message": "α".repeat(400) }).to_string();
        let message = upstream_message(&body, None).expect("message");
        assert!(message.len() <= MAX_ERROR_MESSAGE);
        assert!(message.chars().all(|c| c == 'α'));
    }

    #[test]
    fn retry_after_header_wins_and_is_capped() {
        let error = Error::Api {
            status: 429,
            message: upstream_message("{}", Some(120)),
        };
        assert_eq!(retry_after(&error), Some(MAX_RETRY_AFTER));
        let error = Error::Api {
            status: 429,
            message: upstream_message("{}", Some(2)),
        };
        assert_eq!(retry_after(&error), Some(Duration::from_secs(2)));
    }

    #[test]
    fn only_transient_failures_are_retryable() {
        assert!(Error::Transport("boom".into()).is_retryable());
        for status in [429, 500, 529] {
            assert!(
                Error::Api {
                    status,
                    message: None
                }
                .is_retryable(),
                "{status} must be retryable"
            );
        }
        for status in [401, 422] {
            assert!(
                !Error::Api {
                    status,
                    message: None
                }
                .is_retryable(),
                "{status} must not be retryable"
            );
        }
    }
}
