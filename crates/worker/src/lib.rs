// Force-link integration crates so inventory::submit! registrations are included
extern crate everruns_container_sandbox;
extern crate everruns_integrations_brave_search;
extern crate everruns_integrations_browserless;
extern crate everruns_integrations_daytona;
extern crate everruns_integrations_deno;
extern crate everruns_integrations_docker;
extern crate everruns_integrations_duckduckgo;
extern crate everruns_integrations_e2b;
extern crate everruns_integrations_sprites;

pub mod activities;
pub mod adapters;
pub mod app_builder;
pub mod durable_runner;
pub mod durable_worker;
pub mod grpc_adapters;
pub mod grpc_durable_store;
pub mod grpc_worker_adapters;
pub mod leased_resource_cleanup;
pub mod mcp_executor;
pub mod platform;
pub mod runner;
pub mod runtime_host;
pub mod session_lifecycle;
pub mod unified_worker;
pub mod worker_adapters;

// Re-export main types
pub use durable_runner::{
    DirectDurableStore, DurableRunner, DurableStoreBackend, DurableTurnInput, DurableTurnOutput,
    InMemoryDurableStore,
};
pub use durable_worker::{DurableWorker, DurableWorkerConfig, ShutdownHandle};
pub use grpc_durable_store::{
    ClaimedTask as GrpcClaimedTask, GrpcDurableStore, HeartbeatResponse as GrpcHeartbeatResponse,
    WorkflowStatus as GrpcWorkflowStatus,
};
pub use runner::{AgentRunner, RunnerBackend, create_runner, create_runner_with_backend};

// Re-export LLM driver factory helpers
pub use adapters::{create_driver_registry, create_llm_driver};
pub use platform::{default_platform_definition, default_platform_definition_for_grade};

// Re-export gRPC adapters for worker communication with control plane
pub use grpc_adapters::{
    GrpcAgentStore, GrpcClient, GrpcEventEmitter, GrpcLlmProviderStore, GrpcMessageRetriever,
    GrpcSessionFileStore, GrpcSessionStore, TurnContext, load_turn_context,
};

// Re-export task worker types
pub use grpc_worker_adapters::GrpcWorkerAdapters;
pub use runtime_host::WorkerRuntimeHost;
pub use unified_worker::{TaskWorker, TaskWorkerConfig};
pub use worker_adapters::{
    AdapterAgentStore, AdapterEventEmitter, AdapterLlmProviderStore, AdapterMessageRetriever,
    AdapterSessionFileStore, AdapterSessionStore, TurnContext as WorkerTurnContext, WorkerAdapters,
};

// Re-export OpenAI driver from the openai crate
pub use everruns_openai::OpenAILlmDriver;

// Re-export app builder for composable worker configurations
pub use app_builder::WorkerAppBuilder;
