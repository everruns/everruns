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
//! # Example
//!
//! ```ignore
//! use everruns_durable::worker::{WorkerPool, WorkerPoolConfig};
//!
//! // Configure the worker pool
//! let config = WorkerPoolConfig::new(vec!["process_order".to_string()])
//!     .with_worker_id("order-worker-1")
//!     .with_max_concurrency(20);
//!
//! // Create and start the pool
//! let pool = WorkerPool::new(store, config);
//!
//! pool.register_handler("process_order", |task| async move {
//!     let order: Order = serde_json::from_value(task.input)?;
//!     // Process the order...
//!     Ok(json!({"status": "completed"}))
//! });
//!
//! pool.start().await?;
//!
//! // Graceful shutdown
//! pool.shutdown().await?;
//! ```

mod backpressure;
mod poller;
mod pool;

pub use backpressure::{BackpressureConfig, BackpressureError, BackpressureState};
pub use poller::{AdaptivePoller, PollerConfig, PollerError, TaskPoller};
pub use pool::{WorkerPool, WorkerPoolConfig, WorkerPoolError, WorkerPoolStatus};
