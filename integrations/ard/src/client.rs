//! HTTP client for ARD registries.
//!
//! Every outbound request target is SSRF-validated. The registry base URL comes
//! from operator config (never the model), and any resolved resource URL is
//! validated again before use in `tools.rs`.

use everruns_core::url_validation::{UrlValidationError, validate_safe_url};
use serde_json::{Value, json};

use crate::config::{ResolvedEntry, SearchResponse};

/// Errors from talking to an ARD registry.
#[derive(Debug)]
pub enum ClientError {
    /// The registry/resolve URL failed SSRF validation.
    UnsafeUrl(UrlValidationError),
    /// Transport-level failure.
    Transport(String),
    /// The registry returned a non-success status.
    Status(u16),
    /// Response body could not be parsed.
    Decode(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafeUrl(e) => write!(f, "Unsafe registry URL: {e}"),
            Self::Transport(e) => write!(f, "Registry request failed: {e}"),
            Self::Status(s) => write!(f, "Registry returned HTTP {s}"),
            Self::Decode(e) => write!(f, "Could not parse registry response: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Thin client bound to a single registry base URL.
pub struct RegistryClient {
    base_url: String,
    http: reqwest::Client,
    /// When true, skip local-URL blocking on the registry base (test/dev only).
    allow_local_urls: bool,
}

impl RegistryClient {
    /// Construct a client for a registry base URL.
    pub fn new(base_url: impl Into<String>, allow_local_urls: bool) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::new(),
            allow_local_urls,
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// Validate a URL against SSRF rules, honoring `allow_local_urls`.
    fn check_url(&self, url: &str) -> Result<(), ClientError> {
        match validate_safe_url(url) {
            Ok(_) => Ok(()),
            // `allow_local_urls` bypasses ONLY local-address blocking, never
            // scheme/parse errors.
            Err(UrlValidationError::BlockedHost(_)) if self.allow_local_urls => Ok(()),
            Err(e) => Err(ClientError::UnsafeUrl(e)),
        }
    }

    /// `POST /search` — discover ranked resources for a query.
    pub async fn search(&self, body: Value) -> Result<SearchResponse, ClientError> {
        let url = self.endpoint("search");
        self.check_url(&url)?;
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        resp.json::<SearchResponse>()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))
    }

    /// `POST /resolve` — resolve a single URN to its envelope.
    pub async fn resolve(&self, urn: &str) -> Result<ResolvedEntry, ClientError> {
        let url = self.endpoint("resolve");
        self.check_url(&url)?;
        let resp = self
            .http
            .post(&url)
            .json(&json!({ "urn": urn }))
            .send()
            .await
            .map_err(|e| ClientError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ClientError::Status(resp.status().as_u16()));
        }
        resp.json::<ResolvedEntry>()
            .await
            .map_err(|e| ClientError::Decode(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_joins_paths() {
        let c = RegistryClient::new("https://ard.example.com/", false);
        assert_eq!(c.endpoint("search"), "https://ard.example.com/search");
        assert_eq!(c.endpoint("/resolve"), "https://ard.example.com/resolve");
    }

    #[test]
    fn check_url_blocks_local_by_default() {
        let c = RegistryClient::new("http://127.0.0.1:9000", false);
        assert!(c.check_url("http://127.0.0.1:9000/search").is_err());
    }

    #[test]
    fn check_url_allows_local_when_opted_in() {
        let c = RegistryClient::new("http://127.0.0.1:9000", true);
        assert!(c.check_url("http://127.0.0.1:9000/search").is_ok());
    }

    #[test]
    fn check_url_never_bypasses_bad_scheme() {
        let c = RegistryClient::new("ftp://evil.example", true);
        assert!(c.check_url("ftp://evil.example/search").is_err());
    }
}
