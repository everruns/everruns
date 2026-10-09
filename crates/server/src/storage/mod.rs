// Storage layer for Everruns server (control plane)
// Decision: Support both PostgreSQL (production) and in-memory (dev mode)
//
// Layout:
// - `repositories/`: PostgreSQL repositories (`impl Database`); an entity's
//   rows sit beside its repository in `repositories/<entity>/rows.rs` and are
//   re-exported here as `crate::storage::*`.
// - `models`: re-exports kept for saas only.
// - `runtime/`: adapters implementing the runtime's store and registry traits
//   (agents, harnesses, sessions, messages, providers, files, tasks, ...).

pub mod a2a_push_configs;
pub mod agent_avatars;
pub mod agent_trigger_deliveries;
pub mod agent_trigger_mcp_subscriptions;
pub mod backend;
pub mod blob_store;
pub mod command_idempotency;
pub mod connection_resolver;
pub mod encryption;
pub mod entity_changes;
mod health_issues;
mod ingress;
pub mod manager_context;
pub use health_issues::*;
pub mod late_generation_usage;
pub mod mcp_catalog;
pub mod mcp_event_subscriptions;
pub mod mcp_tool_cache;
mod message_history_timing;
pub mod models;
mod system_decisions;
// Server storage updates share durable's `UpdateField`: the server already
// depends on `everruns-durable` and passes these fields to its schedule store.
pub use everruns_durable::UpdateField;
pub mod agentid;
pub mod org_slack_connections;
pub mod pact_delegation;
pub mod password;
pub mod reporting;
pub mod repositories;
pub mod repository;
pub mod runtime;
pub mod runtime_identity;
mod session_turn_claim;
pub use session_turn_claim::*;
pub mod test_database;
pub mod transaction;

#[cfg(test)]
mod backend_tests;
#[cfg(test)]
mod event_tests;
#[cfg(test)]
mod sql_columns_tests;

pub use a2a_push_configs::*;
pub use agent_avatars::*;
pub use backend::StorageBackend;
pub use connection_resolver::{DbConnectionResolver, GitHubAppTokenMinter, NoopConnectionResolver};
pub use encryption::{
    ENCRYPTED_COLUMNS, EncryptedColumn, EncryptedPayload, EncryptionService,
    generate_encryption_key,
};
pub use ingress::{
    AgentChannelSummaryRow, CreateAgentChannelRow, IngressChannelRow, UpdateAgentChannelRow,
};
pub use late_generation_usage::*;
pub use mcp_catalog::*;
pub use mcp_event_subscriptions::*;
pub use mcp_tool_cache::*;
pub use org_slack_connections::*;
pub use repositories::*;
pub use repository::*;
pub use runtime::agent::{DbAgentStore, create_db_agent_store};
pub use runtime::agents_api::PgAgentsApiStore;
pub use runtime::compaction_checkpoint::DbCompactionCheckpointStore;
pub use runtime::durable_tool_results::PgDurableToolResultStore;
pub use runtime::harness::{DbHarnessStore, create_db_harness_store};
pub use runtime::leased_resource::{
    DbLeasedResourceStore, row_to_domain as leased_resource_row_to_domain,
};
pub use runtime::message::{DbMessageRetriever, create_db_message_retriever};
pub use runtime::native_async::PgNativeAsyncStore;
pub use runtime::partial_stream::PgPartialStreamStore;
pub use runtime::provider::{DbProviderStore, create_db_provider_store};
pub use runtime::sandbox_checkpoint::{PgSandboxCheckpointStore, PrimarySandboxRecord};
pub use runtime::session::{DbSessionStore, create_db_session_store};
pub use runtime::session_file::{DbSessionFileStore, create_db_session_file_store};
pub use runtime::session_resource::DbSessionResourceRegistry;
pub use runtime::session_schedule::DbSessionScheduleStore;
pub use runtime::session_storage::{
    DbSessionStorageStore, create_db_session_storage_store,
    create_db_session_storage_store_without_encryption,
};
pub use runtime::session_task::DbSessionTaskRegistry;
pub use runtime::subagent_spawn_handles::PgSubagentSpawnStore;
pub use system_decisions::SystemDecisions;
