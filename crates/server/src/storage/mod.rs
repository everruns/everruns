// Storage layer for Everruns server (control plane)
// Decision: Support both PostgreSQL (production) and in-memory (dev mode)
//
// This crate provides database implementations for core traits:
// - DbAgentStore: implements AgentStore for agent retrieval
// - DbSessionStore: implements SessionStore for session retrieval
// - DbMessageRetriever: implements MessageRetriever for message loading
// - DbSessionFileStore: implements SessionFileSystem for session filesystem
// - DbSessionStorageStore: implements SessionStorageStore for key/value and secret storage
// - DbProviderStore: implements ProviderStore for LLM provider retrieval

pub mod agent_store;
pub mod backend;
pub mod blob_store;
pub mod compaction_checkpoint_store;
pub mod connection_resolver;
pub mod durable_tool_results;
pub mod encryption;
pub mod harness_store;
pub mod leased_resource_store;
pub mod memory;
mod message_history_timing;
pub mod message_store;
pub mod models;
pub mod partial_stream;
pub mod password;
pub mod provider_store;
pub mod reporting;
pub mod repositories;
pub mod repository;
pub mod sandbox_checkpoint_store;
pub mod session_file_store;
pub mod session_resource_store;
pub mod session_schedule_store;
pub mod session_storage_store;
pub mod session_store;
pub mod session_task_store;
pub mod subagent_spawn_handles;

#[cfg(test)]
mod event_tests;

pub use agent_store::{DbAgentStore, create_db_agent_store};
pub use backend::StorageBackend;
pub use compaction_checkpoint_store::DbCompactionCheckpointStore;
pub use connection_resolver::{DbConnectionResolver, GitHubAppTokenMinter, NoopConnectionResolver};
pub use durable_tool_results::PgDurableToolResultStore;
pub use encryption::{
    ENCRYPTED_COLUMNS, EncryptedColumn, EncryptedPayload, EncryptionService,
    generate_encryption_key,
};
pub use harness_store::{DbHarnessStore, create_db_harness_store};
pub use leased_resource_store::{
    DbLeasedResourceStore, row_to_domain as leased_resource_row_to_domain,
};
pub use memory::InMemoryDatabase;
pub use message_store::{DbMessageRetriever, create_db_message_retriever};
pub use models::*;
pub use partial_stream::PgPartialStreamStore;
pub use provider_store::{DbProviderStore, create_db_provider_store};
pub use repositories::*;
pub use repository::*;
pub use sandbox_checkpoint_store::PgSandboxCheckpointStore;
pub use session_file_store::{DbSessionFileStore, create_db_session_file_store};
pub use session_resource_store::DbSessionResourceRegistry;
pub use session_schedule_store::DbSessionScheduleStore;
pub use session_storage_store::{
    DbSessionStorageStore, create_db_session_storage_store,
    create_db_session_storage_store_without_encryption,
};
pub use session_store::{DbSessionStore, create_db_session_store};
pub use session_task_store::DbSessionTaskRegistry;
pub use subagent_spawn_handles::PgSubagentSpawnStore;
