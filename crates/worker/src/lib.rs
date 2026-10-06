#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// The process composes runtime services only through its entry crate,
// `everruns-durable-engine`.
pub use everruns_durable_engine::{core, durable, engine, host};
// Worker-only core surface. The worker's own `everruns-core` dependency selects
// these features; durable-engine does not need them.
pub use everruns_durable_engine::core::mcp;
pub mod activities;
pub mod adapters;
pub mod app_builder;
mod catalog_cli;
pub use everruns_durable_engine::durable_runner;
pub use everruns_durable_engine::durable_turn;
pub mod grpc_adapters;
pub mod grpc_command_transport;
pub mod grpc_durable_runner;
pub mod grpc_durable_store;
pub mod grpc_files_adapter;
mod grpc_sandbox_persistence;
pub mod grpc_slack_actions;
pub mod grpc_sqldb_adapter;
mod grpc_task_store;
pub mod grpc_worker_adapters;
pub mod leased_resource_cleanup;
pub mod mcp_elicitation_consent;
pub mod mcp_executor;
pub mod phase_reads;
pub mod platform;
pub mod turn_reads;
pub use everruns_durable_engine::runner;
pub mod runtime_host;
pub mod session_lifecycle;
pub mod session_task_reaper;
mod stream_heartbeater;
mod system_decisions;
pub use everruns_durable_engine::task_error;
pub use everruns_durable_engine::task_heartbeat;
pub use everruns_durable_engine::turn_store;
pub mod task_wakeup;
pub use everruns_durable_engine::turn_driver;
pub mod turn_host;
pub mod unified_worker;
#[cfg(test)]
mod unified_worker_test_adapters;
#[cfg(test)]
mod unified_worker_wake_tests;
pub mod worker_adapters;
pub mod write_behind;

// Re-export main types
pub use durable_runner::{DurableRunner, DurableTaskNotifier, DurableTurnInput, DurableTurnOutput};
pub use grpc_durable_runner::{
    connect_grpc_durable_runner, create_runner, grpc_durable_runner_from_env,
};
pub use grpc_durable_store::{
    GrpcDurableStore, HeartbeatResponse as GrpcHeartbeatResponse,
    WorkflowStatus as GrpcWorkflowStatus,
};
pub use runner::{AgentRunner, RunnerBackend, create_runner_with_backend};

// Re-export LLM driver factory helpers
pub use adapters::{create_chat_driver, create_driver_registry};
pub use platform::{default_host_composition, default_host_composition_for_grade};
pub use system_decisions::{
    DECISIONS_DRIVER_ENV, DECISIONS_MODEL_ENV, DECISIONS_OPENAI_PREVIEW_ENV, SystemDecisions,
};

// Re-export gRPC adapters for worker communication with control plane
pub use grpc_adapters::{
    GrpcAdapter, GrpcBudgetChecker, GrpcClient, GrpcOrgAdapter, TurnContext, load_turn_context,
};

// Re-export task worker types
pub use grpc_worker_adapters::GrpcWorkerAdapters;
pub use runtime_host::WorkerRuntimeHost;
pub use stream_heartbeater::GrpcTaskHeartbeater;
pub use unified_worker::{ShutdownHandle, TaskWorker, TaskWorkerConfig};
pub use worker_adapters::{
    OrgAdapter, SessionAdapter, TurnContext as WorkerTurnContext, WorkerAdapters,
};

// Re-export OpenAI driver from the openai crate
pub use everruns_drivers::openai::OpenAIChatDriver;

// Re-export app builder for composable worker configurations
pub use app_builder::WorkerAppBuilder;
