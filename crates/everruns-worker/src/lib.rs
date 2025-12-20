pub mod activities;
pub mod adapters;
pub mod runner;
pub mod session_workflow;
pub mod temporal_activities;
pub mod temporal_client;
pub mod temporal_runner;
pub mod temporal_types;
pub mod unified_tool_executor;
pub mod worker;
pub mod workflow_registry;
pub mod workflow_traits;

// Re-export main types
pub use runner::{create_runner, AgentRunner, RunnerConfig};
pub use session_workflow::TemporalSessionWorkflow;
pub use temporal_runner::{run_temporal_worker, TemporalRunner};
pub use temporal_types::WorkflowAction;
pub use worker::TemporalWorker;
pub use workflow_registry::{WorkflowFactory, WorkflowRegistry, WorkflowRegistryBuilder};
pub use workflow_traits::{Workflow, WorkflowInput};

// Re-export adapters for core integration
pub use adapters::{
    create_db_agent_loop, create_db_agent_loop_with_registry, create_db_event_emitter,
    create_db_message_store, create_openai_provider, create_unified_tool_executor,
    create_unified_tool_executor_with_registry, DbEventEmitter, DbMessageStore,
};

// Re-export unified tool executor
pub use unified_tool_executor::UnifiedToolExecutor;

// Re-export OpenAI provider from the openai crate
pub use everruns_openai::OpenAiProvider;
