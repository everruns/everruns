// LLM Provider Rate Limiter (EVE-7)
//
// Enforces per-provider concurrency limits on LLM API calls using Tokio semaphores.
// Prevents 429 bursts when many workers call the same provider simultaneously.
//
// Design: Wraps BoxedLlmDriver with a semaphore-guarded driver that acquires a
// permit before each chat_completion_stream call. The permit is held for the
// duration of the streaming response, released when the stream is dropped.
//
// Configuration via environment variables:
//   LLM_MAX_CONCURRENT_PER_PROVIDER=50  (default)
//
// Future: integrate DistributedCircuitBreaker for automatic backoff on sustained 429s.

use crate::error::Result;
use crate::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, LlmCallConfig, LlmDriver, LlmMessage, LlmResponse,
    LlmResponseStream, LlmStreamEvent, ProviderType,
};
use crate::openresponses_protocol::{CompactRequest, CompactResponse};
use async_trait::async_trait;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Configuration for LLM rate limiting.
#[derive(Debug, Clone)]
pub struct LlmRateLimitConfig {
    /// Maximum concurrent LLM calls per provider type
    pub max_concurrent_per_provider: usize,
}

impl Default for LlmRateLimitConfig {
    fn default() -> Self {
        Self {
            max_concurrent_per_provider: 50,
        }
    }
}

impl LlmRateLimitConfig {
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            max_concurrent_per_provider: std::env::var("LLM_MAX_CONCURRENT_PER_PROVIDER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.max_concurrent_per_provider),
        }
    }
}

/// Manages per-provider semaphores for concurrency limiting.
///
/// Shared across all workers in a process. Thread-safe via Arc<Semaphore>.
pub struct LlmRateLimiter {
    semaphores: HashMap<ProviderType, Arc<Semaphore>>,
    config: LlmRateLimitConfig,
}

impl LlmRateLimiter {
    pub fn new(config: LlmRateLimitConfig) -> Self {
        let mut semaphores = HashMap::new();
        let providers = [
            ProviderType::OpenAI,
            ProviderType::OpenAICompletions,
            ProviderType::Anthropic,
            ProviderType::Gemini,
        ];
        let provider_count = providers.len();
        for provider in providers {
            semaphores.insert(
                provider,
                Arc::new(Semaphore::new(config.max_concurrent_per_provider)),
            );
        }
        tracing::info!(
            max_concurrent = config.max_concurrent_per_provider,
            providers = provider_count,
            "LLM rate limiter initialized"
        );
        Self {
            semaphores,
            config,
        }
    }

    /// Wrap a driver with rate limiting for the given provider.
    pub fn wrap(
        &self,
        driver: BoxedLlmDriver,
        provider_type: ProviderType,
    ) -> BoxedLlmDriver {
        // LlmSim doesn't need rate limiting
        if provider_type == ProviderType::LlmSim {
            return driver;
        }
        let semaphore = self
            .semaphores
            .get(&provider_type)
            .cloned()
            .unwrap_or_else(|| Arc::new(Semaphore::new(self.config.max_concurrent_per_provider)));

        Box::new(RateLimitedDriver {
            inner: driver,
            semaphore,
            provider_type,
        })
    }

    /// Get current available permits for a provider (for metrics).
    pub fn available_permits(&self, provider_type: &ProviderType) -> Option<usize> {
        self.semaphores
            .get(provider_type)
            .map(|s| s.available_permits())
    }
}

/// LLM driver wrapper that acquires a semaphore permit before each call.
struct RateLimitedDriver {
    inner: BoxedLlmDriver,
    semaphore: Arc<Semaphore>,
    provider_type: ProviderType,
}

#[async_trait]
impl LlmDriver for RateLimitedDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| crate::error::AgentLoopError::llm("LLM rate limiter semaphore closed"))?;

        tracing::debug!(
            provider = %self.provider_type,
            available = self.semaphore.available_permits(),
            "LLM rate limit permit acquired"
        );

        let stream = self.inner.chat_completion_stream(messages, config).await?;

        // Wrap stream to hold permit alive for the duration of streaming
        Ok(Box::pin(PermitGuardedStream {
            inner: stream,
            _permit: permit,
        }))
    }

    async fn chat_completion(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        let _permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| crate::error::AgentLoopError::llm("LLM rate limiter semaphore closed"))?;

        self.inner.chat_completion(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        self.inner.list_models().await
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    async fn compact(&self, request: CompactRequest) -> Result<Option<CompactResponse>> {
        self.inner.compact(request).await
    }
}

/// Stream wrapper that holds a semaphore permit until the stream is dropped.
struct PermitGuardedStream {
    inner: LlmResponseStream,
    _permit: OwnedSemaphorePermit,
}

impl Stream for PermitGuardedStream {
    type Item = Result<LlmStreamEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_config_defaults() {
        let config = LlmRateLimitConfig::default();
        assert_eq!(config.max_concurrent_per_provider, 50);
    }

    #[test]
    fn rate_limiter_creates_semaphores_for_all_providers() {
        let limiter = LlmRateLimiter::new(LlmRateLimitConfig {
            max_concurrent_per_provider: 10,
        });
        assert_eq!(
            limiter.available_permits(&ProviderType::OpenAI),
            Some(10)
        );
        assert_eq!(
            limiter.available_permits(&ProviderType::Anthropic),
            Some(10)
        );
        assert_eq!(
            limiter.available_permits(&ProviderType::Gemini),
            Some(10)
        );
        assert_eq!(
            limiter.available_permits(&ProviderType::OpenAICompletions),
            Some(10)
        );
        // LlmSim not tracked
        assert_eq!(limiter.available_permits(&ProviderType::LlmSim), None);
    }
}
