use axum::http::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Response from a test request
pub struct TestResponse {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

impl TestResponse {
    /// Get the status code
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get the response headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get the body as a string
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// Get the raw response body bytes
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// Parse the body as JSON
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("Failed to parse JSON: {}. Body: {}", e, self.text()))
    }

    /// Parse the body as a JSON Value
    pub fn json_value(&self) -> Value {
        self.json()
    }

    /// Assert status and return self for chaining
    pub fn assert_status(self, expected: StatusCode) -> Self {
        assert_eq!(
            self.status,
            expected,
            "Expected status {}, got {}. Body: {}",
            expected,
            self.status,
            self.text()
        );
        self
    }

    /// Assert success (2xx) status
    pub fn assert_success(self) -> Self {
        assert!(
            self.status.is_success(),
            "Expected success status, got {}. Body: {}",
            self.status,
            self.text()
        );
        self
    }
}
