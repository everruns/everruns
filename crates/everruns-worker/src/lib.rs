pub mod activities;
pub mod adapters;
pub mod agent_workflow;
pub mod client;
pub mod runner;
pub mod types;
pub mod unified_tool_executor;
pub mod worker;
pub mod workflow_registry;
pub mod workflow_traits;

// Re-export main types
pub use agent_workflow::{AgentWorkflow, AgentWorkflowInput};
pub use runner::{create_runner, run_worker, AgentRunner, RunnerConfig, TemporalRunner};
pub use types::WorkflowAction;
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
