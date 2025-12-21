// Database-backed adapters for core traits
//
// These implementations are now in everruns-storage.
// This file re-exports them for backward compatibility.

pub use everruns_storage::{create_db_message_store, DbMessageStore};
