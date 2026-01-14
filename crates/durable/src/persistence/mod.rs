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
    CircuitBreakerState, ClaimedTask, DlqEntry, DlqFilter, HeartbeatResponse, Pagination,
    StoreError, SystemHealth, TaskDefinition, TaskFailureOutcome, TaskFilter, TaskInfo, TaskStatus,
    TraceContext, WorkerFilter, WorkerInfo, WorkflowEventInfo, WorkflowEventStore, WorkflowFilter,
    WorkflowInfo, WorkflowInfoExtended, WorkflowStatus, event_type_name,
};
