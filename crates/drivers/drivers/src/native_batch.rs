//! Transport for explicit application-owned jobs. Never replay a mutation.
use std::{collections::HashSet, io::Write, time::Duration};

use everruns_contracts::{ProviderEndpoint, batch::*};
use reqwest::Method;
#[cfg(any(
    feature = "openai",
    feature = "groq",
    feature = "mistral",
    feature = "gemini"
))]
use serde_json::Value;

pub(super) struct BoundedBody {
    pub bytes: Vec<u8>,
    pub max_bytes: usize,
}
impl Write for BoundedBody {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.max_bytes.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("batch exceeds request byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn valid_segment(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        && id != "."
        && id != ".."
}
pub(super) fn validate(requests: &[BatchRequest], max: usize) -> Result<(), BatchError> {
    if requests.is_empty() || requests.len() > max {
        return Err(BatchError::InvalidRequest(
            "batch request count exceeds provider limit",
        ));
    }
    let mut ids = HashSet::new();
    for item in requests {
        if item.id.is_empty()
            || item.id.len() > 64
            || !item
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            || !ids.insert(&item.id)
        {
            return Err(BatchError::InvalidRequest(
                "invalid or duplicate request ID",
            ));
        }
        let c = &item.config;
        if item.messages.is_empty() || c.max_tokens == Some(0) {
            return Err(BatchError::InvalidRequest(
                "messages and a positive token budget are required",
            ));
        }
        if !c.limits.is_unbounded()
            || c.capture_request
            || !c.extra_headers.is_empty()
            || !c.driver_options.is_empty()
            || c.provider_opaque_context.is_some()
            || c.previous_response_id.is_some()
            || c.reasoning_state.is_some()
            || c.tool_baseline.is_some()
            || c.system_prompt_baseline.is_some()
            || c.tool_search.as_ref().is_some_and(|s| s.enabled)
        {
            return Err(BatchError::InvalidRequest(
                "batch does not support streaming limits, capture, per-item headers or provider execution options",
            ));
        }
    }
    Ok(())
}
#[derive(Clone, Copy)]
pub(super) struct Http<'a> {
    pub endpoint: &'a ProviderEndpoint,
    pub timeout: Duration,
    pub max_bytes: usize,
}
impl Http<'_> {
    #[cfg(any(
        feature = "openai",
        feature = "groq",
        feature = "mistral",
        feature = "gemini"
    ))]
    pub async fn request(
        self,
        method: Method,
        path: &str,
        body: Vec<u8>,
        content_type: &str,
        creating: bool,
    ) -> Result<Vec<u8>, BatchError> {
        let url = self
            .endpoint
            .url(path)
            .ok_or(BatchError::InvalidRequest("provider has no base URL"))?;
        self.request_url(method, url, body, content_type, creating)
            .await
    }
    /// A control-plane path on the configured origin; never a provider-returned URL.
    pub async fn request_url(
        self,
        method: Method,
        url: String,
        body: Vec<u8>,
        content_type: &str,
        creating: bool,
    ) -> Result<Vec<u8>, BatchError> {
        let base = reqwest::Url::parse(self.endpoint.base_url().unwrap_or_default())
            .map_err(|_| BatchError::InvalidRequest("invalid provider URL"))?;
        let target = reqwest::Url::parse(&url)
            .map_err(|_| BatchError::InvalidRequest("invalid control URL"))?;
        if base.origin() != target.origin() {
            return Err(BatchError::InvalidRequest(
                "control URL must use configured origin",
            ));
        }
        let failure = || {
            if creating {
                BatchError::SubmissionUncertain
            } else {
                BatchError::Transport
            }
        };
        tokio::time::timeout(self.timeout, async {
            let resolved = self
                .endpoint
                .resolve(method.as_str(), url, &body)
                .await
                .map_err(|_| BatchError::InvalidRequest("provider authentication failed"))?;
            // THREAT[TM-LLM-056]: Credentials stay on configured origins. Provider
            // output URLs, redirects and raw error bodies are never followed/exposed.
            let client = everruns_contracts::driver_helpers::shared_request_http_client();
            let mut request = client
                .request(method, resolved.url)
                .timeout(self.timeout)
                .header("content-type", content_type);
            for (name, value) in resolved.headers {
                request = request.header(name, value);
            }
            let mut response = request.body(body).send().await.map_err(|_| failure())?;
            let code = response.status().as_u16();
            if !(200..300).contains(&code) {
                return Err(if creating && (code == 408 || code >= 500) {
                    BatchError::SubmissionUncertain
                } else {
                    BatchError::Http { status: code }
                });
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
                if chunk.len() > self.max_bytes.saturating_sub(bytes.len()) {
                    return Err(if creating {
                        BatchError::SubmissionUncertain
                    } else {
                        BatchError::ResponseTooLarge
                    });
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| failure())?
    }
    #[cfg(any(
        feature = "openai",
        feature = "groq",
        feature = "mistral",
        feature = "gemini"
    ))]
    pub async fn json(
        self,
        method: Method,
        path: &str,
        body: Vec<u8>,
        creating: bool,
    ) -> Result<Value, BatchError> {
        let bytes = self
            .request(method, path, body, "application/json", creating)
            .await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            if creating {
                BatchError::SubmissionUncertain
            } else {
                BatchError::Protocol("invalid job JSON")
            }
        })
    }
}
pub(super) fn http_error(code: u64) -> BatchItemError {
    match code {
        400 | 404 | 422 => BatchItemError::InvalidRequest,
        401 => BatchItemError::Authentication,
        403 => BatchItemError::Permission,
        429 => BatchItemError::RateLimited,
        500..=599 => BatchItemError::Server,
        _ => BatchItemError::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_serialization_rejects_growth_before_allocating_the_overflow() {
        let mut body = BoundedBody {
            bytes: Vec::new(),
            max_bytes: 8,
        };
        body.write_all(b"12345678").unwrap();
        assert!(body.write_all(b"overflow").is_err());
        assert_eq!(body.bytes, b"12345678");
    }
    #[test]
    fn resource_segments_cannot_change_the_credential_destination() {
        for id in [
            "",
            ".",
            "..",
            "../files",
            "job?key=private",
            "https://evil.invalid",
            "job#fragment",
            "a%2fb",
            "a\\b",
        ] {
            assert!(!valid_segment(id), "{id}");
        }
        assert!(valid_segment("file_safe-123.456"));
    }
}
