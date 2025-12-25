// Postgres storage layer with sqlx
//
// This crate provides database implementations for core traits:
// - DbMessageStore: implements MessageStore for message persistence
// - DbSessionFileStore: implements SessionFileStore for session filesystem

pub mod encryption;
pub mod message_store;
pub mod models;
pub mod password;
pub mod repositories;
pub mod session_file_store;

pub use encryption::{
    generate_encryption_key, EncryptedColumn, EncryptedPayload, EncryptionService,
    ENCRYPTED_COLUMNS,
};
pub use message_store::{create_db_message_store, DbMessageStore};
pub use models::*;
pub use repositories::*;
pub use session_file_store::{create_db_session_file_store, DbSessionFileStore};
