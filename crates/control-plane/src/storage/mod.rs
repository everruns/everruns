// Storage layer for Everruns control-plane
// Decision: Support both PostgreSQL (production) and in-memory (dev mode)
//
// This crate provides database implementations for core traits:
// - DbAgentStore: implements AgentStore for agent retrieval
// - DbSessionStore: implements SessionStore for session retrieval
// - DbMessageStore: implements MessageStore for message persistence
// - DbSessionFileStore: implements SessionFileStore for session filesystem
// - DbLlmProviderStore: implements LlmProviderStore for LLM provider retrieval

pub mod agent_store;
pub mod backend;
pub mod encryption;
pub mod llm_provider_store;
pub mod memory;
pub mod message_store;
pub mod models;
pub mod password;
pub mod repositories;
pub mod session_file_store;
pub mod session_store;

#[cfg(test)]
mod event_tests;

pub use agent_store::{create_db_agent_store, DbAgentStore};
pub use backend::StorageBackend;
pub use encryption::{
    generate_encryption_key, EncryptedColumn, EncryptedPayload, EncryptionService,
    ENCRYPTED_COLUMNS,
};
pub use llm_provider_store::{create_db_llm_provider_store, DbLlmProviderStore};
pub use memory::InMemoryDatabase;
pub use message_store::{create_db_message_store, DbMessageStore};
pub use models::*;
pub use repositories::*;
pub use session_file_store::{create_db_session_file_store, DbSessionFileStore};
pub use session_store::{create_db_session_store, DbSessionStore};
