//! Distributed circuit breaker implementation
//!
//! Circuit breaker state is shared across workers via PostgreSQL, enabling
//! coordinated failure handling in distributed systems.
//!
//! # Status: FUTURE FEATURE
//!
//! This module is fully implemented but not yet integrated into the workflow
//! execution pipeline. The database table (`durable_circuit_breaker_state`)
//! and store operations exist, but no code path currently instantiates
//! `DistributedCircuitBreaker`.
//!
//! TODO: Integrate circuit breakers for:
//! - LLM provider calls (protect against provider outages)
//! - External tool executions (prevent cascading failures)
//! - Rate limiting coordination across workers

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::sync::RwLock;

use super::{CircuitBreakerConfig, CircuitState};
use crate::persistence::{StoreError, WorkflowEventStore};

/// Error types for circuit breaker operations
#[derive(Debug, Error)]
pub enum CircuitBreakerError {
    /// Circuit is open, calls are not allowed
    #[error("circuit breaker is open")]
    Open,

    /// Circuit is in half-open state with no permits available
    #[error("circuit breaker half-open, no permits available")]
    HalfOpenExhausted,

    /// Store error
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// Cached circuit breaker state for local reads
#[derive(Debug, Clone)]
struct CachedState {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    opened_at: Option<DateTime<Utc>>,
    cached_at: DateTime<Utc>,
}

impl CachedState {
    fn is_stale(&self, max_age: Duration) -> bool {
        let age = Utc::now()
            .signed_duration_since(self.cached_at)
            .to_std()
            .unwrap_or(Duration::MAX);
        age > max_age
    }
}

/// Permit that must be held during a protected call
pub struct CircuitBreakerPermit<'a> {
    breaker: &'a DistributedCircuitBreaker,
}

impl<'a> CircuitBreakerPermit<'a> {
    fn new(breaker: &'a DistributedCircuitBreaker) -> Self {
        Self { breaker }
    }

    /// Report the call succeeded
    pub async fn success(self) -> Result<(), CircuitBreakerError> {
        self.breaker.record_success().await
    }

    /// Report the call failed
    pub async fn failure(self) -> Result<(), CircuitBreakerError> {
        self.breaker.record_failure().await
    }
}

/// Distributed circuit breaker that shares state via PostgreSQL
///
/// # Example
///
/// ```ignore
/// use everruns_durable::reliability::{DistributedCircuitBreaker, CircuitBreakerConfig};
///
/// let breaker = DistributedCircuitBreaker::new(
///     "external_service",
///     CircuitBreakerConfig::default(),
///     store,
/// );
///
/// // Try to make a call
/// match breaker.allow().await {
///     Ok(permit) => {
///         match make_external_call().await {
///             Ok(result) => permit.success().await?,
///             Err(e) => permit.failure().await?,
///         }
///     }
///     Err(CircuitBreakerError::Open) => {
///         // Circuit is open, fail fast
///         return Err("Service unavailable");
///     }
/// }
/// ```
pub struct DistributedCircuitBreaker {
    /// Unique key identifying this circuit breaker
    key: String,
    /// Circuit breaker configuration
    config: CircuitBreakerConfig,
    /// Store for persisting state
    store: Arc<dyn WorkflowEventStore>,
    /// Local cache to reduce database reads
    local_cache: RwLock<Option<CachedState>>,
    /// How long to cache state locally
    cache_duration: Duration,
}

impl DistributedCircuitBreaker {
    /// Create a new distributed circuit breaker
    pub fn new(
        key: impl Into<String>,
        config: CircuitBreakerConfig,
        store: Arc<dyn WorkflowEventStore>,
    ) -> Self {
        Self {
            key: key.into(),
            config,
            store,
            local_cache: RwLock::new(None),
            cache_duration: Duration::from_secs(1),
        }
    }

    /// Set the local cache duration
    pub fn with_cache_duration(mut self, duration: Duration) -> Self {
        self.cache_duration = duration;
        self
    }

    /// Get the circuit breaker key
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Get the circuit breaker configuration
    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }

    /// Check if a call should be allowed
    ///
    /// Returns a permit that must be used to report success/failure.
    pub async fn allow(&self) -> Result<CircuitBreakerPermit<'_>, CircuitBreakerError> {
        let state = self.get_state().await?;

        match state.state {
            CircuitState::Closed => Ok(CircuitBreakerPermit::new(self)),
            CircuitState::Open => {
                // Check if reset_timeout has passed
                if self.should_transition_to_half_open(&state) {
                    self.transition_to_half_open().await?;
                    Ok(CircuitBreakerPermit::new(self))
                } else {
                    Err(CircuitBreakerError::Open)
                }
            }
            CircuitState::HalfOpen => {
                // In half-open state, allow the call to test if service recovered
                Ok(CircuitBreakerPermit::new(self))
            }
        }
    }

    /// Check current state without acquiring a permit
    pub async fn state(&self) -> Result<CircuitState, CircuitBreakerError> {
        let state = self.get_state().await?;
        Ok(state.state)
    }

    /// Record a successful call
    async fn record_success(&self) -> Result<(), CircuitBreakerError> {
        let state = self.get_state().await?;

        match state.state {
            CircuitState::Closed => {
                // In closed state, success resets failure count
                // This is handled by the sliding window, no action needed
                Ok(())
            }
            CircuitState::HalfOpen => {
                // In half-open, count successes
                let new_success_count = state.success_count + 1;

                if new_success_count >= self.config.success_threshold {
                    // Enough successes, close the circuit
                    self.transition_to_closed().await?;
                } else {
                    // Record success but stay half-open
                    self.store
                        .update_circuit_breaker(
                            &self.key,
                            CircuitState::HalfOpen,
                            state.failure_count,
                            new_success_count,
                        )
                        .await?;
                }

                // Invalidate cache
                *self.local_cache.write().await = None;
                Ok(())
            }
            CircuitState::Open => {
                // Shouldn't happen - can't have success in open state
                Ok(())
            }
        }
    }

    /// Record a failed call
    async fn record_failure(&self) -> Result<(), CircuitBreakerError> {
        let state = self.get_state().await?;

        match state.state {
            CircuitState::Closed => {
                let new_failure_count = state.failure_count + 1;

                if new_failure_count >= self.config.failure_threshold {
                    // Threshold exceeded, open the circuit
                    self.transition_to_open().await?;
                } else {
                    // Record failure but stay closed
                    self.store
                        .update_circuit_breaker(
                            &self.key,
                            CircuitState::Closed,
                            new_failure_count,
                            0,
                        )
                        .await?;
                }

                // Invalidate cache
                *self.local_cache.write().await = None;
                Ok(())
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open state reopens the circuit
                self.transition_to_open().await?;
                *self.local_cache.write().await = None;
                Ok(())
            }
            CircuitState::Open => {
                // Shouldn't happen - can't have failure in open state
                Ok(())
            }
        }
    }

    /// Get current state, using cache if available
    async fn get_state(&self) -> Result<CachedState, CircuitBreakerError> {
        // Check local cache first
        {
            let cache = self.local_cache.read().await;
            if let Some(cached) = cache.as_ref() {
                if !cached.is_stale(self.cache_duration) {
                    return Ok(cached.clone());
                }
            }
        }

        // Cache miss or stale, fetch from database
        let db_state = self.store.get_circuit_breaker(&self.key).await?;

        let cached = match db_state {
            Some(state) => CachedState {
                state: state.state,
                failure_count: state.failure_count,
                success_count: state.success_count,
                opened_at: state.opened_at,
                cached_at: Utc::now(),
            },
            None => {
                // Initialize new circuit breaker
                self.store
                    .create_circuit_breaker(&self.key, &self.config)
                    .await?;
                CachedState {
                    state: CircuitState::Closed,
                    failure_count: 0,
                    success_count: 0,
                    opened_at: None,
                    cached_at: Utc::now(),
                }
            }
        };

        // Update cache
        *self.local_cache.write().await = Some(cached.clone());

        Ok(cached)
    }

    /// Check if circuit should transition from Open to HalfOpen
    fn should_transition_to_half_open(&self, state: &CachedState) -> bool {
        if let Some(opened_at) = state.opened_at {
            let elapsed = Utc::now()
                .signed_duration_since(opened_at)
                .to_std()
                .unwrap_or(Duration::ZERO);
            elapsed >= self.config.reset_timeout
        } else {
            false
        }
    }

    /// Transition to Open state
    async fn transition_to_open(&self) -> Result<(), CircuitBreakerError> {
        self.store
            .update_circuit_breaker(&self.key, CircuitState::Open, 0, 0)
            .await?;
        *self.local_cache.write().await = None;
        Ok(())
    }

    /// Transition to HalfOpen state
    async fn transition_to_half_open(&self) -> Result<(), CircuitBreakerError> {
        self.store
            .update_circuit_breaker(&self.key, CircuitState::HalfOpen, 0, 0)
            .await?;
        *self.local_cache.write().await = None;
        Ok(())
    }

    /// Transition to Closed state
    async fn transition_to_closed(&self) -> Result<(), CircuitBreakerError> {
        self.store
            .update_circuit_breaker(&self.key, CircuitState::Closed, 0, 0)
            .await?;
        *self.local_cache.write().await = None;
        Ok(())
    }

    /// Reset the circuit breaker (for testing or admin operations)
    pub async fn reset(&self) -> Result<(), CircuitBreakerError> {
        self.transition_to_closed().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::InMemoryWorkflowEventStore;

    async fn create_test_breaker() -> DistributedCircuitBreaker {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        DistributedCircuitBreaker::new(
            "test_service",
            CircuitBreakerConfig::default()
                .with_failure_threshold(3)
                .with_success_threshold(2)
                .with_reset_timeout(Duration::from_millis(100)),
            store,
        )
        .with_cache_duration(Duration::ZERO) // Disable caching for tests
    }

    #[tokio::test]
    async fn test_starts_closed() {
        let breaker = create_test_breaker().await;
        let state = breaker.state().await.unwrap();
        assert_eq!(state, CircuitState::Closed);
    }

    #[tokio::test]
    async fn test_allows_calls_when_closed() {
        let breaker = create_test_breaker().await;
        let permit = breaker.allow().await.unwrap();
        permit.success().await.unwrap();
    }

    #[tokio::test]
    async fn test_opens_after_failure_threshold() {
        let breaker = create_test_breaker().await;

        // Record failures up to threshold
        for _ in 0..3 {
            let permit = breaker.allow().await.unwrap();
            permit.failure().await.unwrap();
        }

        // Circuit should be open now
        let result = breaker.allow().await;
        assert!(matches!(result, Err(CircuitBreakerError::Open)));
    }

    #[tokio::test]
    async fn test_transitions_to_half_open_after_timeout() {
        let breaker = create_test_breaker().await;

        // Open the circuit
        for _ in 0..3 {
            let permit = breaker.allow().await.unwrap();
            permit.failure().await.unwrap();
        }

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should allow a test call (half-open)
        let permit = breaker.allow().await.unwrap();
        let state = breaker.state().await.unwrap();
        assert_eq!(state, CircuitState::HalfOpen);

        permit.success().await.unwrap();
    }

    #[tokio::test]
    async fn test_closes_after_success_threshold_in_half_open() {
        let breaker = create_test_breaker().await;

        // Open the circuit
        for _ in 0..3 {
            let permit = breaker.allow().await.unwrap();
            permit.failure().await.unwrap();
        }

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Two successes should close the circuit
        for _ in 0..2 {
            let permit = breaker.allow().await.unwrap();
            permit.success().await.unwrap();
        }

        let state = breaker.state().await.unwrap();
        assert_eq!(state, CircuitState::Closed);
    }

    #[tokio::test]
    async fn test_reopens_on_failure_in_half_open() {
        let breaker = create_test_breaker().await;

        // Open the circuit
        for _ in 0..3 {
            let permit = breaker.allow().await.unwrap();
            permit.failure().await.unwrap();
        }

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // One failure in half-open should reopen
        let permit = breaker.allow().await.unwrap();
        permit.failure().await.unwrap();

        let result = breaker.allow().await;
        assert!(matches!(result, Err(CircuitBreakerError::Open)));
    }

    #[tokio::test]
    async fn test_reset() {
        let breaker = create_test_breaker().await;

        // Open the circuit
        for _ in 0..3 {
            let permit = breaker.allow().await.unwrap();
            permit.failure().await.unwrap();
        }

        // Reset should close it
        breaker.reset().await.unwrap();
        let state = breaker.state().await.unwrap();
        assert_eq!(state, CircuitState::Closed);
    }
}
