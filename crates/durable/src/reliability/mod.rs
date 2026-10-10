//! Reliability patterns for durable execution
//!
//! This module provides:
//! - [`RetryPolicy`] - Configurable retry with exponential backoff
//! - [`CircuitBreakerConfig`] - Circuit breaker configuration
//! - [`DistributedCircuitBreaker`] - Distributed circuit breaker using PostgreSQL

mod circuit_breaker;
mod distributed_circuit_breaker;
mod retry;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitState};
pub use distributed_circuit_breaker::{
    CircuitBreakerError, CircuitBreakerPermit, DistributedCircuitBreaker,
};
pub use retry::RetryPolicy;
