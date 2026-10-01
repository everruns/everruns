//! Worker pool for task execution
//!
//! This module provides:
//! - [`WorkerPool`] - Main worker pool with concurrent task execution
//! - [`BackpressureConfig`] - Load-aware task acceptance configuration
//! - [`PollerConfig`] - Task polling with exponential backoff
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                       WorkerPool                             │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
//! │  │ TaskPoller  │  │  Heartbeat  │  │  Stale Reclaimer    │  │
//! │  │  (polling)  │  │   (5s)      │  │     (30s)           │  │
//! │  └──────┬──────┘  └─────────────┘  └─────────────────────┘  │
//! │         │                                                    │
//! │         ▼                                                    │
//! │  ┌─────────────────────────────────────────────────────┐    │
//! │  │              BackpressureState                       │    │
//! │  │  (high/low watermarks, load tracking)               │    │
//! │  └─────────────────────────────────────────────────────┘    │
//! │         │                                                    │
//! │         ▼                                                    │
//! │  ┌─────────────────────────────────────────────────────┐    │
//! │  │         Task Executor (Semaphore-limited)           │    │
//! │  │  [Task 1] [Task 2] [Task 3] ... [Task N]            │    │
//! │  └─────────────────────────────────────────────────────┘    │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! See [`WorkerPool`] for a runnable example.

mod backpressure;
mod poller;
mod pool;

pub use backpressure::{
    BackpressureConfig, BackpressureError, BackpressureState, ResourceMetrics, ResourceMonitor,
};
pub use poller::{AdaptivePoller, PollerConfig, PollerError, TaskPoller};
pub use pool::{WorkerPool, WorkerPoolConfig, WorkerPoolError, WorkerPoolStatus};
