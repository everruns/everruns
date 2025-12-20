pub mod activities;
pub mod adapters;
pub mod runner;
pub mod unified_tool_executor;
pub mod worker;

// In-process execution (non-durable, tokio tasks)
pub mod inprocess;

// Temporal integration for durable workflow execution
pub mod temporal;

// Re-export main types
pub use runner::{create_runner, AgentRunner, RunnerConfig, RunnerMode};
pub use worker::TemporalWorker;

// Re-export in-process types
pub use inprocess::{InProcessRunner, InProcessWorkflow};

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
