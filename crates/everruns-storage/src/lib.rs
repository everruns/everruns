// Postgres storage layer with sqlx
//
// This crate provides database implementations for core traits:
// - DbMessageStore: implements MessageStore for message persistence

pub mod adapters;
pub mod encryption;
pub mod models;
pub mod password;
pub mod repositories;

pub use adapters::{create_db_message_store, DbMessageStore};
pub use encryption::{
    generate_encryption_key, EncryptedColumn, EncryptedPayload, EncryptionService,
    ENCRYPTED_COLUMNS,
};
pub use models::*;
pub use repositories::*;
