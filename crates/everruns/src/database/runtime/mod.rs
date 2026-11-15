#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Sandboxed SQLite operations shared by hosted and embedded sessions.
#![allow(missing_docs)]
pub mod authorizer;
pub mod error;
pub mod executor;
pub mod limits;
mod local;
pub mod types;
pub mod validation;
pub use local::LocalSqlDbStore;
