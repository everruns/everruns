//! Persistence layer for durable execution
//!
//! This module provides:
//! - [`WorkflowEventStore`] trait for workflow and event persistence
//! - [`InMemoryWorkflowEventStore`] for testing
//! - [`PostgresWorkflowEventStore`] for production

mod memory;
mod postgres;
mod store;

pub use memory::InMemoryWorkflowEventStore;
pub use postgres::PostgresWorkflowEventStore;
pub use store::{
    ClaimedTask, DlqEntry, DlqFilter, HeartbeatResponse, Pagination, StoreError, TaskDefinition,
    TaskFailureOutcome, TaskStatus, TraceContext, WorkerFilter, WorkerInfo, WorkflowEventStore,
    WorkflowInfo, WorkflowStatus,
};
