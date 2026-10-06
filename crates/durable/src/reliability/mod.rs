//! Reliability patterns for durable execution
//!
//! This module provides:
//! - [`RetryPolicy`] - Configurable retry with exponential backoff
//! - [`CircuitBreakerConfig`] - Circuit breaker configuration
//! - [`DistributedCircuitBreaker`] - Distributed circuit breaker using PostgreSQL
//! - `TimeoutManager` - Activity timeout handling (experimental `workflows`
//!   feature)

mod circuit_breaker;
mod distributed_circuit_breaker;
mod retry;
#[cfg(feature = "workflows")]
mod timeout;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitState};
pub use distributed_circuit_breaker::{
    CircuitBreakerError, CircuitBreakerPermit, DistributedCircuitBreaker,
};
pub use retry::RetryPolicy;
#[cfg(feature = "workflows")]
pub use timeout::{TaskTimingInfo, TimeoutConfig, TimeoutError, TimeoutManager, TimeoutType};
