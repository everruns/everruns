//! Reliability patterns for durable execution
//!
//! This module provides:
//! - [`RetryPolicy`] - Configurable retry with exponential backoff
//! - [`CircuitBreakerConfig`] - Circuit breaker configuration
//! - [`DistributedCircuitBreaker`] - Distributed circuit breaker using PostgreSQL
//! - [`TimeoutManager`] - Activity timeout handling

mod circuit_breaker;
mod distributed_circuit_breaker;
mod retry;
mod timeout;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitState};
pub use distributed_circuit_breaker::{
    CircuitBreakerError, CircuitBreakerPermit, DistributedCircuitBreaker,
};
pub use retry::RetryPolicy;
pub use timeout::{TimeoutConfig, TimeoutError, TimeoutManager};
