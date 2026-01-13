// DEV_MODE in-process worker
//
// Decision: Run worker in-process with control-plane for DEV_MODE
// This module provides direct adapters and an in-process worker that
// eliminates the need for a separate worker process and gRPC communication.
//
// Usage: In DEV_MODE, spawn the InProcessWorker as a background task alongside
// the HTTP server. The worker uses the same InMemoryWorkflowEventStore as the
// runner, enabling full workflow execution without external dependencies.

pub mod direct_adapters;
pub mod worker;

pub use worker::{InProcessWorker, InProcessWorkerConfig};
