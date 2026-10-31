#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Worker source reaches core and the durable queue through these re-exports
// (`crate::core`, `crate::durable`, ...), never by crate name; see
// `scripts/lib/check-durable-isolation.sh`.
pub use everruns_core as core;
pub use everruns_core::{engine, host, mcp};
pub use everruns_durable as durable;
pub mod activities;
pub mod actor_lease_store;
pub mod actor_runtime;
pub mod adapters;
pub mod app_builder;
mod catalog_cli;
pub mod grpc_actor_lease_store;
pub mod grpc_adapters;
mod grpc_balance;
pub mod grpc_command_transport;
pub mod grpc_durable_store;
pub mod grpc_files_adapter;
mod grpc_partial_stream;
mod grpc_sandbox_persistence;
pub mod grpc_saved_scripts;
pub mod grpc_slack_actions;
pub mod grpc_sqldb_adapter;
mod grpc_task_store;
#[cfg(test)]
mod grpc_test_env;
pub mod grpc_user_mcp;
pub mod grpc_worker_adapters;
pub mod internal_commands;
pub mod leased_resource_cleanup;
pub mod mcp_elicitation_consent;
pub mod mcp_executor;
mod model_wait_slots;
pub mod phase_reads;
pub mod platform;
mod reveal_storage;
pub mod runtime_host;
pub mod self_checks;
pub mod session_lifecycle;
pub mod session_task_reaper;
mod system_decisions;
pub mod task_driver;
pub mod task_store;
pub mod task_wakeup;
pub mod turn_reads;
pub mod unified_worker;
#[cfg(test)]
mod unified_worker_test_adapters;
#[cfg(test)]
mod unified_worker_wake_tests;
pub mod worker_adapters;
pub mod write_behind;

// Re-export main types
pub use grpc_durable_store::GrpcDurableStore;

// Re-export LLM driver factory helpers
pub use adapters::{create_chat_driver, create_driver_registry};
pub use platform::{default_host_composition, default_host_composition_for_grade};
pub use system_decisions::{
    SystemDecisions, UTILITY_DECISION_DRIVER_ENV, UTILITY_DECISION_MODEL_ENV,
};

// Re-export gRPC adapters for worker communication with control plane
pub use grpc_adapters::{
    GrpcAdapter, GrpcBudgetChecker, GrpcClient, GrpcOrgAdapter, TurnContext, load_turn_context,
};

// Re-export task worker types
pub use actor_lease_store::{
    ACTOR_LEASE_LOST, ActorFence, ActorJournal, ActorLeaseLost, ActorLeaseStore, ActorRelease,
    ActorRenewal, LeasedActor, actor_lease_lost, is_actor_lease_lost,
};
pub use grpc_actor_lease_store::GrpcActorLeaseStore;
pub use grpc_worker_adapters::GrpcWorkerAdapters;
pub use runtime_host::WorkerRuntimeHost;
pub use unified_worker::{ShutdownHandle, TaskWorker, TaskWorkerConfig};
pub use worker_adapters::{
    OrgAdapter, SessionAdapter, TurnContext as WorkerTurnContext, WorkerAdapters,
};

// Re-export OpenAI driver from the openai crate
pub use everruns_drivers::openai::OpenAIChatDriver;

// Re-export app builder for composable worker configurations
pub use app_builder::WorkerAppBuilder;
