// Observability Configuration
//
// Configuration for observability backends, loaded from environment variables.

use std::env;

/// Configuration for observability integrations
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Whether observability is enabled globally
    pub enabled: bool,

    /// Langfuse-specific configuration
    pub langfuse: Option<LangfuseConfig>,
}

impl ObservabilityConfig {
    /// Create configuration from environment variables
    ///
    /// Environment variables:
    /// - `OBSERVABILITY_ENABLED`: Enable/disable observability (default: true if any backend configured)
    /// - `LANGFUSE_PUBLIC_KEY`: Langfuse public key (pk-lf-...)
    /// - `LANGFUSE_SECRET_KEY`: Langfuse secret key (sk-lf-...)
    /// - `LANGFUSE_HOST`: Langfuse host (default: https://cloud.langfuse.com)
    /// - `LANGFUSE_RELEASE`: Application release/version tag
    pub fn from_env() -> Self {
        let langfuse = LangfuseConfig::from_env();

        // Default enabled if any backend is configured
        let default_enabled = langfuse.is_some();
        let enabled = env::var("OBSERVABILITY_ENABLED")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(default_enabled);

        Self { enabled, langfuse }
    }

    /// Check if any observability backend is configured and enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled && (self.langfuse.is_some())
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Langfuse-specific configuration
#[derive(Debug, Clone)]
pub struct LangfuseConfig {
    /// Langfuse public key (pk-lf-...)
    pub public_key: String,

    /// Langfuse secret key (sk-lf-...)
    pub secret_key: String,

    /// Langfuse host (e.g., https://cloud.langfuse.com)
    pub host: String,

    /// Application release/version tag
    pub release: Option<String>,

    /// Batch flush interval in milliseconds
    pub flush_interval_ms: u64,

    /// Maximum batch size before forced flush
    pub max_batch_size: usize,
}

impl LangfuseConfig {
    /// Create configuration from environment variables
    ///
    /// Returns None if required variables are not set.
    pub fn from_env() -> Option<Self> {
        let public_key = env::var("LANGFUSE_PUBLIC_KEY").ok()?;
        let secret_key = env::var("LANGFUSE_SECRET_KEY").ok()?;

        // Must have both keys
        if public_key.is_empty() || secret_key.is_empty() {
            return None;
        }

        let host =
            env::var("LANGFUSE_HOST").unwrap_or_else(|_| "https://cloud.langfuse.com".to_string());

        let release = env::var("LANGFUSE_RELEASE").ok();

        let flush_interval_ms = env::var("LANGFUSE_FLUSH_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5000);

        let max_batch_size = env::var("LANGFUSE_MAX_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        Some(Self {
            public_key,
            secret_key,
            host,
            release,
            flush_interval_ms,
            max_batch_size,
        })
    }

    /// Get the OTLP endpoint for this configuration
    pub fn otlp_endpoint(&self) -> String {
        format!("{}/api/public/otel", self.host.trim_end_matches('/'))
    }

    /// Generate the Basic Auth header value
    pub fn auth_header(&self) -> String {
        use base64::Engine;
        let credentials = format!("{}:{}", self.public_key, self.secret_key);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        format!("Basic {}", encoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_langfuse_config_auth_header() {
        let config = LangfuseConfig {
            public_key: "pk-lf-test".to_string(),
            secret_key: "sk-lf-secret".to_string(),
            host: "https://cloud.langfuse.com".to_string(),
            release: None,
            flush_interval_ms: 5000,
            max_batch_size: 100,
        };

        let header = config.auth_header();
        assert!(header.starts_with("Basic "));

        // Decode and verify
        use base64::Engine;
        let encoded = header.strip_prefix("Basic ").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let decoded_str = String::from_utf8(decoded).unwrap();
        assert_eq!(decoded_str, "pk-lf-test:sk-lf-secret");
    }

    #[test]
    fn test_langfuse_config_otlp_endpoint() {
        let config = LangfuseConfig {
            public_key: "pk".to_string(),
            secret_key: "sk".to_string(),
            host: "https://cloud.langfuse.com".to_string(),
            release: None,
            flush_interval_ms: 5000,
            max_batch_size: 100,
        };

        assert_eq!(
            config.otlp_endpoint(),
            "https://cloud.langfuse.com/api/public/otel"
        );

        // Test with trailing slash
        let config2 = LangfuseConfig {
            host: "https://cloud.langfuse.com/".to_string(),
            ..config
        };
        assert_eq!(
            config2.otlp_endpoint(),
            "https://cloud.langfuse.com/api/public/otel"
        );
    }
}
