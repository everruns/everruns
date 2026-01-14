// Storage layer for Everruns control-plane
// Decision: Support both PostgreSQL (production) and in-memory (dev mode)
//
// This crate provides database implementations for core traits:
// - DbAgentStore: implements AgentStore for agent retrieval
// - DbSessionStore: implements SessionStore for session retrieval
// - DbMessageRetriever: implements MessageRetriever for message loading
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

pub use agent_store::{DbAgentStore, create_db_agent_store};
pub use backend::StorageBackend;
pub use encryption::{
    ENCRYPTED_COLUMNS, EncryptedColumn, EncryptedPayload, EncryptionService,
    generate_encryption_key,
};
pub use llm_provider_store::{DbLlmProviderStore, create_db_llm_provider_store};
pub use memory::InMemoryDatabase;
#[allow(deprecated)]
pub use message_store::{DbMessageRetriever, create_db_message_retriever, create_db_message_store};
pub use models::*;
pub use repositories::*;
pub use session_file_store::{DbSessionFileStore, create_db_session_file_store};
pub use session_store::{DbSessionStore, create_db_session_store};
